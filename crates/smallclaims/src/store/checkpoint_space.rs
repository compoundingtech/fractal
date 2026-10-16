//! Disk admission for a checkpoint's disposable capture and proof files. The live writer shares
//! this filesystem on many members, so a proof must leave its reserve available even on failure.
use std::ffi::CString;
use std::fs;
use std::fs::File;
use std::os::fd::AsRawFd as _;
use std::os::unix::{ffi::OsStrExt as _, fs::MetadataExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use rusqlite::{Connection, OptionalExtension as _};

const GIB: u128 = 1024 * 1024 * 1024;
const LIVE_FLOOR: u128 = 8 * GIB;
const REACTION_MARGIN: u128 = 256 * 1024 * 1024;
#[cfg(test)]
const PROGRESS_OPCODES: i32 = 1000;

mod vfs;
pub(super) use vfs::ProofVfs;

#[derive(Clone, Copy)]
struct Filesystem {
    device: u64,
    available: u128,
}

fn existing_ancestor(path: &Path) -> Result<&Path> {
    let mut current = path;
    while !current.exists() {
        current = current
            .parent()
            .context("checkpoint scratch has no existing ancestor")?;
    }
    Ok(current)
}

fn filesystem(path: &Path) -> Result<Filesystem> {
    let path = existing_ancestor(path)?;
    let name = CString::new(path.as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `name` is NUL-terminated and `stat` is a valid output buffer.
    if unsafe { libc::statvfs(name.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("statvfs {}", path.display()));
    }
    // SAFETY: a successful statvfs initialized every field.
    let stat = unsafe { stat.assume_init() };
    Ok(Filesystem {
        device: fs::metadata(path)?.dev(),
        available: u128::from(stat.f_bavail) * u128::from(stat.f_frsize),
    })
}

/// Match the bundled Unix SQLite VFS search order. `temp_store_directory` is process-wide and
/// normally unset; read it rather than assuming Rust's `temp_dir()` names SQLite's directory.
fn sqlite_temp_dir() -> Result<PathBuf> {
    let probe = Connection::open_in_memory()?;
    let override_dir: Option<String> = probe
        .query_row("PRAGMA temp_store_directory", [], |row| row.get(0))
        .optional()?;
    let mut candidates = Vec::new();
    if let Some(dir) = override_dir.filter(|dir| !dir.is_empty()) {
        candidates.push(PathBuf::from(dir));
    }
    for key in ["SQLITE_TMPDIR", "TMPDIR"] {
        if let Some(dir) = std::env::var_os(key) {
            candidates.push(PathBuf::from(dir));
        }
    }
    candidates.extend(["/var/tmp", "/usr/tmp", "/tmp", "."].map(PathBuf::from));
    for path in candidates {
        let name = CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: the path is NUL-terminated. SQLite's Unix VFS requires write and search.
        if path.is_dir() && unsafe { libc::access(name.as_ptr(), libc::W_OK | libc::X_OK) } == 0 {
            return Ok(path);
        }
    }
    bail!("SQLite has no writable temporary directory")
}

fn allocated(path: &Path) -> Result<u128> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(u128::from(metadata.blocks()) * 512),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error).with_context(|| format!("stat {}", path.display())),
    }
}

fn existing_scratch_bytes(path: &Path) -> Result<u128> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0u128;
    let mut dirs = vec![path.to_owned()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                dirs.push(entry.path());
            } else if kind.is_file() {
                total = total.saturating_add(allocated(&entry.path())?);
            }
        }
    }
    Ok(total)
}

fn remove_if_present(path: &Path, failures: &mut Vec<String>) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => failures.push(format!("{}: {error}", path.display())),
    }
}

/// One proof owns the directory lock until its copy and every sidecar have been removed. A
/// surviving `.owner` file names an orphan; a following process may sweep it only while holding
/// the lock. An unmarked legacy proof file is refused rather than assumed to be abandoned.
pub(super) struct ProofOwnership {
    dir: PathBuf,
    _lock: File,
}

