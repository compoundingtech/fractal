use super::*;
// The same print macros as main.rs: CLI output goes through st3's writer.
use fractal_daemon::{cli_print as print, cli_println as println};

// Some fleet commands historically print prose even with --json. Keep those bytes
// stable for callers; only the human rendering uses the installed fractal command.
fn render_fabric_invite_sent(
    output: &mut impl std::io::Write,
    invite: &str,
    name: &str,
    command: &str,
    json_output: bool,
) -> std::io::Result<()> {
    let program = if json_output { "st" } else { "fractal" };
    writeln!(
        output,
        "Sent invite {invite} to {name}'s Fabric inbox. On {name}, run:\n  {program} fleet {command} --fabric-inbox"
    )
}

fn render_offline_leave(
    output: &mut impl std::io::Write,
    node: &str,
    json_output: bool,
) -> std::io::Result<()> {
    let program = if json_output { "st" } else { "fractal" };
    writeln!(
        output,
        "Left without reaching a member. On another member run: {program} fleet remove {node} --reason \"left offline\""
    )
}

fn leave_restart_hint(json_output: bool) -> &'static str {
    if json_output {
        "Stop st3 replication-worker; st3 up now runs local-only after a restart."
    } else {
        "Stop fractal replication-worker; fractal up now runs local-only after a restart."
    }
}

fn render_removed_member_hint(
    output: &mut impl std::io::Write,
    name: &str,
    json_output: bool,
) -> std::io::Result<()> {
    let program = if json_output { "st" } else { "fractal" };
    writeln!(
        output,
        "Writes {name} made after this node's last exchange with it are not accepted. On {name}, if it still runs: {program} uninstall"
    )
}

fn legacy_removal_hint(json_output: bool) -> &'static str {
    if json_output {
        "This member still accepts legacy exchanges from config peers, so a machine that keeps the fleet secret can pose as one of them until st fleet migrate --finish."
    } else {
        "This member still accepts legacy exchanges from config peers, so a machine that keeps the fleet secret can pose as one of them until fractal fleet migrate --finish."
    }
}

fn fleet_restart_hint(json_output: bool) -> &'static str {
    if json_output {
        "Restart st3 daemon and st3 replication-worker for the change to take effect.\n\
         The old worker no longer publishes endpoints after its settings change; restart it to resume publication.\n"
    } else {
        "Restart fractal daemon and fractal replication-worker for the change to take effect.\n\
         The old worker no longer publishes endpoints after its settings change; restart it to resume publication.\n"
    }
}

fn migration_restart_hint(legacy_toggle: bool, json_output: bool) -> &'static str {
    match (legacy_toggle, json_output) {
        (true, true) => "Restart st3 replication-worker for this to take effect.",
        (true, false) => "Restart fractal replication-worker for this to take effect.",
        (false, true) => "Start st3 up and st3 replication-worker; legacy exchanges stay accepted until st fleet migrate --finish.",
        (false, false) => "Start fractal up and fractal replication-worker; legacy exchanges stay accepted until fractal fleet migrate --finish.",
    }
}

fn parse_fleet_duration(text: &str) -> Result<u64> {
    let text = text.trim();
    let (number, unit) = text
        .find(|character: char| !character.is_ascii_digit())
        .map_or((text, "s"), |split| text.split_at(split));
    let number: u64 = number
        .parse()
        .context("a duration is a number and a unit, like 15m")?;
    Ok(number
        * match unit {
            "s" => 1,
            "m" => 60,
            "h" => 3600,
            _ => anyhow::bail!("a duration unit is s, m, or h"),
        })
}

fn read_code_without_echo() -> Result<String> {
    use std::io::{BufRead as _, IsTerminal as _};
    let stdin = std::io::stdin();
    let terminal = stdin.is_terminal();
    let mut saved = None;
    if terminal {
        eprint!("Paste the join code: ");
        // SAFETY: plain termios calls on standard input; the saved settings are restored below.
        unsafe {
            let mut settings: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut settings) == 0 {
                saved = Some(settings);
                settings.c_lflag &= !libc::ECHO;
                libc::tcsetattr(0, libc::TCSANOW, &settings);
            }
        }
    }
    let mut line = String::new();
    let result = stdin.lock().read_line(&mut line);
    if let Some(settings) = saved {
        // SAFETY: restores the settings read above.
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &settings);
        }
        eprintln!();
    }
    result?;
    Ok(line.trim().to_owned())
}

const FABRIC_PAIRING_UNAVAILABLE: &str = "Fabric invitations need st-fabric-adapter installed and \
     Fabric running with --runtime-peers; use --via tailscale for a Tailscale-only invitation";

/// The pairing helper beside st, named by ST_FABRIC_ADAPTER, or on PATH. A configured
/// path that is not a file counts as no helper, so `auto` invitations route around it.
fn fabric_adapter() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ST_FABRIC_ADAPTER") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("st-fabric-adapter")))
        .filter(|path| path.is_file())
        .or_else(|| fractal_daemon::fleet::transport::resolve_tool(None, "st-fabric-adapter"))
}

/// The `--via` an invitation is created with. Without the pairing helper, a Fabric endpoint
/// cannot carry a new machine in, so `auto` uses this member's other endpoints, and an
/// invitation that only Fabric could carry fails here, before any invite exists.
fn invite_via(requested: &str, endpoints: &[Value], pairing: bool) -> Result<String> {
    let advertises = |transport: &str| endpoints.iter().any(|endpoint| endpoint["transport"] == transport);
    if pairing || !matches!(requested, "auto" | "fabric") || !advertises("fabric") {
        return Ok(requested.to_owned());
    }
    if requested == "auto"
        && let Some(other) = ["tailscale", "loopback"].into_iter().find(|transport| advertises(transport))
    {
        return Ok(other.to_owned());
    }
    anyhow::bail!(FABRIC_PAIRING_UNAVAILABLE)
}

/// The Fabric peer `name` names in this machine's own `fabric peers` list, by name or NodeID.
fn trusted_fabric_peer(fabric: &std::path::Path, name: &str) -> Result<()> {
    let output = std::process::Command::new(fabric)
        .arg("peers")
        .output()
        .with_context(|| format!("run {} peers", fabric.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "fabric peers failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let listed = String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let mut fields = line.split('\t');
        let id = fields.next().unwrap_or_default();
        let peer = fields.next().unwrap_or_default();
        id == name || peer == name
    });
    anyhow::ensure!(
        listed,
        "{name} is not a trusted Fabric peer of this machine (see fabric peers); trust it in \
         Fabric first or show the code with --code-only"
    );
    Ok(())
}

/// Hand the code to `name`'s Fabric inbox. Fabric answers only after the far side has
/// committed the file, so success here means it arrived.
fn send_code_over_fabric(
    config: &Config,
    fabric: &std::path::Path,
    name: &str,
    created: &fractal_daemon::api::FleetInviteCreated,
) -> Result<()> {
    let temporary = config
        .state_dir
        .join("fleet")
        .join(format!(".send-{}.code", std::process::id()));
    fractal_daemon::fleet::join::write_private(&temporary, created.code.as_bytes())?;
    let invite_id = created.invite.trim_start_matches("fleet-invite/");
    let sent = std::process::Command::new(fabric)
        .args(["send-file", name])
        .arg(&temporary)
        .args(["--as", &format!("st-fleet-join-{invite_id}.code")])
        .stdin(std::process::Stdio::null())
        .output();
    let _ = fs::remove_file(&temporary);
    let sent = sent.with_context(|| format!("run {} send-file", fabric.display()))?;
    anyhow::ensure!(
        sent.status.success(),
        "fabric send-file to {name} failed: {}",
        String::from_utf8_lossy(&sent.stderr).trim()
    );
    Ok(())
}

