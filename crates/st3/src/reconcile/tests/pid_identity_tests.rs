//! Isolated native PTY records must never give control over a reused PID or TID.
use super::*;
use std::io::{BufRead as _, Read as _, Write as _};
use std::os::unix::net::UnixListener;
use std::process::{Child, Command, Stdio};

const BOOT_TAG: &str = "st3.boot-id";

#[test]
fn identity_bystander() {
    if std::env::var_os("ST_PID_IDENTITY_BYSTANDER").is_none() {
        return;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        sender
            .send(unsafe { libc::syscall(libc::SYS_gettid) } as u32)
            .unwrap();
        loop {
            std::thread::park();
        }
    });
    let tid = receiver.recv().unwrap();
    println!("identity-ready {} {tid}", std::process::id());
    std::io::stdout().flush().unwrap();
    // The parent keeps stdin open until the fixture ends. There is no timed
    // process discovery and this helper can never name a fleet process.
    std::io::stdin().read_to_end(&mut Vec::new()).unwrap();
}

struct Bystander(Child, u32, u32);
impl Bystander {
    fn new() -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "reconcile::tests::pid_identity_tests::identity_bystander",
                "--nocapture",
            ])
            .env("ST_PID_IDENTITY_BYSTANDER", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let ids = std::io::BufReader::new(output)
            .lines()
            .find_map(|line| {
                line.unwrap()
                    .split_once("identity-ready ")
                    .map(|(_, ids)| ids.to_owned())
            })
            .unwrap();
        let fields: Vec<_> = ids.split_whitespace().collect();
        let pid = fields[0].parse().unwrap();
        let tid = fields[1].parse().unwrap();
        assert_ne!(pid, tid);
        Self(child, pid, tid)
    }
}
impl Drop for Bystander {
    fn drop(&mut self) {
        // Only this test's owned helper is killed, including on assertion failure.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn native_snapshot_and_control_reject_stale_and_unconfirmable_pty_owners() {
    let mut bystander = Bystander::new();
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .unwrap()
        .trim()
        .to_owned();
    for case in [
        "reused-pid",
        "nonleader",
        "boot-mismatch",
        "missing-boot",
        "unreadable-record",
    ] {
        let root = tempfile::tempdir().unwrap();
        let registry = root.path().join("pty");
        std::fs::create_dir(&registry).unwrap();
        let pid = if case == "nonleader" {
            bystander.2
        } else {
            bystander.1
        };
        // The owner is gone: a socket entry survives but connections are refused.
        drop(UnixListener::bind(registry.join("orchid.worker.sock")).unwrap());
        std::fs::write(registry.join("orchid.worker.pid"), pid.to_string()).unwrap();
        let mut record = serde_json::json!({
            "generation": "old-generation", "createdAt": "old", "daemonPid": pid,
            "daemonStartToken": format!("linux:{}", st_runtime::process_start_token(pid).unwrap()),
            "tags": {},
        });
        record["tags"][BOOT_TAG] = Value::String(boot.clone());
        match case {
            "reused-pid" => record["daemonStartToken"] = Value::String("linux:0".into()),
            "boot-mismatch" => {
                record["tags"][BOOT_TAG] =
                    Value::String("00000000-0000-0000-0000-000000000000".into())
            }
            "missing-boot" => {
                record["tags"].as_object_mut().unwrap().remove(BOOT_TAG);
            }
            _ => {}
        }
        let metadata = registry.join("orchid.worker.json");
        std::fs::write(&metadata, record.to_string()).unwrap();
        if case == "unreadable-record" {
            std::fs::remove_file(&metadata).unwrap();
            std::fs::create_dir(&metadata).unwrap();
        }
        let runtime = NativeRuntime::new(root.path(), None, Path::new("unused-producer"));
        let observed = runtime.snapshot_ptys().unwrap().remove(0);
        assert_eq!(
            observed.status,
            if matches!(case, "missing-boot" | "unreadable-record") {
                "unknown"
            } else {
                "vanished"
            },
            "{case}"
        );
        let expected = if case == "unreadable-record" {
            None
        } else {
            Some(format!("{pid}:old"))
        };
        assert!(
            runtime
                .stop("orchid.worker", true, expected.as_deref())
                .is_err(),
            "{case}"
        );
        assert!(
            bystander.0.try_wait().unwrap().is_none(),
            "{case}: stop signalled the bystander"
        );
        assert!(
            runtime
                .kill("orchid.worker", true, expected.as_deref())
                .is_err(),
            "{case}"
        );
        assert!(
            bystander.0.try_wait().unwrap().is_none(),
            "{case}: kill signalled the bystander"
        );
        assert!(
            registry.join("orchid.worker.pid").exists(),
            "{case}: refusal retired the registry"
        );
    }
}

#[tokio::test]
async fn isolated_native_reconciler_replaces_a_lost_owner_without_signalling_the_bystander() {
    use std::os::unix::fs::PermissionsExt as _;
    let mut bystander = Bystander::new();
    let root = tempfile::tempdir().unwrap();
    let registry = root.path().join("pty");
    std::fs::create_dir(&registry).unwrap();
    drop(UnixListener::bind(registry.join("orchid.worker.sock")).unwrap());
    std::fs::write(registry.join("orchid.worker.pid"), bystander.1.to_string()).unwrap();
    std::fs::write(
        registry.join("orchid.worker.json"),
        serde_json::json!({
            "generation":"lost-generation", "createdAt":"old", "daemonPid":bystander.1,
            "daemonStartToken":"linux:0",
            "tags": {"st3.boot-id": std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim()},
        })
        .to_string(),
    )
    .unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let producer = bin.join("pty");
    // The producer publishes only after exact-record retirement. It adopts
    // the owned helper as this fixture's new daemon and copies the launch
    // evidence exactly as the real producer's metadata publication does.
    std::fs::write(&producer, format!(r#"#!/bin/sh
test "$1" = run || exit 90
shift
boot= operation= id=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --id) id="$2"; shift 2 ;;
    --tag)
      case "$2" in
        st3.boot-id=*) boot="${{2#st3.boot-id=}}" ;;
        st3.start-operation=*) operation="${{2#st3.start-operation=}}" ;;
      esac
      shift 2 ;;
    --) break ;;
    *) shift ;;
  esac
