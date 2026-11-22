use super::*;
use std::sync::Arc;

fn fixture() -> (Store, tempfile::TempDir, ScratchRequest) {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap().join("dedicated");
    let store = Store::open_memory("host-a").unwrap();
    store.start_resource_authority("host-a", "epoch-a").unwrap();
    owner(&store, "running", "executing", "generation-a");
    let request = ScratchRequest { id: "scratch/test".into(), host: "host-a".into(),
        owner: "mission-run/owner".into(), owner_generation: "run-generation/generation-a".into(),
        root: root.to_string_lossy().into_owned() };
    (store, directory, request)
}

fn release(store: &Store, handle: &ScratchHandle) -> Result<bool> {
    // These real store/filesystem fixtures have no native runtimes or seats.
    store.release_scratch(handle, &ScratchReleaseProof::after_quiescence(&handle.owner, &handle.owner_generation))
}

fn record(store: &Store, id: &str) -> Record {
    read(&store.connection.write(), id).unwrap().unwrap()
}

fn save(store: &Store, record: &Record) {
    write(&store.connection.write(), record).unwrap();
}

fn owner(store: &Store, status: &str, phase: &str, generation: &str) {
    store.connection.write().execute("INSERT INTO mission_runs
        (id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms)
        VALUES('owner','mission/test','revision',?1,'revision','owner','workspace/test','person/test','{}','once',?2,?3,'1','1')
        ON CONFLICT(id) DO UPDATE SET current_generation_id=excluded.current_generation_id,status=excluded.status,phase=excluded.phase",
        params![generation, status, phase]).unwrap();
}

#[test]
fn duplicate_specification_and_released_tombstone_are_immutable() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    assert_eq!(store.acquire_scratch(&request).unwrap(), handle);
    fs::write(Path::new(&handle.path).join("payload"), b"retained").unwrap();
    let mut changed = request.clone();
    changed.owner_generation = "run-generation/other".into();
    assert!(store.acquire_scratch(&changed).is_err());
    assert!(release(&store, &handle).unwrap());
    assert!(release(&store, &handle).unwrap());
    assert!(!Path::new(&handle.path).exists());
    assert!(store.acquire_scratch(&request).is_err());
}

#[test]
fn interrupted_intent_create_and_receipt_resume_exact_allocation() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    let mut intent = record(&store, &request.id);
    fs::remove_dir(&handle.path).unwrap();
    intent.state = "intent".into();
    intent.object = None;
    save(&store, &intent);
    assert_eq!(store.acquire_scratch(&request).unwrap(), handle);
    let first = identity(Path::new(&handle.path)).unwrap();
    // Crash after mkdir but before its inode receipt: only the persisted reserved
    // generation name is adopted; retry never chooses a second path.
    save(&store, &intent);
    assert_eq!(store.acquire_scratch(&request).unwrap(), handle);
    assert_eq!(identity(Path::new(&handle.path)).unwrap(), first);
    assert_eq!(store.scratch_inventory("host-a", None, 10).unwrap().len(), 1);
}

#[test]
fn release_intent_resumes_after_removal_and_before_completion_receipt() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    let mut interrupted = record(&store, &request.id);
    interrupted.state = "releasing".into();
    save(&store, &interrupted);
    assert!(store.acquire_scratch(&request).is_err());
    fs::remove_dir_all(&handle.path).unwrap();
    assert!(release(&store, &handle).unwrap());
    assert_eq!(record(&store, &request.id).state, "released");
}

#[test]
fn release_owner_covers_intent_without_acquire_receipt() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    let mut intent = record(&store, &request.id);
    fs::remove_dir(&handle.path).unwrap();
    intent.state = "intent".into();
    intent.object = None;
    save(&store, &intent);
    assert!(store.release_scratch_for_owner(&request.host, &request.owner, &request.owner_generation,
        &ScratchReleaseProof::after_quiescence(&request.owner, &request.owner_generation)).unwrap());
    assert_eq!(record(&store, &request.id).state, "released");
    assert!(!Path::new(&handle.path).exists());
}

