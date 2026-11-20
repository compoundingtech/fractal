//! Disk admission for a checkpoint's disposable capture files. The live writer shares this
//! filesystem on many members, so a capture must leave its reserve available even on failure.
use std::ffi::CString;
use std::fs;
use std::fs::File;
use std::os::fd::AsRawFd as _;
use std::os::unix::{ffi::OsStrExt as _, fs::MetadataExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _};

const GIB: u128 = 1024 * 1024 * 1024;
/// Exact binary capability expected by the checkpoint deployment gate.
/// Change this only when the admission/refusal contract changes and the deploy gate is updated.
pub const CHECKPOINT_SPACE_GATE_VERSION: u32 = 2;
const LIVE_FLOOR: u128 = 8 * GIB;
const REACTION_MARGIN: u128 = 256 * 1024 * 1024;
/// A scoped capture stages only the envelopes holding its kinds' claims and what judging them
/// reads, so it is charged the snapshot image scaled by those kinds' share of the store's claims,
/// times this margin, at least `SCOPED_STAGING_FLOOR` and at most the image. A staged claim takes
/// far fewer bytes than a live one (about 1.4 KiB against 17 KiB on a 53 GB member), and the
/// capture checks space before each page, so a share that undercounts is stopped, not overrun.
const SCOPED_STAGING_MARGIN: u128 = 2;
const SCOPED_STAGING_FLOOR: u128 = GIB;

/// The staging allowance a capture is charged: the whole snapshot image, or for a scoped one,
/// its share (`scoped` of `total` claims) as above.
fn capture_staging(snapshot_logical: u128, share: Option<(u128, u128)>) -> u128 {
    match share {
        Some((scoped, total)) if total > 0 => (snapshot_logical.saturating_mul(scoped).saturating_mul(SCOPED_STAGING_MARGIN) / total)
            .max(SCOPED_STAGING_FLOOR)
            .min(snapshot_logical),
        _ => snapshot_logical,
    }
}
#[cfg(test)]
const PROGRESS_OPCODES: i32 = 1000;

#[derive(Clone, Copy)]
struct Filesystem {
    device: u64,
    available: u128,
    granule: u128,
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
        granule: u128::from(stat.f_frsize),
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

/// Another capture holds the directory lock. It is released when that capture finishes, so a
/// caller that can wait may try again.
#[derive(Debug)]
pub struct CheckpointProofBusy {
    pub dir: PathBuf,
}

impl std::fmt::Display for CheckpointProofBusy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "another checkpoint capture owns {}", self.dir.display())
    }
}

impl std::error::Error for CheckpointProofBusy {}

/// One capture owns the scratch directory lock while it runs. A surviving `.owner` file names a
/// proof copy an older build left behind; the owner of the lock sweeps it. An unmarked legacy
/// proof file is refused rather than assumed to be abandoned.
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
            .truncate(false)
            .open(dir.join("proof.lock"))?;
        // SAFETY: the descriptor stays open for the ownership lifetime.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return Err(CheckpointProofBusy { dir: dir.to_owned() }.into());
            }
            return Err(error)
                .with_context(|| format!("another checkpoint capture owns {}", dir.display()));
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
            if let Some(stem) = name.strip_suffix(".owner")
                && stem.starts_with("proof-")
                && stem.ends_with(".sqlite3")
            {
                let mut this_proof = Vec::new();
                self.remove_set(&self.dir.join(stem), &mut this_proof);
                failures.extend(this_proof);
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
                "fractal: checkpoint proof orphan cleanup refused: {}",
                failures.join("; ")
            );
            bail!("checkpoint proof cleanup refused: {}", failures.join("; "));
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
        if failures.is_empty()
            && let Err(error) = File::open(&self.dir).and_then(|dir| dir.sync_all())
        {
            failures.push(format!("sync {}: {error}", self.dir.display()));
        }
        if failures.is_empty() {
            remove_if_present(
                &PathBuf::from(format!("{}.owner", proof.display())),
                failures,
            );
        }
    }

}

