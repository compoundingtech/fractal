use super::*;

const REQUEST_BUDGET: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub enum DiscoveryError {
    Deadline,
    HistoryPending,
    Busy,
    Failed(String),
}
impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deadline => f.write_str("native session discovery deadline exceeded; background discovery is still running"),
            Self::HistoryPending => f.write_str("native session history is still being indexed"),
            Self::Busy => f.write_str("another exact transcript read is still running"),
            Self::Failed(error) => write!(f, "native session discovery failed: {error}"),
        }
    }
}
impl std::error::Error for DiscoveryError {}

fn classify(error: anyhow::Error) -> DiscoveryError {
    match error.downcast::<DiscoveryError>() {
        Ok(error) => error,
        Err(error) => DiscoveryError::Failed(format!("{error:#}")),
    }
}

#[derive(Default)]
struct Entry {
    running: bool,
    generation: u64,
    result: Option<(Instant, std::result::Result<ExternalDiscovery, DiscoveryError>)>,
}
// One live and one history-inclusive worker per home. No request holds the mutex
// across filesystem or subprocess work, and timed-out callers never spawn duplicates.
static REQUESTS: LazyLock<(Mutex<HashMap<(PathBuf, bool), Entry>>, Condvar)> =
    LazyLock::new(|| (Mutex::new(HashMap::new()), Condvar::new()));

pub(super) fn discover(home: Option<&Path>, history: bool, fresh: bool) -> Result<ExternalDiscovery> {
    let Some(home) = home else { return Ok(ExternalDiscovery::default()); };
    let key = (home.to_owned(), history);
    let deadline = Instant::now() + REQUEST_BUDGET;
    let (lock, ready) = &*REQUESTS;
    let mut entries = lock.lock().expect("discovery requests poisoned");
    let entry = entries.entry(key.clone()).or_default();
    if !fresh && let Some((at, result)) = &entry.result && at.elapsed() < DISCOVERY_CACHE_TTL {
        return result.clone().map_err(Into::into);
    }
    let generation = entry.generation;
    if !entry.running {
        entry.running = true;
        let key = key.clone();
        std::thread::spawn(move || {
            let result = discover_uncached(&key.0, None, history)
                .map_err(classify);
            let mut entries = REQUESTS.0.lock().expect("discovery requests poisoned");
            let entry = entries.get_mut(&key).expect("registered discovery");
            entry.running = false;
            entry.generation += 1;
            entry.result = Some((Instant::now(), result));
            REQUESTS.1.notify_all();
        });
    }
    while entries[&key].generation == generation {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() { return Err(DiscoveryError::Deadline.into()); }
        entries = ready.wait_timeout(entries, remaining).expect("discovery requests poisoned").0;
    }
    entries[&key].result.as_ref().expect("completed discovery").1.clone().map_err(Into::into)
}

pub(super) fn find(home: Option<&Path>, id: &str, fresh: bool) -> Result<Option<ExternalConversation>> {
    let started = Instant::now();
    if !id.starts_with("session/external-") { return Ok(None); }
    let Some(home) = home else { return Ok(None); };
    // Live resolution is shared with list, never a synchronous history walk.
    let discovery = discover(Some(home), false, fresh)?;
    let live = discovery.into_conversation(id);
    let strict_live_omp = matches!(live.as_ref(),
        Some(ExternalConversation::Readable(session)) if session.driver == ExternalDriver::Omp);
    if live.is_some() && (!fresh || !strict_live_omp) { return Ok(live); }
    start_history_inventory(Some(home));
    let cold = HISTORY.0.lock().expect("history inventory mutex poisoned")
        .get(home).is_none_or(|inventory| inventory.index.is_none());
    if cold || fresh { historical_files(home)?; }
    let mut files = {
        let inventories = HISTORY.0.lock().expect("history inventory mutex poisoned");
        let inventory = inventories.get(home).expect("registered history");
        if fresh && inventory.refreshing { return Err(DiscoveryError::HistoryPending.into()); }
        let Some(index) = &inventory.index else { return Err(DiscoveryError::HistoryPending.into()); };
        index.get(id).cloned().unwrap_or_default()
    };
    // Complete-ID indexed lookup is independent of history size and list pages.
    if fresh {
        let live_process = if let Some(ExternalConversation::Readable(session)) = live {
            files.push(SessionMetadata {
                driver: session.driver, native_id: session.native_id, transcript: session.transcript,
                codex_home: session.codex_home, cwd: session.cwd, title: session.title,
                started_at_unix_ms: session.started_at_unix_ms,
                updated_at_unix_ms: session.updated_at_unix_ms, revision: session.revision,
            });
            session.process
        } else { None };
        let exact = revalidate(home, id, files, REQUEST_BUDGET.saturating_sub(started.elapsed()))?;
        return Ok(exact.map(|conversation| match conversation {
            ExternalConversation::Readable(mut session) => {
                session.process = live_process;
                ExternalConversation::Readable(session)
            }
            unavailable => unavailable,
        }));
    }
    Ok(assemble_discovery(files, Vec::new(), Some(id), true)?.into_conversation(id))
}

