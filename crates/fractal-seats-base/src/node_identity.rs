//! A state directory owns a stable writer/placement identity, independent of computer names.
//! Bootstrap old installs from local launch receipts, never peer presence or host claims.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _};
use smallclaims::store::AdmittedStore;

use crate::config::{Config, STORE_DATABASE_FILES, store_dir};

const FILE: &str = "node-identity.json";

#[derive(Debug)]
pub struct StateLock {
    _file: File,
    _state_directory_lock: File,
    admitted_store: Option<Arc<AdmittedStore>>,
    state_dir: PathBuf,
}

impl StateLock {
    pub fn admitted_store(&self) -> Option<Arc<AdmittedStore>> {
        self.admitted_store.clone()
    }

    /// Inspect writer provenance under this guard without persisting a node pin.
    /// Explicit stores must first be admitted with `lock_store_directory`.
    pub fn resolve_identity(&self, config: &mut Config) -> Result<()> {
        self.verify_store_directory(config)?;
        resolve_with_admitted(config, self.admitted_store.as_deref())?;
        self.verify_store_directory(config)
    }

    /// Refuse a pathname that no longer names the directory fenced by this guard.
    pub fn verify_store_directory(&self, config: &Config) -> Result<()> {
        if let Some(admitted) = &self.admitted_store {
            return admitted.verify_directory();
        }
        let directory = store_dir(config);
        let current = fs::metadata(&directory)
            .with_context(|| format!("inspect locked store directory {}", directory.display()))?;
        let held = self._state_directory_lock.metadata()?;
        anyhow::ensure!(
            current.is_dir() && current.dev() == held.dev() && current.ino() == held.ino(),
            "configured store directory {} no longer names the locked directory; refusing replacement or symlink retarget",
            directory.display()
        );
        reject_store_database_symlinks(&directory)
    }

    /// Called only after the join handshake (or its checkpoint resume) has succeeded.
    /// An explicit admission establishes a new writer, unlike a computer-name change.
    pub fn record_fleet_join(&self, joined: &crate::fleet::join::Joined) -> Result<()> {
        self.record_admission(&joined.fleet_id, &joined.name)
    }

    pub fn record_fleet_found(&self, founded: &crate::fleet::join::Founded) -> Result<()> {
        self.record_admission(&founded.fleet_id, &founded.node)
    }

    fn record_admission(&self, fleet_id: &str, node: &str) -> Result<()> {
        let membership = crate::config::FleetFile::load(&self.state_dir)?
            .context("successful fleet admission has no saved membership")?;
        anyhow::ensure!(
            membership.node.as_deref() == Some(node) && membership.fleet_id == fleet_id,
            "saved membership does not match the successful fleet admission"
        );
        self.write_pin(node, true)
    }

    fn write_pin(&self, node: &str, replace: bool) -> Result<()> {
        anyhow::ensure!(
            !node.trim().is_empty() && node.trim() == node,
            "invalid node identity"
        );
        let path = self.state_dir.join(FILE);
        let mut pending = tempfile::NamedTempFile::new_in(&self.state_dir)?;
        serde_json::to_writer(&mut pending, node)?;
        pending.write_all(b"\n")?;
        pending.as_file().sync_all()?;
        if replace {
            pending.persist(&path)
        } else {
            pending.persist_noclobber(&path)
        }
        .with_context(|| format!("pin node identity at {}", path.display()))?;
        File::open(&self.state_dir)?.sync_all()?;
        Ok(())
    }
}

/// Hold for the daemon lifetime, including before store migration and runtime initialization.
/// The state lock also fences a second daemon using different socket paths on the same state.
pub fn lock(state_dir: &Path) -> Result<StateLock> {
    fs::create_dir_all(state_dir)?;
    // Use the directory inode as the shared ownership fence: an explicit store
    // may be another daemon's default state directory, under a different name.
    let directory_lock = lock_directory_for(state_dir, "state")?;
    let path = state_dir.join("node-identity.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)?;
    // SAFETY: the open file owns this descriptor until StateLock drops.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "another daemon owns the state directory {}",
                state_dir.display()
            )
        });
    }
    Ok(StateLock {
        _file: lock,
        _state_directory_lock: directory_lock,
        admitted_store: None,
        state_dir: state_dir.into(),
    })
}

/// Hold the daemon's store-directory fence without creating any lock or database files.
pub fn lock_directory(directory: &Path) -> Result<File> {
    lock_directory_for(directory, "store")
}

fn lock_directory_for(directory: &Path, kind: &str) -> Result<File> {
    let file = File::open(directory)
        .with_context(|| format!("open {kind} directory {} for locking", directory.display()))?;
    // SAFETY: the returned File owns the descriptor for the lock's entire lifetime.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!("another daemon owns the {kind} directory {}", directory.display())
        });
    }
    Ok(file)
}

/// Fence an explicitly configured store for daemon startup or fleet admission.
/// Directory locks add no store files and also conflict with default-state writers.
pub fn lock_store_directory(guard: &mut StateLock, config: &Config) -> Result<()> {
    anyhow::ensure!(
        fs::canonicalize(&guard.state_dir)? == fs::canonicalize(&config.state_dir)?,
        "store directory lock requires ownership of the configured state directory"
    );
    let separate_store = validate_store_location(config)?;
    if config.store_dir.is_some() {
        anyhow::ensure!(
            guard.admitted_store.is_none(),
            "configured store directory is already locked by this state guard"
        );
        let directory = store_dir(config);
        let locked = if separate_store {
            lock_directory(&directory)?
        } else {
            // A duplicate descriptor shares the existing open-file-description lock.
            guard._state_directory_lock.try_clone()?
        };
        guard.admitted_store = Some(AdmittedStore::from_locked_directory(&directory, locked)?);
    }
    guard.verify_store_directory(config)?;
    Ok(())
}

/// Service stop can return before the old daemon has closed its file descriptors.
pub async fn lock_after_stop(state_dir: &Path) -> Result<StateLock> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match lock(state_dir) {
            Err(error)
                if tokio::time::Instant::now() < deadline
                    && error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::WouldBlock) =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            result => return result,
        }
    }
}

