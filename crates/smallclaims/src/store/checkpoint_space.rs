//! Disk admission for a checkpoint's disposable capture and proof files. The live writer shares
//! this filesystem on many members, so a proof must leave its reserve available even on failure.
use std::ffi::CString;
use std::fs;
use std::fs::File;
use std::io::Read as _;
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
// The six temporary tables created by `checkpoint::prove`. Measure their empty database
// in SQLite itself rather than fitting a minimum to a small-store failure.
const PROOF_SCHEMA: &str =
    /* sqlite-large-value: allow-for-now lv-hb94c80746170 owner=smallclaims::store::checkpoint_space TODO=chunk-or-reference-variable-columns */
    "
CREATE TABLE sealed_claims(id TEXT PRIMARY KEY);
CREATE TABLE sealed_blobs(hash TEXT PRIMARY KEY);
CREATE TABLE sealed_envelopes(writer TEXT, sequence INTEGER, envelope_hash TEXT,
    PRIMARY KEY(writer, sequence, envelope_hash));
CREATE TABLE proof_answers(phase INTEGER, subject TEXT, text TEXT, object INTEGER,
    PRIMARY KEY(phase, subject));
CREATE TABLE proof_readers(phase INTEGER, subject TEXT, ordinal INTEGER, reader TEXT,
    text TEXT, PRIMARY KEY(phase, subject, ordinal));
CREATE TABLE proof_sources(subject TEXT PRIMARY KEY, sources TEXT);";
#[cfg(test)]
const PROGRESS_OPCODES: i32 = 1000;

mod vfs;
pub(super) use vfs::ProofVfs;

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
            .truncate(false)
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

fn live_size(path: &Path) -> Result<(u128, u128, u128, u128, u128, u128, u128)> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h821b199ed4a2 owner=smallclaims::store::checkpoint_space TODO=trace-statement-and-value-representation */ "BEGIN")?;
    let page_size: u128 =
        u128::try_from(connection.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))?)?;
    let page_count: u128 =
        u128::try_from(connection.query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))?)?;
    let auto_vacuum: u128 =
        u128::try_from(
            connection.query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i64>(0))?,
        )?;
    // sqlite_schema has no row of its own, but VACUUM rebuilds that b-tree too.
    let btrees: u128 = u128::try_from(connection.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type IN ('table', 'index')",
        [],
        |row| row.get::<_, i64>(0),
    )?)?
    .checked_add(1)
    .context("VACUUM b-tree count overflow")?;
    // SQLite's page-1 header byte 20 is the reserved space per page. Read it while
    // the same source read transaction that supplied P, N and schema is still open.
    let mut header = [0_u8; 21];
    File::open(path)?.read_exact(&mut header)?;
    let usable_page_size = page_size
        .checked_sub(u128::from(header[20]))
        .context("invalid SQLite reserved-byte count")?;
    let metadata =
        fs::metadata(path).with_context(|| format!("stat live database {}", path.display()))?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h731eff28e6bb owner=smallclaims::store::checkpoint_space TODO=trace-statement-and-value-representation */ "COMMIT")?;
    Ok((
        u128::from(metadata.blocks()) * 512,
        u128::from(metadata.len()),
        page_size,
        page_count,
        auto_vacuum,
        btrees,
        usable_page_size,
    ))
}

fn vacuum_structural_pages(
    usable_page_size: u128,
    page_count: u128,
    auto_vacuum: u128,
    btrees: u128,
) -> Result<(u128, u128)> {
    // Rebuilding a b-tree can add a root and an interior page. Pointer-map pages apply
    // only when auto_vacuum is enabled. This is a guarded copy cap, not an admission reserve.
    let tree_pages = btrees
        .checked_mul(2)
        .context("VACUUM b-tree allowance overflow")?;
    let pointer_pages = if auto_vacuum == 0 {
        0
    } else {
        let entries_per_page = usable_page_size / 5;
        if entries_per_page == 0 {
            bail!("SQLite page too small for pointer map");
        }
        page_count
            .checked_add(entries_per_page - 1)
            .context("pointer-map allowance overflow")?
            / entries_per_page
    };
    Ok((tree_pages, pointer_pages))
}

