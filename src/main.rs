mod app;
mod compute;
mod db;
mod guard;
mod market;
mod middleware;
mod models;
mod providers;
mod render;
mod routes;
mod scheduler;
mod seed;
mod stream;
mod templates;

pub use app::{AppState, Config};

use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sqlx=warn")),
        )
        .init();

    // Subcommand dispatch; anything else falls through to the HTTP server.
    let mut args = std::env::args().skip(1);
    if let Some(cmd) = args.next() {
        match cmd.as_str() {
            "seed" => return run_seed().await,
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => {
                eprintln!("unknown subcommand: {other}");
                print_usage();
                std::process::exit(2);
            }
        }
    }

    serve().await
}

fn print_usage() {
    eprintln!(
        "finance: single-binary axum market-watching app\n\
         \n\
         Usage:\n  \
           finance         run the HTTP server\n  \
           finance seed    (re-)import the curated universe and its history\n"
    );
}

async fn run_seed() -> anyhow::Result<()> {
    let state = AppState::from_env().await?;
    let client = providers::http::build_client(&state.config);
    // Yahoo serves deep daily history (one `interval=1d&range=max` call per
    // symbol) as well as live quotes; it is the app's only price source.
    let history = providers::yahoo::YahooProvider::new(client);
    seed::run(&state.pool, &state.config, &history).await
}

async fn serve() -> anyhow::Result<()> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8000);

    let state = AppState::from_env().await?;
    scheduler::spawn(state.pool.clone(), state.config.clone(), state.hub.clone());
    let router = app::router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("finance listening on http://{addr}");
    axum::serve(listener, router).await?;
    Ok(())
}