pub fn acquire(config: &mut Config) -> Result<StateLock> {
    validate_store_location(config)?;
    let mut lock = lock(&config.state_dir)?;
    lock_store_directory(&mut lock, config)?;
    finish_acquire(config, lock)
}

fn finish_acquire(config: &mut Config, lock: StateLock) -> Result<StateLock> {
    lock.resolve_identity(config)?;
    config.validate()?;
    let path = config.state_dir.join(FILE);
    if !path.exists() {
        lock.write_pin(&config.node, false)?;
    }
    Ok(lock)
}

/// Read-only identity lookup for workers, service generation, and fleet commands.
/// Explicit stores use only the durable pin here: database provenance is admitted
/// under the exclusive directory fence during daemon startup.
pub fn resolve(config: &mut Config) -> Result<()> {
    if config.store_dir.is_some() {
        validate_store_location(config)?;
        let node = read_pin(config)?.context(
            "explicit store has no stable node identity; run guarded daemon startup or fleet admission before resolving its identity",
        )?;
        return apply_node(config, node);
    }
    resolve_with_admitted(config, None)
}

fn read_pin(config: &Config) -> Result<Option<String>> {
    let path = config.state_dir.join(FILE);
    match fs::read(&path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice::<String>(&bytes)
                .with_context(|| format!("read stable node identity {}", path.display()))?,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("read stable node identity"),
    }
}

fn resolve_with_admitted(config: &mut Config, admitted: Option<&AdmittedStore>) -> Result<()> {
    let separate_store = validate_store_location(config)?;
    let database = config.store_path();
    let pinned = read_pin(config)?;
    // The daemon records owner-local metadata even before it launches any seats.
    // Older stores can prove their writer through local receipts or a restore marker,
    // never through arbitrary replicated claim origins.
    let connection = inspect_database(config, admitted)?;
    if config.store_dir.is_some()
        && let Some(connection) = connection.as_ref()
    {
        let expected = pinned.as_deref().unwrap_or(&config.node);
        let writer = configured_writer_from(connection, expected)?;
        anyhow::ensure!(
            !separate_store || writer.is_some(),
            "cannot establish the local writer of configured store {}; preserve its database and restore the matching state directory and node identity",
            database.display()
        );
        if let Some(writer) = writer {
            anyhow::ensure!(
                writer == expected,
                "configured store {} belongs to local writer `{writer}`, but state directory {} belongs to `{expected}`; restore the matching store and node-identity.json before starting",
                database.display(),
                config.state_dir.display()
            );
        }
    }
    let node = match pinned {
        Some(node) => {
            if let Some(writer) = connection
                .as_ref()
                .map(restored_writer_from)
                .transpose()?
                .flatten()
            {
                anyhow::ensure!(
                    writer == node,
                    "restored database requires writer `{writer}`, but this directory is pinned to `{node}`; restore into a fresh state directory without an existing node pin, then configure the restored writer"
                );
            }
            Some(node)
        }
        None => connection
            .as_ref()
            .map(|connection| previous_local_node(connection, &config.node))
            .transpose()?
            .flatten(),
    };
    if let Some(node) = node {
        apply_node(config, node)?;
    }
    Ok(())
}

fn apply_node(config: &mut Config, node: String) -> Result<()> {
    anyhow::ensure!(
        !node.trim().is_empty() && node.trim() == node,
        "invalid stable node identity in {}",
        config.state_dir.display()
    );
    if let Some(member) = config.fleet.as_ref().and_then(|file| file.node.as_deref()) {
        anyhow::ensure!(
            member == node,
            "state belongs to node `{node}`, but fleet membership pins `{member}`; restore the matching configuration and keys before starting, without editing membership or deleting state"
        );
    }
    if config.node != node {
        eprintln!(
            "st: retaining stable node `{node}` for {}; configured name `{}` does not rename its writer or live seats",
            config.state_dir.display(),
            config.node
        );
        config.node = node;
    }
    Ok(())
}

/// An explicit path is an operator-selected existing directory, never a mkdir request.
/// Inspect both locations before opening SQLite or persisting a node pin.
fn validate_store_location(config: &Config) -> Result<bool> {
    let directory = store_dir(config);
    if config.store_dir.is_none() {
        reject_store_database_symlinks(&directory)?;
        return Ok(false);
    }
    let metadata = fs::metadata(&directory).with_context(|| {
        format!(
            "configured store directory (store_dir) {} must already exist and be a directory",
            directory.display()
        )
    })?;
    anyhow::ensure!(
        metadata.is_dir(),
        "configured store directory (store_dir) {} is not a directory",
        directory.display()
    );
    reject_store_database_symlinks(&directory)?;
    let configured = fs::canonicalize(&directory)?;
    let state = match fs::canonicalize(&config.state_dir) {
        Ok(state) => Some(state),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("inspect state directory for duplicate stores"),
    };
    if state.as_ref() == Some(&configured) {
        return Ok(false);
    }
    let contains_database = |directory: &Path| -> Result<bool> {
        for name in STORE_DATABASE_FILES {
            // symlink_metadata also sees broken links: they are not an empty store.
            match fs::symlink_metadata(directory.join(name)) {
                Ok(_) => return Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("inspect store database files"),
            }
        }
        Ok(false)
    };
    anyhow::ensure!(
        !(contains_database(&config.state_dir)? && contains_database(&directory)?),
        "ambiguous store locations: SQLite database files exist in both state directory {} and configured store directory {}; move the complete database, WAL and SHM together while stopped before starting",
        config.state_dir.display(),
        directory.display()
    );
    Ok(true)
}

fn reject_store_database_symlinks(directory: &Path) -> Result<()> {
    for name in STORE_DATABASE_FILES {
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "configured store database file {} must not be a symlink",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect configured store database files"),
        }
    }
    Ok(())
}

fn has_columns(connection: &Connection, table: &str, required: &[&str]) -> Result<bool> {
    // Table names are internal constants, never configuration values.
    let mut query = connection.prepare(/* sqlite-large-value: allow-for-now lv-h6a28c840b5dc owner=fractal_seats_base::node_identity TODO=trace-statement-and-value-representation */ &format!("PRAGMA table_info({table})"))?;
    let columns = query
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(required
        .iter()
        .all(|name| columns.iter().any(|column| column == name)))
}