#[test]
fn generation_epoch_and_concurrent_release_are_fenced() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    let mut stale = handle.clone();
    stale.generation = "not-current".into();
    assert!(release(&store, &stale).is_err());
    store.start_resource_authority("host-a", "epoch-b").unwrap();
    assert!(release(&store, &handle).is_err());
    let current = store.scratch_handle(&request.id).unwrap().unwrap();
    assert_eq!(current.path, handle.path);
    let store = Arc::new(store);
    let workers: Vec<_> = (0..2).map(|_| {
        let store = Arc::clone(&store);
        let handle = current.clone();
        std::thread::spawn(move || store.release_scratch(&handle, &ScratchReleaseProof::after_quiescence("mission-run/owner", "run-generation/generation-a")).unwrap())
    }).collect();
    for worker in workers { assert!(worker.join().unwrap()); }
    assert_eq!(record(&store, &request.id).state, "released");
}

#[test]
fn live_and_unknown_consumers_retain_custody() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    for state in ["running", "uncertain"] {
        let body = json!({"state":state,"command":{"cwd":handle.path}}).to_string();
        store.connection.write().execute("INSERT INTO shell_resources_v1(id,host,state,body) VALUES('consumer','host-a',?1,?2)
            ON CONFLICT(id) DO UPDATE SET state=excluded.state,body=excluded.body", params![state, body]).unwrap();
        assert!(!release(&store, &handle).unwrap());
        assert!(Path::new(&handle.path).is_dir());
        assert_ne!(store.scratch_inventory("host-a", None, 1).unwrap()[0].consumer_observation, OwnerObservation::Terminal);
    }
    store.connection.write().execute("UPDATE shell_resources_v1 SET state='completed' WHERE id='consumer'", []).unwrap();
    assert!(release(&store, &handle).unwrap());
}

#[test]
fn replaced_directory_and_symlink_root_are_not_reclaimed() {
    let (store, directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    let displaced = Path::new(&request.root).join("displaced");
    fs::rename(&handle.path, &displaced).unwrap();
    fs::create_dir(&handle.path).unwrap();
    fs::write(Path::new(&handle.path).join("foreign"), b"keep").unwrap();
    assert!(release(&store, &handle).is_err());
    assert!(Path::new(&handle.path).join("foreign").exists());
    fs::remove_dir_all(&handle.path).unwrap();
    std::os::unix::fs::symlink(&displaced, &handle.path).unwrap();
    assert!(release(&store, &handle).is_err());
    let alias = fs::canonicalize(directory.path()).unwrap().join("alias");
    std::os::unix::fs::symlink(&request.root, &alias).unwrap();
    let mut aliased = request.clone();
    aliased.id = "scratch/alias".into();
    aliased.root = alias.to_string_lossy().into_owned();
    assert!(store.acquire_scratch(&aliased).is_err());
}

#[test]
fn missing_held_object_is_not_recreated() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    fs::remove_dir(&handle.path).unwrap();
    assert!(store.acquire_scratch(&request).is_err());
    assert!(!Path::new(&handle.path).exists());
    assert!(release(&store, &handle).is_err());
    assert_eq!(record(&store, &request.id).state, "held");
}

