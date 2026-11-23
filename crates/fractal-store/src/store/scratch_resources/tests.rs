use super::*;
use std::sync::Arc;

fn fixture() -> (Store, tempfile::TempDir, ScratchRequest) {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap().join("dedicated");
    let store = Store::open_memory("host-a").unwrap();
    store.start_resource_authority("host-a", "epoch-a").unwrap();
    owner(&store, "running", "normal", "generation-a");
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
    let connection = store.connection.write();
    let updated = connection.execute(/* sqlite-large-value: allow-for-now lv-he0bc37451fe9 owner=fractal_store::store::scratch_resources::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET current_generation_id=?1,status=?2,phase=?3 WHERE id='owner'",
        params![generation, status, phase]).unwrap();
    if updated == 0 {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h40d4f40c9604 owner=fractal_store::store::scratch_resources::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs
            (id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms)
            VALUES('owner','mission/test','revision',?1,'revision','owner','workspace/test','person/test','{}','once',?2,?3,'1','1')",
            params![generation, status, phase]).unwrap();
    }
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
fn missing_receipted_object_without_quarantine_is_unknown_not_released() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    let mut interrupted = record(&store, &request.id);
    interrupted.state = "releasing".into();
    save(&store, &interrupted);
    assert!(store.acquire_scratch(&request).is_err());
    fs::remove_dir_all(&handle.path).unwrap();
    assert!(release(&store, &handle).is_err());
    assert_ne!(record(&store, &request.id).state, "released");
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
fn live_open_file_retains_custody_and_inventory_stays_non_probing() {
    let (store, _directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    let payload = Path::new(&handle.path).join("payload");
    fs::write(&payload, b"owned").unwrap();
    let consumer = fs::File::open(&payload).unwrap();
    assert!(!release(&store, &handle).unwrap());
    assert_eq!(fs::read(&payload).unwrap(), b"owned");
    let row = store.scratch_inventory("host-a", None, 1).unwrap().remove(0);
    assert_eq!(row.consumer_observation, OwnerObservation::Unknown);
    assert!(row.uninspectable.is_some());
    drop(consumer);
    assert!(release(&store, &handle).unwrap());
    assert_eq!(store.scratch_inventory("host-a", None, 1).unwrap()[0].consumer_observation,
        OwnerObservation::Terminal);
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
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-ha2b4809b35ad owner=fractal_store::store::scratch_resources::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mission_runs WHERE id='owner'", []).unwrap();
    assert!(!release(&store, &handle).unwrap());
    assert!(Path::new(&handle.path).is_dir());
    assert_eq!(store.scratch_inventory("host-a", None, 1).unwrap()[0].owner_observation, OwnerObservation::Unknown);
    owner(&store, "completed", "terminal", "different-generation");
    assert!(!release(&store, &handle).unwrap());
    assert!(Path::new(&handle.path).is_dir());
    assert_eq!(store.scratch_inventory("host-a", None, 1).unwrap()[0].owner_observation, OwnerObservation::Unknown);
    owner(&store, "running", "normal", "generation-a");
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
    owner(&store, "running", "normal", "generation-a");
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
    for phase in ["final", "cleanup-completed", "cleanup-failed", "cleanup-cancelled"] {
        owner(&store, "running", phase, "generation-a");
        assert!(store.acquire_scratch(&request).is_err());
    }
    owner(&store, "completed", "terminal", "generation-a");
    assert!(store.acquire_scratch(&request).is_err());
    owner(&store, "running", "normal", "successor");
    assert!(store.acquire_scratch(&request).is_err());
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h1bbc5325cd30 owner=fractal_store::store::scratch_resources::tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mission_runs WHERE id='owner'", []).unwrap();
    assert!(store.acquire_scratch(&request).is_err());
    owner(&store, "running", "normal", "generation-a");
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h5eee83bba1b5 owner=fractal_store::store::scratch_resources::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_run_deadlines(run_id,timeout_ms,deadline_at_unix_ms) VALUES('owner',1,'1')", []).unwrap();
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
        owner(&store, "running", "normal", "generation-a");
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
        INTERRUPT_QUARANTINE.with(|interrupt| interrupt.set(Some("removed")));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| release(&store, &resumed))).is_err());
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
    owner(&store, "running", "normal", "generation-a");
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

thread_local! {
    static BEFORE_REMOVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static BEFORE_HANDOFF: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static INTERRUPT_QUARANTINE: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

pub(super) fn before_remove() {
    BEFORE_REMOVE.with(|hook| {
        if let Some(hook) = hook.take() { hook(); }
    });
}

pub(super) fn quarantine_boundary(boundary: &str) {
    if boundary == "handoff" {
        BEFORE_HANDOFF.with(|hook| {
            if let Some(hook) = hook.take() { hook(); }
        });
    }
    INTERRUPT_QUARANTINE.with(|interrupt| {
        if interrupt.get() == Some(boundary) {
            interrupt.set(None);
            panic!("interrupted quarantine at {boundary}");
        }
    });
}

#[test]
fn empty_replacement_at_atomic_handoff_is_restored_and_never_adopted() {
    let (store, directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    fs::write(Path::new(&handle.path).join("payload"), b"original").unwrap();
    let original = directory.path().join("original");
    let moved = original.clone();
    let allocation = handle.path.clone();
    BEFORE_HANDOFF.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
        fs::rename(&allocation, &moved).unwrap();
        fs::create_dir(&allocation).unwrap();
    })));
    assert!(release(&store, &handle).is_err());
    assert!(Path::new(&handle.path).is_dir());
    assert_eq!(fs::read(original.join("payload")).unwrap(), b"original");
    assert_eq!(fs::read_dir(&handle.path).unwrap().count(), 0);
    assert_eq!(record(&store, &request.id).state, "releasing");
    assert_eq!(record(&store, &request.id).quarantine.unwrap().phase, "failed");
    assert!(release(&store, &handle).is_err());
    assert!(Path::new(&handle.path).is_dir());
    assert_eq!(fs::read(original.join("payload")).unwrap(), b"original");
}

#[test]
fn quarantine_handoff_and_removal_interruptions_resume_after_reopen() {
    for boundary in ["handoff", "moved", "deleting", "removed"] {
        let directory = tempfile::tempdir().unwrap();
        let canonical = fs::canonicalize(directory.path()).unwrap();
        let database = canonical.join("store.sqlite");
        let request = ScratchRequest { id: "scratch/quarantine-reopen".into(), host: "host-a".into(),
            owner: "mission-run/owner".into(), owner_generation: "run-generation/generation-a".into(),
            root: canonical.join("dedicated").to_string_lossy().into_owned() };
        let store = Store::open(&database, "host-a").unwrap();
        store.start_resource_authority("host-a", "epoch-a").unwrap();
        owner(&store, "running", "normal", "generation-a");
        let handle = store.acquire_scratch(&request).unwrap();
        fs::write(Path::new(&handle.path).join("payload"), b"owned").unwrap();
        INTERRUPT_QUARANTINE.with(|interrupt| interrupt.set(Some(boundary)));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| release(&store, &handle))).is_err());
        let custody = record(&store, &request.id);
        assert_eq!(custody.state, "releasing");
        assert!(custody.quarantine.unwrap().identity.is_some());
        drop(store);
        let store = Store::open(&database, "host-a").unwrap();
        store.start_resource_authority("host-a", "epoch-b").unwrap();
        let handle = store.scratch_handle(&request.id).unwrap().unwrap();
        assert!(release(&store, &handle).unwrap(), "{boundary}");
        assert!(!Path::new(&handle.path).exists());
        assert_eq!(record(&store, &request.id).state, "released");
    }
}

#[test]
fn restoring_intent_reconciles_both_locations_after_interruption() {
    for boundary in ["restoring", "restored"] {
        let directory = tempfile::tempdir().unwrap();
        let canonical = fs::canonicalize(directory.path()).unwrap();
        let database = canonical.join("store.sqlite");
        let request = ScratchRequest { id: "scratch/restore-reopen".into(), host: "host-a".into(),
            owner: "mission-run/owner".into(), owner_generation: "run-generation/generation-a".into(),
            root: canonical.join("dedicated").to_string_lossy().into_owned() };
        let store = Store::open(&database, "host-a").unwrap();
        store.start_resource_authority("host-a", "epoch-a").unwrap();
        owner(&store, "running", "normal", "generation-a");
        let handle = store.acquire_scratch(&request).unwrap();
        fs::write(Path::new(&handle.path).join("payload"), b"owned").unwrap();
        INTERRUPT_QUARANTINE.with(|interrupt| interrupt.set(Some("deleting")));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| release(&store, &handle))).is_err());
        let custody = record(&store, &request.id);
        let quarantine = custody.quarantine.unwrap();
        assert_eq!(quarantine.phase, "deleting");
        let private_payload = Path::new(&request.root).join(&quarantine.name).join(&quarantine.entry).join("payload");
        let consumer = fs::File::open(&private_payload).unwrap();
        INTERRUPT_QUARANTINE.with(|interrupt| interrupt.set(Some(boundary)));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| release(&store, &handle))).is_err());
        let custody = record(&store, &request.id);
        assert_eq!(custody.quarantine.unwrap().phase, "restoring");
        assert_eq!(Path::new(&handle.path).exists(), boundary == "restored");
        drop(store);

        let store = Store::open(&database, "host-a").unwrap();
        store.start_resource_authority("host-a", "epoch-b").unwrap();
        let handle = store.scratch_handle(&request.id).unwrap().unwrap();
        assert!(!release(&store, &handle).unwrap(), "{boundary}: restoration is not a release receipt");
        assert_ne!(record(&store, &request.id).state, "released");
        assert_eq!(fs::read(Path::new(&handle.path).join("payload")).unwrap(), b"owned");
        assert!(!release(&store, &handle).unwrap(), "{boundary}: the live file still retains custody");
        assert_eq!(store.scratch_inventory("host-a", None, 1).unwrap()[0].consumer_observation,
            OwnerObservation::Unknown);
        drop(consumer);
        assert!(release(&store, &handle).unwrap());
        assert!(!Path::new(&handle.path).exists());
    }
}

#[test]
fn interrupted_wrong_object_handoff_restores_without_adoption_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = fs::canonicalize(directory.path()).unwrap();
    let database = canonical.join("store.sqlite");
    let request = ScratchRequest { id: "scratch/quarantine-mismatch".into(), host: "host-a".into(),
        owner: "mission-run/owner".into(), owner_generation: "run-generation/generation-a".into(),
        root: canonical.join("dedicated").to_string_lossy().into_owned() };
    let store = Store::open(&database, "host-a").unwrap();
    store.start_resource_authority("host-a", "epoch-a").unwrap();
    owner(&store, "running", "normal", "generation-a");
    let handle = store.acquire_scratch(&request).unwrap();
    fs::write(Path::new(&handle.path).join("payload"), b"original").unwrap();
    let original = canonical.join("original");
    let moved = original.clone();
    let allocation = handle.path.clone();
    BEFORE_HANDOFF.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
        fs::rename(&allocation, &moved).unwrap();
        fs::create_dir(&allocation).unwrap();
    })));
    INTERRUPT_QUARANTINE.with(|interrupt| interrupt.set(Some("moved")));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| release(&store, &handle))).is_err());
    drop(store);
    let store = Store::open(&database, "host-a").unwrap();
    store.start_resource_authority("host-a", "epoch-b").unwrap();
    let handle = store.scratch_handle(&request.id).unwrap().unwrap();
    assert!(release(&store, &handle).is_err());
    assert!(Path::new(&handle.path).is_dir());
    assert_eq!(fs::read_dir(&handle.path).unwrap().count(), 0);
    assert_eq!(fs::read(original.join("payload")).unwrap(), b"original");
    assert_ne!(record(&store, &request.id).state, "released");
    assert!(release(&store, &handle).is_err());
}

#[test]
fn renamed_root_after_final_check_never_deletes_symlink_target() {
    let (store, directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    fs::create_dir(Path::new(&handle.path).join("nested")).unwrap();
    fs::write(Path::new(&handle.path).join("nested/payload"), b"owned").unwrap();
    let root = Path::new(&request.root).to_path_buf();
    let displaced = directory.path().join("displaced");
    let foreign = directory.path().join("foreign");
    let basename = Path::new(&handle.path).file_name().unwrap();
    fs::create_dir_all(foreign.join(basename)).unwrap();
    let sentinel = foreign.join(basename).join("sentinel");
    fs::write(&sentinel, b"unrelated").unwrap();
    let original = displaced.join(basename);
    BEFORE_REMOVE.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
        fs::rename(&root, &displaced).unwrap();
        std::os::unix::fs::symlink(&foreign, &root).unwrap();
    })));
    assert!(release(&store, &handle).unwrap());
    assert_eq!(fs::read(sentinel).unwrap(), b"unrelated");
    assert!(!original.exists());
    assert_eq!(record(&store, &request.id).state, "released");
}

#[test]
fn renamed_ancestor_after_final_check_never_deletes_symlink_target() {
    let (store, directory, mut request) = fixture();
    request.root = directory.path().join("ancestor/dedicated").to_string_lossy().into_owned();
    let handle = store.acquire_scratch(&request).unwrap();
    fs::write(Path::new(&handle.path).join("payload"), b"owned").unwrap();
    let ancestor = directory.path().join("ancestor");
    let displaced = directory.path().join("displaced");
    let foreign = directory.path().join("foreign");
    let basename = Path::new(&handle.path).file_name().unwrap();
    fs::create_dir_all(foreign.join("dedicated").join(basename)).unwrap();
    let sentinel = foreign.join("dedicated").join(basename).join("sentinel");
    fs::write(&sentinel, b"unrelated").unwrap();
    let original = displaced.join("dedicated").join(basename);
    BEFORE_REMOVE.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
        fs::rename(&ancestor, &displaced).unwrap();
        std::os::unix::fs::symlink(&foreign, &ancestor).unwrap();
    })));
    assert!(release(&store, &handle).unwrap());
    assert_eq!(fs::read(sentinel).unwrap(), b"unrelated");
    assert!(!original.exists());
    assert_eq!(record(&store, &request.id).state, "released");
}

#[test]
fn allocation_swap_after_final_check_retains_original_and_replacement() {
    let (store, directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    fs::write(Path::new(&handle.path).join("payload"), b"owned").unwrap();
    let allocation = Path::new(&handle.path).to_path_buf();
    let displaced = directory.path().join("displaced");
    let payload = displaced.join("payload");
    let foreign = directory.path().join("foreign");
    fs::create_dir(&foreign).unwrap();
    let sentinel = foreign.join("sentinel");
    fs::write(&sentinel, b"unrelated").unwrap();
    BEFORE_REMOVE.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
        fs::rename(&allocation, &displaced).unwrap();
        std::os::unix::fs::symlink(&foreign, &allocation).unwrap();
    })));
    assert!(release(&store, &handle).is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"unrelated");
    assert_eq!(fs::read(payload).unwrap(), b"owned");
    assert!(fs::symlink_metadata(&handle.path).unwrap().file_type().is_symlink());
    assert_eq!(record(&store, &request.id).state, "releasing");
}

#[test]
fn recursive_removal_unlinks_directory_symlinks_without_traversal() {
    let (store, directory, request) = fixture();
    let handle = store.acquire_scratch(&request).unwrap();
    let foreign = directory.path().join("foreign");
    fs::create_dir(&foreign).unwrap();
    let sentinel = foreign.join("sentinel");
    fs::write(&sentinel, b"unrelated").unwrap();
    let nested = Path::new(&handle.path).join("nested");
    fs::create_dir(&nested).unwrap();
    std::os::unix::fs::symlink(&foreign, nested.join("alias")).unwrap();
    assert!(release(&store, &handle).unwrap());
    assert_eq!(fs::read(sentinel).unwrap(), b"unrelated");
    assert!(!Path::new(&handle.path).exists());
    assert_eq!(record(&store, &request.id).state, "released");
}
