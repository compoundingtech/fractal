//! Closed, owner-local reboot/restore observations. No repair, GC, hooks or restore code runs.
use super::*;
use anyhow::{Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, Metadata, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 32_768;
const MAX_DEPTH: usize = 32;
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
// Audited dotfiles nixpkgs/home-manager/modules/tmux/resilience.py, deployed source:
// publish_snapshot 525-540; retained_heads 589-620; reconstruct 1205-1338.
// This recognizes source content, not a deployment-specific executable location.
const ADAPTER_SHA256: &str = "9a421495181d8ea5013a0810f7d1602024befe7bf41569762378e12161dd7321";

struct Observation<'a> {
    operation: &'a str,
    records: BTreeMap<PathBuf, (u64, String)>,
    bytes: u64,
    entries: usize,
    exclusions: BTreeSet<PathBuf>,
    sockets: BTreeSet<PathBuf>,
    tmux_binary: Option<PathBuf>,
    roots: &'a [PathBuf],
}

pub(super) fn scan_home_stores(
    home: &Path,
    state_root: &Path,
    pty_roots: &[PathBuf],
    operation: &str,
) -> Result<Vec<RebootWitnessScan>> {
    ensure!(!operation.is_empty(), "home-store operation is invalid");
    let fractal = home.join(".local/state/fractal");
    let mut observation = Observation {
        operation,
        records: BTreeMap::new(),
        bytes: 0,
        entries: 0,
        exclusions: BTreeSet::new(),
        sockets: BTreeSet::new(),
        tmux_binary: None,
        roots: pty_roots,
    };
    // Explicitly authorized ledger exclusion. Non-requeue of revoked operations is
    // proved independently by the caller; this database also holds the witnesses.
    // The accepted reboot-witness contract explicitly authorizes exactly these
    // ledger basenames and SQLite's corresponding WAL/SHM companions.
    for root in [state_root, fractal.as_path()] {
        for name in ["claims.sqlite3", "claims.sqlite3-wal", "claims.sqlite3-shm"] {
            observation.exclusions.insert(root.join(name));
        }
        // Non-relaunch telemetry only, never a directory wildcard:
        // smallclaims/src/profile.rs:7-10 describes minute/slow/in-flight
        // diagnostics; :153 writes slow.jsonl, :1310-1311 rotates in-flight,
        // :1422 writes minutes.jsonl, :1432-1433 publishes totals atomically.
        for name in [
            "slow.jsonl", "minutes.jsonl", "in-flight.jsonl", "in-flight.jsonl.1",
            "totals.json", "totals.json.tmp",
        ] {
            observation.exclusions.insert(root.join("profile").join(name));
        }
        // fractal/src/main.rs:16442 appends API warnings; diagnostics tests at
        // :293-294 read them, not a launch/restore consumer.
        observation.exclusions.insert(root.join("driver-api-warnings.log"));
        // fractal-telemetry/src/recorder.rs:5-7,90: command audit output.
        // Receipts and all other recorder inputs remain in the bounded scan.
        observation.exclusions.insert(root.join("recorder/commands.jsonl"));
    }
    let mut roots = BTreeSet::from([
        state_root.to_path_buf(), fractal, home.join(".local/state/pty"),
        home.join(".config/tmux"), home.join(".local/state/tmux"),
        home.join(".config/systemd/user"),
    ]);
    roots.extend(pty_roots.iter().cloned());
    // These caches have no admitted restore schema. Even an operation-free cache
    // must not be mistaken for a supported adapter.
    let caches = [
        home.join(".cache/zellij"), home.join(".local/share/zellij"),
        home.join(".tmux/resurrect"), home.join(".local/share/tmux/resurrect"),
        home.join(".local/state/tmux-resurrect"), home.join(".config/tmux/plugins"),
        home.join(".tmux/plugins"),
    ];
    for root in &roots {
        observation.walk(root, 0)?;
    }
    for cache in caches {
        if let Some(metadata) = checked_metadata(&cache)? {
            ensure!(metadata.is_dir() && children(&cache)?.is_empty(), "unsupported restore cache is nonempty");
        }
        observation.walk(&cache, 0)?;
    }
    // Legacy standalone tmux config is also an input, not an omission hidden by
    // the directory inventory. Unknown control planes are refused below.
    for config in [home.join(".tmux.conf"), home.join(".config/tmux/tmux.conf")] {
        if let Some(metadata) = checked_metadata(&config)? {
            ensure!(metadata.is_file(), "unsupported tmux configuration");
            let bytes = observation.file(&config, &metadata)?;
            validate_tmux_config(&bytes)?;
        }
    }
    for root in pty_roots {
        if checked_metadata(root)?.is_none() {
            continue; // Only an actual ENOENT proves absence.
        }
        let inventory = pty_core::registry::list::list_sessions_strict_in(
            root, &pty_core::registry::list::ListOptions::default(),
        );
        ensure!(inventory.complete && inventory.errors.is_empty(), "PTY inventory is incomplete");
        for path in children(root)? {
            if path.extension().is_some_and(|extension| extension == "json") {
                let metadata = checked_metadata(&path)?.ok_or_else(|| anyhow::anyhow!("PTY metadata disappeared"))?;
                let bytes = observation.file(&path, &metadata)?;
                inspect_metadata(&bytes, &mut observation)?;
            }
        }
    }
    let sockets: Vec<_> = observation.sockets.iter().cloned().collect();
    for socket in sockets {
        inspect_tmux(&socket, &mut observation)?;
    }
    let mut digest = Sha256::new();
    for (path, (length, hash)) in &observation.records {
        hash_field(&mut digest, path.as_os_str().as_encoded_bytes());
        hash_field(&mut digest, &length.to_be_bytes());
        hash_field(&mut digest, hash.as_bytes());
    }
    for path in &observation.exclusions {
        hash_field(&mut digest, path.as_os_str().as_encoded_bytes());
    }
    Ok(vec![RebootWitnessScan {
        store: "owner-home-restore-stores.v1".into(),
        records: observation.records.len() as u64,
        bytes: observation.bytes,
        digest: format!("{:x}", digest.finalize()),
        exclusions: observation.exclusions.iter().map(|path| path.to_string_lossy().into_owned()).collect(),
    }])
}

