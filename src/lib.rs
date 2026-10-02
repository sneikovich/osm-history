//! Query log for the Overpass fetcher: `POST /events` stores one query, `GET /healthz` checks the DB.

pub mod ua;

use axum::Router;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use sqlx::PgPool;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

const MAX_TAGS: usize = 32;
const MAX_TAG_LEN: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AreaType {
    None,
    Around,
    Bbox,
}

impl AreaType {
    fn as_str(self) -> &'static str {
        match self {
            AreaType::None => "none",
            AreaType::Around => "around",
            AreaType::Bbox => "bbox",
        }
    }

    fn coords_len(self) -> Option<usize> {
        match self {
            AreaType::None => None,
            AreaType::Around => Some(3),
            AreaType::Bbox => Some(4),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Node,
    Way,
    Relation,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Node => "node",
            Kind::Way => "way",
            Kind::Relation => "relation",
        }
    }
}

/// One fetcher query as reported by `overpass-server`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub tags: Vec<String>,
    pub area_type: AreaType,
    pub coords: Option<Vec<f64>>,
    pub kind: Option<Kind>,
    /// HTTP status the fetcher answered with.
    pub status: u16,
    pub element_count: Option<u32>,
    pub duration_ms: u32,
    /// Parsed into OS/browser family; the raw string is discarded.
    pub user_agent: Option<String>,
}

impl Event {
    pub fn validate(&self) -> Result<(), String> {
        if self.tags.len() > MAX_TAGS {
            return Err(format!("at most {MAX_TAGS} tags"));
        }
        if self.tags.iter().any(|t| t.len() > MAX_TAG_LEN) {
            return Err(format!("tag longer than {MAX_TAG_LEN} bytes"));
        }
        let got = self.coords.as_ref().map(Vec::len);
        if got != self.area_type.coords_len() {
            return Err(format!(
                "area_type {} expects {} coords",
                self.area_type.as_str(),
                self.area_type.coords_len().unwrap_or(0)
            ));
        }
        if !(100..=599).contains(&self.status) {
            return Err("status must be an HTTP status code".into());
        }
        Ok(())
    }
}

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/events", post(record))
        .route("/healthz", get(healthz))
        .with_state(pool)
}

fn error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

async fn record(State(pool): State<PgPool>, body: Result<Json<Event>, JsonRejection>) -> Response {
    let ev = match body {
        Ok(Json(ev)) => ev,
        Err(e) => return error(e.status(), e.body_text()),
    };
    if let Err(msg) = ev.validate() {
        return error(StatusCode::UNPROCESSABLE_ENTITY, msg);
    }
    let (os, browser) = ev.user_agent.as_deref().map_or((None, None), ua::parse);

    let res = sqlx::query(
        "INSERT INTO queries \
         (tags, area_type, coords, kind, status, element_count, duration_ms, client_os, client_browser) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&ev.tags)
    .bind(ev.area_type.as_str())
    .bind(&ev.coords)
    .bind(ev.kind.map(Kind::as_str))
    .bind(ev.status as i16)
    .bind(ev.element_count.map(|n| n.min(i32::MAX as u32) as i32))
    .bind(ev.duration_ms.min(i32::MAX as u32) as i32)
    .bind(os)
    .bind(browser)
    .execute(&pool)
    .await;

    match res {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("insert failed: {e}");
            error(StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
        }
    }
}

async fn healthz(State(pool): State<PgPool>) -> Response {
    match sqlx::query("SELECT 1").execute(&pool).await {
        Ok(_) => "ok".into_response(),
        Err(e) => error(StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: serde_json::Value) -> Result<Event, String> {
        let ev: Event = serde_json::from_value(json).map_err(|e| e.to_string())?;
        ev.validate().map(|_| ev)
    }

    #[test]
    fn accepts_valid_events() {
        let base = serde_json::json!({
            "tags": ["amenity=cafe"], "area_type": "around", "coords": [50.45, 30.52, 300],
            "kind": "node", "status": 200, "element_count": 26, "duration_ms": 812,
            "user_agent": "curl/8"
        });
        assert!(event(base).is_ok());
        assert!(
            event(serde_json::json!({
                "tags": [], "area_type": "none", "coords": null, "kind": null,
                "status": 400, "element_count": null, "duration_ms": 0, "user_agent": null
            }))
            .is_ok()
        );
    }

    #[test]
    fn rejects_inconsistent_events() {
        let bad = [
            // coords don't match area_type
            serde_json::json!({"tags": [], "area_type": "bbox", "coords": [1, 2, 3], "status": 200, "duration_ms": 1}),
            serde_json::json!({"tags": [], "area_type": "none", "coords": [1, 2, 3], "status": 200, "duration_ms": 1}),
            serde_json::json!({"tags": [], "area_type": "around", "status": 200, "duration_ms": 1}),
            // unknown enum / field / status
            serde_json::json!({"tags": [], "area_type": "circle", "status": 200, "duration_ms": 1}),
            serde_json::json!({"tags": [], "area_type": "none", "kind": "area", "status": 200, "duration_ms": 1}),
            serde_json::json!({"tags": [], "area_type": "none", "status": 200, "duration_ms": 1, "ip": "1.2.3.4"}),
            serde_json::json!({"tags": [], "area_type": "none", "status": 42, "duration_ms": 1}),
        ];
        for json in bad {
            assert!(event(json.clone()).is_err(), "{json}");
        }
    }
}
