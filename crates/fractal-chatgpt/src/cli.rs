//! Shared transport runner; each binary owns its closed flag surface.
use crate::{Adapter, Profile, http_router};
use rmcp::ServiceExt;
use std::{net::SocketAddr, path::Path};
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

pub async fn run(
    socket: &Path,
    credential_file: &Path,
    profile: Profile,
    listen: Option<SocketAddr>,
) -> Result<(), Box<dyn std::error::Error>> {
    // stderr is essential for stdio framing; dependency logs stay disabled.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off"));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::io::stderr)
                .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                    metadata.target() == "st_chatgpt::call"
                }))
                .with_filter(filter),
        )
        .try_init()?;
    let adapter = Adapter::with_profile(socket, credential_file, profile)?;
    if let Some(listen) = listen {
        if !listen.ip().is_loopback() {
            return Err("the MCP listener must be loopback-only".into());
        }
        axum::serve(
            tokio::net::TcpListener::bind(listen).await?,
            http_router(adapter),
        )
        .await?;
    } else {
        adapter
            .serve(rmcp::transport::stdio())
            .await?
            .waiting()
            .await?;
    }
    Ok(())
}