fn hash_field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

fn checked_metadata(path: &Path) -> Result<Option<Metadata>> {
    ensure!(!path.components().any(|part| matches!(part, Component::ParentDir)), "unsafe store path");
    // Reject symlink ancestors too: checking only the leaf permits redirected roots.
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => ensure!(!metadata.file_type().is_symlink(), "unsafe store symlink"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(_) => bail!("home-store metadata is unreadable"),
        }
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!("home-store metadata is unreadable"),
    }
}

fn children(path: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(path).map_err(|_| anyhow::anyhow!("home-store enumeration is unreadable"))?;
    let mut paths = Vec::new();
    for entry in entries {
        ensure!(paths.len() < MAX_ENTRIES, "home-store entry bound exceeded");
        paths.push(entry.map_err(|_| anyhow::anyhow!("home-store enumeration is incomplete"))?.path());
    }
    paths.sort();
    Ok(paths)
}

impl Observation<'_> {
    fn inspect(&self, bytes: &[u8]) -> Result<()> {
        ensure!(!bytes.windows(self.operation.len()).any(|window| window == self.operation.as_bytes()),
            "revoked operation remains in a home restore input");
        Ok(())
    }

    fn record(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.inspect(path.as_os_str().as_encoded_bytes())?;
        self.inspect(bytes)?;
        ensure!(self.records.contains_key(path) || self.records.len() < MAX_ENTRIES, "home-store record bound exceeded");
        // Decode JSON string escapes too; raw-only matching could miss an env
        // alias whose operation was written with Unicode escapes.
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
            let normalized = serde_json::to_vec(&value)
                .map_err(|_| anyhow::anyhow!("home-store JSON encoding failed"))?;
            self.inspect(&normalized)?;
        }
        for line in bytes.split(|byte| *byte == b'\n') {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) {
                let normalized = serde_json::to_vec(&value)
                    .map_err(|_| anyhow::anyhow!("home-store JSON encoding failed"))?;
                self.inspect(&normalized)?;
            }
        }
        let value = (bytes.len() as u64, format!("{:x}", Sha256::digest(bytes)));
        if let Some(previous) = self.records.get(path) {
            ensure!(*previous == value, "home restore input changed during scan");
        } else {
            self.bytes = self.bytes.checked_add(value.0).ok_or_else(|| anyhow::anyhow!("home-store size bound exceeded"))?;
            ensure!(self.bytes <= MAX_TOTAL_BYTES, "home-store total size bound exceeded");
            self.records.insert(path.to_path_buf(), value);
        }
        Ok(())
    }

    fn file(&mut self, path: &Path, metadata: &Metadata) -> Result<Vec<u8>> {
        ensure!(metadata.is_file() && metadata.len() <= MAX_FILE_BYTES, "home-store file type or size is unsupported");
        ensure!(metadata.mode() & 0o444 != 0, "home-store file is unreadable");
        let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path).map_err(|_| anyhow::anyhow!("home-store file is unreadable"))?;
        let opened = file.metadata().map_err(|_| anyhow::anyhow!("home-store file metadata is unreadable"))?;
        ensure!(opened.is_file() && opened.dev() == metadata.dev() && opened.ino() == metadata.ino(), "home-store file changed during scan");
        let mut bytes = Vec::new();
        (&mut file).take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)
            .map_err(|_| anyhow::anyhow!("home-store file is unreadable"))?;
        ensure!(bytes.len() as u64 <= MAX_FILE_BYTES, "home-store file size bound exceeded");
        let after = checked_metadata(path)?.ok_or_else(|| anyhow::anyhow!("home-store file disappeared"))?;
        ensure!(after.dev() == opened.dev() && after.ino() == opened.ino()
            && after.len() == bytes.len() as u64 && opened.len() == after.len()
            && opened.mtime() == after.mtime() && opened.mtime_nsec() == after.mtime_nsec()
            && opened.ctime() == after.ctime() && opened.ctime_nsec() == after.ctime_nsec(),
            "home-store file changed during scan");
        self.record(path, &bytes)?;
        Ok(bytes)
    }

    fn walk(&mut self, path: &Path, depth: usize) -> Result<()> {
        ensure!(depth <= MAX_DEPTH, "home-store depth bound exceeded");
        if self.exclusions.contains(path) {
            return Ok(());
        }
        self.entries += 1;
        ensure!(self.entries <= MAX_ENTRIES, "home-store entry bound exceeded");
        let Some(metadata) = checked_metadata(path)? else {
            self.record(path, b"absent")?;
            return Ok(());
        };
        // SAFETY: geteuid(2) has no preconditions. Root-relative discovery must
        // not silently broaden this owner-local proof to another user's state.
        ensure!(metadata.uid() == unsafe { libc::geteuid() }, "home-store owner is unsupported");
        if metadata.is_dir() {
            ensure!(metadata.mode() & 0o444 != 0, "home-store directory is unreadable");
            self.record(path, b"directory")?;
            for child in children(path)? {
                self.walk(&child, depth + 1)?;
            }
            let after = checked_metadata(path)?.ok_or_else(|| anyhow::anyhow!("home-store directory disappeared"))?;
            ensure!(metadata.dev() == after.dev() && metadata.ino() == after.ino()
                && metadata.mtime() == after.mtime() && metadata.mtime_nsec() == after.mtime_nsec()
                && metadata.ctime() == after.ctime() && metadata.ctime_nsec() == after.ctime_nsec(),
                "home-store directory changed during scan");
        } else if metadata.is_file() {
            let bytes = self.file(path, &metadata)?;
            if path.extension().is_some_and(|extension| extension == "conf") {
                self.discover_adapter(&bytes)?;
            }
            if path.file_name().is_some_and(|name| name.as_encoded_bytes().ends_with(b".events.jsonl")) {
                inspect_events(&bytes)?;
            }
            if self.roots.iter().any(|root| path.parent() == Some(root.join("operations").as_path()))
                && path.extension().is_some_and(|extension| extension == "json") {
                inspect_operation(&bytes)?;
            }
        } else if metadata.file_type().is_socket() {
            // PTY sockets are not launch data; tmux sockets must be queried rather
            // than read as files. Only sockets in the closed input roots qualify.
            if !self.roots.iter().any(|root| path.parent() == Some(root.as_path())) {
                self.sockets.insert(path.to_path_buf());
            }
            self.record(path, b"socket")?;
        } else {
            bail!("unsupported home-store file type");
        }
        Ok(())
    }

    fn discover_adapter(&mut self, bytes: &[u8]) -> Result<()> {
        let text = std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("unknown tmux control-plane encoding"))?;
        for token in text.split(|character: char| character.is_whitespace() || matches!(character, '"' | '\'')) {
            if !token.ends_with("/bin/tmux-resilience") {
                continue;
            }
            let path = Path::new(token);
            ensure!(path.is_absolute(), "unknown tmux adapter source");
            let metadata = checked_metadata(path)?.ok_or_else(|| anyhow::anyhow!("tmux adapter source is missing"))?;
            let source = self.file(path, &metadata)?;
            ensure!(format!("{:x}", Sha256::digest(&source)) == ADAPTER_SHA256, "unknown tmux restore adapter version");
            let source = std::str::from_utf8(&source).map_err(|_| anyhow::anyhow!("unknown tmux adapter encoding"))?;
            let binary = source.lines().find_map(|line| line.strip_prefix("TMUX = \"").and_then(|line| line.strip_suffix('"')))
                .ok_or_else(|| anyhow::anyhow!("tmux adapter executable declaration is missing"))?;
            let binary = PathBuf::from(binary);
            ensure!(binary.is_absolute(), "unknown tmux adapter executable");
            let metadata = checked_metadata(&binary)?.ok_or_else(|| anyhow::anyhow!("tmux executable is missing"))?;
            self.file(&binary, &metadata)?;
            ensure!(self.tmux_binary.as_ref().is_none_or(|previous| previous == &binary), "conflicting tmux adapter executables");
            self.tmux_binary = Some(binary);
        }
        Ok(())
    }
}