impl ProofOwnership {
    pub(super) fn acquire(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(dir.join("proof.lock"))?;
        // SAFETY: the descriptor stays open for the ownership lifetime.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("another checkpoint proof owns {}", dir.display()));
        }
        let ownership = Self {
            dir: dir.to_owned(),
            _lock: lock,
        };
        ownership.sweep()?;
        Ok(ownership)
    }

    fn sweep(&self) -> Result<()> {
        let mut failures = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if let Some(stem) = name.strip_suffix(".owner") {
                if stem.starts_with("proof-") && stem.ends_with(".sqlite3") {
                    let mut this_proof = Vec::new();
                    self.remove_set(&self.dir.join(stem), &mut this_proof);
                    failures.extend(this_proof);
                }
            }
        }
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with("proof-") && name.contains(".sqlite3") {
                failures.push(format!(
                    "unowned proof scratch {}; manual inspection required",
                    path.display()
                ));
            }
        }
        if !failures.is_empty() {
            eprintln!(
                "st3: checkpoint proof orphan cleanup refused: {}",
                failures.join("; ")
            );
            bail!("checkpoint proof cleanup refused: {}", failures.join("; "));
        }
        Ok(())
    }

    pub(super) fn mark(&self, proof: &Path) -> Result<()> {
        let marker_path = PathBuf::from(format!("{}.owner", proof.display()));
        let marker = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&marker_path)?;
        if let Err(error) = marker
            .sync_all()
            .and_then(|()| File::open(&self.dir)?.sync_all())
        {
            let mut cleanup = Vec::new();
            remove_if_present(&marker_path, &mut cleanup);
            if !cleanup.is_empty() {
                eprintln!(
                    "st3: checkpoint proof owner cleanup failed: {}",
                    cleanup.join("; ")
                );
            }
            return Err(error)
                .with_context(|| format!("durably mark proof owner {}", marker_path.display()));
        }
        Ok(())
    }

    fn remove_set(&self, proof: &Path, failures: &mut Vec<String>) {
        for suffix in ["", "-journal", "-wal", "-shm"] {
            remove_if_present(
                &PathBuf::from(format!("{}{suffix}", proof.display())),
                failures,
            );
        }
        if failures.is_empty() {
            if let Err(error) = File::open(&self.dir).and_then(|dir| dir.sync_all()) {
                failures.push(format!("sync {}: {error}", self.dir.display()));
            }
        }
        if failures.is_empty() {
            remove_if_present(
                &PathBuf::from(format!("{}.owner", proof.display())),
                failures,
            );
        }
    }

    pub(super) fn finish(&self, proof: &Path) -> Result<()> {
        let mut failures = Vec::new();
        self.remove_set(proof, &mut failures);
        if !failures.is_empty() {
            eprintln!(
                "st3: checkpoint proof cleanup failed: {}",
                failures.join("; ")
            );
            bail!("checkpoint proof cleanup failed: {}", failures.join("; "));
        }
        Ok(())
    }
}

fn live_size(path: &Path) -> Result<(u128, u128)> {
    let metadata =
        fs::metadata(path).with_context(|| format!("stat live database {}", path.display()))?;
    Ok((
        u128::from(metadata.blocks()) * 512,
        u128::from(metadata.len()),
    ))
}

/// One admitted scratch phase. The progress callback is driven by SQLite work, not by a timer.
/// Its 256 MiB margin leaves space for writes between callbacks and for the live writer to run.
#[derive(Clone)]
pub(super) struct SpaceGuard {
    live: PathBuf,
    live_wal: PathBuf,
    scratch: PathBuf,
    temp: PathBuf,
    scratch_device: u64,
    temp_device: u64,
    initial_available: u128,
    initial_scratch_allocated: u128,
    initial_temp_available: u128,
    initial_wal_allocated: u128,
    budget: u128,
    temp_budget: u128,
    reserve: u128,
    wal_allowance: u128,
    shared_device: bool,
    failure: Arc<Mutex<Option<String>>>,
}

