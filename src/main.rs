mod calendar;
mod error;
mod ical_bridge;
mod server;

use anyhow::Result;
use rmcp::ServiceExt;

#[tokio::main]
async fn main() -> Result<()> {
    // Log to stderr — stdout is the MCP stdio transport
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("Starting Tempo MCP server v{}", env!("CARGO_PKG_VERSION"));

    let server = server::TempoServer::new();
    let router = server.into_router();
    let service = router.serve(rmcp::transport::io::stdio()).await?;

    tracing::info!("Tempo is ready");
    service.waiting().await?;

    Ok(())
}
