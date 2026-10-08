use clap::Parser;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

/// Store Overpass fetcher queries in PostgreSQL.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[arg(long, env = "HISTORY_LISTEN", default_value = "0.0.0.0:8081")]
    listen: SocketAddr,

    /// postgres://user:password@host/db
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,

    /// How long to keep retrying the initial DB connection, seconds
    #[arg(long, env = "HISTORY_CONNECT_WAIT", default_value_t = 30)]
    connect_wait: u64,

    /// amqp://user:password@host/vhost; unset disables the RabbitMQ consumer
    #[arg(long, env = "RABBITMQ_URL", hide_env_values = true)]
    rabbitmq_url: Option<String>,
}

/// Postgres is often still starting when the stack comes up; keep trying for a while.
async fn connect(url: &str, wait: Duration) -> Result<PgPool, sqlx::Error> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(3))
            .connect(url)
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(e) if tokio::time::Instant::now() < deadline => {
                eprintln!("database not ready ({e}), retrying in 1s");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let pool = match connect(&cli.database_url, Duration::from_secs(cli.connect_wait)).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: database: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Takes an advisory lock, so concurrent replicas don't race.
    if let Err(e) = history::MIGRATOR.run(&pool).await {
        eprintln!("error: migrations: {e}");
        return ExitCode::FAILURE;
    }

    let listener = match tokio::net::TcpListener::bind(cli.listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: bind {}: {e}", cli.listen);
            return ExitCode::FAILURE;
        }
    };
    eprintln!("listening on {}", cli.listen);

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let consumer = cli.rabbitmq_url.map(|url| {
        let pool = pool.clone();
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            let stopped = async move {
                let _ = stop.wait_for(|s| *s).await;
            };
            history::consumer::run(&url, pool, stopped).await;
        })
    });

    let served = axum::serve(listener, history::router(pool))
        .with_graceful_shutdown(shutdown_signal())
        .await;
    let _ = stop_tx.send(true);
    if let Some(task) = consumer {
        let _ = task.await;
    }
    if let Err(e) = served {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
