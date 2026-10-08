//! RabbitMQ consumer: reads events published by the fetcher from [`QUEUE`] and stores them.

use crate::{Event, StoreError, store};
use futures_lite::StreamExt;
use lapin::options::{
    BasicAckOptions, BasicConsumeOptions, BasicNackOptions, BasicQosOptions, QueueDeclareOptions,
};
use lapin::types::{AMQPValue, FieldTable};
use lapin::{Channel, Connection, ConnectionProperties};
use sqlx::PgPool;
use std::time::Duration;

/// Durable queue the fetcher publishes to (default exchange, routing key = queue name).
pub const QUEUE: &str = "history.events";
/// Malformed messages are dead-lettered here instead of being retried forever.
pub const DEAD_QUEUE: &str = "history.events.dead";

const PREFETCH: u16 = 20;
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const DB_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Declare both queues. The fetcher calls the same thing, so arguments must stay identical.
pub async fn declare_queues(channel: &Channel) -> lapin::Result<()> {
    channel
        .queue_declare(
            DEAD_QUEUE.into(),
            QueueDeclareOptions::durable(),
            FieldTable::default(),
        )
        .await?;
    let mut args = FieldTable::default();
    args.insert(
        "x-dead-letter-exchange".into(),
        AMQPValue::LongString("".into()),
    );
    args.insert(
        "x-dead-letter-routing-key".into(),
        AMQPValue::LongString(DEAD_QUEUE.into()),
    );
    channel
        .queue_declare(QUEUE.into(), QueueDeclareOptions::durable(), args)
        .await?;
    Ok(())
}

/// Decode and validate a message body.
pub fn parse(body: &[u8]) -> Result<Event, String> {
    let ev: Event = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    ev.validate()?;
    Ok(ev)
}

/// Consume until `shutdown` resolves; reconnects to the broker whenever the connection drops.
pub async fn run(url: &str, pool: PgPool, shutdown: impl Future<Output = ()>) {
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => return,
            res = session(url, &pool) => {
                match res {
                    Ok(()) => eprintln!("rabbitmq: consumer stream ended, reconnecting"),
                    Err(e) => eprintln!("rabbitmq: {e}, reconnecting in {}s", RECONNECT_DELAY.as_secs()),
                }
            }
        }
        tokio::select! {
            _ = &mut shutdown => return,
            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
        }
    }
}

async fn session(url: &str, pool: &PgPool) -> lapin::Result<()> {
    let conn = Connection::connect(url, ConnectionProperties::default()).await?;
    let channel = conn.create_channel().await?;
    declare_queues(&channel).await?;
    channel
        .basic_qos(PREFETCH, BasicQosOptions::default())
        .await?;
    let mut consumer = channel
        .basic_consume(
            QUEUE.into(),
            "history".into(),
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;
    eprintln!("rabbitmq: consuming {QUEUE}");

    while let Some(delivery) = consumer.next().await {
        let delivery = delivery?;
        match parse(&delivery.data) {
            Err(msg) => {
                eprintln!("rabbitmq: dropping invalid event ({msg}) to {DEAD_QUEUE}");
                delivery.nack(BasicNackOptions::default()).await?;
            }
            Ok(ev) => loop {
                match store(pool, &ev).await {
                    Ok(()) => {
                        delivery.ack(BasicAckOptions::default()).await?;
                        break;
                    }
                    Err(StoreError::Invalid(msg)) => {
                        eprintln!("rabbitmq: dropping invalid event ({msg}) to {DEAD_QUEUE}");
                        delivery.nack(BasicNackOptions::default()).await?;
                        break;
                    }
                    // Keep the message unacked and try again; the broker holds the rest.
                    Err(StoreError::Db(e)) => {
                        eprintln!(
                            "insert failed: {e}, retrying in {}s",
                            DB_RETRY_DELAY.as_secs()
                        );
                        tokio::time::sleep(DB_RETRY_DELAY).await;
                    }
                }
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fetcher_payload() {
        let body = br#"{"tags":["amenity=cafe"],"area_type":"around","coords":[50.4501,30.5234,300.0],"kind":"node","status":200,"element_count":18,"duration_ms":4447,"user_agent":"curl/8"}"#;
        assert!(parse(body).is_ok());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"not json").is_err());
        assert!(
            parse(br#"{"tags":[],"area_type":"bbox","coords":[1],"status":200,"duration_ms":1}"#)
                .is_err()
        );
    }
}