fn inspect_metadata(bytes: &[u8], observation: &mut Observation<'_>) -> Result<()> {
    let metadata: pty_core::registry::metadata::SessionMetadata = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("malformed PTY launch metadata"))?;
    ensure!(metadata.extra.is_empty() && !metadata.command.is_empty() && !metadata.cwd.is_empty(), "unknown PTY launch metadata schema");
    ensure!(metadata.recovery.is_none(), "legacy recovery metadata schema is not admitted");
    // SessionInfo intentionally omits these. Deserialize the raw record, and scan
    // its canonical representation as well as raw bytes (JSON escapes included).
    let normalized = serde_json::to_vec(&metadata).map_err(|_| anyhow::anyhow!("PTY metadata encoding failed"))?;
    observation.inspect(&normalized)?;
    if Path::new(&metadata.command).file_name().is_some_and(|name| name == "tmux")
        || metadata.tags.as_ref().is_some_and(|tags| tags.get("run.role").is_some_and(|role| role == "tmux-server")) {
        // dotfiles tmux/lifecycle.py:293-306,540 passes an explicit -S socket
        // to the PTY controller. Never infer the PATH launcher's default socket.
        let sockets: Vec<_> = metadata.args.windows(2).filter(|pair| pair[0] == "-S").collect();
        ensure!(sockets.len() == 1, "unknown tmux controller socket schema");
        let socket = PathBuf::from(&sockets[0][1]);
        ensure!(socket.is_absolute(), "unknown tmux controller socket");
        if let Some(metadata) = checked_metadata(&socket)? {
            ensure!(metadata.file_type().is_socket(), "invalid tmux controller socket");
            observation.sockets.insert(socket.clone());
        } else {
            observation.record(&socket, b"absent")?;
        }
        for pair in metadata.args.windows(2).filter(|pair| pair[0] == "-f") {
            let config = Path::new(&pair[1]);
            ensure!(config.is_absolute(), "unknown tmux controller configuration");
            let metadata = checked_metadata(config)?.ok_or_else(|| anyhow::anyhow!("tmux controller configuration is missing"))?;
            let bytes = observation.file(config, &metadata)?;
            observation.discover_adapter(&bytes)?;
            // An arbitrary external config may import hooks or plugins from
            // outside the closed store set. Such control planes are not admitted.
            validate_tmux_config(&bytes)?;
        }
    }
    if let Some(tags) = &metadata.tags {
        if let Some(binding) = tags.get("ptyfile") {
            let path = Path::new(binding);
            ensure!(path.is_absolute(), "unsupported PTY restore binding");
            // pty-lifecycle/src/gc.rs:750-778 re-reads the sibling pty.toml,
            // not necessarily the tag's basename. Read that exact effective input.
            let path = path.parent().ok_or_else(|| anyhow::anyhow!("unsupported PTY restore binding"))?.join("pty.toml");
            let metadata = checked_metadata(&path)?.ok_or_else(|| anyhow::anyhow!("PTY restore input is missing"))?;
            observation.file(&path, &metadata)?;
            bail!("PTY file restore schema is not admitted");
        }
    }
    Ok(())
}

