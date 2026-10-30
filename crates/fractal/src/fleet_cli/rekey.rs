//! Prepare and publish through the old route before guarded Fabric activation.
use super::*;
use fractal_daemon::cli_println as println;
use serde::{Deserialize, Serialize};
use std::path::Path;

const OVERLAP_SECONDS: u64 = 600;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Committed,
    Activating,
    Complete,
    Aborted,
}

#[derive(Debug, Deserialize, Serialize)]
struct Progress {
    fleet_id: String,
    member_key: String,
    old_node: String,
    next_node: String,
    deadline: u64,
    phase: Phase,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn save(path: &Path, progress: &Progress) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let parent = path.parent().context("rekey journal has no parent")?;
    let temporary = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true).mode(0o600).open(&temporary)?;
    file.write_all(&serde_json::to_vec(progress)?)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn load(path: &Path) -> Result<Option<Progress>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).context("read Fabric rekey journal")?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn fabric_json(program: &Path, args: &[&str]) -> Result<Value> {
    // kill_on_drop bounds a hung subprocess without touching the supervised daemon.
    let output = tokio::time::timeout(COMMAND_TIMEOUT,
        tokio::process::Command::new(program).args(args).kill_on_drop(true).output())
        .await.context("Fabric identity command timed out; retry the same rekey command")??;
    anyhow::ensure!(output.status.success(), "Fabric identity command failed: {}{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).context("Fabric identity command returned malformed JSON")
}

fn identity(value: &Value, field: &str) -> Result<String> {
    let node = value[field].as_str().context("Fabric identity response has no NodeID")?;
    anyhow::ensure!(node.len() == 64 && node.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Fabric identity response has an invalid NodeID");
    Ok(node.to_owned())
}

async fn publish(client: &Client, progress: &Progress, action: &str) -> Result<()> {
    let _: Value = client.post("/v1/internal/fleet/fabric-rekey", &json!({
        "action":action, "old_node":progress.old_node, "next_node":progress.next_node,
        "deadline":progress.deadline,
    })).await?;
    Ok(())
}

fn confirmed(view: &fractal_daemon::fleet::FleetView, progress: &Progress, state: &str) -> bool {
    view.members.iter().any(|member| {
        member.state == "current" && member.member_key != progress.member_key
            && member.endpoints.iter().any(|endpoint| {
                endpoint["transport"] == "fabric-rekey-receipt"
                    && endpoint["member_key"] == progress.member_key
                    && endpoint["old_node"] == progress.old_node
                    && endpoint["next_node"] == progress.next_node
                    && endpoint["deadline"] == progress.deadline
                    && endpoint["state"] == state
            })
    })
}

async fn wait_for_receipt(client: &Client, progress: &Progress, state: &str) -> Result<bool> {
    loop {
        let remaining = progress.deadline.saturating_sub(now());
        if remaining == 0 { return Ok(false); }
        // A slow local API must not extend the authorization window.
        let view = tokio::time::timeout(Duration::from_secs(remaining),
            client.get::<fractal_daemon::fleet::FleetView>("/v1/internal/fleet/membership")).await;
        match view {
            Ok(Ok(view)) if confirmed(&view, progress, state) && now() < progress.deadline => return Ok(true),
            Ok(Ok(_)) => {},
            Ok(Err(error)) => return Err(error.context("read signed adapter receipts; retry rekey to resume")),
            Err(_) => return Ok(false),
        }
        tokio::time::sleep(Duration::from_secs(1).min(Duration::from_secs(progress.deadline.saturating_sub(now())))).await;
    }
}

async fn abort(client: &Client, path: &Path, progress: &mut Progress) -> Result<()> {
    // Publish before marking terminal: a lost reply is retried, never reported as a rollback.
    publish(client, progress, "abort").await.context("could not publish rekey rollback; old Fabric identity is unchanged; retry rekey")?;
    progress.phase = Phase::Aborted;
    save(path, progress)?;
    anyhow::bail!("no peer confirmed Fabric rekey before the ten-minute deadline; aborted, old identity remains active")
}

pub(super) async fn run(client: &Client, config: &Config, json_output: bool) -> Result<()> {
    use fractal_daemon::fleet::transport::resolve_tool;
    let _lock = lock_fleet_settings(&config.state_dir).await?;
    let file = fractal_daemon::config::FleetFile::load(&config.state_dir)?
        .context("this machine is not a fleet member")?;
    anyhow::ensure!(file.transports.iter().any(|transport| transport == "fabric"),
        "this machine is not a Fabric fleet member");
    let view: fractal_daemon::fleet::FleetView = client.get("/v1/internal/fleet/membership").await?;
    let member = view.members.iter().find(|member| member.name == config.node && member.state == "current")
        .context("this machine is not a current fleet member")?;
    anyhow::ensure!(member.endpoints.iter().any(|endpoint| endpoint["transport"] == "fabric"),
        "this member has no published Fabric endpoint");
    let program = resolve_tool(file.fabric.as_deref(), "fabric").context("fabric is not installed")?;
    let status = fabric_json(&program, &["identity", "status", "--json"]).await
        .context("Fabric is too old or unavailable: install the Fabric v0.9.4 identity lifecycle API")?;
    let current = identity(&status, "current")?;
    let path = config.state_dir.join("fleet/fabric-rekey.json");
    let saved = load(&path)?;
    let mut progress = match saved {
        Some(progress) if !matches!(progress.phase, Phase::Complete | Phase::Aborted) => {
            anyhow::ensure!(progress.fleet_id == file.fleet_id && progress.member_key == member.member_key,
                "Fabric rekey journal belongs to another fleet member");
            anyhow::ensure!(current == progress.old_node || current == progress.next_node,
                "Fabric identity changed outside this rekey; refusing to overwrite durable progress");
            progress
        },
        _ => {
            anyhow::ensure!(member.endpoints.iter().any(|endpoint| endpoint["transport"] == "fabric" && endpoint["node"] == current),
                "published Fabric identity does not match the local identity");
            let prepared = fabric_json(&program, &["identity", "prepare", "--json"]).await?;
            let old_node = identity(&prepared, "current")?;
            let next_node = identity(&prepared, "next")?;
            anyhow::ensure!(old_node == current && old_node != next_node, "Fabric prepared an unexpected identity");
            let progress = Progress { fleet_id:file.fleet_id.clone(), member_key:member.member_key.clone(),
                old_node, next_node, deadline:now() + OVERLAP_SECONDS, phase:Phase::Prepared };
            save(&path, &progress)?;
            progress
        },
    };
    if current == progress.next_node {
        anyhow::ensure!(matches!(progress.phase, Phase::Activating), "new identity active without durable activation decision");
    } else if progress.phase == Phase::Prepared {
        if now() >= progress.deadline { return abort(client, &path, &mut progress).await; }
        publish(client, &progress, "prepare").await?;
        if !wait_for_receipt(client, &progress, "prepared").await? {
            return abort(client, &path, &mut progress).await;
        }
        // Save intent before publication; retrying commit after a lost reply is safe.
        progress.phase = Phase::Committed;
        save(&path, &progress)?;
    }
    if progress.phase == Phase::Committed {
        if now() >= progress.deadline { return abort(client, &path, &mut progress).await; }
        publish(client, &progress, "commit").await?;
        if !wait_for_receipt(client, &progress, "committed").await? {
            return abort(client, &path, &mut progress).await;
        }
        // From here the next identity is permanently authorized by a peer. Never
        // roll back on an uncertain activate reply: Fabric's guards reconcile it.
        progress.phase = Phase::Activating;
        save(&path, &progress)?;
    }
    let activated = fabric_json(&program, &["identity", "activate", "--expect-current", &progress.old_node,
        "--expect-next", &progress.next_node, "--json"]).await
        .context("Fabric activation incomplete; rerun fleet transports rekey fabric to resume the guarded activation")?;
    anyhow::ensure!(identity(&activated, "active")? == progress.next_node, "Fabric activated an unexpected identity");
    progress.phase = Phase::Complete;
    save(&path, &progress)?;
    if json_output {
        println!("{}", json!({"state":"complete","previous":progress.old_node,"active":progress.next_node,"deadline":progress.deadline}));
    } else {
        println!("Fabric re-keyed in place: {} -> {}. Offline peers recover when signed membership converges.", progress.old_node, progress.next_node);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::{get, post}};
    use std::{os::unix::fs::PermissionsExt, sync::{Arc, Mutex}};

    const OLD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const NEXT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const FLEET: &str = "68d50a41-99c6-443a-8791-b09fa2b8e468";

    fn progress(phase: Phase, deadline: u64) -> Progress {
        Progress { fleet_id:FLEET.into(), member_key:"own-key".into(),
            old_node:OLD.into(), next_node:NEXT.into(), deadline, phase }
    }

    fn membership(rotation: Option<&Value>) -> Value {
        let node = if rotation.is_some_and(|entry| entry["action"] == "commit") { NEXT } else { OLD };
        let receipts = rotation.filter(|entry| entry["action"] != "abort").map(|entry| json!([{
            "transport":"fabric-rekey-receipt", "member_key":"own-key", "old_node":OLD,
            "next_node":NEXT, "deadline":entry["deadline"],
            "state":if entry["action"] == "commit" {"committed"} else {"prepared"},
        }])).unwrap_or_else(|| json!([]));
        json!({"anchor":"anchor","members":[
            {"name":"local","member_key":"own-key","state":"current","mode":"listening","start":1,
                "endpoints":[{"transport":"fabric","node":node,"protocol":"st3/fleet/fleet"}]},
            {"name":"remote","member_key":"remote-key","state":"current","mode":"listening","start":1,"endpoints":receipts}
        ]})
    }

    struct Fixture {
        root: tempfile::TempDir,
        config: Config,
        client: Client,
        requests: Arc<Mutex<Vec<Value>>>,
        server: tokio::task::JoinHandle<()>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) { self.server.abort(); }
    }
    impl Fixture {
        async fn new(fail_activation: bool) -> Self {
            let root = tempfile::tempdir().unwrap();
            let program = root.path().join("fabric");
            // A real subprocess boundary with durable active identity and a lost
            // activation reply; no process-global environment changes.
            std::fs::write(&program, format!(r#"#!/bin/sh
echo "$*" >> '{dir}/calls'
case "$2" in
status)
  if test -f '{dir}/active'; then id='{next}'; else id='{old}'; fi
  printf '{{"current":"%s","state":"active"}}\n' "$id";;
prepare) printf '{{"current":"{old}","next":"{next}"}}\n';;
activate)
  if test '{fail}' = true && ! test -f '{dir}/active'; then
    touch '{dir}/active'
    exit 1
  fi
  touch '{dir}/active'
  printf '{{"previous":"{old}","active":"{next}","outcome":"already_active"}}\n';;
*) exit 2;;
esac
"#, dir=root.path().display(), old=OLD, next=NEXT, fail=fail_activation)).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            let config = Config { node:"local".into(), state_dir:root.path().to_path_buf(), ..Config::default() };
            fractal_daemon::config::FleetFile {
                fleet_id:FLEET.into(), node:Some("local".into()),
                transports:vec!["fabric".into()], fabric:Some(program), ..Default::default()
            }.save(root.path()).unwrap();
            let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
            let reads = requests.clone();
            let writes = requests.clone();
            let app = Router::new()
                .route("/v1/internal/fleet/membership", get(move || {
                    let reads = reads.clone();
                    async move { Json(membership(reads.lock().unwrap().last())) }
                }))
                .route("/v1/internal/fleet/fabric-rekey", post(move |Json(request):Json<Value>| {
                    let writes = writes.clone();
                    async move { writes.lock().unwrap().push(request); Json(json!({"published":true})) }
                }));
            let socket = root.path().join("api.sock");
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            let client = Client::unix(socket);
            Self { root, config, client, requests, server }
        }
    }

    #[tokio::test]
    async fn rekey_flow_prepares_publishes_both_phases_then_activates() {
        let fixture = Fixture::new(false).await;
        run(&fixture.client, &fixture.config, true).await.unwrap();
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.iter().map(|request| request["action"].as_str().unwrap()).collect::<Vec<_>>(), ["prepare","commit"]);
        assert_eq!(requests[0]["deadline"], requests[1]["deadline"]);
        let saved = load(&fixture.root.path().join("fleet/fabric-rekey.json")).unwrap().unwrap();
        assert_eq!(saved.phase, Phase::Complete);
        let calls = std::fs::read_to_string(fixture.root.path().join("calls")).unwrap();
        assert!(calls.contains("identity prepare --json"));
        assert!(calls.contains(&format!("identity activate --expect-current {OLD} --expect-next {NEXT} --json")));
    }

    #[tokio::test]
    async fn rekey_lost_activation_reply_resumes_without_preparing_or_extending_deadline() {
        let fixture = Fixture::new(true).await;
        assert!(run(&fixture.client, &fixture.config, true).await.is_err());
        let path = fixture.root.path().join("fleet/fabric-rekey.json");
        let saved = load(&path).unwrap().unwrap();
        assert_eq!(saved.phase, Phase::Activating);
        run(&fixture.client, &fixture.config, true).await.unwrap();
        let resumed = load(&path).unwrap().unwrap();
        assert_eq!(resumed.phase, Phase::Complete);
        assert_eq!(resumed.deadline, saved.deadline);
        assert_eq!(fixture.requests.lock().unwrap().len(), 2);
        let calls = std::fs::read_to_string(fixture.root.path().join("calls")).unwrap();
        assert_eq!(calls.matches("identity prepare").count(), 1);
        assert_eq!(calls.matches("identity activate").count(), 2);
    }

    #[tokio::test]
    async fn rekey_elapsed_unpublished_journal_aborts_without_activation() {
        let fixture = Fixture::new(false).await;
        let path = fixture.root.path().join("fleet/fabric-rekey.json");
        save(&path, &progress(Phase::Prepared, now() - 1)).unwrap();
        let error = run(&fixture.client, &fixture.config, true).await.unwrap_err();
        assert!(error.to_string().contains("old identity remains active"));
        assert_eq!(load(&path).unwrap().unwrap().phase, Phase::Aborted);
        assert_eq!(fixture.requests.lock().unwrap()[0]["action"], "abort");
        assert!(!std::fs::read_to_string(fixture.root.path().join("calls")).unwrap().contains("activate"));
    }

    #[tokio::test]
    async fn rekey_refuses_old_fabric_before_preparing() {
        let fixture = Fixture::new(false).await;
        let program = fixture.root.path().join("fabric");
        std::fs::write(&program, "#!/bin/sh\nexit 2\n").unwrap();
        let error = run(&fixture.client, &fixture.config, true).await.unwrap_err();
        assert!(error.to_string().contains("Fabric is too old"));
        assert!(fixture.requests.lock().unwrap().is_empty());
        assert!(!fixture.root.path().join("fleet/fabric-rekey.json").exists());
    }

    #[tokio::test]
    async fn rekey_refuses_non_fabric_member_without_running_fabric() {
        let fixture = Fixture::new(false).await;
        let mut file = fractal_daemon::config::FleetFile::load(fixture.root.path()).unwrap().unwrap();
        file.transports = vec!["tailscale".into()];
        file.save(fixture.root.path()).unwrap();
        let error = run(&fixture.client, &fixture.config, true).await.unwrap_err();
        assert!(error.to_string().contains("not a Fabric fleet member"));
        assert!(!fixture.root.path().join("calls").exists());
        assert!(fixture.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn rekey_receipts_require_remote_current_exact_phase_and_identity() {
        let progress = progress(Phase::Prepared, now() + 100);
        let request = json!({"action":"prepare","deadline":progress.deadline});
        let mut view:fractal_daemon::fleet::FleetView = serde_json::from_value(membership(Some(&request))).unwrap();
        assert!(confirmed(&view, &progress, "prepared"));
        assert!(!confirmed(&view, &progress, "committed"));
        view.members[1].endpoints[0]["deadline"] = json!(progress.deadline + 1);
        assert!(!confirmed(&view, &progress, "prepared"));
        view.members[1].endpoints[0]["deadline"] = json!(progress.deadline);
        view.members[1].state = "ended".into();
        assert!(!confirmed(&view, &progress, "prepared"));
        view.members[1].state = "current".into();
        view.members[1].member_key = progress.member_key.clone();
        assert!(!confirmed(&view, &progress, "prepared"));
    }
}
