//! General-purpose stdio MCP adapter for a paired assistant/mcp grant.
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "fractal-mcp")]
struct Args {
    #[arg(long)]
    socket: PathBuf,
    #[arg(long)]
    credential_file: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    fractal_chatgpt::cli::run(
        &args.socket,
        &args.credential_file,
        fractal_chatgpt::Profile::Mcp,
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_stdio_only_and_paths_are_required() {
        let base = [
            "fractal-mcp",
            "--socket",
            "/tmp/fractal-client.sock",
            "--credential-file",
            "/tmp/credential",
        ];
        assert!(Args::try_parse_from(base).is_ok());
        for flag in ["--listen", "--transport", "--profile", "--principal"] {
            assert!(Args::try_parse_from(base.into_iter().chain([flag, "stdio"])).is_err());
        }
        assert!(Args::try_parse_from(["fractal-mcp"]).is_err());
        assert!(Args::try_parse_from(["fractal-mcp", "--socket", "/tmp/fractal-client.sock"]).is_err());
        assert!(Args::try_parse_from(["fractal-mcp", "--credential-file", "/tmp/credential"]).is_err());
    }
}