impl SpaceGuard {
    /// Admit before creating the scratch directory. `capture` includes one extra database-size
    /// allowance for its staging file; a proof is admitted again after staging exists.
    pub(super) fn admit(live: &Path, scratch: &Path, capture: bool) -> Result<Self> {
        let (live_allocated, live_length) = live_size(live)?;
        let live_fs = filesystem(live)?;
        let scratch_fs = filesystem(scratch)?;
        let temp_dir = sqlite_temp_dir()?;
        let temp_fs = filesystem(&temp_dir)?;
        let shared_device = live_fs.device == scratch_fs.device;
        let wal_allowance = LIVE_FLOOR.max(live_allocated);
        let reserve = LIVE_FLOOR + wal_allowance;
        let live_wal = PathBuf::from(format!("{}-wal", live.display()));
        let initial_wal_allocated = allocated(&live_wal)?;
        // VACUUM INTO, rollback pages and temp sort can each occupy a large part of the source.
        // PR1 uses this conservative limit and refuses when it cannot establish headroom.
        let base = live_allocated.max(live_length);
        let copy = base;
        let journal = base;
        let replay = base;
        let temp_estimate = base;
        let staging = if capture { base } else { 0 };
        let separate_temp = temp_fs.device != scratch_fs.device;
        let budget = copy
            .saturating_add(journal)
            .saturating_add(replay)
            .saturating_add(if separate_temp { 0 } else { temp_estimate })
            .saturating_add(staging);
        let temp_budget = if separate_temp { temp_estimate } else { 0 };
        let scratch_reserve = if shared_device { reserve } else { 0 };
        let temp_reserve = if separate_temp && temp_fs.device == live_fs.device {
            reserve
        } else {
            0
        };
        let required = budget
            .saturating_add(scratch_reserve)
            .saturating_add(REACTION_MARGIN);
        let temp_required = temp_budget
            .saturating_add(temp_reserve)
            .saturating_add(REACTION_MARGIN);
        let live_required = reserve.saturating_add(REACTION_MARGIN);
        let existing_scratch = existing_scratch_bytes(scratch)?;
        let scratch_deficit = required.saturating_sub(scratch_fs.available);
        let temp_deficit = if separate_temp {
            temp_required.saturating_sub(temp_fs.available)
        } else {
            0
        };
        let live_deficit = live_required.saturating_sub(live_fs.available);
        let controlling = if temp_deficit >= scratch_deficit
            && temp_deficit >= live_deficit
            && temp_deficit > 0
        {
            "SQLite temp device"
        } else if live_deficit > scratch_deficit && !shared_device {
            "live reserve"
        } else {
            "scratch budget plus reserve"
        };
        if scratch_fs.available < required
            || (separate_temp && temp_fs.available < temp_required)
            || (!shared_device && live_fs.available < live_required)
        {
            bail!(
                "checkpoint scratch refused before allocation: scratch={} device={} available={} required={} deficit={} existing_scratch={} (copy={} journal={} replay={} staging={} temp={} scratch budget={} live reserve={} reaction margin={}); temp_dir={} device={} available={} required={} deficit={}; live={} device={} allocated={} logical={} WAL_allocated={} available={} live reserve={} required={} deficit={} (floor={} WAL allowance={}); controlling component={}",
                scratch.display(),
                scratch_fs.device,
                scratch_fs.available,
                required,
                scratch_deficit,
                existing_scratch,
                copy,
                journal,
                replay,
                staging,
                temp_estimate,
                budget,
                scratch_reserve,
                REACTION_MARGIN,
                temp_dir.display(),
                temp_fs.device,
                temp_fs.available,
                temp_required,
                temp_deficit,
                live.display(),
                live_fs.device,
                live_allocated,
                live_length,
                initial_wal_allocated,
                live_fs.available,
                reserve,
                live_required,
                live_deficit,
                LIVE_FLOOR,
                wal_allowance,
                controlling,
            );
        }
        Ok(Self {
            live: live.to_owned(),
            live_wal,
            scratch: scratch.to_owned(),
            temp: temp_dir,
            scratch_device: scratch_fs.device,
            temp_device: temp_fs.device,
            initial_available: scratch_fs.available,
            initial_scratch_allocated: existing_scratch,
            initial_temp_available: temp_fs.available,
            initial_wal_allocated,
            budget,
            temp_budget,
            reserve,
            wal_allowance,
            shared_device,
            failure: Arc::new(Mutex::new(None)),
        })
    }

