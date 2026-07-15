use std::io;

use agent_ferry_core::AgentFerryPaths;
use agent_ferry_daemon::Daemon;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let paths = AgentFerryPaths::discover().map_err(io::Error::other)?;
    let daemon = Daemon::bind(paths)?;
    daemon
        .serve_until(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}
