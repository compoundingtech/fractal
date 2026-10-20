//! MCP transports for the paired client gateway; no daemon-control access.
use clap::Parser;
use rmcp::ServiceExt;
use st_chatgpt::{Adapter, http_router};
use std::{net::SocketAddr, path::PathBuf};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    socket: PathBuf,
    #[arg(long)]
    credential_file: PathBuf,
    #[arg(long, default_value = "http", value_parser = ["http", "stdio"])]
    transport: String,
    #[arg(long, default_value = "127.0.0.1:8787")]
    listen: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    // Default off; stderr keeps stdio MCP framing untouched. Even a broad
    // RUST_LOG setting cannot enable HTTP/header/body logs from dependencies.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off"));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_ansi(false)
            .with_writer(std::io::stderr)
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| metadata.target() == "st_chatgpt::call"))
            .with_filter(filter))
        .try_init()?;
    if !args.listen.ip().is_loopback() {
        return Err("the MCP listener must be loopback-only".into());
    }
    let adapter = Adapter::new(&args.socket, &args.credential_file)?;
    if args.transport == "stdio" {
        adapter.serve(rmcp::transport::stdio()).await?.waiting().await?;
        return Ok(());
    }
    axum::serve(tokio::net::TcpListener::bind(args.listen).await?, http_router(adapter)).await?;
    Ok(())
}