/// Revoke an invite whose code never reached the new machine, saying so on `error`. The
/// flag is whether the revocation took effect.
async fn revoke_undelivered(
    client: &Client,
    invite: &str,
    person: &str,
    error: anyhow::Error,
) -> (anyhow::Error, bool) {
    let revoked: Result<Value> = client
        .post(
            "/v1/internal/fleet/invites/revoke",
            &fractal_daemon::api::FleetInviteRevokeRequest {
                invite: invite.to_owned(),
                reason: "the invitation was not delivered".into(),
                person: person.to_owned(),
            },
        )
        .await;
    match revoked {
        Ok(_) => (error.context(format!("revoked {invite}, which was not delivered")), true),
        Err(_) => (
            error.context(format!(
                "{invite} was not delivered and stays open until it expires; revoke it with \
                 fractal fleet invites revoke {invite}"
            )),
            false,
        ),
    }
}

/// Write a join code to a new 0600 file at `path`, replacing any file there. Unlike the state
/// directory's private writes, this leaves the person's chosen directory and its mode alone.
fn write_code_file(path: &std::path::Path, code: &[u8]) -> Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let name = path
        .file_name()
        .with_context(|| format!("{} does not name a file", path.display()))?;
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".{}.tmp", std::process::id()));
    let temporary = directory.join(temporary_name);
    let written = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(code)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.with_context(|| format!("write the join code to {}", path.display()))
}

/// How long `--fabric-inbox` waits for a code: the longest a Fabric invitation lives.
const FABRIC_INBOX_WAIT: Duration = Duration::from_secs(600);

/// This machine's Fabric home, which holds its inbox.
fn fabric_home() -> Result<PathBuf> {
    std::env::var_os("FABRIC_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/fabric"))
        })
        .context("HOME is not set")
}

/// The one `st-fleet-join-*.code` file in `home`'s Fabric inbox, waiting up to `wait` for
/// `fractal fleet invite --send-fabric` on the sponsor to deliver it. Fabric renames a
/// finished transfer into `inbox/SENDER/`, so a watch on each directory along that path
/// wakes this when the code lands.
fn wait_for_fabric_inbox_code(
    home: &std::path::Path,
    wait: Duration,
    json_output: bool,
) -> Result<PathBuf> {
    use notify::Watcher as _;
    let deadline = std::time::Instant::now() + wait;
    let (events, woken) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(events)?;
    let mut watched = BTreeMap::new();
    let mut announced = false;
    loop {
        // Watch before looking, so a directory or code that appears after the look raises an
        // event. A directory made while its parent's watch began raises none, so list again
        // until no new directory appears.
        loop {
            let mut added = false;
            for (directory, inode) in fabric_inbox_directories(home)? {
                if watched.get(&directory) == Some(&inode) {
                    continue;
                }
                let _ = watcher.unwatch(&directory);
                match watcher.watch(&directory, notify::RecursiveMode::NonRecursive) {
                    // Gone again: its parent's watch reports what replaces it.
                    Err(error) if matches!(error.kind, notify::ErrorKind::PathNotFound) => {}
                    watching => {
                        watching.with_context(|| format!("watch {}", directory.display()))?
                    }
                }
                watched.insert(directory, inode);
                added = true;
            }
            if !added {
                break;
            }
        }
        if let Some(path) = fabric_inbox_code(home)? {
            return Ok(path);
        }
        let Some(remaining) = deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|remaining| !remaining.is_zero())
        else {
            anyhow::bail!(
                "no join code arrived in the Fabric inbox within {}s; on the sponsor run \
                 fractal fleet invite NAME --send-fabric",
                wait.as_secs()
            );
        };
        if !announced && !json_output {
            eprintln!("Waiting for a join code in this machine's Fabric inbox…");
            announced = true;
        }
        await_fabric_inbox_event(&woken, remaining)?;
    }
}

/// Wait up to `remaining` for the inbox watch to report a change: any event, or the deadline,
/// means look again. A watch that reports an error may not report the code either, so the
/// error ends the wait.
fn await_fabric_inbox_event(
    woken: &std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    remaining: Duration,
) -> Result<()> {
    let mut events = match woken.recv_timeout(remaining) {
        Ok(event) => vec![event],
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Vec::new(),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("the watch on the Fabric inbox stopped")
        }
    };
    events.extend(woken.try_iter());
    for event in events {
        event.context("the watch on the Fabric inbox failed")?;
    }
    Ok(())
}

/// The directories a delivered code passes through, with their inodes so a recreated one is
/// watched again: the Fabric home, its inbox once Fabric has made it, and each sender's
/// directory in the inbox.
fn fabric_inbox_directories(home: &std::path::Path) -> Result<Vec<(PathBuf, u64)>> {
    fn inode(path: &std::path::Path) -> Result<Option<u64>> {
        use std::os::unix::fs::MetadataExt as _;
        match fs::metadata(path) {
            Ok(metadata) => Ok(metadata.is_dir().then(|| metadata.ino())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
        }
    }
    let home_inode = inode(home)?.with_context(|| {
        format!(
            "no Fabric home at {}; set FABRIC_HOME if Fabric keeps it elsewhere",
            home.display()
        )
    })?;
    let mut directories = vec![(home.to_path_buf(), home_inode)];
    let inbox = home.join("inbox");
    let Some(inbox_inode) = inode(&inbox)? else {
        return Ok(directories);
    };
    directories.push((inbox.clone(), inbox_inode));
    for sender in fs::read_dir(&inbox)
        .with_context(|| format!("read the Fabric inbox {}", inbox.display()))?
    {
        let sender = sender?.path();
        if let Some(sender_inode) = inode(&sender)? {
            directories.push((sender, sender_inode));
        }
    }
    Ok(directories)
}

/// The one `st-fleet-join-*.code` file in `home`'s Fabric inbox, if one has arrived.
fn fabric_inbox_code(home: &std::path::Path) -> Result<Option<PathBuf>> {
    let inbox = home.join("inbox");
    let senders = match fs::read_dir(&inbox) {
        Ok(senders) => senders,
        // Fabric makes the inbox when the first file arrives.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("read the Fabric inbox {}", inbox.display()))
        }
    };
    let mut found = Vec::new();
    for sender in senders {
        let sender = sender?.path();
        if !sender.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&sender)? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("st-fleet-join-") && name.ends_with(".code"))
            {
                found.push(path);
            }
        }
    }
    match found.len() {
        0 => Ok(None),
        1 => Ok(Some(found.remove(0))),
        _ => anyhow::bail!(
            "{} join codes in the Fabric inbox {}; remove the old ones",
            found.len(),
            inbox.display()
        ),
    }
}

async fn lock_fleet_admission(
    client: &Client,
    config: &Config,
    services: bool,
    verb: &str,
) -> Result<fractal_daemon::node_identity::StateLock> {
    if client.get::<Value>("/v1/health").await.is_ok() {
        anyhow::ensure!(
            services,
            "stop the running st3 daemon first: nothing may write while this machine {verb}"
        );
        fractal_daemon::service::stop()?;
        fractal_daemon::node_identity::lock_after_stop(&config.state_dir).await
    } else {
        fractal_daemon::node_identity::lock(&config.state_dir)
    }
}