fn inspect_operation(bytes: &[u8]) -> Result<()> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("malformed PTY operation record"))?;
    exact_keys(&value, &["schema", "revision", "updatedAt", "operationId", "envelope", "envelopeSha256", "writer", "state", "phase", "decision", "participants", "incarnation", "retirement", "transfer"])?;
    let record: pty_core::operation::OperationRecord = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("malformed PTY operation record"))?;
    ensure!(record.schema == pty_core::operation::RECORD_SCHEMA, "unknown PTY operation schema");
    record.check().map_err(|_| anyhow::anyhow!("inconsistent PTY operation record"))?;
    let envelope = serde_json::to_vec(&record.envelope).map_err(|_| anyhow::anyhow!("malformed PTY launch envelope"))?;
    let envelope = pty_core::operation::parse_envelope(&envelope).map_err(|_| anyhow::anyhow!("unknown PTY launch envelope"))?;
    ensure!(envelope.operation_id == record.operation_id, "inconsistent PTY operation envelope");
    Ok(())
}

fn exact_keys(value: &serde_json::Value, keys: &[&str]) -> Result<()> {
    let object = value.as_object().ok_or_else(|| anyhow::anyhow!("malformed restore schema"))?;
    ensure!(object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key)), "unknown restore schema fields");
    Ok(())
}

fn inspect_events(bytes: &[u8]) -> Result<()> {
    for line in bytes.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()) {
        let event: pty_core::events::Event = serde_json::from_slice(line).map_err(|_| anyhow::anyhow!("malformed PTY event store"))?;
        if event.r#type == "user.tmux.snapshot" {
            let snapshot = event.payload.get("data").ok_or_else(|| anyhow::anyhow!("malformed tmux restore event"))?;
            validate_snapshot(snapshot)?;
        } else if event.r#type.starts_with("user.") {
            // No other user-defined event consumer is in the admitted adapter set.
            bail!("unknown PTY restore event adapter");
        }
        // No event log is excluded. In particular an older retained snapshot is
        // still an input even if a newer head exists (retained_heads, 589-620).
    }
    Ok(())
}