#[test]
fn inventory_is_bounded_read_only_and_owner_evidence_is_positive() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    store.connection.write().execute("DELETE FROM mission_runs WHERE id='owner'", []).unwrap();
    assert!(!release(&store, &handle).unwrap());
    assert!(Path::new(&handle.path).is_dir());
    assert_eq!(store.scratch_inventory("host-a", None, 1).unwrap()[0].owner_observation, OwnerObservation::Unknown);
    owner(&store, "completed", "terminal", "different-generation");
    assert!(!release(&store, &handle).unwrap());
    assert!(Path::new(&handle.path).is_dir());
    assert_eq!(store.scratch_inventory("host-a", None, 1).unwrap()[0].owner_observation, OwnerObservation::Unknown);
    owner(&store, "running", "executing", "generation-a");
    assert_eq!(store.scratch_inventory("host-a", None, 1).unwrap()[0].owner_observation, OwnerObservation::Live);
    owner(&store, "failed", "terminal", "generation-a");
    assert_eq!(store.scratch_inventory("host-a", None, 1).unwrap()[0].owner_observation, OwnerObservation::Terminal);
    assert!(Path::new(&handle.path).is_dir());
    assert_eq!(record(&store, &request.id).state, "held");
    assert!(store.scratch_inventory("host-a", Some(&request.id), 1).unwrap().is_empty());
    assert!(store.scratch_inventory("host-a", None, 0).is_err());
    assert!(store.scratch_inventory("host-a", None, 1001).is_err());
}

#[test]
fn real_scratch_lifecycle_and_terminal_owner_dry_run_receipt() {
    let (store, _directory, request) = fixture();
    owner(&store, "running", "executing", "generation-a");
    let handle = store.acquire_scratch(&request).unwrap();
    fs::write(Path::new(&handle.path).join("artifact"), b"real scratch payload").unwrap();
    let held = store.scratch_inventory("host-a", None, 10).unwrap();
    assert!(release(&store, &handle).unwrap());
    let released = store.scratch_inventory("host-a", None, 10).unwrap();
    let mut orphan_request = request.clone();
    orphan_request.id = "scratch/z-terminal-owner".into();
    let orphan = store.acquire_scratch(&orphan_request).unwrap();
    owner(&store, "cancelled", "terminal", "generation-a");
    let dry_run = store.scratch_inventory("host-a", Some(&request.id), 10).unwrap();
    assert_eq!(dry_run.len(), 1);
    assert_eq!(dry_run[0].owner_observation, OwnerObservation::Terminal);
    assert!(Path::new(&orphan.path).is_dir());
    println!("{}", json!({"acquire":handle,"held":held,"released":released,
        "terminalOwnerDryRun":dry_run,"dryRunPathRetained":Path::new(&orphan.path).is_dir()}));
}

#[test]
fn late_acquire_refuses_terminal_missing_successor_and_expired_owner() {
    let (store, _directory, request) = fixture();
    owner(&store, "completed", "terminal", "generation-a");
    assert!(store.acquire_scratch(&request).is_err());
    owner(&store, "running", "executing", "successor");
    assert!(store.acquire_scratch(&request).is_err());
    store.connection.write().execute("DELETE FROM mission_runs WHERE id='owner'", []).unwrap();
    assert!(store.acquire_scratch(&request).is_err());
    owner(&store, "running", "executing", "generation-a");
    store.connection.write().execute("INSERT INTO mission_run_deadlines(run_id,timeout_ms,deadline_at_unix_ms) VALUES('owner',1,'1')", []).unwrap();
    assert!(store.acquire_scratch(&request).is_err());
    assert!(store.scratch_inventory("host-a", None, 1).unwrap().is_empty());
}

#[test]
fn persisted_intent_and_release_resume_across_store_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let directory = fs::canonicalize(directory.path()).unwrap();
    let database = directory.join("store.sqlite");
    let request = ScratchRequest { id: "scratch/reopen".into(), host: "host-a".into(),
        owner: "mission-run/owner".into(), owner_generation: "run-generation/generation-a".into(),
        root: directory.join("dedicated").to_string_lossy().into_owned() };
    let handle = {
        let store = Store::open(&database, "host-a").unwrap();
        store.start_resource_authority("host-a", "epoch-a").unwrap();
        owner(&store, "running", "executing", "generation-a");
        let handle = store.acquire_scratch(&request).unwrap();
        let mut intent = record(&store, &request.id);
        intent.state = "intent".into();
        intent.object = None;
        save(&store, &intent);
        handle
    };
    {
        let store = Store::open(&database, "host-a").unwrap();
        store.start_resource_authority("host-a", "epoch-b").unwrap();
        let resumed = store.acquire_scratch(&request).unwrap();
        assert_eq!(resumed.path, handle.path);
        assert_eq!(resumed.generation, handle.generation);
        assert_eq!(resumed.authority_epoch, "epoch-b");
        let mut interrupted = record(&store, &request.id);
        interrupted.state = "releasing".into();
        save(&store, &interrupted);
        fs::remove_dir_all(&handle.path).unwrap();
    }
    let store = Store::open(&database, "host-a").unwrap();
    store.start_resource_authority("host-a", "epoch-c").unwrap();
    let resumed = store.scratch_handle(&request.id).unwrap().unwrap();
    assert!(release(&store, &resumed).unwrap());
    assert_eq!(record(&store, &request.id).state, "released");
}