/// Pairing credentials travel over the helper's private stdin/stdout, never argv.
async fn bootstrap_fabric_invite(
    endpoint: &Endpoint,
    adapter: Option<&std::path::Path>,
    created: &mut fractal_daemon::api::FleetInviteCreated,
) -> Result<()> {
    let code = fractal_daemon::fleet::code::JoinCode::decode(&created.code)?;
    if code.migrate
        || !code.endpoints.iter().any(|endpoint| {
            matches!(endpoint, fractal_daemon::fleet::code::CodeEndpoint::Fabric { .. })
        })
    {
        return Ok(());
    }
    let Endpoint::Unix(socket) = endpoint else {
        anyhow::bail!("Fabric bootstrap invitations must be issued on the sponsor machine");
    };
    let helper = adapter.context(FABRIC_PAIRING_UNAVAILABLE)?;
    let mut child = tokio::process::Command::new(helper)
        .arg("--st-socket")
        .arg(socket)
        .arg("pairing-invite")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("start the Fabric invitation helper")?;
    {
        use tokio::io::AsyncWriteExt as _;
        let mut input = child.stdin.take().context("open pairing helper input")?;
        input
            .write_all(&serde_json::to_vec(&json!({ "code": created.code }))?)
            .await?;
        input.write_all(b"\n").await?;
        input.shutdown().await?;
    }
    let output = child.wait_with_output().await?;
    anyhow::ensure!(
        output.status.success(),
        "Fabric pairing failed; it needs st-fabric-adapter running on this machine and Fabric \
         running with --runtime-peers"
    );
    #[derive(serde::Deserialize)]
    struct PairedInvite {
        code: String,
    }
    let paired: PairedInvite =
        serde_json::from_slice(&output.stdout).context("decode the Fabric invitation helper")?;
    let paired_code = fractal_daemon::fleet::code::JoinCode::decode(&paired.code)?;
    anyhow::ensure!(
        paired_code.fleet_id == code.fleet_id && paired_code.invite == code.invite,
        "Fabric invitation helper returned another invitation"
    );
    created.expires_at_unix_ms = paired_code.expires_at.saturating_mul(1000);
    created.code = paired.code;
    Ok(())
}