fn validate_snapshot(snapshot: &serde_json::Value) -> Result<()> {
    exact_keys(snapshot, &["schema", "complete", "incarnation", "ownerPtySession", "sequence", "trigger", "topology", "contentHash"])?;
    ensure!(snapshot["schema"] == 3 && snapshot["complete"] == true && snapshot["sequence"].as_u64().is_some_and(|value| value > 0), "unsupported tmux snapshot schema");
    for field in ["incarnation", "ownerPtySession", "trigger", "contentHash"] {
        ensure!(snapshot[field].as_str().is_some_and(|value| !value.is_empty()), "malformed tmux snapshot");
    }
    let topology = &snapshot["topology"];
    exact_keys(topology, &["sessions", "windows", "links"])?;
    let sessions = topology["sessions"].as_array().ok_or_else(|| anyhow::anyhow!("malformed tmux topology"))?;
    let windows = topology["windows"].as_array().ok_or_else(|| anyhow::anyhow!("malformed tmux topology"))?;
    let links = topology["links"].as_array().ok_or_else(|| anyhow::anyhow!("malformed tmux topology"))?;
    let mut session_ids = BTreeSet::new();
    let mut window_ids = BTreeSet::new();
    let mut pane_ids = BTreeSet::new();
    let mut session_names = BTreeSet::new();
    let mut session_orders = BTreeSet::new();
    for session in sessions {
        exact_keys(session, &["id", "name", "order", "created", "attached"])?;
        strings(session, &["id", "name"])?;
        numbers(session, &["order", "created", "attached"])?;
        ensure!(session_ids.insert(session["id"].as_str().unwrap()), "duplicate tmux session");
        ensure!(!session["id"].as_str().unwrap().is_empty()
            && session_names.insert(session["name"].as_str().unwrap())
            && session_orders.insert(session["order"].as_u64().unwrap()), "malformed tmux session identity or order");
    }
    ensure!(session_orders.iter().copied().eq(0..sessions.len() as u64), "noncontiguous tmux session order");
    for window in windows {
        exact_keys(window, &["id", "name", "layout", "panes"])?;
        strings(window, &["id", "name", "layout"])?;
        ensure!(window_ids.insert(window["id"].as_str().unwrap()), "duplicate tmux window");
        ensure!(!window["id"].as_str().unwrap().is_empty(), "malformed tmux window identity");
        let panes = window["panes"].as_array().ok_or_else(|| anyhow::anyhow!("malformed tmux panes"))?;
        ensure!(!panes.is_empty(), "malformed tmux panes");
        let mut indexes = BTreeSet::new();
        let mut active = 0;
        for pane in panes {
            exact_keys(pane, &["id", "index", "title", "currentPath", "startCommand", "width", "height", "left", "top", "active", "lineage"])?;
            strings(pane, &["id", "title", "currentPath", "startCommand"])?;
            numbers(pane, &["index", "width", "height", "left", "top"])?;
            ensure!(pane["active"].is_boolean() && pane["width"].as_u64() != Some(0) && pane["height"].as_u64() != Some(0), "malformed tmux pane");
            ensure!(pane["startCommand"] == "", "non-shell tmux restore command is unsupported");
            ensure!(pane_ids.insert(pane["id"].as_str().unwrap()), "duplicate tmux pane");
            ensure!(!pane["id"].as_str().unwrap().is_empty()
                && indexes.insert(pane["index"].as_u64().unwrap()), "malformed tmux pane identity or index");
            active += usize::from(pane["active"] == true);
            let lineage = &pane["lineage"];
            match lineage["kind"].as_str() {
                Some("unbound") => exact_keys(lineage, &["kind"] )?,
                Some("missing-pty") => { exact_keys(lineage, &["kind", "ptySessionId"])?; strings(lineage, &["ptySessionId"])?; },
                Some("pty") => {
                    exact_keys(lineage, &["kind", "ptySessionId", "agentId", "harnessSessionId"])?;
                    strings(lineage, &["ptySessionId"])?;
                    ensure!(["agentId", "harnessSessionId"].iter().all(|key| lineage[*key].is_null() || lineage[*key].is_string()), "malformed tmux lineage");
                },
                _ => bail!("unknown tmux lineage schema"),
            }
        }
        ensure!(active == 1, "malformed tmux active pane count");
    }
    let mut memberships = BTreeSet::new();
    let mut linked_windows = BTreeSet::new();
    let mut active_windows = BTreeMap::<&str, usize>::new();
    for link in links {
        exact_keys(link, &["sessionId", "windowId", "index", "active"])?;
        strings(link, &["sessionId", "windowId"])?;
        numbers(link, &["index"])?;
        ensure!(link["active"].is_boolean() && session_ids.contains(link["sessionId"].as_str().unwrap()) && window_ids.contains(link["windowId"].as_str().unwrap()), "malformed tmux link");
        let session = link["sessionId"].as_str().unwrap();
        ensure!(memberships.insert((session, link["index"].as_u64().unwrap())), "duplicate tmux window membership");
        linked_windows.insert(link["windowId"].as_str().unwrap());
        *active_windows.entry(session).or_default() += usize::from(link["active"] == true);
    }
    ensure!(linked_windows == window_ids && session_ids.iter().all(|session| active_windows.get(session) == Some(&1)), "malformed tmux window memberships");
    // pty-core enables preserve_order; explicitly sort recursively to match
    // Python canonical(sort_keys=True), independently of unified crate features.
    let mut canonical = topology.clone();
    canonical.sort_all_objects();
    let canonical = serde_json::to_vec(&canonical).map_err(|_| anyhow::anyhow!("malformed tmux topology"))?;
    ensure!(snapshot["contentHash"].as_str() == Some(format!("{:x}", Sha256::digest(&canonical)).as_str()), "tmux snapshot digest mismatch");
    Ok(())
}