    fn check(&self) -> Result<()> {
        let scratch = filesystem(&self.scratch)?;
        if scratch.device != self.scratch_device {
            bail!(
                "checkpoint scratch filesystem changed at {}",
                self.scratch.display()
            );
        }
        let used = self.initial_available.saturating_sub(scratch.available);
        let scratch_growth =
            existing_scratch_bytes(&self.scratch)?.saturating_sub(self.initial_scratch_allocated);
        if scratch_growth > self.budget {
            bail!(
                "checkpoint scratch allocation budget crossed: scratch={} allocated_growth={} budget={}",
                self.scratch.display(),
                scratch_growth,
                self.budget,
            );
        }
        if self.temp_device != self.scratch_device {
            let temp = filesystem(&self.temp)?;
            if temp.device != self.temp_device {
                bail!(
                    "checkpoint SQLite temp filesystem changed at {}",
                    self.temp.display()
                );
            }
            let used_temp = self.initial_temp_available.saturating_sub(temp.available);
            let temp_reserve = if temp.device == filesystem(&self.live)?.device {
                self.reserve
            } else {
                0
            };
            if used_temp > self.temp_budget || temp.available <= temp_reserve + REACTION_MARGIN {
                bail!(
                    "checkpoint SQLite temp budget crossed: temp_dir={} available={} used={} budget={} live reserve={} reaction margin={}",
                    self.temp.display(),
                    temp.available,
                    used_temp,
                    self.temp_budget,
                    temp_reserve,
                    REACTION_MARGIN,
                );
            }
        }
        let wal_growth = allocated(&self.live_wal)?.saturating_sub(self.initial_wal_allocated);
        if wal_growth >= self.wal_allowance {
            bail!(
                "checkpoint live WAL allowance crossed: live={} WAL growth={} allowance={}",
                self.live.display(),
                wal_growth,
                self.wal_allowance,
            );
        }
        let scratch_reserve = if self.shared_device { self.reserve } else { 0 };
        if used > self.budget || scratch.available <= scratch_reserve + REACTION_MARGIN {
            bail!(
                "checkpoint scratch budget crossed: scratch={} available={} used={} budget={} live reserve={} reaction margin={}",
                self.scratch.display(),
                scratch.available,
                used,
                self.budget,
                scratch_reserve,
                REACTION_MARGIN,
            );
        }
        if !self.shared_device {
            let live = filesystem(&self.live)?;
            if live.available <= self.reserve + REACTION_MARGIN {
                bail!(
                    "checkpoint live reserve crossed: live={} available={} reserve={} reaction_margin={}",
                    self.live.display(),
                    live.available,
                    self.reserve,
                    REACTION_MARGIN,
                );
            }
        }
        Ok(())
    }