pub(super) async fn run_fleet(endpoint: &Endpoint, command: FleetCommand, json_output: bool) -> Result<()> {
    let manages_services = match &command {
        FleetCommand::Create(args) => !args.no_service,
        FleetCommand::Join(args) => !args.no_service,
        FleetCommand::Leave(args) => !args.no_service,
        FleetCommand::Mode(args) => !args.no_service,
        FleetCommand::Migrate(args) => !args.no_service,
        FleetCommand::Transports { command } => match command {
            FleetTransportsCommand::Add { no_service, .. }
            | FleetTransportsCommand::Remove { no_service, .. }
            | FleetTransportsCommand::Prefer { no_service, .. } => !*no_service,
        },
        _ => false,
    };
    fractal_daemon::service::preflight(manages_services)?;
    let mut config = Config::load_unvalidated(None)?;
    // Explicit admissions can repair a pin after their prior attempt saved membership.
    // Other controls must use the daemon's resolved identity, including leave's runtime check.
    if !matches!(&command, FleetCommand::Join(_) | FleetCommand::Create(_)) {
        config.apply_fleet_file()?;
        fractal_daemon::node_identity::resolve(&mut config)?;
    }
    let client = Client::new(endpoint.clone());
    match command {
        FleetCommand::Create(args) => {
            anyhow::ensure!(
                fractal_daemon::config::FleetFile::load(&config.state_dir)?.is_none(),
                "this machine is already in a fleet"
            );
            anyhow::ensure!(
                config.fleet_id.is_none(),
                "config.toml already configures a fleet with config peers; move it to membership with fractal fleet migrate"
            );
            fractal_daemon::node_identity::resolve(&mut config)?;
            let node = args.name.unwrap_or_else(|| config.node.clone());
            anyhow::ensure!(
                fractal_daemon::store::valid_fleet_node_name(&node),
                "`{node}` cannot name a fleet member; use --name"
            );
            let services = !args.no_service && services_installed()?;
            let state_identity =
                lock_fleet_admission(&client, &config, services, "founds a fleet").await?;
            let founded =
                fractal_daemon::fleet::join::found(&config.state_dir, &node, &args.member.settings())?;
            state_identity.record_fleet_found(&founded)?;
            drop(state_identity);
            if json_output {
                return print_value(&founded, true);
            }
            println!(
                "Created fleet {} with {} as its first member.",
                founded.fleet_id, founded.node
            );
            if services {
                fractal_daemon::service::install(Config::load_with_fleet(None)?)?;
            }
            print!("{}", cli_help::fleet_created_next_steps(services));
            Ok(())
        }
        FleetCommand::Invite(args) => {
            let person = fleet_person(args.actor.clone(), &config)?;
            let expires_seconds = parse_fleet_duration(&args.expires)?;
            // Everything that can refuse runs before the invite exists, so a refusal leaves
            // no open invite behind.
            let fabric = if args.send_fabric {
                let name = args
                    .name
                    .as_deref()
                    .context("--send-fabric needs the NAME of the machine to send it to")?;
                let fabric = fractal_daemon::config::FleetFile::load(&config.state_dir)?
                    .and_then(|file| file.fabric)
                    .or_else(|| fractal_daemon::fleet::transport::resolve_tool(None, "fabric"))
                    .context("--send-fabric needs the fabric command")?;
                trusted_fabric_peer(&fabric, name)?;
                Some((fabric, name))
            } else {
                None
            };
            let adapter = fabric_adapter();
            let mut via = args.via.clone();
            if adapter.is_none() && !args.migrate {
                via = invite_via(&via, &member_endpoints(&client, &config.node).await?, false)?;
            }
            let create = |via: String| {
                let request = fractal_daemon::api::FleetInviteRequest {
                    name: args.name.clone(),
                    expires_seconds,
                    via: Some(via),
                    migrate: args.migrate,
                    person: person.clone(),
                };
                let client = &client;
                async move {
                    client
                        .post::<_, fractal_daemon::api::FleetInviteCreated>(
                            "/v1/internal/fleet/invites",
                            &request,
                        )
                        .await
                }
            };
            let mut created = create(via).await?;
            if let Err(error) = bootstrap_fabric_invite(endpoint, adapter.as_deref(), &mut created).await {
                // Revoke first, so nothing that fails below leaves the undelivered invite open.
                let (error, revoked) = revoke_undelivered(&client, &created.invite, &person, error).await;
                if !revoked || args.via != "auto" || args.migrate {
                    return Err(error);
                }
                // `auto` still has the member's other endpoints when Fabric cannot pair.
                let endpoints = match member_endpoints(&client, &config.node).await {
                    Ok(endpoints) => endpoints,
                    Err(lookup) => return Err(error.context(format!("{lookup:#}"))),
                };
                let Ok(fallback) = invite_via("auto", &endpoints, false) else {
                    return Err(error);
                };
                eprintln!("{error:#}; inviting over {fallback} instead.");
                created = create(fallback).await?;
            }
            let command = if args.migrate { "migrate" } else { "join" };
            let delivered = if let Some(path) = &args.code_file {
                write_code_file(path, created.code.as_bytes())
            } else if let Some((fabric, name)) = &fabric {
                send_code_over_fabric(&config, fabric, name, &created)
            } else {
                Ok(())
            };
            if let Err(error) = delivered {
                return Err(revoke_undelivered(&client, &created.invite, &person, error).await.0);
            }
            if let Some(path) = &args.code_file {
                println!("{}\t{}", created.invite, path.display());
            } else if let Some((_, name)) = &fabric {
                render_fabric_invite_sent(
                    &mut cli_output::stdout(),
                    &created.invite,
                    name,
                    command,
                    json_output,
                )?;
            } else if args.code_only {
                println!("{}", created.code);
            } else if json_output {
                return print_value(&created, true);
            } else {
                let expires_in =
                    (u128::from(created.expires_at_unix_ms).saturating_sub(unix_ms()) / 60_000) + 1;
                let target = args.name.as_deref().unwrap_or("the new machine");
                println!(
                    "Invite {} for {target} expires in about {expires_in} minutes.",
                    created.invite
                );
                println!("On {target}, run:\n  fractal fleet {command} {}", created.code);
                println!(
                    "The code is a one-use credential. To avoid shell history, run \
                     `fractal fleet {command}` and paste it at the hidden prompt instead."
                );
            }
            Ok(())
        }
        FleetCommand::Invites { all, command } => match command {
            None => {
                let invites: Vec<fractal_daemon::store::FleetInviteView> = client
                    .get(&format!("/v1/internal/fleet/invites?all={all}"))
                    .await?;
                if json_output {
                    return print_value(&invites, true);
                }
                println!("INVITES  {}", invites.len());
                for invite in invites {
                    let detail = match invite.state.as_str() {
                        "redeemed" => format!(
                            "by {} (key {}…)",
                            invite.redeemed_name.as_deref().unwrap_or("?"),
                            invite
                                .redeemed_key
                                .as_deref()
                                .map(|key| &key[..key.len().min(8)])
                                .unwrap_or("?")
                        ),
                        "revoked" => invite.revoked_reason.clone().unwrap_or_default(),
                        _ => format!("for {}", invite.name.as_deref().unwrap_or("any name")),
                    };
                    println!(
                        "{}  {}  sponsor {}  {}",
                        invite.invite, invite.state, invite.sponsor, detail
                    );
                }
                Ok(())
            }
            Some(FleetInvitesCommand::Revoke {
                invite,
                reason,
                actor,
            }) => {
                let person = fleet_person(actor, &config)?;
                let _: Value = client
                    .post(
                        "/v1/internal/fleet/invites/revoke",
                        &fractal_daemon::api::FleetInviteRevokeRequest {
                            invite: invite.clone(),
                            reason,
                            person,
                        },
                    )
                    .await?;
                println!("revoked\t{invite}");
                Ok(())
            }
        },
        FleetCommand::Join(args) => {
            let (code, code_path) = if args.fabric_inbox {
                let path = wait_for_fabric_inbox_code(&fabric_home()?, FABRIC_INBOX_WAIT, json_output)?;
                (fs::read_to_string(&path)?, Some(path))
            } else if let Some(path) = &args.code_file {
                (fs::read_to_string(path)?, Some(path.clone()))
            } else {
                match args.code.as_deref() {
                    Some("-") => {
                        let mut text = String::new();
                        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
                        (text, None)
                    }
                    Some(code) => (code.to_owned(), None),
                    None => (read_code_without_echo()?, None),
                }
            };
            let use_services = !args.no_service && services_installed()?;
            let state_identity =
                lock_fleet_admission(&client, &config, use_services, "joins").await?;
            let joined = fractal_daemon::fleet::join::join(&fractal_daemon::fleet::join::JoinOptions {
                state_dir: config.state_dir.clone(),
                configured_node: config.node.clone(),
                code: code.trim().to_owned(),
                name: args.name.clone(),
                via: args.via.clone(),
                settings: args.member.settings(),
                legacy_secret_file: None,
                fabric_protocol: None,
                runtime: fractal_daemon::store::runtime(),
                version: env!("CARGO_PKG_VERSION").into(),
            })
            .await?;
            state_identity.record_fleet_join(&joined)?;
            drop(state_identity);
            if let Some(path) = code_path {
                let _ = fs::remove_file(path);
            }
            if json_output {
                print_value(&joined, true)?;
            } else {
                println!(
                    "{} joined fleet {} through {}{}.",
                    joined.name,
                    joined.fleet_id,
                    joined.sponsor,
                    if joined.resumed { " (resumed)" } else { "" }
                );
            }
            let mut sync_state = None;
            if use_services {
                fractal_daemon::service::install(Config::load_with_fleet(None)?)?;
                if !json_output {
                    println!("The st services now run as a fleet member.");
                }
                if !args.no_wait {
                    match wait_for_first_sync(&client, Duration::from_secs(30 * 60), !json_output)
                        .await?
                    {
                        // The join already printed its JSON; a failed first sync still fails.
                        Some(first) if json_output => anyhow::ensure!(
                            first.state == "verified",
                            "{}",
                            render_first_sync(&first, now_ms())
                        ),
                        Some(first) => {
                            sync_state = Some(first.state.clone());
                            if first.state != "verified" {
                                print!(
                                    "{}",
                                    cli_help::fleet_next_steps(use_services, sync_state.as_deref())
                                );
                            }
                            report_first_sync(&first, false)?;
                        }
                        None if json_output => {}
                        None => println!(
                            "This machine is a member and still syncing; fractal fleet wait waits for \
                             the first sync to end and checks it."
                        ),
                    }
                }
            } else if !json_output {
                println!(
                    "Start fractal up and fractal replication-worker to begin syncing; fractal fleet wait \
                     waits for the first sync to end and checks it."
                );
            }
            if !json_output {
                print!(
                    "{}",
                    cli_help::fleet_next_steps(use_services, sync_state.as_deref())
                );
            }
            Ok(())
        }
        FleetCommand::Wait { timeout } => {
            let timeout = Duration::from_secs(parse_fleet_duration(&timeout)?);
            let started = std::time::Instant::now();
            let since = now_ms();
            let first = wait_for_first_sync(&client, timeout, !json_output)
                .await?
                .with_context(|| {
                    format!(
                        "the first sync has not ended after {} s; fractal replication status shows \
                         how far it got",
                        timeout.as_secs()
                    )
                })?;
            if first.state != "verified" {
                return report_first_sync(&first, json_output);
            }
            // A first sync is verified once, long ago after a restart. A wait is a gate for now:
            // it also needs an exchange since it began at which this node held everything.
            let caught_up = wait_for_caught_up(
                &client,
                since,
                timeout.saturating_sub(started.elapsed()),
                !json_output,
            )
            .await?;
            let caught_up = match caught_up {
                Ok(caught_up) => caught_up,
                Err(waiting) => anyhow::bail!(
                    "{}\nbut this node has not caught up since this wait began {} s ago: {}; fractal \
                     replication status shows how far it got",
                    render_first_sync(&first, now_ms()),
                    started.elapsed().as_secs(),
                    waiting
                ),
            };
            // Holding the fleet's envelopes is not enough when this node admitted some the fleet
            // fenced: its claims then differ from every other member's.
            let residue: Vec<smallclaims::store::FleetAdmissionResidue> =
                client.get("/v1/internal/fleet/admission-residue").await?;
            if let Some(fenced) = render_fenced_admissions(&residue) {
                anyhow::bail!("{}\nbut {fenced}", render_first_sync(&first, now_ms()));
            }
            if json_output {
                let mut value = serde_json::to_value(&first)?;
                value["caught_up"] = serde_json::to_value(&caught_up)?;
                return print_value(&value, true);
            }
            println!("{}", render_first_sync(&first, now_ms()));
            println!("{}", render_caught_up(&caught_up, now_ms()));
            Ok(())
        }
        FleetCommand::Remove(args) => run_fleet_remove(&client, &config, args, json_output).await,
        FleetCommand::Migrate(args) => run_fleet_migrate(&client, &config, args, json_output).await,
        FleetCommand::Mode(args) => {
            let lock = lock_fleet_settings(&config.state_dir).await?;
            let mut file = fractal_daemon::config::FleetFile::load(&config.state_dir)?
                .context("this machine is not a fleet member")?;
            file.mode = match args.mode.as_str() {
                "listening" => fractal_daemon::config::FleetMode::Listening,
                "dial-out" => fractal_daemon::config::FleetMode::DialOut,
                _ => anyhow::bail!("the mode is listening or dial-out"),
            };
            file.port = match file.mode {
                fractal_daemon::config::FleetMode::DialOut => None,
                fractal_daemon::config::FleetMode::Listening => Some(match args.port.or(file.port) {
                    Some(port) => port,
                    None => fractal_daemon::fleet::join::free_port(fractal_daemon::fleet::join::DEFAULT_PORT)?,
                }),
            };
            file.save(&config.state_dir)?;
            drop(lock);
            println!(
                "This member is now {}; it announces the change when its replication worker starts.",
                file.mode.as_str()
            );
            refresh_fleet_services(args.no_service, json_output)?;
            Ok(())
        }
        FleetCommand::Transports { command } => {
            run_fleet_transports(&client, &config, command, json_output).await
        }
        FleetCommand::Leave(args) => {
            let person = fleet_person(args.actor.clone(), &config)?;
            if args.cancel {
                let _: Value = client
                    .post("/v1/internal/fleet/leave/cancel", &json!({}))
                    .await?;
                println!("leave cancelled");
                return Ok(());
            }
            let use_services = !args.no_service && services_installed()?;
            let confirmed = fleet_leave(
                &client,
                &config,
                &person,
                args.offline,
                args.force,
                parse_fleet_duration(&args.wait)?,
            )
            .await?;
            match confirmed {
                Some(member) => println!("{member} holds everything this machine wrote."),
                None => render_offline_leave(&mut cli_output::stdout(), &config.node, json_output)?,
            }
            if use_services {
                fractal_daemon::service::install(Config::load_with_fleet(None)?)?;
                println!("Fractal now runs local-only on this machine.");
            } else {
                println!("{}", leave_restart_hint(json_output));
            }
            Ok(())
        }
        FleetCommand::Status => {
            let status: fractal_daemon::api::FleetStatus = client.get("/v1/internal/fleet/status").await?;
            if json_output {
                return print_value(&status, true);
            }
            println!(
                "FLEET  {}   this node: {}",
                status.fleet_id.as_deref().unwrap_or("none"),
                status.node
            );
            println!("TRANSPORTS  {}", status.transports.join(","));
            if let Some(protocol) = &status.fabric_protocol {
                println!("FABRIC-PROTOCOL  {protocol}");
            }
            if let Some(removed) = &status.removed {
                println!("REMOVED  {}", removed.describe(status.fleet_id.as_deref()));
            }
            println!("MEMBER  MODE  STATE  ROUTE-ENDPOINTS");
            for member in &status.view.members {
                let transports = member
                    .endpoints
                    .iter()
                    .filter_map(|endpoint| endpoint["transport"].as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                println!(
                    "{}  {}  {}{}  {}",
                    member.name,
                    member.mode,
                    member.state,
                    member
                        .ended
                        .as_deref()
                        .map(|ended| format!(" ({ended})"))
                        .unwrap_or_default(),
                    if transports.is_empty() {
                        "-".into()
                    } else {
                        transports
                    }
                );
            }
            for peer in &status.peers {
                println!(
                    "PEER  {}  {}{}",
                    peer.peer,
                    peer.status,
                    peer.refusal_reason
                        .as_deref()
                        .or(peer.last_error.as_deref())
                        .map(|error| format!("  {error}"))
                        .unwrap_or_default()
                );
            }
            for invite in &status.invites {
                println!("INVITE  {}  {}", invite.invite, invite.state);
            }
            Ok(())
        }
    }
}

async fn run_fleet_remove(client: &Client, config: &Config, args: FleetRemoveArgs, json_output: bool) -> Result<()> {
    let person = fleet_person(args.actor, config)?;
    let removal: fractal_daemon::store::FleetRemoval = client
        .post(
            "/v1/internal/fleet/remove",
            &fractal_daemon::api::FleetRemoveRequest {
                name: args.name.clone(),
                reason: args.reason,
                person,
            },
        )
        .await?;
    println!(
        "Removed {} (high water {}). Each member refuses it once this removal reaches it.",
        removal.name, removal.high_water
    );
    for invite in &removal.revoked_invites {
        println!("revoked\t{invite}");
    }
    render_removed_member_hint(&mut cli_output::stdout(), &removal.name, json_output)?;
    if fractal_daemon::config::FleetFile::load(&config.state_dir)?.is_some_and(|file| file.legacy_peers) {
        println!("{}", legacy_removal_hint(json_output));
    }
    Ok(())
}

async fn lock_fleet_settings(state_dir: &std::path::Path) -> Result<std::fs::File> {
    let state_dir = state_dir.to_path_buf();
    tokio::task::spawn_blocking(move || fractal_daemon::config::FleetFile::lock_settings(&state_dir)).await?
}

async fn member_endpoints(client: &Client, node: &str) -> Result<Vec<Value>> {
    let view: fractal_daemon::fleet::FleetView = client.get("/v1/internal/fleet/membership").await?;
    let member = view
        .members
        .iter()
        .find(|member| member.name == node)
        .context("this node has no fleet membership yet; wait for its first sync")?;
    anyhow::ensure!(
        member.state == "current",
        "this node is not a current fleet member"
    );
    Ok(member.endpoints.clone())
}

/// Transport changes use mode's atomic settings write and service refresh. They also
/// publish immediately, so removal is announced before unexposure. The caller holds the
/// settings lock until publication and unexposure end.
async fn save_and_publish_fleet_settings(
    client: &Client,
    config: &Config,
    file: &fractal_daemon::config::FleetFile,
    mut endpoints: Vec<Value>,
) -> Result<()> {
    if file.mode == fractal_daemon::config::FleetMode::DialOut {
        endpoints.clear();
    } else {
        endpoints.retain(|endpoint| match endpoint["transport"].as_str() {
            Some("fabric") => file.transports.iter().any(|name| name == "fabric"),
            Some(name @ ("loopback" | "tailscale")) => {
                (file.transports.iter().any(|transport| transport == name)
                    || (name == "loopback" && file.advertise_loopback))
                    && endpoint["address"]
                        .as_str()
                        .and_then(|address| address.parse::<std::net::SocketAddr>().ok())
                        .is_some_and(|address| Some(address.port()) == file.port)
            }
            _ => false,
        });
    }
    file.save(&config.state_dir)?;
    let _: Value = client
        .post(
            "/v1/internal/fleet/endpoints",
            &fractal_daemon::api::FleetEndpointsRequest {
                mode: file.mode.as_str().into(),
                endpoints,
            },
        )
        .await
        .context("fleet.toml saved; endpoint publication did not complete; restart fractal daemon and fractal replication-worker to apply saved relay settings and announce the saved endpoints")?;
    Ok(())
}

fn refresh_fleet_services(no_service: bool, json_output: bool) -> Result<()> {
    if !no_service && services_installed()? {
        fractal_daemon::service::install(Config::load_with_fleet(None)?)?;
    } else {
        print!("{}", fleet_restart_hint(json_output));
    }
    Ok(())
}

async fn run_fleet_transports(
    client: &Client,
    config: &Config,
    command: FleetTransportsCommand,
    json_output: bool,
) -> Result<()> {
    use fractal_daemon::fleet::transport::{Fabric, default_fabric_protocol, resolve_tool};
    let lock = lock_fleet_settings(&config.state_dir).await?;
    let mut file = fractal_daemon::config::FleetFile::load(&config.state_dir)?
        .context("this machine is not a fleet member")?;
    anyhow::ensure!(
        file.removed.is_none(),
        "this member was removed from its fleet"
    );
    let transport = match &command {
        FleetTransportsCommand::Add { transport, .. }
        | FleetTransportsCommand::Remove { transport, .. }
        | FleetTransportsCommand::Prefer { transport, .. } => transport,
    };
    let enabled = file
        .transports
        .iter()
        .any(|configured| configured == transport);
    if matches!(&command, FleetTransportsCommand::Prefer { .. }) {
        anyhow::ensure!(
            enabled,
            "{transport} is not enabled on this member; preference only reorders enabled transports"
        );
        if file.transports.first() == Some(transport) {
            println!("{transport} is already preferred.");
            return Ok(());
        }
    }
    // Re-adding can change local preference, but never reconfigures the exposure.
    if let FleetTransportsCommand::Add {
        prefer, no_service, ..
    } = &command
        && enabled
    {
        if *prefer && file.transports.first() != Some(transport) {
            file.transports.retain(|configured| configured != transport);
            file.transports.insert(0, transport.clone());
            file.save(&config.state_dir)?;
            drop(lock);
            println!("Fabric is now preferred; its existing protocol is unchanged.");
            return refresh_fleet_services(*no_service, json_output);
        }
        println!("Fabric is already enabled; remove it before changing its protocol.");
        return Ok(());
    }
    if matches!(&command, FleetTransportsCommand::Remove { .. }) {
        anyhow::ensure!(enabled, "Fabric is not configured on this member");
        anyhow::ensure!(
            file.transports
                .iter()
                .any(|configured| configured != transport),
            "cannot remove this member's last transport"
        );
    }
    let mut endpoints = member_endpoints(client, &config.node).await?;
    let protocol = file
        .fabric_protocol
        .clone()
        .unwrap_or_else(|| default_fabric_protocol(&file.fleet_id));
    let no_service = match command {
        FleetTransportsCommand::Prefer {
            transport,
            no_service,
        } => {
            let index = file
                .transports
                .iter()
                .position(|configured| configured == &transport)
                .expect("preferred transport was checked as enabled");
            file.transports[..=index].rotate_right(1);
            save_and_publish_fleet_settings(client, config, &file, endpoints).await?;
            println!("{transport} is now preferred; transport exposure is unchanged.");
            no_service
        }
        FleetTransportsCommand::Add {
            fabric_protocol,
            prefer,
            no_service,
            ..
        } => {
            anyhow::ensure!(
                file.mode == fractal_daemon::config::FleetMode::Listening,
                "adding Fabric requires a listening member; run fractal fleet mode listening first"
            );
            let port = file
                .port
                .context("adding Fabric requires this member's configured replication port")?;
            anyhow::ensure!(
                port != 0,
                "adding Fabric requires a fixed nonzero replication port; run fractal fleet mode listening --port PORT first"
            );
            let protocol = fabric_protocol.unwrap_or(protocol);
            anyhow::ensure!(
                !protocol.trim().is_empty(),
                "Fabric protocol must not be empty"
            );
            let fabric = Fabric::new(
                resolve_tool(file.fabric.as_deref(), "fabric")
                    .context("this machine cannot use Fabric: fabric is not installed")?,
            );
            let node = fabric
                .id()
                .await
                .context("this machine cannot use Fabric")?;
            fabric
                .expose(&protocol, &format!("127.0.0.1:{port}"))
                .await
                .context("Fabric exposure failed; fleet settings and endpoints are unchanged")?;
            if prefer {
                file.transports.insert(0, "fabric".into());
            } else {
                file.transports.push("fabric".into());
            }
            file.fabric_protocol = Some(protocol.clone());
            endpoints.retain(|endpoint| endpoint["transport"] != "fabric");
            endpoints.push(json!({"transport":"fabric", "node":node, "protocol":protocol}));
            // A lost publication response can follow a committed claim. Keep the exposure
            // reachable and report the incomplete operation rather than undoing that route.
            save_and_publish_fleet_settings(client, config, &file, endpoints)
                .await
                .with_context(|| format!("Fabric remains exposed as {protocol}"))?;
            println!("Fabric enabled with protocol {protocol}.");
            no_service
        }
        FleetTransportsCommand::Remove { no_service, .. } => {
            // A configured but unavailable carrier cannot provide the promised rollback.
            let local = fractal_daemon::fleet::transport::local_addresses();
            anyhow::ensure!(
                (file.mode == fractal_daemon::config::FleetMode::DialOut
                    && file
                        .transports
                        .iter()
                        .any(|transport| match transport.as_str() {
                            "loopback" => local.iter().any(std::net::IpAddr::is_loopback),
                            "tailscale" =>
                                local.iter().any(fractal_daemon::fleet::transport::is_tailnet_address)
                                    && resolve_tool(file.tailscale.as_deref(), "tailscale")
                                        .is_some(),
                            _ => false,
                        }))
                    || endpoints.iter().any(|endpoint| {
                        let Some(transport) = endpoint["transport"].as_str() else {
                            return false;
                        };
                        transport != "fabric"
                            && file.transports.iter().any(|name| name == transport)
                            && endpoint["address"]
                                .as_str()
                                .and_then(|address| address.parse::<std::net::SocketAddr>().ok())
                                .is_some_and(|address| {
                                    local.contains(&address.ip())
                                        && Some(address.port()) == file.port
                                })
                    }),
                "cannot remove Fabric: no remaining configured transport is usable on this machine"
            );
            file.transports.retain(|transport| transport != "fabric");
            // Retain the existing protocol setting so a later add can restore it.
            save_and_publish_fleet_settings(client, config, &file, endpoints).await?;
            let result = match resolve_tool(file.fabric.as_deref(), "fabric") {
                Some(program) => Fabric::new(program).unexpose(&protocol).await,
                None => Err(anyhow::anyhow!("fabric is not installed")),
            };
            drop(lock);
            // Refresh even when cleanup fails: relay and worker must stop using the carrier.
            refresh_fleet_services(no_service, json_output)?;
            if let Err(error) = result {
                println!(
                    "Fabric endpoint removed; unexposure still needs cleanup: {error:#}. When Fabric is available, run fabric unexpose {protocol}"
                );
            }
            println!(
                "Fabric removed; remaining transports: {}.",
                file.transports.join(",")
            );
            return Ok(());
        }
    };
    drop(lock);
    refresh_fleet_services(no_service, json_output)
}

/// Settings for a migrating node: its existing replication port unless one is given.
fn migration_settings(args: &FleetMemberArgs, config: &Config) -> fractal_daemon::fleet::join::MemberSettings {
    let mut settings = args.settings();
    if settings.port.is_none() {
        settings.port = config
            .peer_listen
            .as_deref()
            .and_then(|address| address.parse::<std::net::SocketAddr>().ok())
            .map(|address| address.port());
    }
    settings
}

async fn run_fleet_migrate(client: &Client, config: &Config, args: FleetMigrateArgs, json_output: bool) -> Result<()> {
    if args.finish || args.unfinish {
        let mut file = fractal_daemon::config::FleetFile::load(&config.state_dir)?
            .context("this machine has not migrated yet")?;
        if args.finish {
            let status: fractal_daemon::api::FleetStatus = client.get("/v1/internal/fleet/status").await?;
            let waiting = config
                .peers
                .iter()
                .filter(|peer| {
                    let known = status
                        .view
                        .members
                        .iter()
                        .any(|member| member.name == peer.name)
                        || status.view.legacy_removed.contains(&peer.name);
                    !known
                })
                .map(|peer| peer.name.clone())
                .collect::<Vec<_>>();
            anyhow::ensure!(
                waiting.is_empty(),
                "these config peers are neither members nor removed yet: {}",
                waiting.join(", ")
            );
        }
        file.legacy_peers = args.unfinish;
        file.save(&config.state_dir)?;
        if args.finish {
            println!(
                "This machine no longer accepts legacy exchanges. Delete these lines from {}:",
                Config::default_path().display()
            );
            println!("  fleet_id, shared_secret_file, peer_listen, and every [[peers]] entry");
        } else {
            println!("This machine accepts legacy exchanges from config peers again.");
        }
        if !args.no_service && services_installed()? {
            fractal_daemon::service::install(Config::load_with_fleet(None)?)?;
        } else {
            println!("{}", migration_restart_hint(true, json_output));
        }
        return Ok(());
    }
    anyhow::ensure!(
        fractal_daemon::config::FleetFile::load(&config.state_dir)?.is_none(),
        "this machine already has fleet membership settings"
    );
    anyhow::ensure!(
        fractal_daemon::store::valid_fleet_node_name(&config.node),
        "`{}` cannot name a fleet member",
        config.node
    );
    let fleet_id = config
        .fleet_id
        .clone()
        .context("this machine has no config-peer fleet to migrate; use fractal fleet join")?;
    // Read only while the daemon may still be running; the locked migration rechecks
    // its store before writing. Invalid admission must not stop healthy services.
    {
        use rusqlite::OptionalExtension as _;
        let database = config.state_dir.join("claims.sqlite3");
        anyhow::ensure!(
            database.exists(),
            "this store is not bound to a fleet yet; run fractal up once first"
        );
        let connection = rusqlite::Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let bound: Option<String> = connection
            .query_row("SELECT value FROM meta WHERE key='fleet_id'", [], |row| {
                row.get(0)
            })
            .optional()?;
        match bound {
            Some(bound) => anyhow::ensure!(
                bound == fleet_id,
                "this store belongs to fleet {bound}, not the configured {fleet_id}"
            ),
            None => anyhow::bail!("this store is not bound to a fleet yet; run fractal up once first"),
        }
    }
    // fleet.toml resolves a relative path under STATE/fleet, and --finish removes the
    // config.toml override, so record the secret file's absolute path now.
    let configured_secret = config
        .shared_secret_file
        .clone()
        .context("config.toml names no shared_secret_file")?;
    let secret_file = fs::canonicalize(&configured_secret).with_context(|| {
        format!(
            "the shared secret file {} is not readable from here; name it with an absolute path in config.toml",
            configured_secret.display()
        )
    })?;
    let use_services = !args.no_service && services_installed()?;
    let state_identity = lock_fleet_admission(client, config, use_services, "migrates").await?;
    let settings = migration_settings(&args.member, config);
    if args.anchor {
        let founded = fractal_daemon::fleet::join::migrate_anchor(
            &config.state_dir,
            &config.node,
            &fleet_id,
            &secret_file,
            &settings,
            args.fabric_protocol.clone(),
            fractal_daemon::store::runtime(),
        )?;
        state_identity.record_fleet_found(&founded)?;
        println!(
            "{} is the anchor of fleet {}. It admits itself and signs its history when st3 starts.",
            founded.node, founded.fleet_id
        );
    } else {
        let (code, code_path) = if args.fabric_inbox {
            let path = wait_for_fabric_inbox_code(&fabric_home()?, FABRIC_INBOX_WAIT, json_output)?;
            (fs::read_to_string(&path)?, Some(path))
        } else if let Some(path) = &args.code_file {
            (fs::read_to_string(path)?, Some(path.clone()))
        } else {
            match args.code.as_deref() {
                Some("-") => {
                    let mut text = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
                    (text, None)
                }
                Some(code) => (code.to_owned(), None),
                None => (read_code_without_echo()?, None),
            }
        };
        let joined = fractal_daemon::fleet::join::join(&fractal_daemon::fleet::join::JoinOptions {
            state_dir: config.state_dir.clone(),
            configured_node: config.node.clone(),
            code: code.trim().to_owned(),
            name: Some(config.node.clone()),
            via: args.via.clone(),
            settings,
            legacy_secret_file: Some(secret_file),
            fabric_protocol: args.fabric_protocol.clone(),
            runtime: fractal_daemon::store::runtime(),
            version: env!("CARGO_PKG_VERSION").into(),
        })
        .await?;
        anyhow::ensure!(
            joined.migrate,
            "that code is a join code; use fractal fleet join"
        );
        state_identity.record_fleet_join(&joined)?;
        if let Some(path) = code_path {
            let _ = fs::remove_file(path);
        }
        println!(
            "{} migrated to membership in fleet {} through {}.",
            joined.name, joined.fleet_id, joined.sponsor
        );
    }
    drop(state_identity);
    if use_services {
        fractal_daemon::service::install(Config::load_with_fleet(None)?)?;
        println!(
            "The st3 services now run as a fleet member, with legacy exchanges still accepted."
        );
    } else {
        println!("{}", migration_restart_hint(false, json_output));
    }
    Ok(())
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Wait for this node's first sync to end, printing progress when `progress` is set. It ends at
/// the first exchange at which this node holds the same envelopes as a peer and has admitted
/// them all. Returns None on timeout.
async fn wait_for_first_sync(
    client: &Client,
    timeout: Duration,
    progress: bool,
) -> Result<Option<fractal_daemon::model::ReplicationFirstSync>> {
    let started = std::time::Instant::now();
    let mut reported = None::<std::time::Instant>;
    loop {
        if let Ok(status) = client
            .get::<ReplicationStatus>("/v1/replication/status")
            .await
        {
            // A removed node never finishes syncing; say why instead of waiting out the timeout.
            if let Some(removed) = &status.removed {
                anyhow::bail!("{}", removed.describe(status.fleet_id.as_deref()));
            }
            let first = status.first_sync.clone().context(
                "this node has no first sync to wait for: it did not join with fractal fleet join",
            )?;
            match first.state.as_str() {
                "verified" | "failed" => return Ok(Some(first)),
                _ => {
                    if progress && reported.is_none_or(|at| at.elapsed() >= Duration::from_secs(10))
                    {
                        reported = Some(std::time::Instant::now());
                        let behind = status
                            .peers
                            .iter()
                            .filter_map(|peer| {
                                let sync = peer.sync.as_ref()?;
                                Some(format!(
                                    "{} has {} this node lacks",
                                    peer.peer,
                                    envelope_count(sync.peer_only_envelopes)
                                ))
                            })
                            .collect::<Vec<_>>();
                        println!(
                            "first sync: {} envelopes so far{}",
                            status.received_envelopes,
                            if behind.is_empty() {
                                String::new()
                            } else {
                                format!("; {}", behind.join("; "))
                            }
                        );
                    }
                }
            }
        }
        if started.elapsed() >= timeout {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Wait until [`caught_up_since`] holds, printing progress when `progress` is set. Returns what
/// is still missing at the deadline.
async fn wait_for_caught_up(
    client: &Client,
    since: u128,
    timeout: Duration,
    progress: bool,
) -> Result<Result<CaughtUp, String>> {
    let started = std::time::Instant::now();
    let mut reported = None::<std::time::Instant>;
    let mut missing = "replication status did not answer".to_owned();
    loop {
        if let Ok(status) = client
            .get::<ReplicationStatus>("/v1/replication/status")
            .await
        {
            if let Some(removed) = &status.removed {
                anyhow::bail!("{}", removed.describe(status.fleet_id.as_deref()));
            }
            match caught_up_since(&status, since) {
                Ok(caught_up) => return Ok(Ok(caught_up)),
                Err(waiting) => missing = waiting,
            }
            if progress && reported.is_none_or(|at| at.elapsed() >= Duration::from_secs(10)) {
                reported = Some(std::time::Instant::now());
                println!("catching up: {missing}");
            }
        }
        if started.elapsed() >= timeout {
            return Ok(Err(missing));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Print how a first sync ended, and fail when an earlier build recorded it as failed.
fn report_first_sync(first: &fractal_daemon::model::ReplicationFirstSync, json_output: bool) -> Result<()> {
    if json_output {
        print_value(first, true)?;
        anyhow::ensure!(first.state == "verified", "the first sync failed");
        return Ok(());
    }
    anyhow::ensure!(
        first.state == "verified",
        "{}\nfractal replication status shows this node's sync with each peer; report it with fractal \
         diagnostic.",
        render_first_sync(first, now_ms())
    );
    println!("{}", render_first_sync(first, now_ms()));
    Ok(())
}

#[cfg(test)]
mod output_tests {
    use super::*;

    #[test]
    fn fabric_invite_join_and_migrate_preserve_json_stdout() {
        for command in ["join", "migrate"] {
            for (json_output, program) in [(true, "st"), (false, "fractal")] {
                let mut output = Vec::new();
                render_fabric_invite_sent(
                    &mut output, "fleet-invite/example", "other", command, json_output,
                ).unwrap();
                assert_eq!(
                    String::from_utf8(output).unwrap(),
                    format!("Sent invite fleet-invite/example to other's Fabric inbox. On other, run:\n  {program} fleet {command} --fabric-inbox\n")
                );
            }
        }
    }

    #[test]
    fn offline_leave_preserves_json_stdout() {
        for (json_output, program) in [(true, "st"), (false, "fractal")] {
            let mut output = Vec::new();
            render_offline_leave(&mut output, "local", json_output).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!("Left without reaching a member. On another member run: {program} fleet remove local --reason \"left offline\"\n")
            );
        }
    }

    #[test]
    fn leave_without_services_preserves_json_stdout() {
        assert_eq!(leave_restart_hint(true), "Stop st3 replication-worker; st3 up now runs local-only after a restart.");
        assert_eq!(leave_restart_hint(false), "Stop fractal replication-worker; fractal up now runs local-only after a restart.");
    }

    #[test]
    fn remove_preserves_json_stdout() {
        for (json_output, program) in [(true, "st"), (false, "fractal")] {
            let mut output = Vec::new();
            render_removed_member_hint(&mut output, "other", json_output).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!("Writes other made after this node's last exchange with it are not accepted. On other, if it still runs: {program} uninstall\n")
            );
        }
    }

    #[test]
    fn remove_with_legacy_peers_preserves_json_stdout() {
        assert_eq!(legacy_removal_hint(true), "This member still accepts legacy exchanges from config peers, so a machine that keeps the fleet secret can pose as one of them until st fleet migrate --finish.");
        assert_eq!(legacy_removal_hint(false), "This member still accepts legacy exchanges from config peers, so a machine that keeps the fleet secret can pose as one of them until fractal fleet migrate --finish.");
    }

    #[test]
    fn mode_and_transport_refresh_without_services_preserves_json_stdout() {
        assert_eq!(
            fleet_restart_hint(true),
            "Restart st3 daemon and st3 replication-worker for the change to take effect.\nThe old worker no longer publishes endpoints after its settings change; restart it to resume publication.\n"
        );
        assert_eq!(
            fleet_restart_hint(false),
            "Restart fractal daemon and fractal replication-worker for the change to take effect.\nThe old worker no longer publishes endpoints after its settings change; restart it to resume publication.\n"
        );
    }

    #[test]
    fn migrate_finish_and_unfinish_without_services_preserve_json_stdout() {
        assert_eq!(migration_restart_hint(true, true), "Restart st3 replication-worker for this to take effect.");
        assert_eq!(migration_restart_hint(true, false), "Restart fractal replication-worker for this to take effect.");
    }

    #[test]
    fn migrate_anchor_and_join_without_services_preserve_json_stdout() {
        assert_eq!(migration_restart_hint(false, true), "Start st3 up and st3 replication-worker; legacy exchanges stay accepted until st fleet migrate --finish.");
        assert_eq!(migration_restart_hint(false, false), "Start fractal up and fractal replication-worker; legacy exchanges stay accepted until fractal fleet migrate --finish.");
    }
}

#[cfg(test)]
mod invite_route_tests {
    use super::*;

    fn endpoints(transports: &[&str]) -> Vec<Value> {
        transports.iter().map(|transport| json!({ "transport": transport })).collect()
    }

    #[test]
    fn auto_routes_around_fabric_without_pairing() {
        let both = endpoints(&["fabric", "tailscale", "loopback"]);
        assert_eq!(invite_via("auto", &both, true).unwrap(), "auto");
        assert_eq!(invite_via("auto", &both, false).unwrap(), "tailscale");
        assert_eq!(
            invite_via("auto", &endpoints(&["fabric", "loopback"]), false).unwrap(),
            "loopback"
        );
        assert_eq!(invite_via("auto", &endpoints(&["tailscale"]), false).unwrap(), "auto");
        assert_eq!(invite_via("tailscale", &both, false).unwrap(), "tailscale");
    }

    #[test]
    fn an_invitation_only_fabric_could_carry_needs_pairing() {
        for (via, advertised) in [("auto", &["fabric"][..]), ("fabric", &["fabric", "tailscale"])] {
            let error = invite_via(via, &endpoints(advertised), false).unwrap_err();
            assert_eq!(error.to_string(), FABRIC_PAIRING_UNAVAILABLE);
        }
        assert_eq!(invite_via("fabric", &endpoints(&["fabric"]), true).unwrap(), "fabric");
    }

    #[test]
    fn code_file_is_private_and_leaves_its_directory_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let path = root.path().join("invite.code");
        fs::write(&path, "stale").unwrap();
        write_code_file(&path, b"code").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"code");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(root.path()).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1, "no temporary file remains");
        let missing = root.path().join("missing").join("invite.code");
        assert!(write_code_file(&missing, b"code").is_err());
    }

    #[test]
    fn fabric_inbox_wakes_for_a_code_delivered_after_it_began_waiting() {
        let home = tempfile::tempdir().unwrap();
        let sender = home.path().join("inbox").join("sponsor-id");
        let code = sender.join("st-fleet-join-1.code");
        let delivery = std::thread::spawn({
            let code = code.clone();
            move || {
                std::thread::sleep(Duration::from_millis(200));
                // As Fabric receives: the sender's directory, a partial file, then the rename.
                fs::create_dir_all(&sender).unwrap();
                let partial = code.with_extension("fabric-partial");
                fs::write(&partial, "code").unwrap();
                fs::rename(&partial, &code).unwrap();
            }
        });
        let began = std::time::Instant::now();
        let found = wait_for_fabric_inbox_code(home.path(), Duration::from_secs(60), true).unwrap();
        delivery.join().unwrap();
        assert_eq!(found, code);
        assert!(began.elapsed() < Duration::from_secs(30), "woken by the deadline, not the watch");
    }

    #[test]
    fn fabric_inbox_wait_ends_at_its_deadline() {
        let home = tempfile::tempdir().unwrap();
        let error = wait_for_fabric_inbox_code(home.path(), Duration::from_millis(100), true)
            .unwrap_err();
        assert!(error.to_string().contains("no join code arrived"), "{error:#}");

        let missing = home.path().join("missing");
        let error = wait_for_fabric_inbox_code(&missing, Duration::from_secs(60), true).unwrap_err();
        assert!(error.to_string().contains("no Fabric home"), "{error:#}");
    }

    #[test]
    fn a_watch_error_ends_the_fabric_inbox_wait() {
        let (events, woken) = std::sync::mpsc::channel();
        events.send(Ok(notify::Event::default())).unwrap();
        events.send(Err(notify::Error::generic("inotify watch limit reached"))).unwrap();
        let began = std::time::Instant::now();
        let error = await_fabric_inbox_event(&woken, Duration::from_secs(60)).unwrap_err();
        assert!(began.elapsed() < Duration::from_secs(30));
        assert!(format!("{error:#}").contains("inotify watch limit reached"), "{error:#}");

        events.send(Ok(notify::Event::default())).unwrap();
        await_fabric_inbox_event(&woken, Duration::from_secs(60)).unwrap();
        drop(events);
        let error = await_fabric_inbox_event(&woken, Duration::from_secs(60)).unwrap_err();
        assert!(error.to_string().contains("stopped"), "{error:#}");
    }
}