fn strings(value: &serde_json::Value, keys: &[&str]) -> Result<()> {
    ensure!(keys.iter().all(|key| value[*key].as_str().is_some_and(|text| !text.contains('\0'))), "malformed restore string");
    Ok(())
}

fn numbers(value: &serde_json::Value, keys: &[&str]) -> Result<()> {
    ensure!(keys.iter().all(|key| value[*key].as_u64().is_some()), "malformed restore integer");
    Ok(())
}

fn validate_tmux_config(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("unsupported tmux configuration encoding"))?;
    // Config files are executable control planes. Do not infer safety by absence
    // of the UUID in arbitrary source-file/run-shell/plugin invocations.
    ensure!(text.lines().all(|line| line.trim().is_empty() || line.trim_start().starts_with('#')), "unknown tmux configuration control plane");
    Ok(())
}

fn query_tmux(binary: &Path, socket: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut child = Command::new(binary).args(["-N", "-S"]).arg(socket).args(args)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().map_err(|_| anyhow::anyhow!("tmux restore observation unavailable"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("tmux restore observation unavailable"))?;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map(|_| bytes);
        let _ = send.send(result);
    });
    let result = receive.recv_timeout(QUERY_TIMEOUT);
    let deadline = Instant::now() + QUERY_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline && result.is_ok() => std::thread::sleep(Duration::from_millis(10)),
            _ => { let _ = child.kill(); let _ = child.wait(); break None; },
        }
    };
    let _ = reader.join();
    let bytes = result.map_err(|_| anyhow::anyhow!("tmux restore observation timed out"))?
        .map_err(|_| anyhow::anyhow!("tmux restore observation unreadable"))?;
    ensure!(status.is_some_and(|status| status.success()) && bytes.len() as u64 <= MAX_FILE_BYTES, "tmux restore observation failed or exceeded bound");
    Ok(bytes)
}

fn inspect_tmux(socket: &Path, observation: &mut Observation<'_>) -> Result<()> {
    // PATH may contain the managed launcher (which can create a server). Only
    // the real binary named by the content-pinned adapter may be invoked here.
    let binary = observation.tmux_binary.clone()
        .ok_or_else(|| anyhow::anyhow!("tmux socket has no admitted configured adapter"))?;
    for (index, args) in [
        vec!["show-environment", "-g"], vec!["show-options", "-g"],
        vec!["show-options", "-s"], vec!["show-hooks", "-g"],
        vec!["list-sessions", "-F", "#{session_id}"],
    ].iter().enumerate() {
        let bytes = query_tmux(&binary, socket, args)?;
        observation.record(&socket.with_extension(format!("witness-query-{index}")), &bytes)?;
        if index == 1 { validate_tmux_options(&bytes)?; }
        if index == 3 { validate_hooks(&bytes, observation)?; }
        if index == 4 {
            let text = std::str::from_utf8(&bytes).map_err(|_| anyhow::anyhow!("malformed tmux session inventory"))?;
            for session in text.lines() {
                ensure!(session.starts_with('$') && session[1..].bytes().all(|byte| byte.is_ascii_digit()) && session.len() > 1, "malformed tmux session identity");
                for (kind, args) in [
                    ("env", vec!["show-environment", "-t", session]),
                    ("options", vec!["show-options", "-t", session]),
                    ("hooks", vec!["show-hooks", "-t", session]),
                ] {
                    let bytes = query_tmux(&binary, socket, &args)?;
                    observation.record(&socket.with_extension(format!("witness-{session}-{kind}")), &bytes)?;
                    if kind == "options" { validate_tmux_options(&bytes)?; }
                    if kind == "hooks" { validate_hooks(&bytes, observation)?; }
                }
            }
        }
    }
    Ok(())
}

fn validate_tmux_options(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("malformed tmux options"))?;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("default-command ") {
            ensure!(value == "\"\"" || value.is_empty(), "tmux default command is unsupported");
        }
        if let Some(value) = line.strip_prefix("default-shell ") {
            let value = value.trim_matches('"');
            ensure!(Path::new(value).is_absolute() && ["sh", "bash", "zsh", "fish"].iter().any(|name| Path::new(value).file_name().is_some_and(|file| file == *name)), "tmux default shell is unsupported");
        }
        ensure!(!line.to_ascii_lowercase().contains("resurrect") && !line.to_ascii_lowercase().contains("continuum") && !line.starts_with("@plugin "), "unknown tmux restore adapter");
    }
    Ok(())
}