done
test ! -e "$PTY_ROOT/$id.pid" || exit 91
test ! -e "$PTY_ROOT/$id.json" || exit 92
printf '%s' '{pid}' > "$PTY_ROOT/$id.pid"
printf '{{"generation":"replacement-generation","createdAt":"replacement","daemonPid":{pid},"daemonStartToken":"linux:{token}","tags":{{"st3.boot-id":"%s","st3.start-operation":"%s"}}}}' "$boot" "$operation" > "$PTY_ROOT/$id.json"
: > "$PTY_ROOT/$id.sock"
"#, pid=bystander.1, token=st_runtime::process_start_token(bystander.1).unwrap())).unwrap();
    std::fs::set_permissions(&producer, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Arc::new(Store::open(&root.path().join("claims.db"), "node").unwrap());
    apply_source(
        &store,
        &format!(
            "version 2\nagent \"orchid/worker\" {{ workspace {:?}; command \"true\"; restart \"always\"; env {{ PATH {:?}; }} }}",
            root.path().display().to_string(),
            format!("{}:${{PATH}}", bin.display()),
        ),
        "identity-replacement",
    );
    // Native launches resolve `pty` from the member's declared environment,
    // independently of the constructor's observation/control binary. Ensure
    // this test cannot accidentally launch the installed producer.
    let desired = store
        .desired_subjects_named(&["agent/orchid/worker".into()])
        .unwrap()
        .remove(0);
    let member = desired.member.as_ref().unwrap();
    assert!(member.terminal);
    let environment =
        member_environment(&member.environment, &launch_executable().unwrap(), None).unwrap();
    assert_eq!(
        st_runtime::resolve_executable("pty", &environment).unwrap(),
        producer
    );
    let runtime = Arc::new(NativeRuntime::new(root.path(), None, &producer));
    let reconciler = Arc::new(Reconciler::new(
        store.clone(),
        runtime.clone(),
        "node".into(),
        Arc::new(Notify::new()),
    ));
    reconciler.reconcile_once().unwrap();
    let live = reconciler.start_jobs.live.clone();
    tokio::time::timeout(
        Duration::from_secs(30),
        tokio::task::spawn_blocking(move || {
            let mut jobs = live.0.lock().unwrap();
            while !jobs.is_empty() {
                jobs = live.1.wait(jobs).unwrap();
            }
        }),
    )
    .await
    .unwrap()
    .unwrap();
    reconciler.reconcile_once().unwrap();
    let record: Value =
        serde_json::from_slice(&std::fs::read(registry.join("orchid.worker.json")).unwrap())
            .unwrap();
    assert_eq!(record["generation"], "replacement-generation");
    let replacement = runtime.snapshot_ptys().unwrap().remove(0);
    assert_eq!(replacement.status, "running");
    assert_eq!(
        replacement.incarnation_id,
        Some(format!("{}:replacement", bystander.1))
    );
    assert!(
        bystander.0.try_wait().unwrap().is_none(),
        "replacement signalled the reused PID owner"
    );
    let observations = store
        .observations_for("agent/orchid/worker", "runtime.observed")
        .unwrap();
    assert!(
        observations
            .iter()
            .any(|claim| claim.body["fields"]["status"] == "vanished"),
        "{observations:?}"
    );
    assert!(
        store
            .observations_for("agent/orchid/worker", "runtime.action.failed")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn explicit_legacy_stop_names_missing_proof_without_signals_or_timeout_escalation() {
    let mut bystander = Bystander::new();
    let root = tempfile::tempdir().unwrap();
    let registry = root.path().join("pty");
    std::fs::create_dir(&registry).unwrap();
    drop(UnixListener::bind(registry.join("orchid.worker.sock")).unwrap());
    std::fs::write(registry.join("orchid.worker.pid"), bystander.1.to_string()).unwrap();
    let record = serde_json::json!({"createdAt":"old", "daemonPid":bystander.1});
    std::fs::write(registry.join("orchid.worker.json"), record.to_string()).unwrap();
    let store = Arc::new(Store::open(&root.path().join("claims.db"), "node").unwrap());
    apply_source(
        &store,
        "version 2\nstop \"agent/orchid/worker\"",
        "explicit-legacy-stop",
    );
    let runtime = Arc::new(NativeRuntime::new(
        root.path(),
        None,
        Path::new("unused-producer"),
    ));
    let reconciler = Reconciler::new(
        store.clone(),
        runtime.clone(),
        "node".into(),
        Arc::new(Notify::new()),
    );
    let observation = runtime.snapshot_ptys().unwrap().remove(0);
    assert_eq!(observation.status, "unknown");
    // Independently exercise both native physical-control checks. Removing
    // the stop's Matching check would signal this test's owned bystander.
    assert!(
        runtime
            .stop("orchid.worker", true, observation.incarnation_id.as_deref())
            .is_err()
    );
    assert!(
        bystander.0.try_wait().unwrap().is_none(),
        "native stop signalled the bystander"
    );
    assert!(
        runtime
            .kill("orchid.worker", true, observation.incarnation_id.as_deref())
            .is_err()
    );
    assert!(
        bystander.0.try_wait().unwrap().is_none(),
        "native kill signalled the bystander"
    );
    // Exercise the actual stop reconciler with an already expired deadline.
    // It must never create a terminate request that a later pass could escalate.
    for status in ["unknown", "exited", "vanished"] {
        let mut observation = observation.clone();
        observation.status = status.into();
        assert!(
            !reconciler
                .reconcile_runtime_stop_inner(
                    "agent/orchid/worker",
                    "orchid.worker",
                    true,
                    observation.incarnation_id.as_deref(),
                    0,
                    Some(&observation),
                    None
                )
                .unwrap()
        );
    }
    let diagnostics = store
        .observations_for("agent/orchid/worker", "runtime.action.failed")
        .unwrap();
    assert!(
        diagnostics.iter().any(|claim| {
            let fields = &claim.body["fields"];
            let reason = fields["reason"].as_str().unwrap_or_default();
            fields["action"] == "terminate"
                && reason.contains("explicit stop refused")
                && reason.contains("daemon start token")
                && reason.contains("boot identity")
        }),
        "{diagnostics:?}"
    );
    assert!(
        store
            .observations_for("agent/orchid/worker", "runtime.action.requested")
            .unwrap()
            .is_empty()
    );
    assert!(bystander.0.try_wait().unwrap().is_none());
    assert_eq!(
        std::fs::read_to_string(registry.join("orchid.worker.json")).unwrap(),
        record.to_string()
    );
}
