#![forbid(unsafe_code)]

use std::{path::PathBuf, sync::Arc};
use anyhow::{Context as _, Result, ensure};
use fractal_light_node_agent::{config::Config, performance::Meter, service::{self, Operation}};

const USAGE: &str = "Usage: fractal-light-node-agent --config <private-config.json>\n       fractal-light-node-agent service <install|start|stop|restart|health|uninstall> --config <absolute-private-config.json>\n\nGraph-free physical seat agent: authenticated Observe/Stop, durable custody,\nand GET /v1/performance on the private state-directory agent.sock.\nRemote Start is unsupported. Configuration must be owned and private.\nService operations require an interactive Darwin GUI login. Install does not start.\n";

#[cfg(unix)]
fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let Some(option) = args.next() else { anyhow::bail!(USAGE) };
    if option == "--help" || option == "-h" {
        ensure!(args.next().is_none(), "unexpected arguments after --help");
        print!("{USAGE}");
        return Ok(());
    }
    if option == "service" {
        let operation = Operation::parse(&args.next().context("service requires an operation")?)?;
        ensure!(args.next().is_some_and(|option| option == "--config"), "service requires --config");
        let path = PathBuf::from(args.next().context("--config requires an absolute private configuration file")?);
        ensure!(args.next().is_none(), "unexpected service arguments");
        let report = match service::run(operation, &path) {
            Ok(report) => report,
            Err(error) if operation == Operation::Health => {
                println!("{}", serde_json::json!({
                    "label": service::LABEL, "local_status": "unhealthy",
                    "graph_ready": false, "pid": null, "process_instance_id": null,
                    "reason": format!("{error:#}"),
                }));
                return Err(error);
            },
            Err(error) => return Err(error),
        };
        println!("{}", serde_json::to_string(&report)?);
        ensure!(operation != Operation::Health || report["local_status"] == "healthy"
            || report["local_status"] == "degraded", "light-node service is unhealthy");
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
