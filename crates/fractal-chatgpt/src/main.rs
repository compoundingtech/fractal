//! MCP transports for the paired client gateway; no daemon-control access.
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};

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
    // Keep the historical loopback admission even when --transport=stdio.
    if !args.listen.ip().is_loopback() {
        return Err("the MCP listener must be loopback-only".into());
    }
    fractal_chatgpt::cli::run(&args.socket, &args.credential_file, fractal_chatgpt::Profile::ChatGpt,
        (args.transport != "stdio").then_some(args.listen)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_flags_and_defaults_are_preserved() {
        let base = ["st-chatgpt", "--socket", "/tmp/st3-client.sock", "--credential-file", "/tmp/credential"];
        let args = Args::try_parse_from(base).unwrap();
        assert_eq!(args.transport, "http");
        assert_eq!(args.listen, "127.0.0.1:8787".parse::<SocketAddr>().unwrap());
        for transport in ["http", "stdio"] {
            let args = Args::try_parse_from(base.into_iter().chain(["--transport", transport, "--listen", "127.0.0.1:9000"])).unwrap();
            assert_eq!(args.transport, transport);
            assert_eq!(args.listen.port(), 9000);
        }
        assert!(Args::try_parse_from(base.into_iter().chain(["--transport", "other"])).is_err());
    }
}