fn validate_hooks(bytes: &[u8], observation: &mut Observation<'_>) -> Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("malformed tmux hooks"))?;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Some((_, command)) = line.split_once(' ') else { bail!("unknown tmux hook"); };
        let command = command.strip_prefix("run-shell -b \"").or_else(|| command.strip_prefix("run-shell \""))
            .ok_or_else(|| anyhow::anyhow!("unknown tmux hook adapter"))?;
        let (source, arguments) = command.split_once(' ').ok_or_else(|| anyhow::anyhow!("unknown tmux hook adapter"))?;
        let path = Path::new(source);
        ensure!(path.is_absolute() && path.file_name().is_some_and(|name| name == "tmux-resilience"), "unknown tmux hook adapter");
        let known = arguments == "commit-binding --socket #{q:socket_path} --pane #{q:pane_id} --pty-session #{q:@agent-pty-session-id}\""
            || arguments.strip_prefix("request --socket #{q:socket_path} --trigger ").and_then(|trigger| trigger.strip_suffix('"'))
                .is_some_and(|trigger| !trigger.is_empty() && trigger.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'));
        ensure!(known, "unknown tmux hook invocation");
        let metadata = checked_metadata(path)?.ok_or_else(|| anyhow::anyhow!("tmux adapter source is missing"))?;
        let bytes = observation.file(path, &metadata)?;
        ensure!(format!("{:x}", Sha256::digest(&bytes)) == ADAPTER_SHA256, "unknown tmux restore adapter version");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OPERATION: &str = "ed2d1877-4114-4f47-92fa-b1fc01e4fedd";

    fn observation() -> Observation<'static> {
        Observation {
            operation: OPERATION, records: BTreeMap::new(), bytes: 0, entries: 0,
            exclusions: BTreeSet::new(), sockets: BTreeSet::new(), tmux_binary: None, roots: &[],
        }
    }

    #[test]
    fn empty_owner_stores_are_positive_and_deterministic() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        let first = scan_home_stores(home.path(), &state, &[], OPERATION).unwrap();
        let second = scan_home_stores(home.path(), &state, &[], OPERATION).unwrap();
        assert_eq!(first, second);
        assert!(first[0].records > 0);
        assert_eq!(first[0].exclusions.len(), 22);
    }

    #[test]
    fn telemetry_exclusions_are_exact_and_never_hide_receipts_or_restore_inputs() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        fs::create_dir_all(state.join("profile")).unwrap();
        fs::create_dir_all(state.join("recorder")).unwrap();
        for name in ["slow.jsonl", "minutes.jsonl", "in-flight.jsonl", "in-flight.jsonl.1", "totals.json", "totals.json.tmp"] {
            fs::write(state.join("profile").join(name), OPERATION).unwrap();
        }
        fs::write(state.join("driver-api-warnings.log"), OPERATION).unwrap();
        fs::write(state.join("recorder/commands.jsonl"), OPERATION).unwrap();
        assert!(scan_home_stores(home.path(), &state, &[], OPERATION).is_ok());
        fs::write(state.join("recorder/receipts.jsonl"), OPERATION).unwrap();
        assert!(scan_home_stores(home.path(), &state, &[], OPERATION).is_err());
        fs::remove_file(state.join("recorder/receipts.jsonl")).unwrap();
        fs::write(state.join("profile/restore.json"), OPERATION).unwrap();
        assert!(scan_home_stores(home.path(), &state, &[], OPERATION).is_err());
    }

    #[test]
    fn raw_environment_aliases_tags_and_command_are_restore_inputs() {
        for field in ["extraEnv", "sessionEnv", "env", "tags", "command", "args"] {
            let mut value = json!({"command":"/bin/sh","args":[],"cwd":"/tmp"});
            value[field] = match field {
                "command" => json!(OPERATION),
                "args" => json!([OPERATION]),
                _ => json!({"ST3_START_OPERATION":OPERATION,"FRACTAL_START_OPERATION":OPERATION}),
            };
            assert!(inspect_metadata(&serde_json::to_vec(&value).unwrap(), &mut observation()).is_err());
        }
    }

    #[test]
    fn unknown_or_malformed_launch_metadata_refuses() {
        for value in [
            json!({"command":"/bin/sh","cwd":"/tmp","futureRelaunch":{}}),
            json!({"command":"/bin/sh","cwd":"/tmp","extraEnv":[]}),
            json!({"command":"/bin/sh","cwd":"/tmp","args":3}),
        ] {
            assert!(inspect_metadata(&serde_json::to_vec(&value).unwrap(), &mut observation()).is_err());
        }
        assert!(inspect_metadata(br#"{"command":"/bin/sh","cwd":"/tmp","args":[]}"#, &mut observation()).is_ok());
    }

    #[test]
    fn operation_record_hits_and_unknown_schema_refuse() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("pty");
        fs::create_dir_all(root.join("operations")).unwrap();
        fs::write(root.join("operations/input.json"), format!(r#"{{"schema":"pty.launch-operation.v1","envelope":{{"operationId":"{OPERATION}"}}}}"#)).unwrap();
        assert!(scan_home_stores(home.path(), &home.path().join("state"), &[root.clone()], OPERATION).is_err());
        fs::write(root.join("operations/input.json"), br#"{"schema":"pty.launch-operation.v2"}"#).unwrap();
        assert!(scan_home_stores(home.path(), &home.path().join("state"), &[root], OPERATION).is_err());
    }

    #[test]
    fn ptyfile_effective_sibling_is_read_and_never_excluded() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("pty.toml");
        fs::write(&file, OPERATION).unwrap();
        let value = json!({"command":"/bin/sh","cwd":"/tmp","tags":{"ptyfile":file}});
        let error = inspect_metadata(&serde_json::to_vec(&value).unwrap(), &mut observation()).unwrap_err();
        assert!(error.to_string().contains("revoked operation"));
        assert!(!error.to_string().contains(OPERATION));
    }

    fn snapshot() -> serde_json::Value {
        let topology = json!({"sessions":[],"windows":[],"links":[]});
        let mut canonical = topology.clone();
        canonical.sort_all_objects();
        json!({
            "schema":3,"complete":true,"incarnation":"known","ownerPtySession":"owner",
            "sequence":1,"trigger":"test","topology":topology,
            "contentHash":format!("{:x}",Sha256::digest(serde_json::to_vec(&canonical).unwrap()))
        })
    }

    #[test]
    fn snapshot_schema_and_digest_are_validated() {
        let mut value = snapshot();
        assert!(validate_snapshot(&value).is_ok());
        value["schema"] = json!(4);
        assert!(validate_snapshot(&value).is_err());
        value = snapshot();
        value["contentHash"] = json!("invalid");
        assert!(validate_snapshot(&value).is_err());
    }

    #[test]
    fn snapshot_event_hits_are_not_hidden_as_telemetry() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        fs::create_dir(&state).unwrap();
        let mut value = snapshot();
        value["trigger"] = json!(OPERATION);
        fs::write(state.join("owner.events.jsonl"), serde_json::to_vec(&json!({
            "session":"owner","type":"user.tmux.snapshot","ts":"now","data":value
        })).unwrap()).unwrap();
        assert!(scan_home_stores(home.path(), &state, &[], OPERATION).is_err());
    }

    #[test]
    fn unknown_adapter_cache_and_hooks_refuse() {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join(".cache/zellij");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("restore"), b"safe-looking").unwrap();
        assert!(scan_home_stores(home.path(), &home.path().join("state"), &[], OPERATION).is_err());
        assert!(validate_hooks(b"session-created[0] run-shell \"unknown-adapter\"\n", &mut observation()).is_err());
    }

    #[test]
    fn exact_ledger_exclusions_do_not_hide_launch_files() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        fs::create_dir(&state).unwrap();
        for name in ["claims.sqlite3", "claims.sqlite3-wal", "claims.sqlite3-shm"] {
            fs::write(state.join(name), OPERATION).unwrap();
        }
        let scans = scan_home_stores(home.path(), &state, &[], OPERATION).unwrap();
        assert!(scans[0].exclusions.contains(&state.join("claims.sqlite3").to_string_lossy().into_owned()));
        fs::write(state.join("launch.json"), OPERATION).unwrap();
        assert!(scan_home_stores(home.path(), &state, &[], OPERATION).is_err());
    }

    #[test]
    fn symlink_unreadable_type_and_size_bound_refuse() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        fs::create_dir(&state).unwrap();
        std::os::unix::fs::symlink(home.path().join("absent"), state.join("restore")).unwrap();
        assert!(scan_home_stores(home.path(), &state, &[], OPERATION).is_err());
        fs::remove_file(state.join("restore")).unwrap();
        let file = fs::File::create(state.join("large")).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(scan_home_stores(home.path(), &state, &[], OPERATION).is_err());
        assert!(children(&state.join("large")).is_err());
    }

    #[test]
    fn unreadable_file_and_directory_refuse_even_with_elevated_reader() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        fs::create_dir(&state).unwrap();
        let file = state.join("restore");
        fs::write(&file, b"input").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0)).unwrap();
        assert!(scan_home_stores(home.path(), &state, &[], OPERATION).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0)).unwrap();
        let result = scan_home_stores(home.path(), &state, &[], OPERATION);
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn escaped_operation_and_aggregate_bounds_refuse() {
        let mut observed = observation();
        let encoded = format!("{{\n\"env\":{{\"ST3_START_OPERATION\":\"{}\"}}\n}}", OPERATION.replace('e', "\\u0065"));
        assert!(observed.record(Path::new("/fixture"), encoded.as_bytes()).is_err());
        observed.bytes = MAX_TOTAL_BYTES;
        assert!(observed.record(Path::new("/fixture"), b"input").is_err());
        observed.entries = MAX_ENTRIES;
        assert!(observed.walk(Path::new("/fixture"), 0).is_err());
        assert!(observation().walk(Path::new("/fixture"), MAX_DEPTH + 1).is_err());
    }
}