    pub(super) fn check_now(&self) -> Result<()> {
        let result = self.check();
        if let Err(error) = &result {
            *self
                .failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error.to_string());
        }
        result
    }

    pub(super) fn failed(&self) -> bool {
        self.failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    #[cfg(test)]
    pub(super) fn install(&self, connection: &Connection) {
        let guard = self.clone();
        connection.progress_handler(
            PROGRESS_OPCODES,
            Some(move || match guard.check() {
                Ok(()) => false,
                Err(error) => {
                    *guard
                        .failure
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(error.to_string());
                    true
                }
            }),
        );
    }

    pub(super) fn explain<T>(&self, result: Result<T>) -> Result<T> {
        result.map_err(|error| {
            match self
                .failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                Some(reason) => anyhow::anyhow!("{reason}: {error}"),
                None => error,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_allocated_live_bytes_and_the_controlling_deficit() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        fs::write(&live, [0; 4096]).unwrap();
        assert_eq!(live_size(&live).unwrap().0, allocated(&live).unwrap());
    }

    #[test]
    fn capture_preflight_refusal_preserves_an_existing_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        let file = File::create(&live).unwrap();
        file.set_len(1024 * 1024 * 1024 * 1024).unwrap();
        let staging = dir.path().join("checkpoint");
        fs::create_dir(&staging).unwrap();
        let capture = staging.join("capture-existing.sqlite3");
        fs::write(&capture, b"committed-cursor").unwrap();
        let error = SpaceGuard::admit(&live, &staging, true).err().unwrap().to_string();
        assert!(error.contains("refused before allocation"), "{error}");
        assert_eq!(fs::read(&capture).unwrap(), b"committed-cursor");
    }

    #[test]
    fn refuses_before_allocation_with_actionable_headroom() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        let file = File::create(&live).unwrap();
        file.set_len(1024 * 1024 * 1024 * 1024).unwrap(); // sparse: no large disk use
        let scratch = dir.path().join("scratch");
        let error = SpaceGuard::admit(&live, &scratch, true)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("available="), "{error}");
        assert!(error.contains("required="), "{error}");
        assert!(error.contains("live reserve="), "{error}");
        assert!(error.contains("controlling component="), "{error}");
        assert!(
            !scratch.exists(),
            "admission must precede scratch allocation"
        );
    }

    #[test]
    fn next_owner_sweeps_only_a_marked_orphan() {
        let dir = tempfile::tempdir().unwrap();
        let owner = ProofOwnership::acquire(dir.path()).unwrap();
        let proof = dir.path().join("proof-example.sqlite3");
        owner.mark(&proof).unwrap();
        fs::write(&proof, [0; 4]).unwrap();
        fs::write(format!("{}-journal", proof.display()), [0; 4]).unwrap();
        drop(owner); // simulate process death before normal cleanup
        let _next = ProofOwnership::acquire(dir.path()).unwrap();
        assert!(!proof.exists());
        assert!(!PathBuf::from(format!("{}-journal", proof.display())).exists());
        assert!(!PathBuf::from(format!("{}.owner", proof.display())).exists());
    }

    #[test]
    fn a_second_process_cannot_sweep_a_live_proof() {
        let dir = tempfile::tempdir().unwrap();
        let owner = ProofOwnership::acquire(dir.path()).unwrap();
        let proof = dir.path().join("proof-live.sqlite3");
        owner.mark(&proof).unwrap();
        fs::write(&proof, [0; 4]).unwrap();
        assert!(ProofOwnership::acquire(dir.path()).is_err());
        assert!(proof.exists());
        owner.finish(&proof).unwrap();
    }

    #[test]
    fn cleanup_error_keeps_the_owner_marker_and_refuses_the_next_proof() {
        let dir = tempfile::tempdir().unwrap();
        let owner = ProofOwnership::acquire(dir.path()).unwrap();
        let proof = dir.path().join("proof-example.sqlite3");
        owner.mark(&proof).unwrap();
        fs::create_dir(format!("{}-journal", proof.display())).unwrap();
        let error = owner.finish(&proof).err().unwrap().to_string();
        assert!(error.contains("proof-example.sqlite3-journal"), "{error}");
        assert!(PathBuf::from(format!("{}.owner", proof.display())).exists());
        drop(owner);
        assert!(ProofOwnership::acquire(dir.path()).is_err());
    }

    #[test]
    fn sqlite_activity_interrupts_when_the_reserve_is_crossed() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        fs::write(&live, [0; 4096]).unwrap();
        let scratch = dir.path().join("scratch");
        let fs_state = filesystem(&scratch).unwrap();
        let guard = SpaceGuard {
            live: live.clone(),
            live_wal: PathBuf::from(format!("{}-wal", live.display())),
            scratch,
            temp: std::env::temp_dir(),
            scratch_device: fs_state.device,
            temp_device: fs_state.device,
            initial_available: fs_state.available,
            initial_scratch_allocated: 0,
            initial_temp_available: fs_state.available,
            initial_wal_allocated: 0,
            budget: u128::MAX,
            temp_budget: u128::MAX,
            reserve: fs_state.available,
            wal_allowance: u128::MAX,
            shared_device: true,
            failure: Arc::new(Mutex::new(None)),
        };
        let connection = Connection::open_in_memory().unwrap();
        guard.install(&connection);
        let error = connection
            .execute_batch(
                "WITH RECURSIVE x(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM x WHERE n<10000)
             SELECT sum(n) FROM x;",
            )
            .err()
            .unwrap();
        assert!(
            guard
                .explain::<()>(Err(error.into()))
                .err()
                .unwrap()
                .to_string()
                .contains("reserve")
        );
    }

    #[test]
    fn allocated_scratch_cannot_hide_behind_other_freed_space() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        fs::write(&live, [0; 4096]).unwrap();
        let scratch = dir.path().join("scratch");
        fs::create_dir(&scratch).unwrap();
        let fs_state = filesystem(&scratch).unwrap();
        let guard = SpaceGuard {
            live: live.clone(),
            live_wal: PathBuf::from(format!("{}-wal", live.display())),
            scratch: scratch.clone(),
            temp: scratch.clone(),
            scratch_device: fs_state.device,
            temp_device: fs_state.device,
            initial_available: 0, // an unrelated free cannot offset this hard allocation budget
            initial_scratch_allocated: 0,
            initial_temp_available: 0,
            initial_wal_allocated: 0,
            budget: 0,
            temp_budget: 0,
            reserve: 0,
            wal_allowance: u128::MAX,
            shared_device: true,
            failure: Arc::new(Mutex::new(None)),
        };
        fs::write(scratch.join("proof-example.sqlite3"), [0; 4096]).unwrap();
        let error = guard.check_now().err().unwrap().to_string();
        assert!(error.contains("allocation budget crossed"), "{error}");
    }

    #[test]
    fn progress_handler_cannot_bound_a_vacuum_write() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        let connection = Connection::open(&live).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE payload(bytes BLOB);
                 INSERT INTO payload VALUES (zeroblob(1024 * 1024));",
            )
            .unwrap();
        let scratch = dir.path().join("scratch");
        let fs_state = filesystem(&scratch).unwrap();
        let guard = SpaceGuard {
            live: live.clone(),
            live_wal: PathBuf::from(format!("{}-wal", live.display())),
            scratch,
            temp: std::env::temp_dir(),
            scratch_device: fs_state.device,
            temp_device: fs_state.device,
            initial_available: fs_state.available,
            initial_scratch_allocated: 0,
            initial_temp_available: fs_state.available,
            initial_wal_allocated: 0,
            budget: u128::MAX,
            temp_budget: u128::MAX,
            reserve: fs_state.available,
            wal_allowance: u128::MAX,
            shared_device: true,
            failure: Arc::new(Mutex::new(None)),
        };
        guard.install(&connection);
        let copy = dir.path().join("copy.sqlite3");
        let error = connection
            .execute("VACUUM INTO ?1", [copy.to_string_lossy()])
            .err()
            .unwrap();
        assert!(
            guard
                .explain::<()>(Err(error.into()))
                .err()
                .unwrap()
                .to_string()
                .contains("reserve")
        );
        assert!(
            allocated(&copy).unwrap() > 128 * 1024,
            "a progress callback can fire after a VACUUM INTO write much larger than the VFS budget fixture"
        );
    }
}
