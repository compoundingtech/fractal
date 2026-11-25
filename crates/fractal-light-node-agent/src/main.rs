#![forbid(unsafe_code)]

use std::{path::PathBuf, sync::Arc};
use anyhow::{Context as _, Result, ensure};
use fractal_light_node_agent::{config::Config, performance::Meter};

const USAGE: &str = "Usage: fractal-light-node-agent --config <private-config.json>\n\nGraph-free physical seat agent: authenticated Observe/Stop, durable custody,\nand GET /v1/performance on the private state-directory agent.sock.\nRemote Start is unsupported. Configuration must be owned and private.\n";

#[cfg(unix)]
fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let Some(option) = args.next() else { anyhow::bail!(USAGE) };
    if option == "--help" || option == "-h" {
        ensure!(args.next().is_none(), "unexpected arguments after --help");
        print!("{USAGE}");
        return Ok(());
    }
    ensure!(option == "--config", "{USAGE}");
    let path = PathBuf::from(args.next().context("--config requires a private configuration file")?);
    ensure!(args.next().is_none(), "unexpected arguments after --config path");
    let meter = Arc::new(Meter::new().context("initialize own-process CPU accounting")?);
    let config = Config::read(&path)?;
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    runtime.block_on(fractal_light_node_agent::agent::run(config, meter))
}

#[cfg(not(unix))]
fn main() -> Result<()> {
    anyhow::bail!("the physical light-node agent requires a supported Unix native runtime")
}