fn restored_writer_from(connection: &Connection) -> Result<Option<String>> {
    if !has_columns(connection, "meta", &["key", "value"])? {
        return Ok(None);
    }
    Ok(connection
        .query_row(
            "SELECT value FROM meta WHERE key='backup_restore_writer'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?)
}

fn inspect_database(
    config: &Config,
    admitted: Option<&AdmittedStore>,
) -> Result<Option<Connection>> {
    if let Some(admitted) = admitted {
        admitted.verify_directory()?;
        if !admitted.database_path().try_exists()? {
            admitted.verify_directory()?;
            return Ok(None);
        }
        return admitted
            .open_connection(OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map(Some);
    }
    anyhow::ensure!(
        config.store_dir.is_none(),
        "explicit store requires its admitted directory"
    );
    if !config.store_path().try_exists()? {
        return Ok(None);
    }
    Ok(Some(Connection::open_with_flags(
        config.store_path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?))
}

/// Read-only admission preflight, before a fleet handshake can mutate membership.
pub fn check_store_writer_transition(
    config: &Config,
    admitted: Option<&AdmittedStore>,
    requested_node_id: &str,
) -> Result<()> {
    if config.store_dir.is_none() {
        return Ok(());
    }
    validate_store_location(config)?;
    let Some(connection) = inspect_database(config, admitted)? else {
        return Ok(());
    };
    if let Some(writer) = configured_writer_from(&connection, requested_node_id)? {
        anyhow::ensure!(
            writer == requested_node_id,
            "database local writer `{writer}` does not match admitted node `{requested_node_id}`; fleet admission cannot change an existing store writer"
        );
    }
    Ok(())
}

/// Record the daemon's local writer only after read-only startup identity admission
/// and normal store initialization. This permits restarting a store with no seats.
pub fn record_store_writer(
    config: &Config,
    admitted: Option<&AdmittedStore>,
    node: &str,
) -> Result<()> {
    anyhow::ensure!(
        !node.trim().is_empty() && node.trim() == node,
        "invalid local store writer"
    );
    let mut connection = if let Some(admitted) = admitted {
        admitted.open_connection(OpenFlags::SQLITE_OPEN_READ_WRITE)?
    } else {
        anyhow::ensure!(
            config.store_dir.is_none(),
            "explicit store requires its admitted directory"
        );
        Connection::open_with_flags(config.store_path(), OpenFlags::SQLITE_OPEN_READ_WRITE)?
    };
    let transaction = connection.transaction()?;
    if let Some(writer) = configured_writer_from(&transaction, node)? {
        anyhow::ensure!(
            writer == node,
            "database local writer `{writer}` does not match admitted node `{node}`"
        );
    }
    // One immutable node identity uses the existing inline meta representation.
    // Its admitted identity semantics are unchanged; bounded/reference metadata
    // and reclamation remain the producer's representation work.
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h08b5ea8feb89 owner=fractal_seats_base::node_identity TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value) VALUES('local_node_writer',?1)",
        [node],
    )?;
    transaction.commit()?;
    Ok(())
}

fn configured_writer_from(connection: &Connection, requested: &str) -> Result<Option<String>> {
    let marker = if has_columns(connection, "meta", &["key", "value"])? {
        connection
            .query_row(
                "SELECT value FROM meta WHERE key='local_node_writer'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
    } else {
        None
    };
    let restored = restored_writer_from(connection)?;
    if let Some(writer) = restored.as_deref() {
        anyhow::ensure!(
            requested == writer,
            "restored database requires fresh writer `{writer}`, but the configured store expects local writer `{requested}`; restore the matching database and node-identity.json before starting"
        );
    }
    let receipt = previous_local_receipts(connection)?;
    if let Some(writer) = marker.as_deref() {
        for provenance in [restored.as_deref(), receipt.as_deref()].into_iter().flatten() {
            anyhow::ensure!(
                writer == provenance,
                "database local writer marker `{writer}` conflicts with local writer provenance `{provenance}`; preserve the database and restore matching node state"
            );
        }
    }
    Ok(marker.or(restored).or(receipt))
}

fn previous_local_node(connection: &Connection, requested: &str) -> Result<Option<String>> {
    if let Some(writer) = restored_writer_from(connection)? {
        anyhow::ensure!(
            requested == writer,
            "restored database requires fresh writer `{writer}`; configure node to that identity before starting"
        );
        return Ok(Some(writer));
    }
    previous_local_receipts(connection)
}

fn previous_local_receipts(connection: &Connection) -> Result<Option<String>> {
    if !has_columns(
        connection,
        "local_observations",
        &["id", "subject", "kind", "body"],
    )? || !has_columns(
        connection,
        "claims",
        &["id", "subject", "kind", "body", "origin", "store_index"],
    )? {
        return Ok(None);
    }
    // Only runtime.action receipts are local. daemon.started and runtime.observed replicate.
    // A receipt binds this directory to the immutable declaration it actually launched;
    // a matching observation must still report that source's runtime running. A peer's
    // replicated declaration/observation without a local launch receipt cannot select a node.
    let mut query = connection.prepare("SELECT DISTINCT json_extract(d.body,'$.member.host')
        FROM local_observations AS o
        JOIN claims AS d ON d.id=json_extract(o.body,'$.fields.desired_token')
        JOIN claims AS r ON r.subject=o.subject AND r.kind='runtime.observed'
            AND r.origin=json_extract(d.body,'$.member.host')
        WHERE o.kind='runtime.action.succeeded'
            AND json_extract(o.body,'$.fields.action')='start'
            AND json_extract(d.body,'$.member.host') IS NOT NULL
            AND json_extract(r.body,'$.fields.runtime_id')=json_extract(o.body,'$.fields.runtime_id')
            AND json_extract(r.body,'$.fields.status')='running'
            AND (json_extract(o.body,'$.fields.incarnation_id') IS NULL
                OR json_extract(o.body,'$.fields.incarnation_id')=json_extract(r.body,'$.fields.incarnation_id'))
            AND o.id=(SELECT MAX(id) FROM local_observations WHERE subject=o.subject
                AND kind='runtime.action.succeeded' AND json_extract(body,'$.fields.action')='start')
            AND r.store_index=(SELECT MAX(store_index) FROM claims WHERE subject=r.subject
                AND kind='runtime.observed' AND origin=r.origin)")?;
    let hosts = query
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    anyhow::ensure!(
        hosts.len() <= 1,
        "ambiguous local launch placements {}; inspect node configuration and live runtimes before recovery",
        hosts.join(", ")
    );
    Ok(hosts.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admitted(config: &Config) -> Arc<AdmittedStore> {
        let directory = store_dir(config);
        AdmittedStore::from_locked_directory(&directory, lock_directory(&directory).unwrap())
            .unwrap()
    }

    #[test]
    fn explicit_pinned_resolution_succeeds_while_daemon_owns_store() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        let guard = acquire(&mut config).unwrap();
        fs::write(config.store_path(), b"not a SQLite database").unwrap();
        let admitted = guard.admitted_store().unwrap();
        // Public lookup must not open SQLite or attempt a second exclusive flock.
        admitted.set_before_open_hook_for_test(|| panic!("public resolution opened SQLite"));
        let mut worker = config.clone();
        worker.node = "new-computer-name".into();
        resolve(&mut worker).unwrap();
        assert_eq!(worker.node, "orchid");
        assert_eq!(fs::read(config.store_path()).unwrap(), b"not a SQLite database");
        worker.fleet = Some(crate::config::FleetFile {
            node: Some("fern".into()),
            ..crate::config::FleetFile::default()
        });
        assert!(
            resolve(&mut worker)
                .unwrap_err()
                .to_string()
                .contains("fleet membership pins")
        );
    }

    #[test]
    fn explicit_unpinned_resolution_requires_guarded_startup() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        assert!(
            resolve(&mut config)
                .unwrap_err()
                .to_string()
                .contains("run guarded daemon startup or fleet admission")
        );
        let mut guard = lock(&config.state_dir).unwrap();
        lock_store_directory(&mut guard, &config).unwrap();
        guard.resolve_identity(&mut config).unwrap();
        assert_eq!(config.node, "orchid");
        assert!(!config.store_path().exists());
        assert!(!config.state_dir.join(FILE).exists());
    }

    #[test]
    fn startup_refuses_store_swap_or_removal_during_sqlite_open_without_writes() {
        for replacement in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut config = configured_store(root.path());
            local_writer_database(&config.store_path(), "orchid");
            let original = fs::read(config.store_path()).unwrap();
            let directory = store_dir(&config);
            let moved = root.path().join("moved-store");
            let incoming = root.path().join("incoming-store");
            fs::create_dir(&incoming).unwrap();
            local_writer_database(&incoming.join("claims.sqlite3"), "orchid");
            let incoming_bytes = fs::read(incoming.join("claims.sqlite3")).unwrap();
            let mut guard = lock(&config.state_dir).unwrap();
            lock_store_directory(&mut guard, &config).unwrap();
            let admitted = guard.admitted_store().unwrap();
            let hook_directory = directory.clone();
            let hook_moved = moved.clone();
            let hook_incoming = incoming.clone();
            admitted.set_before_open_hook_for_test(move || {
                fs::rename(&hook_directory, &hook_moved).unwrap();
                if replacement {
                    fs::rename(&hook_incoming, &hook_directory).unwrap();
                }
            });
            assert!(finish_acquire(&mut config, guard).is_err());
            assert!(!config.state_dir.join(FILE).exists());
            assert_eq!(fs::read(moved.join("claims.sqlite3")).unwrap(), original);
            let untouched = if replacement { &directory } else { &incoming };
            assert_eq!(
                fs::read(untouched.join("claims.sqlite3")).unwrap(),
                incoming_bytes
            );
            assert_eq!(directory.exists(), replacement);
            for path in [&moved, untouched] {
                assert!(!path.join("claims.sqlite3-wal").exists());
                assert!(!path.join("claims.sqlite3-shm").exists());
            }
        }
    }

    #[test]
    fn configured_database_trio_symlinks_cannot_bypass_default_owner() {
        for name in STORE_DATABASE_FILES {
            for dangling in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let mut explicit = configured_store(root.path());
                let mut owner = Config {
                    node: "orchid".into(),
                    state_dir: root.path().join("owner"),
                    ..Config::default()
                };
                let _owner_guard = acquire(&mut owner).unwrap();
                local_writer_database(&owner.store_path(), "orchid");
                let target = if dangling {
                    owner.state_dir.join("missing")
                } else {
                    owner.store_path()
                };
                let link = store_dir(&explicit).join(name);
                std::os::unix::fs::symlink(&target, &link).unwrap();
                let before = fs::read(owner.store_path()).unwrap();
                assert!(
                    acquire(&mut explicit)
                        .unwrap_err()
                        .to_string()
                        .contains("must not be a symlink")
                );
                assert!(!explicit.state_dir.join("node-identity.lock").exists());
                assert!(!explicit.state_dir.join(FILE).exists());
                assert_eq!(fs::read(owner.store_path()).unwrap(), before);
                assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
            }
        }
    }

    #[test]
    fn default_database_trio_symlinks_are_rejected_before_mutation() {
        for name in STORE_DATABASE_FILES {
            let root = tempfile::tempdir().unwrap();
            let mut config = Config {
                node: "orchid".into(),
                state_dir: root.path().join("state"),
                ..Config::default()
            };
            fs::create_dir(&config.state_dir).unwrap();
            std::os::unix::fs::symlink(
                root.path().join("missing"),
                config.state_dir.join(name),
            )
            .unwrap();
            assert!(
                acquire(&mut config)
                    .unwrap_err()
                    .to_string()
                    .contains("must not be a symlink")
            );
            assert!(!config.state_dir.join("node-identity.lock").exists());
        }
    }

    #[test]
    fn locked_store_refuses_replacement_or_removal_without_recreation() {
        for replacement in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut config = configured_store(root.path());
            let guard = acquire(&mut config).unwrap();
            let directory = store_dir(&config);
            fs::remove_dir(&directory).unwrap();
            if replacement {
                fs::create_dir(&directory).unwrap();
            }
            assert!(guard.verify_store_directory(&config).is_err());
            assert_eq!(directory.exists(), replacement);
            assert!(!config.store_path().exists());
        }
    }

    #[test]
    fn locked_store_refuses_symlink_retarget_including_state_alias() {
        for state_alias in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut config = configured_store(root.path());
            let original = if state_alias {
                config.state_dir.clone()
            } else {
                store_dir(&config)
            };
            let alias = root.path().join("alias");
            std::os::unix::fs::symlink(&original, &alias).unwrap();
            config.store_dir = Some(alias.clone());
            let guard = acquire(&mut config).unwrap();
            guard.verify_store_directory(&config).unwrap();
            fs::remove_file(&alias).unwrap();
            let replacement = root.path().join("replacement");
            fs::create_dir(&replacement).unwrap();
            std::os::unix::fs::symlink(&replacement, &alias).unwrap();
            assert!(guard.verify_store_directory(&config).is_err());
            assert!(!config.store_path().exists());
        }
    }

    #[test]
    fn writer_transition_preflight_is_read_only_and_rejects_existing_writer_changes() {
        let root = tempfile::tempdir().unwrap();
        let config = configured_store(root.path());
        let admitted = admitted(&config);
        check_store_writer_transition(&config, Some(&admitted), "fern").unwrap();
        assert!(!config.store_path().exists());
        let connection = Connection::open(config.store_path()).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);
                 INSERT INTO meta VALUES('local_node_writer','orchid');",
            )
            .unwrap();
        let before = fs::read(config.store_path()).unwrap();
        check_store_writer_transition(&config, Some(&admitted), "orchid").unwrap();
        assert!(
            check_store_writer_transition(&config, Some(&admitted), "fern")
                .unwrap_err()
                .to_string()
                .contains("does not match admitted node")
        );
        assert_eq!(fs::read(config.store_path()).unwrap(), before);
        assert!(!config.state_dir.join(FILE).exists());
    }

    #[test]
    fn explicit_fresh_admission_records_writer_before_any_seat_launch() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        let mut guard = lock(&config.state_dir).unwrap();
        lock_store_directory(&mut guard, &config).unwrap();
        let joined = crate::fleet::join::Joined {
            fleet_id: "11111111-1111-4111-8111-111111111111".into(),
            name: "orchid-admitted".into(),
            sponsor: "fern".into(),
            writer_floor: None,
            migrate: false,
            resumed: false,
        };
        let membership = crate::config::FleetFile {
            fleet_id: joined.fleet_id.clone(),
            node: Some(joined.name.clone()),
            mode: crate::config::FleetMode::DialOut,
            ..crate::config::FleetFile::default()
        };
        membership.save(&config.state_dir).unwrap();
        let connection = Connection::open(config.store_path()).unwrap();
        connection
            .execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);")
            .unwrap();
        record_store_writer(&config, guard.admitted_store().as_deref(), &joined.name).unwrap();
        guard.record_fleet_join(&joined).unwrap();
        drop(guard);
        config.fleet = Some(membership);
        let _guard = acquire(&mut config).unwrap();
        assert_eq!(config.node, joined.name);
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name='local_observations'",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn separate_state_roots_cannot_share_one_live_explicit_store() {
        let root = tempfile::tempdir().unwrap();
        let mut first = configured_store(root.path());
        let mut second = first.clone();
        second.state_dir = root.path().join("second-state");
        let guard = acquire(&mut first).unwrap();
        assert!(
            acquire(&mut second)
                .unwrap_err()
                .to_string()
                .contains("another daemon owns the store directory")
        );
        assert!(!second.state_dir.join(FILE).exists());
        assert!(!first.store_path().exists());
        assert!(!store_dir(&first).join("node-identity.lock").exists());
        drop(guard);
        let _guard = acquire(&mut second).unwrap();
    }

    #[test]
    fn explicit_store_fences_default_state_writer_in_both_directions() {
        let root = tempfile::tempdir().unwrap();
        let mut external = configured_store(root.path());
        let mut default = Config {
            node: "orchid".into(),
            state_dir: store_dir(&external),
            ..Config::default()
        };
        let external_guard = acquire(&mut external).unwrap();
        assert!(
            acquire(&mut default)
                .unwrap_err()
                .to_string()
                .contains("another daemon owns the state directory")
        );
        assert!(!default.state_dir.join("node-identity.lock").exists());
        drop(external_guard);
        let default_guard = acquire(&mut default).unwrap();
        assert!(
            acquire(&mut external)
                .unwrap_err()
                .to_string()
                .contains("another daemon owns the store directory")
        );
        drop(default_guard);
        let _external_guard = acquire(&mut external).unwrap();
    }

    #[test]
    fn explicit_store_without_seats_restarts_using_recorded_local_writer() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        drop(acquire(&mut config).unwrap());
        let connection = Connection::open(config.store_path()).unwrap();
        connection
            .execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);")
            .unwrap();
        record_store_writer(&config, Some(&admitted(&config)), "orchid").unwrap();
        let before = fs::read(config.store_path()).unwrap();
        config.node = "new-computer-name".into();
        drop(acquire(&mut config).unwrap());
        assert_eq!(config.node, "orchid");
        assert_eq!(fs::read(config.store_path()).unwrap(), before);
        record_store_writer(&config, Some(&admitted(&config)), "orchid").unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM meta WHERE key='local_node_writer'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "orchid"
        );
        assert!(record_store_writer(&config, Some(&admitted(&config)), "fern").is_err());
        connection
            .execute("UPDATE meta SET value='fern' WHERE key='local_node_writer'", [])
            .unwrap();
        assert!(
            acquire(&mut config)
                .unwrap_err()
                .to_string()
                .contains("belongs to local writer `fern`")
        );
    }

    #[test]
    fn local_writer_marker_conflicts_are_refused_without_rewriting_metadata() {
        for restored in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut config = configured_store(root.path());
            fs::write(config.state_dir.join(FILE), b"\"orchid\"\n").unwrap();
            local_writer_database(&config.store_path(), if restored { "orchid" } else { "fern" });
            let connection = Connection::open(config.store_path()).unwrap();
            connection
                .execute("INSERT INTO meta VALUES('local_node_writer','orchid')", [])
                .unwrap();
            if restored {
                connection
                    .execute("INSERT INTO meta VALUES('backup_restore_writer','fern')", [])
                    .unwrap();
            }
            let before = fs::read(config.store_path()).unwrap();
            assert!(acquire(&mut config).is_err());
            assert!(record_store_writer(&config, Some(&admitted(&config)), "orchid").is_err());
            assert_eq!(fs::read(config.store_path()).unwrap(), before);
        }
    }

    fn configured_store(root: &Path) -> Config {
        let state = root.join("state");
        let store = root.join("store");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&store).unwrap();
        Config {
            node: "orchid".into(),
            state_dir: state,
            store_dir: Some(store),
            ..Config::default()
        }
    }

    fn local_writer_database(path: &Path, writer: &str) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);
                 CREATE TABLE local_observations(id INTEGER PRIMARY KEY,subject TEXT,kind TEXT,body TEXT);
                 CREATE TABLE claims(store_index INTEGER PRIMARY KEY,id TEXT,subject TEXT,kind TEXT,origin TEXT,body TEXT);",
            )
            .unwrap();
        let desired = serde_json::json!({"member":{"host":writer}}).to_string();
        connection.execute(
            "INSERT INTO claims VALUES(1,'desired','agent/example/one','intent.desired',?1,?2)",
            rusqlite::params![writer, desired],
        ).unwrap();
        connection.execute(
            r#"INSERT INTO claims VALUES(2,'observed','agent/example/one','runtime.observed',?1,'{"fields":{"runtime_id":"one","status":"running"}}')"#,
            [writer],
        ).unwrap();
        connection.execute_batch(
            r#"INSERT INTO local_observations VALUES(1,'agent/example/one','runtime.action.succeeded','{"fields":{"action":"start","desired_token":"desired","runtime_id":"one"}}');"#,
        ).unwrap();
    }

    #[test]
    fn empty_explicit_store_pins_only_the_state_directory() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        let _lock = acquire(&mut config).unwrap();
        assert!(config.state_dir.join(FILE).exists());
        assert!(!config.store_path().exists());
        assert!(!store_dir(&config).join(FILE).exists());
    }

    #[test]
    fn explicit_store_must_exist_and_be_a_directory_before_state_mutation() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        let missing = root.path().join("missing");
        config.store_dir = Some(missing.clone());
        let error = acquire(&mut config).unwrap_err();
        assert!(error.to_string().contains("must already exist"));
        assert!(!missing.exists());
        assert!(!config.state_dir.join("node-identity.lock").exists());
        fs::write(&missing, b"not a directory").unwrap();
        assert!(
            acquire(&mut config)
                .unwrap_err()
                .to_string()
                .contains("not a directory")
        );
        assert!(!config.state_dir.join(FILE).exists());
    }

    #[test]
    fn explicit_store_refuses_any_database_trio_in_both_locations_before_mutation() {
        for state_file in STORE_DATABASE_FILES {
            for store_file in STORE_DATABASE_FILES {
                let root = tempfile::tempdir().unwrap();
                let mut config = configured_store(root.path());
                fs::write(config.state_dir.join(state_file), b"state evidence").unwrap();
                let store_file = store_dir(&config).join(store_file);
                fs::write(&store_file, b"store evidence").unwrap();
                assert!(
                    acquire(&mut config)
                        .unwrap_err()
                        .to_string()
                        .contains("ambiguous store locations")
                );
                assert_eq!(fs::read(&store_file).unwrap(), b"store evidence");
                assert!(!config.state_dir.join("node-identity.lock").exists());
                assert!(!config.state_dir.join(FILE).exists());
            }
        }
    }

    #[test]
    fn explicit_store_accepts_same_canonical_directory() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        let alias = root.path().join("state-alias");
        std::os::unix::fs::symlink(&config.state_dir, &alias).unwrap();
        config.store_dir = Some(alias);
        fs::write(config.state_dir.join("claims.sqlite3-wal"), b"sidecar").unwrap();
        let _lock = acquire(&mut config).unwrap();
        assert_eq!(config.node, "orchid");
    }

    #[test]
    fn ancestor_symlink_store_alias_shares_canonical_state_fence() {
        let root = tempfile::tempdir().unwrap();
        let product = root.path().join("real-product");
        let legacy = root.path().join("legacy-product");
        let directory = product.join("store");
        fs::create_dir_all(&directory).unwrap();
        std::os::unix::fs::symlink(&product, &legacy).unwrap();
        let mut explicit = Config {
            node: "orchid".into(),
            state_dir: directory.clone(),
            store_dir: Some(legacy.join("store")),
            ..Config::default()
        };
        local_writer_database(&explicit.store_path(), "orchid");
        let connection = Connection::open(explicit.store_path()).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 INSERT INTO meta VALUES('local_node_writer','orchid');",
            )
            .unwrap();
        for name in STORE_DATABASE_FILES {
            assert!(directory.join(name).exists());
        }
        let guard = acquire(&mut explicit).unwrap();
        guard.verify_store_directory(&explicit).unwrap();
        assert!(guard.admitted_store().is_some());
        let mut default = Config {
            node: "orchid".into(),
            state_dir: directory,
            ..Config::default()
        };
        assert!(
            acquire(&mut default)
                .unwrap_err()
                .to_string()
                .contains("another daemon owns the state directory")
        );
        let mut separate = explicit.clone();
        separate.state_dir = root.path().join("separate-state");
        assert!(
            acquire(&mut separate)
                .unwrap_err()
                .to_string()
                .contains("another daemon owns the store directory")
        );
        drop(guard);
        let _default_guard = acquire(&mut default).unwrap();
        assert!(
            acquire(&mut explicit)
                .unwrap_err()
                .to_string()
                .contains("another daemon owns the state directory")
        );
    }

    #[test]
    fn explicit_store_local_writer_must_match_state_pin_not_replicated_origin() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        fs::write(config.state_dir.join(FILE), b"\"orchid\"\n").unwrap();
        local_writer_database(&config.store_path(), "fern");
        let connection = Connection::open(config.store_path()).unwrap();
        connection.execute(
            "INSERT INTO claims VALUES(3,'peer','daemon/orchid','daemon.started','orchid','{}')",
            [],
        ).unwrap();
        let before = fs::read(config.store_path()).unwrap();
        assert!(
            acquire(&mut config)
                .unwrap_err()
                .to_string()
                .contains("belongs to local writer `fern`")
        );
        assert_eq!(fs::read(config.store_path()).unwrap(), before);
        assert_eq!(fs::read(config.state_dir.join(FILE)).unwrap(), b"\"orchid\"\n");
        assert_eq!(config.node, "orchid");
    }

    #[test]
    fn explicit_store_refuses_unverifiable_replicated_only_database() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        local_writer_database(&config.store_path(), "orchid");
        let connection = Connection::open(config.store_path()).unwrap();
        connection.execute("DELETE FROM local_observations", []).unwrap();
        assert!(
            acquire(&mut config)
                .unwrap_err()
                .to_string()
                .contains("cannot establish the local writer")
        );
        assert!(!config.state_dir.join(FILE).exists());
        assert_eq!(
            connection.query_row("SELECT COUNT(*) FROM claims", [], |row| row.get::<_, u64>(0)).unwrap(),
            2
        );
    }

    #[test]
    fn explicit_store_matching_local_writer_preserves_pin_and_uses_external_database() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        fs::write(config.state_dir.join(FILE), b"\"orchid\"\n").unwrap();
        local_writer_database(&config.store_path(), "orchid");
        config.node = "new-computer-name".into();
        let before = fs::read(config.store_path()).unwrap();
        let _lock = acquire(&mut config).unwrap();
        assert_eq!(config.node, "orchid");
        assert_eq!(fs::read(config.store_path()).unwrap(), before);
        assert!(!config.state_dir.join("claims.sqlite3").exists());
        assert!(!store_dir(&config).join(FILE).exists());
    }

    #[test]
    fn explicit_store_restore_writer_must_match_state_pin() {
        let root = tempfile::tempdir().unwrap();
        let mut config = configured_store(root.path());
        fs::write(config.state_dir.join(FILE), b"\"orchid\"\n").unwrap();
        let connection = Connection::open(config.store_path()).unwrap();
        connection.execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);
             INSERT INTO meta VALUES('backup_restore_writer','fern');",
        ).unwrap();
        assert!(
            acquire(&mut config)
                .unwrap_err()
                .to_string()
                .contains("requires fresh writer `fern`")
        );
        connection.execute("UPDATE meta SET value='orchid'", []).unwrap();
        let _lock = acquire(&mut config).unwrap();
        assert_eq!(config.node, "orchid");
    }

    #[test]
    fn default_store_keeps_unverifiable_existing_database_behavior() {
        let root = tempfile::tempdir().unwrap();
        let connection = Connection::open(root.path().join("claims.sqlite3")).unwrap();
        connection.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
        let mut config = Config {
            node: "orchid".into(),
            state_dir: root.path().into(),
            ..Config::default()
        };
        let before = fs::read(config.store_path()).unwrap();
        let _lock = acquire(&mut config).unwrap();
        assert_eq!(config.node, "orchid");
        assert_eq!(fs::read(config.store_path()).unwrap(), before);
        assert!(config.state_dir.join(FILE).exists());
    }

    #[test]
    fn state_identity_survives_name_changes_and_reversal_and_fences_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config {
            node: "orchid".into(),
            state_dir: root.path().into(),
            ..Config::default()
        };
        let lock = acquire(&mut config).unwrap();
        let mut duplicate = config.clone();
        assert!(
            acquire(&mut duplicate)
                .unwrap_err()
                .to_string()
                .contains("another daemon")
        );
        drop(lock);
        for requested in ["orchid-laptop", "orchid"] {
            config.node = requested.into();
            let lock = acquire(&mut config).unwrap();
            assert_eq!(config.node, "orchid");
            drop(lock);
        }
    }

    #[test]
    fn upgrade_uses_local_launch_receipts_and_refuses_conflicting_membership() {
        let root = tempfile::tempdir().unwrap();
        let connection = Connection::open(root.path().join("claims.sqlite3")).unwrap();
        connection.execute_batch(r#"CREATE TABLE local_observations(id INTEGER PRIMARY KEY, subject TEXT, kind TEXT, body TEXT);
            CREATE TABLE claims(store_index INTEGER PRIMARY KEY,id TEXT,subject TEXT,kind TEXT,origin TEXT,body TEXT);
            INSERT INTO claims VALUES
            (1,'desired-one','agent/example/one','intent.desired','orchid','{"member":{"host":"orchid"}}'),
            (2,'observed-one','agent/example/one','runtime.observed','orchid','{"fields":{"runtime_id":"one","incarnation_id":"native-one","status":"running"}}'),
            (3,'remote','agent/example/remote','runtime.observed','fern','{"fields":{"runtime_id":"remote","status":"running","host":"fern"}}'),
            (4,'remote-startup','daemon/fern','daemon.started','fern','{"fields":{"status":"running"}}');
            INSERT INTO local_observations VALUES(1,'agent/example/one','runtime.action.succeeded','{"fields":{"action":"start","desired_token":"desired-one","runtime_id":"one","incarnation_id":"native-one"}}');"#).unwrap();
        let mut config = Config {
            node: "orchid-laptop".into(),
            state_dir: root.path().into(),
            ..Config::default()
        };
        resolve(&mut config).unwrap();
        assert_eq!(config.node, "orchid");
        assert!(
            !root.path().join(FILE).exists(),
            "worker inspection cannot pin state"
        );
        let fleet = crate::config::FleetFile {
            node: Some("orchid-laptop".into()),
            ..crate::config::FleetFile::default()
        };
        config.fleet = Some(fleet);
        assert!(
            resolve(&mut config)
                .unwrap_err()
                .to_string()
                .contains("fleet membership pins")
        );
        connection
            .execute("DELETE FROM local_observations", [])
            .unwrap();
        config.fleet = None;
        config.node = "orchid-laptop".into();
        resolve(&mut config).unwrap();
        assert_eq!(
            config.node, "orchid-laptop",
            "replicated history alone cannot rename this node"
        );
    }

    #[test]
    fn restore_refusal_does_not_pin_an_incorrect_writer() {
        let root = tempfile::tempdir().unwrap();
        let connection = Connection::open(root.path().join("claims.sqlite3")).unwrap();
        connection.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT); INSERT INTO meta VALUES('backup_restore_writer','restored-orchid');").unwrap();
        let mut config = Config {
            node: "orchid".into(),
            state_dir: root.path().into(),
            ..Config::default()
        };
        assert!(
            acquire(&mut config)
                .unwrap_err()
                .to_string()
                .contains("fresh writer")
        );
        assert!(!root.path().join(FILE).exists());
        config.node = "restored-orchid".into();
        let _lock = acquire(&mut config).unwrap();
        assert_eq!(config.node, "restored-orchid");
    }

    #[test]
    fn successful_explicit_join_records_its_admitted_identity_under_the_state_lock() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config {
            node: "orchid".into(),
            state_dir: root.path().into(),
            ..Config::default()
        };
        let guard = acquire(&mut config).unwrap();
        let joined = crate::fleet::join::Joined {
            fleet_id: "11111111-1111-4111-8111-111111111111".into(),
            name: "orchid-returned".into(),
            sponsor: "fern".into(),
            writer_floor: None,
            migrate: false,
            resumed: false,
        };
        assert!(guard.record_fleet_join(&joined).is_err());
        let membership = crate::config::FleetFile {
            fleet_id: joined.fleet_id.clone(),
            node: Some(joined.name.clone()),
            mode: crate::config::FleetMode::DialOut,
            ..crate::config::FleetFile::default()
        };
        membership.save(root.path()).unwrap();
        let mut conflicting = config.clone();
        conflicting.fleet = Some(membership.clone());
        assert!(
            resolve(&mut conflicting).is_err(),
            "editing fleet.toml alone cannot rebind the pin"
        );
        guard.record_fleet_join(&joined).unwrap();
        assert!(
            lock(root.path()).is_err(),
            "the join must fence concurrent daemon startup"
        );
        drop(guard);
        let mut updated = config.clone();
        updated.fleet = Some(membership);
        let guard = acquire(&mut updated).unwrap();
        assert_eq!(updated.node, "orchid-returned");
        let mismatched = crate::fleet::join::Joined {
            name: "other-member".into(),
            ..joined
        };
        assert!(guard.record_fleet_join(&mismatched).is_err());
        resolve(&mut updated).unwrap();
        assert_eq!(updated.node, "orchid-returned");
    }

    #[test]
    fn older_schema_without_receipt_columns_keeps_configured_node_for_normal_migration() {
        let root = tempfile::tempdir().unwrap();
        let connection = Connection::open(root.path().join("claims.sqlite3")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE claims(id TEXT, kind TEXT, subject TEXT, body TEXT);
            CREATE TABLE local_observations(subject TEXT, kind TEXT, body TEXT);
            INSERT INTO claims VALUES('legacy','runtime.observed','agent/example/one','{}');",
            )
            .unwrap();
        let mut config = Config {
            node: "orchid".into(),
            state_dir: root.path().into(),
            ..Config::default()
        };
        resolve(&mut config).unwrap();
        assert_eq!(config.node, "orchid");
        assert!(!root.path().join(FILE).exists());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM claims", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            1
        );
        let origin = connection
            .prepare("SELECT origin FROM claims")
            .err()
            .unwrap();
        assert!(
            origin.to_string().contains("no such column"),
            "inspection must not migrate the schema"
        );
    }

    #[test]
    fn ambiguous_local_receipts_refuse_without_pinning_or_rewriting_history() {
        let root = tempfile::tempdir().unwrap();
        let connection = Connection::open(root.path().join("claims.sqlite3")).unwrap();
        connection.execute_batch(r#"CREATE TABLE local_observations(id INTEGER PRIMARY KEY, subject TEXT, kind TEXT, body TEXT);
            CREATE TABLE claims(store_index INTEGER PRIMARY KEY,id TEXT,subject TEXT,kind TEXT,origin TEXT,body TEXT);
            INSERT INTO claims VALUES
            (1,'desired-one','agent/example/one','intent.desired','orchid','{"member":{"host":"orchid"}}'),
            (2,'observed-one','agent/example/one','runtime.observed','orchid','{"fields":{"runtime_id":"one","incarnation_id":"native-one","status":"running"}}'),
            (3,'desired-two','agent/example/two','intent.desired','orchid-old','{"member":{"host":"orchid-old"}}'),
            (4,'observed-two','agent/example/two','runtime.observed','orchid-old','{"fields":{"runtime_id":"two","incarnation_id":"native-two","status":"running"}}');
            INSERT INTO local_observations VALUES
            (1,'agent/example/one','runtime.action.succeeded','{"fields":{"action":"start","desired_token":"desired-one","runtime_id":"one","incarnation_id":"native-one"}}'),
            (2,'agent/example/two','runtime.action.succeeded','{"fields":{"action":"start","desired_token":"desired-two","runtime_id":"two","incarnation_id":"native-two"}}');"#).unwrap();
        let mut config = Config {
            node: "orchid".into(),
            state_dir: root.path().into(),
            ..Config::default()
        };
        assert!(
            acquire(&mut config)
                .unwrap_err()
                .to_string()
                .contains("ambiguous local launch")
        );
        assert!(!root.path().join(FILE).exists());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM claims", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            4
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM local_observations", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn restored_database_under_an_existing_pin_requires_a_fresh_directory() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config {
            node: "orchid".into(),
            state_dir: root.path().into(),
            ..Config::default()
        };
        drop(acquire(&mut config).unwrap());
        let connection = Connection::open(root.path().join("claims.sqlite3")).unwrap();
        connection.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT); INSERT INTO meta VALUES('backup_restore_writer','restored-orchid');").unwrap();
        config.node = "restored-orchid".into();
        assert!(
            acquire(&mut config)
                .unwrap_err()
                .to_string()
                .contains("fresh state directory")
        );
        assert_eq!(
            serde_json::from_slice::<String>(&fs::read(root.path().join(FILE)).unwrap()).unwrap(),
            "orchid"
        );
    }

    #[tokio::test]
    async fn admission_waits_for_a_stopping_daemon_to_release_its_lock() {
        let root = tempfile::tempdir().unwrap();
        let guard = lock(root.path()).unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            drop(guard);
        });
        let admitted = lock_after_stop(root.path()).await.unwrap();
        release.await.unwrap();
        assert!(lock(root.path()).is_err());
        drop(admitted);
    }
}