fn live_size(path: &Path) -> Result<(u128, u128, u128)> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h821b199ed4a2 owner=smallclaims::store::checkpoint_space TODO=trace-statement-and-value-representation */ "BEGIN")?;
    let page_size: u128 =
        u128::try_from(connection.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))?)?;
    let page_count: u128 =
        u128::try_from(connection.query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))?)?;
    let metadata =
        fs::metadata(path).with_context(|| format!("stat live database {}", path.display()))?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h731eff28e6bb owner=smallclaims::store::checkpoint_space TODO=trace-statement-and-value-representation */ "COMMIT")?;
    Ok((u128::from(metadata.blocks()) * 512, page_size, page_count))
}

fn empty_schema_bytes(page_size: u128) -> Result<u128> {
    let connection = Connection::open_in_memory()?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h121eaf6ba3cf owner=smallclaims::store::checkpoint_space TODO=trace-statement-and-value-representation */ &format!("PRAGMA page_size={page_size};"))?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h5154c31b5d2f owner=smallclaims::store::checkpoint_space TODO=trace-statement-and-value-representation */ super::checkpoint_staging::SCHEMA)?;
    let pages: u128 =
        u128::try_from(connection.query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))?)?;
    pages
        .checked_mul(page_size)
        .context("empty checkpoint schema size overflow")
}

