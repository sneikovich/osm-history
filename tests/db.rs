//! Needs a real PostgreSQL: `DATABASE_URL=postgres://... cargo test -- --ignored`

use serde_json::json;
use sqlx::{PgPool, Row};

async fn spawn(pool: PgPool) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, history::router(pool)).into_future());
    format!("http://{addr}")
}

async fn post(base: &str, body: serde_json::Value) -> u16 {
    reqwest::Client::new()
        .post(format!("{base}/events"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn stores_event_without_raw_user_agent() {
    let pool = PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    history::MIGRATOR.run(&pool).await.unwrap();
    let base = spawn(pool.clone()).await;

    let marker = format!("test={}", std::process::id());
    let status = post(
        &base,
        json!({
            "tags": [marker, "name"], "area_type": "bbox", "coords": [50.4, 30.4, 50.5, 30.6],
            "kind": "way", "status": 200, "element_count": 7, "duration_ms": 1234,
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64; rv:143.0) Gecko/20100101 Firefox/143.0"
        }),
    )
    .await;
    assert_eq!(status, 204);

    let row = sqlx::query(
        "SELECT area_type, coords, kind, status, element_count, duration_ms, client_os, client_browser \
         FROM queries WHERE $1 = ANY(tags)",
    )
    .bind(&marker)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("area_type"), "bbox");
    assert_eq!(
        row.get::<Vec<f64>, _>("coords"),
        vec![50.4, 30.4, 50.5, 30.6]
    );
    assert_eq!(row.get::<String, _>("kind"), "way");
    assert_eq!(row.get::<i16, _>("status"), 200);
    assert_eq!(row.get::<i32, _>("element_count"), 7);
    assert_eq!(row.get::<i32, _>("duration_ms"), 1234);
    assert_eq!(row.get::<String, _>("client_os"), "Linux");
    assert_eq!(row.get::<String, _>("client_browser"), "Firefox");

    assert_eq!(
        post(
            &base,
            json!({"tags": [], "area_type": "bbox", "status": 200, "duration_ms": 1})
        )
        .await,
        422
    );
    assert_eq!(post(&base, json!({"nope": 1})).await, 422);

    let health = reqwest::get(format!("{base}/healthz")).await.unwrap();
    assert_eq!(health.status(), 200);
}