pub(super) fn interrupt_acquire(boundary: &str) {
    if std::env::var("FRACTAL_TEST_SCRATCH_BOUNDARY").as_deref() == Ok(boundary) {
        use std::io::Write;
        println!("scratch-boundary-reached:{boundary}");
        std::io::stdout().flush().unwrap();
        loop { std::thread::park(); }
    }
}

#[test]
#[ignore = "subprocess entry point for the SIGKILL acceptance test"]
fn scratch_crash_child() {
    let database = std::env::var("FRACTAL_TEST_SCRATCH_DATABASE").unwrap();
    let request: ScratchRequest = serde_json::from_str(&std::env::var("FRACTAL_TEST_SCRATCH_REQUEST").unwrap()).unwrap();
    let store = Store::open(Path::new(&database), "host-a").unwrap();
    store.start_resource_authority("host-a", "child-epoch").unwrap();
    owner(&store, "running", "executing", "generation-a");
    store.acquire_scratch(&request).unwrap();
    panic!("child must be killed at requested acquire boundary");
}

#[test]
fn sigkill_between_intent_create_and_receipt_resumes_exact_object() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};
    for boundary in ["intent", "created"] {
        let directory = tempfile::tempdir().unwrap();
        let canonical = fs::canonicalize(directory.path()).unwrap();
        let database = canonical.join("store.sqlite");
        let request = ScratchRequest { id: format!("scratch/kill-{boundary}"), host: "host-a".into(),
            owner: "mission-run/owner".into(), owner_generation: "run-generation/generation-a".into(),
            root: canonical.join("dedicated").to_string_lossy().into_owned() };
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "store::scratch_resources::tests::scratch_crash_child", "--nocapture"])
            .env("FRACTAL_TEST_SCRATCH_BOUNDARY", boundary)
            .env("FRACTAL_TEST_SCRATCH_DATABASE", &database)
            .env("FRACTAL_TEST_SCRATCH_REQUEST", serde_json::to_string(&request).unwrap())
            .stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut reached = false;
        let mut line = String::new();
        while output.read_line(&mut line).unwrap() != 0 {
            if line.contains(&format!("scratch-boundary-reached:{boundary}")) { reached = true; break; }
            line.clear();
        }
        if !reached {
            let status = child.wait().unwrap();
            panic!("child exited before {boundary}: {status}");
        }
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(9));
        let store = Store::open(&database, "host-a").unwrap();
        store.start_resource_authority("host-a", "replacement-epoch").unwrap();
        let intent = record(&store, &request.id);
        assert_eq!(intent.state, "intent");
        assert!(intent.object.is_none());
        let before = if boundary == "created" {
            Some(identity(Path::new(&intent.handle.path)).unwrap())
        } else {
            assert!(!Path::new(&intent.handle.path).exists());
            None
        };
        let resumed = store.acquire_scratch(&request).unwrap();
        assert_eq!(resumed.path, intent.handle.path);
        assert_eq!(resumed.generation, intent.handle.generation);
        if let Some(before) = before { assert_eq!(identity(Path::new(&resumed.path)).unwrap(), before); }
        assert!(release(&store, &resumed).unwrap());
    }
}