/// One admitted capture. Each page checks it before it writes, so the live writer keeps its
/// reserve; the 256 MiB margin leaves space for writes between checks.
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
    /// Admit a whole capture before creating the scratch directory: one database-size
    /// allowance for its staging file.
    pub(super) fn admit(live: &Path, scratch: &Path) -> Result<Self> {
        Self::admit_with(live, scratch, None)
    }

    /// `admit` for a capture scoped to kinds holding `share` (`scoped` of `total` claims): its
    /// staging allowance is that share of the image (`capture_staging`).
    pub(super) fn admit_scoped_capture(live: &Path, scratch: &Path, share: (u128, u128)) -> Result<Self> {
        Self::admit_with(live, scratch, Some(share))
    }

    fn admit_with(live: &Path, scratch: &Path, share: Option<(u128, u128)>) -> Result<Self> {
        let (live_allocated, page_size, page_count) = live_size(live)?;
        if page_size == 0 {
            bail!("SQLite source page size is zero");
        }
        let snapshot_logical = page_size
            .checked_mul(page_count)
            .context("snapshot size overflow")?;
        let live_fs = filesystem(live)?;
        let scratch_fs = filesystem(scratch)?;
        let temp_dir = sqlite_temp_dir()?;
        let temp_fs = filesystem(&temp_dir)?;
        let shared_device = live_fs.device == scratch_fs.device;
        let wal_allowance = LIVE_FLOOR.max(live_allocated);
        let reserve = LIVE_FLOOR + wal_allowance;
        let live_wal = PathBuf::from(format!("{}-wal", live.display()));
        let initial_wal_allocated = allocated(&live_wal)?;
        let separate_temp = temp_fs.device != scratch_fs.device;
        // Schema pages live with staging. Owner, lock, sidecars and directory entries use
        // scratch.
        let fixed = (12 * scratch_fs.granule)
            .checked_add(empty_schema_bytes(page_size)?)
            .context("checkpoint scratch fixed floor overflow")?;
        let staging = capture_staging(snapshot_logical, share);
        let minimum = staging
            .checked_add(fixed)
            .context("checkpoint minimum size overflow")?;
        let scratch_reserve = if shared_device { reserve } else { 0 };
        let temp_reserve = if separate_temp && temp_fs.device == live_fs.device {
            reserve
        } else {
            0
        };
        let required = minimum
            .checked_add(scratch_reserve)
            .and_then(|n| n.checked_add(REACTION_MARGIN))
            .context("checkpoint required size overflow")?;
        let temp_required = temp_reserve
            .checked_add(REACTION_MARGIN)
            .context("temp required size overflow")?;
        let live_required = reserve.saturating_add(REACTION_MARGIN);
        let budget = scratch_fs
            .available
            .saturating_sub(scratch_reserve + REACTION_MARGIN);
        let temp_budget = if separate_temp {
            temp_fs
                .available
                .saturating_sub(temp_reserve + REACTION_MARGIN)
        } else {
            0
        };
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
                "checkpoint scratch refused before allocation: scratch={} device={} available={} required={} deficit={} existing_scratch={} (staging={} scratch_fixed={} scratch budget={} live reserve={} reaction margin={}); temp_dir={} device={} available={} required={} deficit={}; live={} device={} allocated={} snapshot_logical={} page_size={} WAL_allocated={} available={} live reserve={} required={} deficit={} (floor={} WAL allowance={}); controlling component={}",
                scratch.display(),
                scratch_fs.device,
                scratch_fs.available,
                required,
                scratch_deficit,
                existing_scratch,
                staging,
                fixed,
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
                snapshot_logical,
                page_size,
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
        Connection::open(&live)
            .unwrap()
            .execute_batch("CREATE TABLE valid(x);")
            .unwrap();
        assert_eq!(live_size(&live).unwrap().0, allocated(&live).unwrap());
    }

    #[test]
    fn snapshot_size_includes_committed_wal_pages() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        let writer = Connection::open(&live).unwrap();
        writer
            .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE payload(bytes BLOB);")
            .unwrap();
        writer
            .execute("INSERT INTO payload VALUES (zeroblob(1024 * 1024))", [])
            .unwrap();
        let main_len = u128::from(fs::metadata(&live).unwrap().len());
        let (_, page_size, page_count) = live_size(&live).unwrap();
        assert!(
            page_size * page_count > main_len,
            "the committed WAL image must exceed the main-file EOF"
        );
        assert!(page_size * page_count > 1024 * 1024);
    }

    /// A store whose image is a terabyte of pages, sparse so it takes no disk: its header's page
    /// count is made stale, so SQLite counts pages from the file's length.
    fn sparse_terabyte_store(live: &Path) {
        use std::os::unix::fs::FileExt as _;
        Connection::open(live)
            .unwrap()
            .execute_batch("CREATE TABLE valid(x);")
            .unwrap();
        let file = File::options().write(true).open(live).unwrap();
        file.set_len(1024 * 1024 * 1024 * 1024).unwrap();
        file.write_all_at(&[0xff; 4], 92).unwrap();
    }

    #[test]
    fn a_refused_capture_preserves_an_existing_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        sparse_terabyte_store(&live);
        let staging = dir.path().join("checkpoint");
        fs::create_dir(&staging).unwrap();
        let capture = staging.join("capture-existing.sqlite3");
        fs::write(&capture, b"committed-cursor").unwrap();
        let error = SpaceGuard::admit(&live, &staging)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("refused before allocation"), "{error}");
        assert_eq!(fs::read(&capture).unwrap(), b"committed-cursor");
    }

    #[test]
    fn a_scoped_capture_is_charged_its_share_of_the_image_with_a_margin() {
        let image = 100 * GIB;
        assert_eq!(capture_staging(image, None), image, "a whole capture stages the image");
        assert_eq!(capture_staging(image, Some((15, 100))), 30 * GIB);
        assert_eq!(capture_staging(image, Some((1, 1_000_000))), SCOPED_STAGING_FLOOR);
        assert_eq!(capture_staging(image, Some((90, 100))), image, "never more than the image");
        assert_eq!(capture_staging(GIB / 2, Some((1, 100))), GIB / 2, "nor more than a small image");
        assert_eq!(capture_staging(image, Some((0, 0))), image, "an empty count is not trusted");
    }

    #[test]
    fn refuses_before_allocation_with_actionable_headroom() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        sparse_terabyte_store(&live);
        let scratch = dir.path().join("scratch");
        let error = SpaceGuard::admit(&live, &scratch)
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

}
