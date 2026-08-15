use std::net::SocketAddr;

use tracing_subscriber::EnvFilter;

mod browser_login;
mod config;
mod crypto;
mod db;
mod error;
mod middleware;
mod migration;
mod models;
mod opencode_account;
mod routes;
mod state;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = config::from_env();
    let db_path = cfg.data_dir.join("opencode2api.db");
    migration::run(&db_path).await?;
    let state = state::AppState::new(&db_path, cfg.web_dist.clone())?;

    let addr: SocketAddr = cfg.bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(
        "opencode2api listening on http://{addr} (db: {})",
        db_path.display()
    );

    let app = routes::build_router(state);
    axum::serve(listener, app).await?;
    Ok(())
}
