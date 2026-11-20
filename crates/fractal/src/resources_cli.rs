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
        #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)] actor: Option<String>,
    },
    /// Inspect an execution, including admission blockers and the actual outcome.
    Show {
        #[arg(required_unless_present = "request_id", conflicts_with = "request_id")] id: Option<String>,
        #[arg(long, conflicts_with = "id")] request_id: Option<uuid::Uuid>,
        #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)] actor: Option<String>,
    },
    /// Cancel an execution and await confirmed retirement or confirmed nonlaunch.
    Cancel {
        #[arg(required_unless_present = "request_id", conflicts_with = "request_id")] id: Option<String>,
        #[arg(long, conflicts_with = "id")] request_id: Option<uuid::Uuid>,
        /// Wait for confirmed retirement or a durable cancellation tombstone.
        #[arg(long)] wait: bool,
        #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)] actor: Option<String>,
    },
    /// Inspect the exact live, enforced member cgroup for an authenticated execution.
    MemberScope {
        #[arg(long)] exec_id: String,
        #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)] actor: Option<String>,
    },
    /// Inspect local kernel member placement offline; this grants no execution authority.
    InMember { #[arg(long)] owner_uid: u32 },
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
pub(super) enum Class { Recovery, Landing, Check, Agent }
impl From<Class> for RequestClass {
    fn from(value: Class) -> Self { match value { Class::Recovery => Self::Recovery, Class::Landing => Self::Landing, Class::Check => Self::Check, Class::Agent => Self::Agent } }
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum CliCoverage { Logical, Enforced }
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum CliIo { Heavy }

#[derive(Args)]
pub(super) struct ExecArgs {
    #[arg(long, required_unless_present = "inherit_parent")] lane: Option<String>,
    /// Host-local workload key; estimates are resolved only by the execution host.
    #[arg(long)] profile: Option<String>,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))] max_members: u32,
    /// Atomically publish the terminal execution outcome after confirmed retirement.
    #[arg(long)] receipt_file: Option<PathBuf>,
    /// Owner-scoped submission identity for recovering an uncertain transport outcome.
    #[arg(long)] request_id: Option<uuid::Uuid>,
    #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)] actor: Option<String>,
    /// Refuse instead of creating a root when no inherited membership marker exists.
    #[arg(long)] require_parent: bool,
    /// Execute under the authenticated parent's frozen lane; implies --require-parent.
    #[arg(long)] inherit_parent: bool,
    /// Defaults to the inherited membership class, or agent outside a resource execution.
    #[arg(long, value_enum)] class: Option<Class>,
    #[arg(long, requires = "memory")] cores: Option<u32>,
    #[arg(long, requires = "cores")] memory: Option<String>,
    #[arg(long, requires = "disk_path")] disk: Option<String>,
    #[arg(long)] disk_path: Option<PathBuf>,
    #[arg(long, value_enum)] io: Option<CliIo>,
    #[arg(long, value_parser = parse_stalled_after)] admission_timeout: Option<u64>,
    #[arg(long, value_parser = parse_stalled_after)] timeout: Option<u64>,
    #[arg(long, value_enum, default_value = default_coverage())] coverage: CliCoverage,
    #[arg(last = true, required = true)] argv: Vec<String>,
}

pub(super) fn in_member(owner_uid: u32, json_output: bool) -> Result<()> {
    let member = fractal_runtime::in_member(owner_uid)?;
    if json_output {
        return print_value(&json!({
            "inside": member.is_some(),
            "laneSlice": member.as_ref().map(|member| member.lane_slice.as_str()),
            "binding": member.as_ref().map(|member| member.binding_hash.as_str()),
        }), true);
    }
    match member {
        Some(member) => println!("inside lane={} binding={}", member.lane_slice, member.binding_hash),
        None => println!("outside"),
    }
    Ok(())
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

impl ResourcesCommand {
    pub(super) fn actor(&self) -> Option<&str> {
        match self {
            Self::Exec(args) => args.actor.as_deref(),
            Self::Ls { actor, .. } | Self::Show { actor, .. } | Self::Cancel { actor, .. }
                | Self::MemberScope { actor, .. } | Self::Hold { actor, .. } | Self::Release { actor, .. } => actor.as_deref(),
            Self::InMember { .. } => None,
        }
    }
}

pub(super) async fn run(client: &Client, person: Option<&str>, command: ResourcesCommand, json_output: bool) -> Result<()> {
    match command {
        ResourcesCommand::Exec(args) => execute(client, args).await,
        ResourcesCommand::Ls { host, class, head, .. } => {
            let mut query = Vec::new();
            if let Some(host) = host { query.push(format!("host={}", urlencoding::encode(&host))); }
            if let Some(class) = class { query.push(format!("class={}", match class { Class::Recovery => "recovery", Class::Landing => "landing", Class::Check => "check", Class::Agent => "agent" })); }
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
        ResourcesCommand::Show { id, request_id, .. } => {
            let path = match (id, request_id) {
                (Some(id), None) => format!("/v1/internal/resources/{}", urlencoding::encode(&id)),
                (None, Some(request_id)) => format!("/v1/internal/resources/by-request/{request_id}"),
                _ => unreachable!("show requires one execution ID or request ID"),
            };
            let row: Value = client.get(&path).await?;
            print_value(&row, json_output)
        }
        ResourcesCommand::Cancel { id, request_id, wait, .. } => {
            if let Some(request_id) = request_id {
                let mut response: shell_resources::RequestCancellation = client.post(
                    &format!("/v1/internal/resources/by-request/{request_id}/cancel"), &json!({})).await?;
                if wait && let Some(execution) = &mut response.execution {
                    while !matches!(execution.state.as_str(), "completed" | "refused" | "cancelled") {
                        *execution = client.post(&format!("/v1/internal/resources/{}/wait?after={}", execution.command.id, execution.revision), &json!({})).await?;
                    }
                }
                return print_value(&serde_json::to_value(response)?, json_output);
            }
            let id = id.context("cancel requires an execution ID or request ID")?;
            let path = format!("/v1/internal/resources/{}", urlencoding::encode(&id));
            let _: Value = client.post(&format!("{path}/cancel"), &json!({})).await?;
            let mut row: Value = client.get(&path).await?;
            let mut execution: Execution = serde_json::from_value(row.get_mut("execution").context("missing execution")?.take())?;
            while !matches!(execution.state.as_str(), "completed" | "refused" | "cancelled") {
                execution = client.post(&format!("{path}/wait?after={}", execution.revision), &json!({})).await?;
            }
            print_value(&serde_json::to_value(execution)?, json_output)
        }
        ResourcesCommand::InMember { owner_uid } => in_member(owner_uid, json_output),
        ResourcesCommand::MemberScope { exec_id, .. } => {
            let scope: Value = client.get(&format!("/v1/internal/resources/{}/member-scope", urlencoding::encode(&exec_id))).await?;
            print_value(&scope, json_output)
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
    let parent = std::env::var(shell_resources::PARENT_ENV).ok().map(|value| serde_json::from_str::<shell_resources::ParentContext>(&value)).transpose()
        .context("invalid nested resource membership marker")?;
    validate_receipt_scope(&args, parent.is_some())?;
    anyhow::ensure!(!(args.require_parent || args.inherit_parent) || parent.is_some(),
        "refused: nested-only execution requires an inherited resource membership marker; FRACTAL_RESOURCE_EXEC_ID is not authority");
    if args.inherit_parent {
        let inherited = &parent.as_ref().context("missing inherited resource membership")?.request;
        anyhow::ensure!(args.lane.as_ref().is_none_or(|lane| *lane == inherited.name)
            && args.profile.as_ref().is_none_or(|profile| Some(profile) == inherited.profile.as_ref())
            && args.class.map(RequestClass::from).is_none_or(|class| class == inherited.class),
            "refused: --inherit-parent cannot change the parent lane, profile or class");
    }
    if args.profile.is_some() || parent.as_ref().is_some_and(|parent| parent.request.profile.is_some())
        || args.receipt_file.is_some() || args.request_id.is_some() || args.max_members != 1 || args.inherit_parent || args.require_parent {
        anyhow::ensure!(capabilities.pointer("/capabilities/resource_profile_keys").and_then(Value::as_u64).is_some_and(|version| version >= 1),
            "refused: daemon requires resource_profile_keys>=1; upgrade the execution host before requesting keyed resources or receipt files");
    }
    let cwd = std::env::current_dir()?;
    let class = args.class.map(RequestClass::from).unwrap_or_else(|| parent.as_ref().map_or(RequestClass::Agent, |parent| parent.request.class.clone()));
    let id = uuid::Uuid::now_v7().to_string();
    let capacity = match (args.cores, args.memory.as_deref()) {
        (None, None) => None,
        (cores, memory) => Some(fractal_runtime::Capacity::declared(cores, memory)?),
    };
    anyhow::ensure!(args.profile.is_some() || parent.as_ref().is_some_and(|parent| parent.request.profile.is_some())
        || args.disk.is_some() || args.disk_path.is_none(),
        "--disk-path requires --disk or --profile");
    let disk_path = args.disk_path.map(|path|
        fs::canonicalize(if path.is_absolute() { path } else { cwd.join(path) })
            .map(|path| path.to_string_lossy().into_owned())).transpose()?;
    let disk = args.disk.map(|bytes| -> Result<DiskRequest> {
        Ok(DiskRequest { bytes: fractal_runtime::parse_lane_memory(&bytes)?,
            path: disk_path.clone().context("--disk requires --disk-path")? })
    }).transpose()?;
    let profile = args.profile.or_else(|| parent.as_ref().and_then(|parent| parent.request.profile.clone()));
    let keyed_disk_path = if profile.is_some() && parent.is_none() && disk.is_none() {
        Some(disk_path.unwrap_or_else(|| cwd.to_string_lossy().into_owned()))
    } else if profile.is_some() && disk.is_none() { disk_path } else { None };
    let mut lane = LaneRequest { id: format!("shell:{id}"), name: args.lane.unwrap_or_default(), host: String::new(), max_members: args.max_members,
        profile, profile_estimate: None, disk_path: keyed_disk_path,
        capacity, class, disk, io: args.io.map(|_| IoClass::Heavy),
        coverage: match args.coverage { CliCoverage::Logical => Coverage::Logical, CliCoverage::Enforced => Coverage::Enforced },
        admission_timeout_ms: args.admission_timeout,
        owner: Owner { run: format!("shell:{id}"), step: format!("shell:{id}"), attempt: 1, invocation: Some(format!("shell:{id}")) } };
    if args.inherit_parent {
        let inherited = &parent.as_ref().context("missing inherited resource membership")?.request;
        anyhow::ensure!(lane.capacity.is_none_or(|capacity| Some(capacity) == inherited.capacity)
            && lane.disk.as_ref().is_none_or(|disk| Some(disk) == inherited.disk.as_ref())
            && lane.io.is_none_or(|io| Some(io) == inherited.io)
            && lane.disk_path.as_ref().is_none_or(|path| inherited.disk.as_ref().is_some_and(|disk| disk.path == *path)),
            "refused: --inherit-parent cannot change the parent resource snapshot");
        let id = lane.id;
        let owner = lane.owner;
        lane = inherited.clone();
        lane.id = id;
        lane.owner = owner;
        lane.profile_estimate = None;
    }
    let command = shell_resources::Command { id: id.clone(), lane, argv: args.argv, cwd: cwd.to_string_lossy().into_owned(),
        request_id: args.request_id.map(|request_id| request_id.to_string()),
        environment: std::env::vars().collect(), timeout_ms: args.timeout, parent: parent.map(|parent| parent.grant) };
    let mut execution: Execution = client.post("/v1/internal/resources/exec", &command).await?;
    let id = execution.command.id.clone();
    let mut announced_admission = false;
    loop {
        if execution.started_at.is_some() && !announced_admission {
            eprintln!("resources: admitted {id} blockedBy=none");
            announced_admission = true;
        }
        eprintln!("resources: {} {} blockedBy={} {}", execution.state, id,
            execution.blocked_by.map_or("none", |blocked| blocked.as_str()), execution.reason);
        if matches!(execution.state.as_str(), "completed" | "refused" | "cancelled")
            && let Some(path) = &args.receipt_file {
            write_receipt(path, &execution)?;
        }
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

fn validate_receipt_scope(args: &ExecArgs, has_parent: bool) -> Result<()> {
    if args.receipt_file.is_some() && (has_parent || args.require_parent || args.inherit_parent) {
        return Err(fractal_daemon::FractalError::new("nested-receipt-unsupported",
            "--receipt-file is supported only for root executions; join nested proof to the outer root receipt").into());
    }
    Ok(())
}

fn write_receipt(path: &Path, execution: &Execution) -> Result<()> {
    let mut receipt = fractal_daemon::store::resource_exec_receipts::receipt_fields(execution);
    receipt.insert("schema_version".into(), json!(1));
    receipt.insert("receipt_subject".into(), json!(execution.receipt_subject));
    receipt.entry("exit_code").or_insert(Value::Null);
    receipt.entry("exit_signal").or_insert(Value::Null);
    let directory = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(file.as_file_mut(), &receipt)?;
    file.as_file_mut().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    fs::File::open(directory)?.sync_all()?;
    Ok(())
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
    let result = PayloadResult { head_start, head_end, clean_start, clean_end, exit_code: status.code(), exit_signal: status.signal(), wall_duration_ms, memory_peak_bytes, ..PayloadResult::default() };
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
    fn local_operator_selector_is_preserved_for_supervisor_commands() {
        for arguments in [
            vec!["exec", "--lane", "checks", "--as", "person/operator", "--", "true"],
            vec!["show", "execution-id", "--as", "person/operator"],
            vec!["cancel", "--request-id", "0192c8d8-97e7-7000-8000-000000000001", "--wait", "--as", "person/operator"],
            vec!["member-scope", "--exec-id", "execution-id", "--as", "person/operator"],
        ] {
            let cli = Cli::try_parse_from(["fractal", "resources"].into_iter().chain(arguments)).unwrap();
            let Command::Resources { command } = cli.command else { panic!("wrong command") };
            assert_eq!(command.actor(), Some("person/operator"));
            let endpoint = Endpoint::Unix(PathBuf::from("/nonexistent/fractal.sock"));
            assert!(execution_client(&endpoint, command.actor().unwrap().to_owned()).is_ok());
        }
    }

    #[test]
    fn offline_membership_requires_explicit_owner_uid() {
        let cli = Cli::try_parse_from(["fractal", "--json", "resources", "in-member", "--owner-uid", "1000"]).unwrap();
        let Command::Resources { command: ResourcesCommand::InMember { owner_uid } } = cli.command else { panic!("wrong command") };
        assert_eq!(owner_uid, 1000);
        assert!(cli.json);
        assert!(Cli::try_parse_from(["fractal", "resources", "in-member"]).is_err());
        assert!(Cli::try_parse_from(["fractal", "resources", "in-member", "--owner-uid", "-1"]).is_err());
    }

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
    fn keyed_exec_keeps_lane_required_and_raw_capacity_paired() {
        let cli = Cli::try_parse_from(["fractal","resources","exec","--lane","checks","--profile","check.unit","--require-parent","--disk-path",".","--max-members","3","--receipt-file","receipt.json","--","true"]).unwrap();
        let Command::Resources { command: ResourcesCommand::Exec(args) } = cli.command else { panic!("wrong command") };
        assert_eq!(args.profile.as_deref(), Some("check.unit"));
        assert_eq!(args.max_members, 3);
        assert!(args.require_parent);
        assert!(args.cores.is_none() && args.memory.is_none());
        assert_eq!(args.receipt_file.as_deref(), Some(Path::new("receipt.json")));
        assert!(Cli::try_parse_from(["fractal","resources","exec","--profile","check.unit","--","true"]).is_err());
        assert!(Cli::try_parse_from(["fractal","resources","exec","--lane","x","--cores","1","--","true"]).is_err());
        assert!(Cli::try_parse_from(["fractal","resources","exec","--lane","x","--memory","1G","--","true"]).is_err());
        assert!(Cli::try_parse_from(["fractal","resources","exec","--lane","x","--max-members","0","--","true"]).is_err());
        let cli = Cli::try_parse_from(["fractal","resources","exec","--lane","checks","--profile","check.unit","--","true"]).unwrap();
        let Command::Resources { command: ResourcesCommand::Exec(args) } = cli.command else { panic!("wrong command") };
        assert_eq!(args.max_members, 1);
    }

    #[test]
    fn inherited_mode_allows_omitting_lane_but_require_parent_alone_does_not() {
        let cli = Cli::try_parse_from(["fractal","resources","exec","--inherit-parent","--","true"]).unwrap();
        let Command::Resources { command: ResourcesCommand::Exec(args) } = cli.command else { panic!("wrong command") };
        assert!(args.inherit_parent);
        assert!(args.lane.is_none());
        assert!(Cli::try_parse_from(["fractal","resources","exec","--require-parent","--","true"]).is_err());
    }
    #[test]
    fn cancel_selects_exact_execution_identity() {
        let cli = Cli::try_parse_from(["fractal", "resources", "cancel", "execution-id"]).unwrap();
        let Command::Resources { command: ResourcesCommand::Cancel { id, .. } } = cli.command else { panic!("wrong command") };
        assert_eq!(id.as_deref(), Some("execution-id"));
    }
    #[test]
    fn member_scope_requires_exact_execution_selector() {
        let cli = Cli::try_parse_from(["fractal", "--json", "resources", "member-scope", "--exec-id", "execution-id"]).unwrap();
        let Command::Resources { command: ResourcesCommand::MemberScope { exec_id, .. } } = cli.command else { panic!("wrong command") };
        assert_eq!(exec_id, "execution-id");
        assert!(Cli::try_parse_from(["fractal", "resources", "member-scope"]).is_err());
    }



    #[test]
    fn terminal_receipt_binds_root_argv_and_preserves_refusal_state() {
        let lane = LaneRequest {
            id: "shell:root".into(), name: "checks".into(), host: "host-a".into(), max_members: 1,
            profile: Some("check.unit".into()), profile_estimate: None, disk_path: None,
            capacity: Some(fractal_runtime::Capacity { cores: 4, memory: 8 << 30 }), class: RequestClass::Check, disk: None, io: None,
            coverage: Coverage::Enforced, admission_timeout_ms: None,
            owner: Owner { run: "shell:root".into(), step: "shell:root".into(), attempt: 1, invocation: None },
        };
        let command = shell_resources::Command { id: "root".into(), request_id: None, lane, argv: vec!["tools/check".into(), "unit".into()],
            cwd: "/work".into(), environment: BTreeMap::from([("AUTH_TOKEN".into(), "sentinel-secret-never-persist".into())]),
            timeout_ms: None, parent: None };
        let root: Execution = serde_json::from_value(json!({
            "command": command, "state": "waiting", "reason": "waiting", "blockedBy": null,
            "queuedAt": 1, "revision": 0, "queueWaitMs": 0, "startedAt": null, "endedAt": null,
            "exitCode": null, "exitSignal": null, "stdout": "", "stderr": "", "cancel": false,
            "timedOut": false, "headStart": null, "cleanStart": null, "result": null,
            "rootExecId": "root", "receiptSubject": null
        })).unwrap();
        let mut refused = root.clone();
        refused.state = "refused".into();
        refused.queue_wait_ms = 17;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("receipt.json");
        fs::write(&path, "old").unwrap();
        write_receipt(&path, &refused).unwrap();
        let receipt: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(receipt["schema_version"], 1);
        assert_eq!(receipt["exec_id"], "root");
        assert_eq!(receipt["state"], "refused");
        assert_eq!(receipt["launched"], false);
        assert_eq!(receipt["queue_wait_ms"], 17);
        assert_eq!(receipt["wall_duration_ms"], 0);
        assert!(receipt["receipt_subject"].is_null());
        assert!(receipt["exit_code"].is_null());
        assert_eq!(receipt["argv"], json!(["tools/check", "unit"]));
        assert_eq!(receipt["argv_sha256"], fractal_daemon::store::resource_exec_receipts::argv_sha256(&root.command.argv));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        let mut completed = refused;
        completed.state = "completed".into();
        completed.started_at = Some(2);
        completed.exit_code = Some(37);
        completed.receipt_subject = Some("resource/exec/abcd/host/check/root".into());
        completed.result = Some(PayloadResult {
            head_start: Some("start-head".into()), head_end: Some("end-head".into()), clean_start: Some(true), clean_end: Some(false),
            exit_code: Some(37), exit_signal: None, wall_duration_ms: 91, memory_peak_bytes: Some(42),
            ..PayloadResult::default()
        });
        write_receipt(&path, &completed).unwrap();
        let receipt: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(receipt["launched"], true);
        assert_eq!(receipt["wall_duration_ms"], 91);
        assert_eq!(receipt["exit_code"], 37);
        assert_eq!(receipt["receipt_subject"], "resource/exec/abcd/host/check/root");
        assert_eq!(receipt["exit_signal"], Value::Null);
        assert_eq!(receipt["head_start"], "start-head");
        assert_eq!(receipt["head_end"], "end-head");
        assert_eq!(receipt["clean_start"], true);
        assert_eq!(receipt["clean_end"], false);
        assert_eq!(receipt["coverage"], "enforced");
        assert_eq!(receipt["class"], "check");
        assert_eq!(receipt["profile_key"], "check.unit");
        assert_eq!(receipt["memory_bytes"], 8_u64 << 30);
        assert_eq!(receipt["memory_peak_bytes"], 42);
        for (key, value) in fractal_daemon::store::resource_exec_receipts::receipt_fields(&completed) {
            assert_eq!(receipt[&key], value, "canonical A.3 field differs: {key}");
        }
        let serialized = fs::read_to_string(&path).unwrap();
        assert!(!serialized.contains("sentinel-secret-never-persist"));
        assert!(!serialized.contains("AUTH_TOKEN"));
        assert!(receipt.get("environment").is_none());
    }

    #[test]
    fn receipt_files_refuse_nested_execution_and_explicit_ids_validate_before_submission() {
        let parse = |argv: Vec<&str>| {
            let cli = Cli::try_parse_from(argv).unwrap();
            let Command::Resources { command: ResourcesCommand::Exec(args) } = cli.command else { panic!("wrong command") };
            args
        };
        let args = parse(vec!["fractal", "resources", "exec", "--lane", "checks", "--receipt-file", "out.json", "--", "true"]);
        assert!(validate_receipt_scope(&args, false).is_ok());
        let error = validate_receipt_scope(&args, true).unwrap_err();
        assert_eq!(error.downcast_ref::<fractal_daemon::FractalError>().unwrap().code, "nested-receipt-unsupported");
        let args = parse(vec!["fractal", "resources", "exec", "--inherit-parent", "--receipt-file", "out.json", "--", "true"]);
        assert!(validate_receipt_scope(&args, false).is_err());
        let args = parse(vec!["fractal", "resources", "exec", "--lane", "checks", "--require-parent", "--receipt-file", "out.json", "--", "true"]);
        assert!(validate_receipt_scope(&args, false).is_err());
        let id = "0192c8d8-97e7-7000-8000-000000000001";
        let args = parse(vec!["fractal", "resources", "exec", "--lane", "checks", "--request-id", id, "--", "true"]);
        assert_eq!(args.request_id.unwrap().to_string(), id);
        assert!(Cli::try_parse_from(["fractal", "resources", "exec", "--lane", "checks", "--request-id", "not-a-uuid", "--", "true"]).is_err());
        assert!(Cli::try_parse_from(["fractal", "resources", "exec", "--lane", "checks", "--exec-id", id, "--", "true"]).is_err());
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
