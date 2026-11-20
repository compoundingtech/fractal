//! Shell executions use the daemon's finite membership owner, never a local semaphore.
use super::*;
use fractal_daemon::cli_println as println;
use fractal_daemon::store::resource_membership::{Coverage, DiskRequest, IoClass, LaneRequest, Owner, RequestClass};
use fractal_daemon::store::shell_resources::{self, Execution, PayloadResult};

#[derive(Subcommand)]
pub(super) enum ResourcesCommand {
    /// Wait for host admission, execute argv, and await confirmed retirement.
    Exec(ExecArgs),
    /// List shell executions and their host custody.
    Ls {
        #[arg(long)] host: Option<String>,
        #[arg(long, value_enum)] class: Option<Class>,
        #[arg(long)] head: Option<String>,
    },
    /// Inspect an execution, including admission blockers and the actual outcome.
    Show { id: String },
    /// Close native host admission without reserving capacity.
    Hold {
        host: String,
        #[arg(long)] reason: String,
        #[arg(long = "operation", required = true)] operations: Vec<String>,
        #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)] actor: Option<String>,
        #[arg(long)] originating_run: Option<String>,
        #[arg(long)] receipt: Option<String>,
    },
    /// Resolve a native host hold at its observed revision.
    Release {
        host: String, id: String,
        #[arg(long)] expected_revision: u64,
        #[arg(long)] resolution: String,
        #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)] actor: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum Class { Landing, Check, Agent }
impl From<Class> for RequestClass {
    fn from(value: Class) -> Self { match value { Class::Landing => Self::Landing, Class::Check => Self::Check, Class::Agent => Self::Agent } }
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum CliCoverage { Logical, Enforced }
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum CliIo { Heavy }

#[derive(Args)]
pub(super) struct ExecArgs {
    #[arg(long)] lane: String,
    /// Defaults to the inherited membership class, or agent outside a resource execution.
    #[arg(long, value_enum)] class: Option<Class>,
    #[arg(long)] cores: u32,
    #[arg(long)] memory: String,
    #[arg(long, requires = "disk_path")] disk: Option<String>,
    #[arg(long, requires = "disk")] disk_path: Option<PathBuf>,
    #[arg(long, value_enum)] io: Option<CliIo>,
    #[arg(long, value_parser = parse_stalled_after)] admission_timeout: Option<u64>,
    #[arg(long, value_parser = parse_stalled_after)] timeout: Option<u64>,
    #[arg(long, value_enum, default_value = default_coverage())] coverage: CliCoverage,
    #[arg(last = true, required = true)] argv: Vec<String>,
}

fn default_coverage() -> &'static str {
    if cfg!(target_os = "linux") { "enforced" } else { "logical" }
}

fn require_exec_platform() -> Result<()> {
    if cfg!(target_os = "macos") {
        return Err(fractal_daemon::FractalError::new("unsupported-platform",
            "resource exec is unavailable on macOS: no whole-tree retirement proof").into());
    }
    Ok(())
}

pub(super) async fn run(client: &Client, person: Option<&str>, command: ResourcesCommand, json_output: bool) -> Result<()> {
    match command {
        ResourcesCommand::Exec(args) => execute(client, args).await,
        ResourcesCommand::Ls { host, class, head } => {
            let mut query = Vec::new();
            if let Some(host) = host { query.push(format!("host={}", urlencoding::encode(&host))); }
            if let Some(class) = class { query.push(format!("class={}", match class { Class::Landing => "landing", Class::Check => "check", Class::Agent => "agent" })); }
            if let Some(head) = head { query.push(format!("head={}", urlencoding::encode(&head))); }
            let inventory: Value = client.get(&format!("/v1/internal/resources?{}", query.join("&"))).await?;
            if json_output { return print_value(&inventory, true); }
            if let Some(memberships) = inventory["memberships"].as_array() {
                for member in memberships {
                    println!("{}  {}  {}  class={}  owner={}  blockedBy={}  {}",
                        member["request"]["id"].as_str().unwrap_or_default(), member["request"]["name"].as_str().unwrap_or_default(),
                        member["state"].as_str().unwrap_or_default(), member["request"]["class"].as_str().unwrap_or_default(),
                        member["request"]["owner"]["run"].as_str().unwrap_or_default(),
                        member["blockedBy"].as_str().unwrap_or("none"), member["reason"].as_str().unwrap_or_default());
                }
            }
            if let Some(holds) = inventory["holds"].as_array() {
                for hold in holds { println!("hold {}  owner={}  {}", hold["id"].as_str().unwrap_or_default(),
                    hold["owner"].as_str().unwrap_or_default(), hold["reason"].as_str().unwrap_or_default()); }
            }
            if let Some(executions) = inventory["executions"].as_array() {
                for execution in executions { println!("exec {}  {}  host={}  exit={}", execution["command"]["id"].as_str().unwrap_or_default(),
                    execution["state"].as_str().unwrap_or_default(), execution["command"]["lane"]["host"].as_str().unwrap_or_default(), execution["exitCode"]); }
            }
            print_value(&inventory["accountant"], false)?;
            Ok(())
        }
        ResourcesCommand::Show { id } => {
            let row: Value = client.get(&format!("/v1/internal/resources/{}", urlencoding::encode(&id))).await?;
            print_value(&row, json_output)
        }
        ResourcesCommand::Hold { host, reason, operations, actor, originating_run, receipt } => hosts_cli::run(client, person,
            hosts_cli::HostsCommand::Hold { host, reason, operations, actor, originating_run, receipt }, json_output).await,
        ResourcesCommand::Release { host, id, expected_revision, resolution, actor } => hosts_cli::run(client, person,
            hosts_cli::HostsCommand::Release { host, id, expected_revision, resolution, actor }, json_output).await,
    }
}

fn require_transient_environment_daemon(capabilities: &Value) -> Result<()> {
    anyhow::ensure!(capabilities.pointer("/capabilities/shell_resource_exec").and_then(Value::as_u64).is_some_and(|version| version >= 2)
        && capabilities.pointer("/capabilities/mission_resource_lane").and_then(Value::as_u64).is_some_and(|version| version >= 2),
        "refused: daemon requires shell_resource_exec>=2 (transient environment) and mission_resource_lane>=2; upgrade the execution host before sending payload environment");
    Ok(())
}

async fn execute(client: &Client, args: ExecArgs) -> Result<()> {
    require_exec_platform()?;
    let capabilities: Value = client.get("/v1/health").await?;
    require_transient_environment_daemon(&capabilities)?;
    let cwd = std::env::current_dir()?;
    let parent = std::env::var(shell_resources::PARENT_ENV).ok().map(|value| serde_json::from_str::<shell_resources::ParentContext>(&value)).transpose()
        .context("invalid nested resource membership marker")?;
    let class = args.class.map(RequestClass::from).unwrap_or_else(|| parent.as_ref().map_or(RequestClass::Agent, |parent| parent.request.class));
    let id = uuid::Uuid::now_v7().to_string();
    let capacity = fractal_runtime::Capacity::declared(Some(args.cores), Some(&args.memory))?;
    let disk = args.disk.map(|bytes| -> Result<DiskRequest> {
        let path = args.disk_path.context("--disk requires --disk-path")?;
        let path = fs::canonicalize(if path.is_absolute() { path } else { cwd.join(path) })?;
        Ok(DiskRequest { bytes: fractal_runtime::parse_lane_memory(&bytes)?, path: path.to_string_lossy().into_owned() })
    }).transpose()?;
    let lane = LaneRequest { id: format!("shell:{id}"), name: args.lane, host: String::new(), max_members: 1,
        capacity: Some(capacity), class, disk, io: args.io.map(|_| IoClass::Heavy),
        coverage: match args.coverage { CliCoverage::Logical => Coverage::Logical, CliCoverage::Enforced => Coverage::Enforced },
        admission_timeout_ms: args.admission_timeout,
        owner: Owner { run: format!("shell:{id}"), step: format!("shell:{id}"), attempt: 1, invocation: Some(format!("shell:{id}")) } };
    let command = shell_resources::Command { id: id.clone(), lane, argv: args.argv, cwd: cwd.to_string_lossy().into_owned(),
        environment: std::env::vars().collect(), timeout_ms: args.timeout, parent: parent.map(|parent| parent.grant) };
    let mut execution: Execution = client.post("/v1/internal/resources/exec", &command).await?;
    let mut announced_admission = false;
    loop {
        if execution.started_at.is_some() && !announced_admission {
            eprintln!("resources: admitted {id} blockedBy=none");
            announced_admission = true;
        }
        eprintln!("resources: {} {} blockedBy={} {}", execution.state, id,
            execution.blocked_by.map_or("none", |blocked| blocked.as_str()), execution.reason);
        match execution.state.as_str() {
            "completed" => {
                cli_output::stdout().write_all(execution.stdout.as_bytes())?;
                std::io::stderr().write_all(execution.stderr.as_bytes())?;
                let code = if execution.timed_out { 124 }
                    else { execution.exit_signal.map_or_else(|| execution.exit_code.unwrap_or(1), |signal| 128 + signal) };
                return if code == 0 { Ok(()) } else { Err(CommandExit(code.clamp(1,255) as u8).into()) };
            }
            "refused" | "cancelled" => anyhow::bail!("refused: {} blockedBy={}", execution.reason,
                execution.blocked_by.map_or("none", |blocked| blocked.as_str())),
            _ => {}
        }
        let path = format!("/v1/internal/resources/{}/wait?after={}", id, execution.revision);
        let wait_body = json!({});
        tokio::select! {
            result = client.post(&path, &wait_body) => execution = result?,
            signal = tokio::signal::ctrl_c() => {
                signal?;
                let _: Value = client.post(&format!("/v1/internal/resources/{id}/cancel"), &json!({})).await?;
            }
        }
    }
}

/// Runs inside the admitted scope; SIGTERM stops the payload while this observer remains
/// alive to reap it and publish final observations before whole-tree retirement.
pub(super) async fn payload(result_path: &Path, argv: Vec<String>) -> Result<()> {
    use std::os::unix::process::ExitStatusExt as _;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    anyhow::ensure!(!argv.is_empty(), "resource payload requires argv");
    let git = || -> Result<(Option<String>,Option<bool>)> {
        let head = match std::process::Command::new("git").args(["rev-parse", "HEAD"]).output() {
            Ok(head) if head.status.success() => Some(String::from_utf8(head.stdout)?.trim().to_owned()),
            _ => None,
        };
        let clean = std::process::Command::new("git").args(["status", "--porcelain", "--untracked-files=normal"]).output()
            .ok().filter(|output| output.status.success()).map(|output| output.stdout.is_empty());
        Ok((head, clean))
    };
    let (head_start, clean_start) = git()?;
    let start_path = result_path.with_extension("started");
    let pending = result_path.with_extension("starting");
    fs::write(&pending, serde_json::to_vec(&(&head_start, clean_start))?)?;
    fs::rename(pending, start_path)?;
    let started = Instant::now();
    let mut child = tokio::process::Command::new(&argv[0]).args(&argv[1..]).spawn()?;
    let status = tokio::select! {
        status = child.wait() => status?,
        _ = terminate.recv() => {
            // Descendants retain charged custody until the authority retires the tree.
            child.start_kill()?;
            child.wait().await?
        }
    };
    let wall_duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (head_end, clean_end) = git()?;
    let enforced = std::env::var(shell_resources::PARENT_ENV).ok()
        .map(|value| serde_json::from_str::<shell_resources::ParentContext>(&value)).transpose()?
        .is_some_and(|parent| parent.request.coverage == Coverage::Enforced);
    let memory_peak_bytes = if cfg!(target_os = "linux") && enforced {
        fs::read_to_string("/proc/self/cgroup").ok().and_then(|groups| groups.lines().find_map(|line| line.strip_prefix("0::")).map(str::to_owned))
            .and_then(|group| fs::read_to_string(Path::new("/sys/fs/cgroup").join(group.trim_start_matches('/')).join("memory.peak")).ok())
            .and_then(|peak| peak.trim().parse().ok())
    } else { None };
    let result = PayloadResult { head_start, head_end, clean_start, clean_end, exit_code: status.code(), exit_signal: status.signal(), wall_duration_ms, memory_peak_bytes };
    let temporary = result_path.with_extension("pending");
    let mut file = fs::File::create(&temporary)?;
    std::io::Write::write_all(&mut file, &serde_json::to_vec(&result)?)?;
    file.sync_all()?;
    fs::rename(temporary, result_path)?;
    fs::File::open(result_path.parent().context("payload result has no parent")?)?.sync_all()?;
    let code = status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    if code == 0 { Ok(()) } else { Err(CommandExit(code.clamp(1,255) as u8).into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exec_requires_resources_and_preserves_payload_argv() {
        let cli = Cli::try_parse_from(["fractal","resources","exec","--lane","checks","--class","check","--cores","4","--memory","24G","--disk","15G","--disk-path",".","--io","heavy","--timeout","5m","--","sh","-c","exit 37"]).unwrap();
        let Command::Resources { command: ResourcesCommand::Exec(args) } = cli.command else { panic!("wrong command") };
        assert_eq!(args.argv, ["sh","-c","exit 37"]);
        assert_eq!(args.timeout, Some(300_000));
        assert_eq!(matches!(args.coverage, CliCoverage::Enforced), cfg!(target_os = "linux"));
        assert!(Cli::try_parse_from(["fractal","resources","exec","--lane","x","--cores","1","--memory","1G","--disk","1G","--","true"]).is_err());
    }
    #[test]
    fn platform_refusal_precedes_daemon_contact_and_capacity_acquisition() {
        let result = require_exec_platform();
        if cfg!(target_os = "macos") {
            let error = result.unwrap_err();
            assert_eq!(error.downcast_ref::<fractal_daemon::FractalError>().unwrap().code, "unsupported-platform");
        } else {
            result.unwrap();
        }
    }
    #[test]
    fn v1_daemon_is_rejected_before_environment_transport() {
        let old = serde_json::json!({"capabilities":{"shell_resource_exec":1,"mission_resource_lane":2}});
        let error = require_transient_environment_daemon(&old).unwrap_err();
        assert!(error.to_string().contains("shell_resource_exec>=2"));
        let current = serde_json::json!({"capabilities":{"shell_resource_exec":2,"mission_resource_lane":2}});
        require_transient_environment_daemon(&current).unwrap();
        assert!(require_transient_environment_daemon(&serde_json::json!({})).is_err());
    }

    #[tokio::test]
    async fn payload_exit_code_is_passed_through_and_result_is_durable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("outcome.json");
        let error = payload(&path, vec!["/bin/sh".into(), "-c".into(), "exit 37".into()]).await.unwrap_err();
        assert_eq!(error.downcast_ref::<CommandExit>().unwrap().0, 37);
        let result: PayloadResult = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(result.exit_code, Some(37));
        assert_eq!(result.exit_signal, None);
    }
}