#[derive(Default)]
struct ExactRead {
    running: bool,
    abandoned: bool,
    result: Option<std::result::Result<ExternalDiscovery, DiscoveryError>>,
}
static EXACT: LazyLock<(Mutex<HashMap<PathBuf, ExactRead>>, Condvar)> =
    LazyLock::new(|| (Mutex::new(HashMap::new()), Condvar::new()));

fn revalidate(home: &Path, id: &str, files: Vec<SessionMetadata>, budget: Duration) -> Result<Option<ExternalConversation>> {
    let deadline = Instant::now() + budget;
    let mut reads = EXACT.0.lock().expect("exact reads poisoned");
    let read = reads.entry(home.to_owned()).or_default();
    if read.running { return Err(DiscoveryError::Busy.into()); }
    read.running = true;
    read.result = None;
    read.abandoned = false;
    let worker_home = home.to_owned();
    let worker_id = id.to_owned();
    std::thread::spawn(move || {
        let result = (|| {
            let mut current = Vec::new();
            for file in files {
                let metadata = if file.driver == ExternalDriver::OpenCode {
                    read_opencode_sessions(&file.transcript, Some(&file.native_id))?.into_iter().next()
                } else {
                    read_metadata(file.driver, &file.transcript)?
                };
                if let Some(metadata) = metadata && metadata.native_id == file.native_id {
                    current.push(metadata);
                }
            }
            assemble_discovery(current, Vec::new(), Some(&worker_id), true)
        })().map_err(|error: anyhow::Error| DiscoveryError::Failed(format!("{error:#}")));
        let mut reads = EXACT.0.lock().expect("exact reads poisoned");
        let read = reads.get_mut(&worker_home).expect("registered exact read");
        if read.abandoned { read.running = false; }
        read.result = Some(result);
        EXACT.1.notify_all();
    });
    while reads[home].result.is_none() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            reads.get_mut(home).expect("registered exact read").abandoned = true;
            return Err(DiscoveryError::Deadline.into());
        }
        reads = EXACT.1.wait_timeout(reads, remaining).expect("exact reads poisoned").0;
    }
    let read = reads.get_mut(home).expect("registered exact read");
    read.running = false;
    Ok(read.result.take().expect("completed exact read")?.into_conversation(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_readers_share_inflight_work_without_holding_io_lock() {
        let home = tempfile::tempdir().unwrap();
        let key = (home.path().to_owned(), false);
        REQUESTS.0.lock().unwrap().insert(key.clone(), Entry { running: true, ..Default::default() });
        let worker_key = key.clone();
        let complete = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let mut entries = REQUESTS.0.lock().unwrap();
            let entry = entries.get_mut(&worker_key).unwrap();
            entry.running = false;
            entry.generation += 1;
            entry.result = Some((Instant::now(), Ok(ExternalDiscovery::default())));
            REQUESTS.1.notify_all();
        });
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| assert!(discover(Some(home.path()), false, true).unwrap().sessions.is_empty()));
            }
        });
        complete.join().unwrap();
    }

    fn cache_empty_live(home: &Path) {
        REQUESTS.0.lock().unwrap().insert((home.to_owned(), false), Entry {
            result: Some((Instant::now(), Ok(ExternalDiscovery::default()))),
            ..Default::default()
        });
    }

    fn metadata(home: &Path, id: &str) -> SessionMetadata {
        SessionMetadata {
            driver: ExternalDriver::Omp, native_id: id.into(),
            transcript: home.join(format!("{id}.jsonl")), codex_home: None,
            cwd: None, title: None, started_at_unix_ms: 1, updated_at_unix_ms: 1,
            revision: "1".into(),
        }
    }

    #[test]
    fn indexed_show_is_independent_of_stalled_history_and_list_page() {
        let home = tempfile::tempdir().unwrap();
        cache_empty_live(home.path());
        let file = metadata(home.path(), "zz-full-session-id");
        let id = external_session_id(file.driver, &file.native_id);
        let mut index = BTreeMap::new();
        index.insert(id.clone(), vec![file]);
        let mut page_files = Vec::new();
        for number in 0..MAX_EXPOSED_HISTORY {
            let file = metadata(home.path(), &format!("older-{number:05}"));
            index.insert(external_session_id(file.driver, &file.native_id), vec![file.clone()]);
            page_files.push(file);
        }
        page_files.extend(index[&id].iter().cloned());
        let page = assemble_discovery(page_files, Vec::new(), None, true).unwrap();
        assert_eq!(page.sessions.len(), MAX_EXPOSED_HISTORY);
        assert!(!page.sessions.iter().any(|session| session.id == id));
        HISTORY.0.lock().unwrap().insert(home.path().to_owned(), HistoryInventory {
            refreshing: true, index: Some(Arc::new(index)), ..Default::default()
        });
        let start = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| assert!(find(Some(home.path()), &id, false).unwrap().is_some()));
            scope.spawn(|| assert!(discover(Some(home.path()), false, false).unwrap().sessions.is_empty()));
        });
        assert!(find(Some(home.path()), &external_session_id(ExternalDriver::Omp, "zz-full-session"), false).unwrap().is_none());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn cold_history_returns_typed_pending_without_scanning_on_show() {
        let home = tempfile::tempdir().unwrap();
        cache_empty_live(home.path());
        HISTORY.0.lock().unwrap().insert(home.path().to_owned(), HistoryInventory {
            refreshing: true, ..Default::default()
        });
        let start = Instant::now();
        let error = find(Some(home.path()), "session/external-unknown", false).err().unwrap();
        assert!(matches!(error.downcast_ref::<DiscoveryError>(), Some(DiscoveryError::HistoryPending)));
        assert!(start.elapsed() < HISTORY_WAIT + Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn slow_exact_storage_does_not_block_list_or_spawn_more_readers() {
        use std::io::Write as _;
        use std::os::unix::ffi::OsStrExt as _;
        let home = tempfile::tempdir().unwrap();
        cache_empty_live(home.path());
        let file = metadata(home.path(), "slow");
        let path = std::ffi::CString::new(file.transcript.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let id = external_session_id(file.driver, &file.native_id);
        let error = revalidate(home.path(), &id, vec![file.clone()], Duration::from_millis(30)).err().unwrap();
        assert!(matches!(error.downcast_ref::<DiscoveryError>(), Some(DiscoveryError::Deadline)));
        assert!(discover(Some(home.path()), false, false).unwrap().sessions.is_empty());
        assert!(matches!(revalidate(home.path(), &id, vec![file.clone()], Duration::from_millis(30))
            .err().unwrap().downcast_ref::<DiscoveryError>(), Some(DiscoveryError::Busy)));
        let mut writer = fs::OpenOptions::new().write(true).open(&file.transcript).unwrap();
        writeln!(writer, "{{\"type\":\"session\",\"id\":\"slow\"}}").unwrap();
        drop(writer);
        let mut reads = EXACT.0.lock().unwrap();
        while reads[home.path()].running {
            reads = EXACT.1.wait_timeout(reads, Duration::from_secs(1)).unwrap().0;
        }
    }

    #[test]
    fn full_id_lookup_rejects_copied_transcripts_regardless_of_mtime() {
        let home = tempfile::tempdir().unwrap();
        let first = metadata(home.path(), "same-id");
        let mut second = first.clone();
        second.transcript = home.path().join("copy.jsonl");
        second.updated_at_unix_ms = 100;
        fs::write(&first.transcript, "{\"type\":\"session\",\"id\":\"same-id\"}\n").unwrap();
        fs::copy(&first.transcript, &second.transcript).unwrap();
        let id = external_session_id(first.driver, &first.native_id);
        assert!(resolve_duplicate_sessions(vec![first, second], &[], Some(&id))
            .unwrap_err().downcast_ref::<AmbiguousSession>().is_some());
    }

    #[test]
    fn history_scan_builds_exact_index_and_revalidation_reads_only_that_file() {
        let home = tempfile::tempdir().unwrap();
        cache_empty_live(home.path());
        let project = home.path().join(".omp/agent/sessions/project");
        fs::create_dir_all(&project).unwrap();
        let transcript = project.join("native.jsonl");
        fs::write(&transcript, "{\"type\":\"session\",\"id\":\"complete-native-id\"}\n").unwrap();
        let files = historical_files(home.path()).unwrap();
        assert_eq!(files.len(), 1);
        let id = external_session_id(ExternalDriver::Omp, "complete-native-id");
        assert!(find(Some(home.path()), &id, false).unwrap().is_some());
        fs::write(&transcript, "{\"type\":\"session\",\"id\":\"different-native-id\"}\n").unwrap();
        let error = revalidate(home.path(), &id, files.as_ref().clone(), Duration::from_secs(1));
        assert!(error.unwrap().is_none(), "changed full header ID cannot inherit the indexed identity");
    }

    #[test]
    fn saved_opencode_fresh_lookup_revalidates_the_exact_sqlite_session() {
        let home = tempfile::tempdir().unwrap();
        let parent = home.path().join(".local/share/opencode");
        fs::create_dir_all(&parent).unwrap();
        let db = Connection::open(parent.join("opencode.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT, title TEXT,
                 time_created INTEGER, time_updated INTEGER);
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_updated INTEGER);
             INSERT INTO session VALUES ('saved', '/workspace', 'Before', 1, 2);
             INSERT INTO session VALUES ('saved-prefix', '/other', 'Other session', 1, 100);"
        ).unwrap();
        historical_files(home.path()).unwrap();
        db.execute("UPDATE session SET title = 'After', time_updated = 3 WHERE id = 'saved'", []).unwrap();
        let id = external_session_id(ExternalDriver::OpenCode, "saved");
        let found = find_fresh(Some(home.path()), &id).unwrap().unwrap();
        assert_eq!(found.native_id, "saved");
        assert_eq!(found.title.as_deref(), Some("After"));
        assert!(found.process.is_none());
        db.execute("DELETE FROM session WHERE id = 'saved'", []).unwrap();
        assert!(find_fresh(Some(home.path()), &id).unwrap().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_omp_fresh_lookup_rejects_copies_but_accepts_physical_file_aliases() {
        use std::os::unix::process::CommandExt as _;
        use std::process::Stdio;
        struct Harness(std::process::Child);
        impl Drop for Harness {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join(".omp/agent/sessions/project");
        fs::create_dir_all(&project).unwrap();
        let transcript = project.join("live.jsonl");
        fs::write(&transcript, "{\"type\":\"session\",\"id\":\"live-full-id\"}\n").unwrap();
        let alias = project.join("alias.jsonl");
        let hardlink = project.join("hardlink.jsonl");
        std::os::unix::fs::symlink(&transcript, &alias).unwrap();
        fs::hard_link(&transcript, &hardlink).unwrap();
        let id = external_session_id(ExternalDriver::Omp, "live-full-id");
        let aliases = [&transcript, &alias, &hardlink].into_iter()
            .map(|path| read_metadata(ExternalDriver::Omp, path).unwrap().unwrap()).collect();
        assert_eq!(resolve_duplicate_sessions(aliases, &[], Some(&id)).unwrap().len(), 1);
        let mut command = crate::environment::command("bash").unwrap();
        command.arg0("omp").args(["-c", "exec 3< \"$1\"; printf ready; read -r line", "omp"])
            .arg(&transcript).stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut harness = Harness(command.spawn().unwrap());
        let mut ready = [0; 5];
        harness.0.stdout.as_mut().unwrap().read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"ready");
        let found = find_fresh(Some(home.path()), &id).unwrap().unwrap();
        assert_eq!(found.process.unwrap().pid, harness.0.id());
        fs::copy(&transcript, project.join("copy.jsonl")).unwrap();
        {
            let mut inventories = HISTORY.0.lock().unwrap();
            inventories.get_mut(home.path()).unwrap().files.as_mut().unwrap().0 =
                Instant::now().checked_sub(DISCOVERY_CACHE_TTL * 2).unwrap();
        }
        let error = find_fresh(Some(home.path()), &id).unwrap_err();
        assert!(error.to_string().contains("multiple matching transcripts"), "{error}");
    }
}