fn empty_schema_bytes(page_size: u128, capture: bool) -> Result<u128> {
    let connection = Connection::open_in_memory()?;
    let temp_page_size: u128 =
        u128::try_from(
            connection.query_row("PRAGMA temp.page_size", [], |row| row.get::<_, i64>(0))?,
        )?;
    let schema_page_size = if capture { page_size } else { temp_page_size };
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hfbcc0086d0e1 owner=smallclaims::store::checkpoint_space TODO=trace-statement-and-value-representation */ &format!("PRAGMA page_size={schema_page_size};"))?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hb33a5a9775c2 owner=smallclaims::store::checkpoint_space TODO=trace-statement-and-value-representation */ if capture { super::checkpoint_staging::SCHEMA } else { PROOF_SCHEMA })?;
    let pages: u128 =
        u128::try_from(connection.query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))?)?;
    pages
        .checked_mul(schema_page_size)
        .context("empty checkpoint schema size overflow")
}

fn clone_bytes(main: u128, wal: u128, granule: u128) -> Result<u128> {
    if granule == 0 {
        bail!("scratch allocation granule is zero");
    }
    let rounded = |length: u128| -> Result<u128> {
        length
            .checked_add(granule - 1)
            .map(|n| n / granule * granule)
            .context("clone allocation rounding overflow")
    };
    rounded(main)?
        .checked_add(rounded(wal)?)
        .context("raw clone size overflow")
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
    page_size: u128,
    granule: u128,
    temp_granule: u128,
    copy_bound: u128,
    vacuum_cap: u128,
    vacuum_tree_pages: u128,
    vacuum_pointer_pages: u128,
    vacuum_wal_pages: u128,
    vacuum_source_logical: Arc<Mutex<u128>>,
    snapshot_logical: u128,
    initial_journal_pages: u128,
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
        let (
            live_allocated,
            live_length,
            page_size,
            page_count,
            auto_vacuum,
            btrees,
            usable_page_size,
        ) = live_size(live)?;
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
        let (vacuum_tree_pages, vacuum_pointer_pages) =
            vacuum_structural_pages(usable_page_size, page_count, auto_vacuum, btrees)?;
        let vacuum_wal_pages = wal_allowance
            / page_size
                .checked_add(24)
                .context("WAL frame size overflow")?;
        let vacuum_cap_pages = page_count
            .checked_add(vacuum_tree_pages)
            .and_then(|n| n.checked_add(vacuum_pointer_pages))
            .and_then(|n| n.checked_add(vacuum_wal_pages))
            .context("VACUUM copy cap overflow")?;
        let vacuum_cap = vacuum_cap_pages
            .checked_mul(page_size)
            .context("VACUUM copy cap bytes overflow")?;
        let reserve = LIVE_FLOOR + wal_allowance;
        let live_wal = PathBuf::from(format!("{}-wal", live.display()));
        let initial_wal_allocated = allocated(&live_wal)?;
        // Admission reserves the SQLite snapshot image for VACUUM. Its larger hard cap
        // draws any extra pages from the guarded remainder. A reflink or fallback byte
        // copy can retain the raw main file and its WAL until the fold completes.
        let wal_length = match fs::metadata(&live_wal) {
            Ok(metadata) => u128::from(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        let clone_copy = clone_bytes(live_length, wal_length, scratch_fs.granule)?;
        let copy = snapshot_logical.max(clone_copy);
        let main_pages = live_length
            .checked_add(page_size - 1)
            .context("main page count overflow")?
            / page_size;
        let initial_journal_pages = page_count.max(main_pages);
        // A rollback journal writes each original page with an 8-byte record header,
        // plus a header and sector alignment. Later transaction growth draws from the
        // available envelope and is guarded before each write.
        let journal = initial_journal_pages
            .checked_mul(
                page_size
                    .checked_add(8)
                    .context("journal record overflow")?,
            )
            .and_then(|n| n.checked_add(page_size.max(65536) * 2))
            .context("journal size overflow")?;
        let separate_temp = temp_fs.device != scratch_fs.device;
        // Schema pages live with staging during capture and on SQLite's temp device
        // during proof. Owner, lock, sidecars and directory entries use scratch.
        let schema_bytes = empty_schema_bytes(page_size, capture)?;
        let fixed = (12 * scratch_fs.granule)
            .checked_add(if capture || !separate_temp {
                schema_bytes
            } else {
                0
            })
            .context("checkpoint scratch fixed floor overflow")?;
        let temp_fixed = if !capture && separate_temp {
            schema_bytes
                .checked_add(2 * temp_fs.granule)
                .context("checkpoint temp fixed floor overflow")?
        } else {
            0
        };
        let staging = if capture { snapshot_logical } else { 0 };
        let minimum = if capture {
            staging.checked_add(fixed)
        } else {
            copy.checked_add(journal).and_then(|n| n.checked_add(fixed))
        }
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
        let temp_required = temp_fixed
            .checked_add(temp_reserve)
            .and_then(|n| n.checked_add(REACTION_MARGIN))
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
        let replay = budget.saturating_sub(minimum);
        let temp_estimate = if separate_temp {
            temp_budget.saturating_sub(temp_fixed)
        } else {
            replay
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
                "checkpoint scratch refused before allocation: scratch={} device={} available={} required={} deficit={} existing_scratch={} (copy={} vacuum_S={} vacuum_cap={} Delta_tree_pages={} Delta_pointer_pages={} G_w_pages={} journal={} scratch_fixed={} replay_envelope={} staging={} temp_fixed={} T_vacuum_envelope={} scratch budget={} live reserve={} reaction margin={}); temp_dir={} device={} available={} required={} deficit={}; live={} device={} allocated={} logical={} snapshot_logical={} page_size={} WAL_allocated={} WAL_logical={} available={} live reserve={} required={} deficit={} (floor={} WAL allowance={}); controlling component={}",
                scratch.display(),
                scratch_fs.device,
                scratch_fs.available,
                required,
                scratch_deficit,
                existing_scratch,
                copy,
                snapshot_logical,
                vacuum_cap,
                vacuum_tree_pages,
                vacuum_pointer_pages,
                vacuum_wal_pages,
                journal,
                fixed,
                replay,
                staging,
                temp_fixed,
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
                snapshot_logical,
                page_size,
                initial_wal_allocated,
                wal_length,
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
            page_size,
            granule: scratch_fs.granule,
            temp_granule: temp_fs.granule,
            copy_bound: copy,
            vacuum_cap,
            vacuum_tree_pages,
            vacuum_pointer_pages,
            vacuum_wal_pages,
            vacuum_source_logical: Arc::new(Mutex::new(snapshot_logical)),
            snapshot_logical,
            initial_journal_pages,
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

    pub(super) fn check_clone_source(&self) -> Result<()> {
        let main = u128::from(fs::metadata(&self.live)?.len());
        let wal = match fs::metadata(&self.live_wal) {
            Ok(metadata) => u128::from(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        if clone_bytes(main, wal, self.granule)? > self.copy_bound {
            bail!(
                "checkpoint raw clone exceeds admitted copy bound: main={main} wal={wal} copy_bound={}",
                self.copy_bound
            );
        }
        self.check_now()
    }

    pub(super) fn check_vacuum_source(&self) -> Result<()> {
        let (_, _, page_size, pages, _, _, _) = live_size(&self.live)?;
        let current = page_size
            .checked_mul(pages)
            .context("current source size overflow")?;
        let source_growth_cap = self
            .vacuum_wal_pages
            .checked_mul(self.page_size)
            .context("source WAL growth bytes overflow")?;
        let source_cap = self
            .snapshot_logical
            .checked_add(source_growth_cap)
            .context("source WAL growth cap overflow")?;
        if page_size != self.page_size || current > source_cap {
            bail!(
                "checkpoint source grew beyond WAL allowance before VACUUM: admitted_logical={} current_logical={} source_cap={} admitted_page_size={} current_page_size={}",
                self.snapshot_logical,
                current,
                source_cap,
                self.page_size,
                page_size
            );
        }
        *self
            .vacuum_source_logical
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = current;
        self.check_now()
    }

    pub(super) fn check_cloned_files(&self, main: &Path, wal: &Path) -> Result<()> {
        let main_len = u128::from(fs::metadata(main)?.len());
        let wal_len = match fs::metadata(wal) {
            Ok(metadata) => u128::from(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        if clone_bytes(main_len, wal_len, self.granule)? > self.copy_bound {
            bail!(
                "checkpoint cloned files exceed admission: main={main_len} wal={wal_len} copy_bound={}",
                self.copy_bound
            );
        }
        self.check_now()
    }

    pub(super) fn check_copied_image(&self, copy: &Path, vfs: &ProofVfs) -> Result<()> {
        let copied = Connection::open_with_flags_and_vfs(
            copy,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
            vfs.name(),
        )?;
        let page_size: u128 =
            u128::try_from(copied.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))?)?;
        let pages: u128 =
            u128::try_from(copied.query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))?)?;
        let logical = page_size
            .checked_mul(pages)
            .context("copied image size overflow")?;
        let eof = u128::from(fs::metadata(copy)?.len());
        let wal = match fs::metadata(format!("{}-wal", copy.display())) {
            Ok(metadata) => u128::from(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        let copy_limit = vfs.copy_limit();
        if page_size != self.page_size || logical > copy_limit || eof > copy_limit || wal != 0 {
            bail!(
                "checkpoint copied image exceeds admission: source_snapshot_logical={} source_pre_vacuum_logical={} source_page_size={} copied_logical={} copied_page_size={} copied_eof={} copied_wal={} copy_bound={} vacuum_tree_pages={} vacuum_pointer_pages={} vacuum_wal_pages={} source_grew_before_vacuum={}",
                self.snapshot_logical,
                *self
                    .vacuum_source_logical
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                self.page_size,
                logical,
                page_size,
                eof,
                wal,
                copy_limit,
                self.vacuum_tree_pages,
                self.vacuum_pointer_pages,
                self.vacuum_wal_pages,
                *self
                    .vacuum_source_logical
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    > self.snapshot_logical
            );
        }
        if logical > self.snapshot_logical {
            let source_before_vacuum = *self
                .vacuum_source_logical
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            eprintln!(
                "st3: checkpoint proof copy exceeds admission snapshot: source_snapshot_logical={} source_pre_vacuum_logical={} copied_logical={} copy_bound={} Delta_tree_pages={} Delta_pointer_pages={} G_w_pages={} E_d_copy_excess={} source_grew_before_vacuum={} structural_or_concurrent_extra={}",
                self.snapshot_logical,
                source_before_vacuum,
                logical,
                copy_limit,
                self.vacuum_tree_pages,
                self.vacuum_pointer_pages,
                self.vacuum_wal_pages,
                logical.saturating_sub(self.snapshot_logical),
                source_before_vacuum > self.snapshot_logical,
                logical.saturating_sub(source_before_vacuum)
            );
        }
        self.check_now()
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
        let (_, main_len, page_size, page_count, _, _, _) = live_size(&live).unwrap();
        assert!(
            page_size * page_count > main_len,
            "the committed WAL image must exceed the main-file EOF"
        );
        assert!(page_size * page_count > 1024 * 1024);
    }

    #[test]
    fn proof_preflight_refusal_preserves_an_existing_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        Connection::open(&live)
            .unwrap()
            .execute_batch("CREATE TABLE valid(x);")
            .unwrap();
        let file = File::options().write(true).open(&live).unwrap();
        file.set_len(1024 * 1024 * 1024 * 1024).unwrap();
        let staging = dir.path().join("checkpoint");
        fs::create_dir(&staging).unwrap();
        let capture = staging.join("capture-existing.sqlite3");
        fs::write(&capture, b"committed-cursor").unwrap();
        let error = SpaceGuard::admit(&live, &staging, false)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("refused before allocation"), "{error}");
        assert_eq!(fs::read(&capture).unwrap(), b"committed-cursor");
    }

    #[test]
    fn refuses_before_allocation_with_actionable_headroom() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        Connection::open(&live)
            .unwrap()
            .execute_batch("CREATE TABLE valid(x);")
            .unwrap();
        let file = File::options().write(true).open(&live).unwrap();
        file.set_len(1024 * 1024 * 1024 * 1024).unwrap(); // sparse: no large disk use
        let scratch = dir.path().join("scratch");
        let error = SpaceGuard::admit(&live, &scratch, false)
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
            page_size: 4096,
            granule: 4096,
            temp_granule: 4096,
            copy_bound: u128::MAX,
            vacuum_cap: u128::MAX,
            vacuum_tree_pages: 0,
            vacuum_pointer_pages: 0,
            vacuum_wal_pages: 0,
            vacuum_source_logical: Arc::new(Mutex::new(4096)),
            snapshot_logical: 4096,
            initial_journal_pages: 1,
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
            page_size: 4096,
            granule: 4096,
            temp_granule: 4096,
            copy_bound: u128::MAX,
            vacuum_cap: u128::MAX,
            vacuum_tree_pages: 0,
            vacuum_pointer_pages: 0,
            vacuum_wal_pages: 0,
            vacuum_source_logical: Arc::new(Mutex::new(4096)),
            snapshot_logical: 4096,
            initial_journal_pages: 1,
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
    fn progress_handler_checks_vacuum_opcodes() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        let connection = Connection::open(&live).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE payload(bytes BLOB);
                 WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4000)
                 INSERT INTO payload SELECT zeroblob(2048) FROM n;",
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
            page_size: 4096,
            granule: 4096,
            temp_granule: 4096,
            copy_bound: u128::MAX,
            vacuum_cap: u128::MAX,
            vacuum_tree_pages: 0,
            vacuum_pointer_pages: 0,
            vacuum_wal_pages: 0,
            vacuum_source_logical: Arc::new(Mutex::new(4096)),
            snapshot_logical: 4096,
            initial_journal_pages: 1,
            budget: u128::MAX,
            temp_budget: u128::MAX,
            reserve: fs_state.available,
            wal_allowance: u128::MAX,
            shared_device: true,
            failure: Arc::new(Mutex::new(None)),
        };
        let copy = dir.path().join("copy.sqlite3");
        let callback_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let copy_peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let callback_hits = Arc::clone(&callback_count);
        let callback_peak = Arc::clone(&copy_peak);
        let callback_guard = guard.clone();
        let callback_copy = copy.clone();
        connection.progress_handler(
            PROGRESS_OPCODES,
            Some(move || {
                callback_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let allocated_now = allocated(&callback_copy).unwrap_or(0) as usize;
                callback_peak.fetch_max(allocated_now, std::sync::atomic::Ordering::Relaxed);
                match callback_guard.check() {
                    Ok(()) => false,
                    Err(error) => {
                        *callback_guard.failure.lock().unwrap() = Some(error.to_string());
                        true
                    }
                }
            }),
        );
        let error = connection
            .execute("VACUUM INTO ?1", [copy.to_string_lossy()])
            .err()
            .unwrap();
        assert!(callback_count.load(std::sync::atomic::Ordering::Relaxed) > 0);
        assert!(
            guard
                .explain::<()>(Err(error.into()))
                .err()
                .unwrap()
                .to_string()
                .contains("reserve")
        );
        // On this SQLite build the first production-rate callback fires before 128 KiB
        // of destination allocation. This fixture proves the opcode callback runs and
        // checks the reserve; the proof VFS fixture tests before-write enforcement.
        eprintln!(
            "VACUUM callback count={} copy peak at callback={} bytes (128 KiB lag not established)",
            callback_count.load(std::sync::atomic::Ordering::Relaxed),
            copy_peak.load(std::sync::atomic::Ordering::Relaxed)
        );
    }
}
