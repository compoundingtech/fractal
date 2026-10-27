//! A proof-local SQLite VFS. All SQLite writes in the proof are admitted before xWrite.
//! Registration is global, but each name is unique and never becomes the default VFS.
//! The boxed registration outlives every file opened through it; Drop leaks it rather than
//! freeing callbacks if a SQLite connection unexpectedly survives the proof.
use super::{ProofOwnership, REACTION_MARGIN, SpaceGuard, filesystem};
use anyhow::{Result, bail};
use rusqlite::{Connection, OpenFlags, ffi};
use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr};
use std::mem::{align_of, size_of};
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

const WINDOW: u128 = 16 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Scratch,
    Temp,
    Live,
    Unknown,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Data,
    Journal,
    Temp,
}

#[derive(Default)]
struct Window {
    ready: bool,
    scratch_used: u128,
    temp_used: u128,
    scratch_pending: u128,
    temp_pending: u128,
    temp_total_requested: u128,
    total_requested: u128,
    requests_since_refresh: u128,
    file_sizes: HashMap<usize, u128>,
    calls: u64,
    refreshes: u64,
    max_request: u128,
    refresh_ns: u128,
    data_writes: u64,
    journal_writes: u64,
    temp_writes: u64,
}

#[repr(C)]
struct Data {
    vfs: ffi::sqlite3_vfs,
    base: *mut ffi::sqlite3_vfs,
    name: CString,
    guard: SpaceGuard,
    files: AtomicUsize,
    self_test_refuse: AtomicBool,
    self_test_hits: AtomicUsize,
    copy_phase: AtomicBool,
    vacuum_phase: AtomicBool,
    copy_limit: Mutex<u128>,
    journal_pages: AtomicUsize,
    sector_size: AtomicUsize,
    #[cfg(test)]
    simulate_unavailable: AtomicBool,
    #[cfg(test)]
    read_calls: AtomicUsize,
    window: Mutex<Window>,
}

// SAFETY: after registration the VFS name, base pointer and guard are immutable. SQLite's
// registration mutex protects the vfs header; open-file count and admission state are atomic or
// mutex protected. Drop unregisters only after all files have closed.
unsafe impl Send for Data {}
unsafe impl Sync for Data {}

pub(in crate::store) struct ProofVfs {
    data: Option<Box<Data>>,
}

impl ProofVfs {
    pub(in crate::store) fn check_clone_source(&self) -> Result<()> {
        self.data
            .as_ref()
            .expect("registered proof VFS")
            .guard
            .check_clone_source()
    }
    pub(in crate::store) fn check_cloned_files(&self, main: &Path, wal: &Path) -> Result<()> {
        self.data
            .as_ref()
            .expect("registered proof VFS")
            .guard
            .check_cloned_files(main, wal)
    }
    pub(in crate::store) fn vacuum_copy(&self) -> Result<()> {
        let data = self.data.as_ref().expect("registered proof VFS");
        data.guard.check_vacuum_source()?;
        *data
            .copy_limit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = data.guard.vacuum_cap;
        data.vacuum_phase.store(true, Ordering::Release);
        Ok(())
    }
    pub(in crate::store) fn copy_limit(&self) -> u128 {
        *self
            .data
            .as_ref()
            .expect("registered proof VFS")
            .copy_limit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(in crate::store) fn begin_proof(&self, connection: &Connection) -> Result<()> {
        self.refresh_journal_pages(connection)?;
        self.data
            .as_ref()
            .expect("registered proof VFS")
            .copy_phase
            .store(false, Ordering::Release);
        Ok(())
    }
    pub(in crate::store) fn refresh_journal_pages(&self, connection: &Connection) -> Result<()> {
        let pages: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        self.data
            .as_ref()
            .expect("registered proof VFS")
            .journal_pages
            .store(usize::try_from(pages)?, Ordering::Release);
        Ok(())
    }
    pub(in crate::store) fn register(guard: &SpaceGuard) -> Result<Self> {
        // SAFETY: SQLite keeps the default VFS registered for the process lifetime.
        let base = unsafe { ffi::sqlite3_vfs_find(ptr::null()) };
        if base.is_null() {
            bail!("checkpoint proof VFS unavailable: no default SQLite VFS");
        }
        if unsafe { (*base).iVersion } < 3 {
            bail!("checkpoint proof VFS unavailable: SQLite VFS version below 3");
        }
        let methods = unsafe { &*base };
        if methods.xOpen.is_none()
            || methods.xDelete.is_none()
            || methods.xAccess.is_none()
            || methods.xFullPathname.is_none()
            || methods.xRandomness.is_none()
            || methods.xSleep.is_none()
            || methods.xCurrentTime.is_none()
        {
            bail!("checkpoint proof VFS unavailable: incomplete default VFS methods");
        }
        // SQLite allocates file storage aligned for its default VFS. Our header is followed by
        // the default VFS file, aligned to max_align_t, within the advertised size.
        let name = CString::new(format!(
            "checkpoint-proof-{}",
            uuid::Uuid::now_v7().simple()
        ))?;
        let mut vfs = unsafe { *base };
        vfs.szOsFile = (size_of::<WrappedFile>()
            + align_of::<libc::max_align_t>()
            + usize::try_from(vfs.szOsFile)?)
        .try_into()?;
        let mut data = Box::new(Data {
            vfs,
            base,
            name,
            guard: guard.clone(),
            files: AtomicUsize::new(0),
            self_test_refuse: AtomicBool::new(false),
            self_test_hits: AtomicUsize::new(0),
            copy_phase: AtomicBool::new(true),
            vacuum_phase: AtomicBool::new(false),
            copy_limit: Mutex::new(guard.clone_cap),
            journal_pages: AtomicUsize::new(usize::try_from(guard.initial_journal_pages)?),
            sector_size: AtomicUsize::new(65536),
            #[cfg(test)]
            simulate_unavailable: AtomicBool::new(false),
            #[cfg(test)]
            read_calls: AtomicUsize::new(0),
            window: Mutex::new(Window::default()),
        });
        data.vfs.zName = data.name.as_ptr();
        data.vfs.pAppData = (&mut *data as *mut Data).cast();
        data.vfs.xOpen = Some(open);
        data.vfs.xDelete = Some(delete);
        data.vfs.xAccess = Some(access);
        data.vfs.xFullPathname = Some(full_pathname);
        data.vfs.xDlOpen = unsafe { (*base).xDlOpen }.map(|_| dl_open as _);
        data.vfs.xDlError = unsafe { (*base).xDlError }.map(|_| dl_error as _);
        data.vfs.xDlSym = unsafe { (*base).xDlSym }.map(|_| dl_sym as _);
        data.vfs.xDlClose = unsafe { (*base).xDlClose }.map(|_| dl_close as _);
        data.vfs.xRandomness = Some(randomness);
        data.vfs.xSleep = Some(sleep);
        data.vfs.xCurrentTime = Some(current_time);
        data.vfs.xGetLastError = unsafe { (*base).xGetLastError }.map(|_| get_last_error as _);
        data.vfs.xCurrentTimeInt64 =
            unsafe { (*base).xCurrentTimeInt64 }.map(|_| current_time_i64 as _);
        data.vfs.xSetSystemCall = unsafe { (*base).xSetSystemCall }.map(|_| set_system_call as _);
        data.vfs.xGetSystemCall = unsafe { (*base).xGetSystemCall }.map(|_| get_system_call as _);
        data.vfs.xNextSystemCall =
            unsafe { (*base).xNextSystemCall }.map(|_| next_system_call as _);
        // SAFETY: data is boxed and its address remains stable until unregister. This does not
        // change the process default VFS. The unique name cannot collide with another proof.
        let rc = unsafe { ffi::sqlite3_vfs_register(&mut data.vfs, 0) };
        if rc != ffi::SQLITE_OK {
            bail!("register checkpoint proof VFS: SQLite code {rc}");
        }
        Ok(Self { data: Some(data) })
    }

    pub(in crate::store) fn name(&self) -> &str {
        self.data
            .as_ref()
            .expect("registered VFS")
            .name
            .to_str()
            .expect("ASCII VFS name")
    }

    pub(in crate::store) fn check_closed(&self) -> Result<()> {
        let data = self.data.as_ref().expect("registered VFS");
        let files = data.files.load(Ordering::Acquire);
        if files != 0 {
            bail!("checkpoint proof VFS still owns {files} SQLite files");
        }
        Ok(())
    }

    #[cfg(test)]
    fn metrics(&self) -> (u64, u64, u128, u128, u64, u64, u64) {
        let window = self
            .data
            .as_ref()
            .expect("registered VFS")
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            window.calls,
            window.refreshes,
            window.max_request,
            window.refresh_ns,
            window.data_writes,
            window.journal_writes,
            window.temp_writes,
        )
    }

    /// A proof never trusts a VFS that silently delegates its allocation writes. Deliberately
    /// reject a tiny SQLite commit before opening the source or destination copy.
    pub(in crate::store) fn self_test(&self, owner: &ProofOwnership, path: &Path) -> Result<()> {
        let data = self.data.as_ref().expect("registered VFS");
        owner.mark(path)?;
        data.self_test_refuse.store(true, Ordering::Release);
        let attempt = (|| -> Result<()> {
            let connection = Connection::open_with_flags_and_vfs(
                path,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
                self.name(),
            )?;
            connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hd90253e72816 owner=smallclaims::store::checkpoint_space::vfs TODO=chunk-or-reference-variable-columns */ "PRAGMA mmap_size=0; CREATE TABLE vfs_guard_probe(x);")?;
            Ok(())
        })();
        data.self_test_refuse.store(false, Ordering::Release);
        let hits = data.self_test_hits.load(Ordering::Acquire);
        *data
            .guard
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let cleanup = owner.finish(path);
        cleanup?;
        if hits == 0 || attempt.is_ok() {
            bail!(
                "checkpoint proof VFS self-test failed before copy at {}: over-budget SQLite write was not refused",
                path.display()
            );
        }
        Ok(())
    }
}

impl Drop for ProofVfs {
    fn drop(&mut self) {
        let Some(mut data) = self.data.take() else {
            return;
        };
        if data.files.load(Ordering::Acquire) != 0 {
            // An unexpected live SQLite handle may still invoke callbacks. Leak rather than UAF.
            std::mem::forget(data);
            return;
        }
        // SAFETY: no open file references this VFS; the boxed address is still stable.
        let rc = unsafe { ffi::sqlite3_vfs_unregister(&mut data.vfs) };
        if rc != ffi::SQLITE_OK {
            std::mem::forget(data);
        }
    }
}

impl Data {
    fn rounded(&self, bytes: u128, class: Class) -> u128 {
        let granule = if class == Class::Temp {
            self.guard.temp_granule
        } else {
            self.guard.granule
        };
        let page = self.guard.page_size.max(granule);
        bytes.saturating_add(page - 1) / page * page
    }
    fn refuse(&self, message: String) -> c_int {
        let scratch_state = filesystem(&self.guard.scratch);
        let live_state = filesystem(&self.guard.live);
        let scratch_required = self.guard.budget
            + if self.guard.shared_device {
                self.guard.reserve
            } else {
                0
            }
            + REACTION_MARGIN;
        let scratch_available = scratch_state
            .as_ref()
            .map(|state| state.available.to_string())
            .unwrap_or_else(|error| format!("UNKNOWN ({error})"));
        let scratch_allocated = super::existing_scratch_bytes(&self.guard.scratch)
            .map(|value| value.to_string())
            .unwrap_or_else(|error| format!("UNKNOWN ({error})"));
        let scratch_deficit = scratch_state
            .as_ref()
            .map(|state| scratch_required.saturating_sub(state.available).to_string())
            .unwrap_or_else(|_| "UNKNOWN".into());
        let live_available = live_state
            .as_ref()
            .map(|state| state.available.to_string())
            .unwrap_or_else(|error| format!("UNKNOWN ({error})"));
        let required_live = self.guard.reserve + REACTION_MARGIN;
        let live_deficit = live_state
            .as_ref()
            .map(|state| required_live.saturating_sub(state.available).to_string())
            .unwrap_or_else(|_| "UNKNOWN".into());
        let reason = format!(
            "checkpoint scratch refused before SQLite write: {message}; scratch={} observed_st_blocks_bytes={} available={} required={} deficit={} budget={}; live={} available={} reserve={} reaction_margin={} required={} deficit={}; controlling component=proof VFS",
            self.guard.scratch.display(),
            scratch_allocated,
            scratch_available,
            scratch_required,
            scratch_deficit,
            self.guard.budget,
            self.guard.live.display(),
            live_available,
            self.guard.reserve,
            REACTION_MARGIN,
            required_live,
            live_deficit,
        );
        *self
            .guard
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
        ffi::SQLITE_FULL
    }

    fn classify(&self, name: *const c_char) -> Class {
        if name.is_null() {
            return Class::Temp;
        }
        // SAFETY: SQLite supplies a NUL-terminated filename to xOpen.
        let bytes = unsafe { CStr::from_ptr(name).to_bytes() };
        let path = Path::new(OsStr::from_bytes(bytes));
        if bytes
            .strip_prefix(self.guard.live.as_os_str().as_bytes())
            .is_some_and(|suffix| {
                suffix.is_empty() || suffix == b"-wal" || suffix == b"-shm" || suffix == b"-journal"
            })
        {
            Class::Live
        } else if path.starts_with(&self.guard.scratch) {
            Class::Scratch
        } else if path.starts_with(&self.guard.temp) {
            Class::Temp
        } else {
            Class::Unknown
        }
    }

    fn before_write(
        &self,
        file_key: usize,
        class: Class,
        kind: Kind,
        amount: u128,
        end: u128,
    ) -> c_int {
        if self
            .guard
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
        {
            return ffi::SQLITE_FULL;
        }
        #[cfg(test)]
        if self.simulate_unavailable.load(Ordering::Acquire) {
            return ffi::SQLITE_OK;
        }
        if self.self_test_refuse.load(Ordering::Acquire) {
            self.self_test_hits.fetch_add(1, Ordering::AcqRel);
            return self.refuse("checkpoint proof VFS deliberate self-test refusal".into());
        }
        if class == Class::Unknown || class == Class::Live {
            return self
                .refuse("checkpoint proof VFS refused an unclassified or live-file write".into());
        }
        if amount > WINDOW {
            return self.refuse(format!(
                "checkpoint proof VFS write request {amount} exceeds {WINDOW} byte bound"
            ));
        }
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let copy_limit = *self
            .copy_limit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if kind == Kind::Data
            && class == Class::Scratch
            && self.copy_phase.load(Ordering::Acquire)
            && end > copy_limit
        {
            let terms = if self.vacuum_phase.load(Ordering::Acquire) {
                format!(
                    "S={} Delta_tree_pages={} Delta_pointer_pages={} G_w_pages={} excess_over_S={} charged_to=E_d",
                    self.guard.snapshot_logical,
                    self.guard.vacuum_tree_pages,
                    self.guard.vacuum_pointer_pages,
                    self.guard.vacuum_wal_pages,
                    end.saturating_sub(self.guard.snapshot_logical)
                )
            } else {
                format!(
                    "C_clone={} admitted_copy={}",
                    self.guard.clone_cap, self.guard.copy_bound
                )
            };
            return self.refuse(format!(
                "checkpoint copy exceeds guarded C: eof={end} C={copy_limit} {terms}"
            ));
        }
        if kind == Kind::Journal && class == Class::Scratch {
            let pages = self.journal_pages.load(Ordering::Acquire) as u128;
            let journal_limit = pages
                .saturating_mul(self.guard.page_size.saturating_add(8))
                .saturating_add(
                    2 * self
                        .guard
                        .page_size
                        .max(self.sector_size.load(Ordering::Acquire) as u128),
                );
            if end > journal_limit {
                return self.refuse(format!(
                    "checkpoint journal exceeds J(Q): eof={end} Q={pages} J={journal_limit}"
                ));
            }
        }
        window.calls += 1;
        match kind {
            Kind::Data => window.data_writes += 1,
            Kind::Journal => window.journal_writes += 1,
            Kind::Temp => window.temp_writes += 1,
        }
        window.max_request = window.max_request.max(amount);
        window.total_requested = window.total_requested.saturating_add(amount);
        let temp_separate = self.guard.temp_device != self.guard.scratch_device;
        if !window.ready || window.requests_since_refresh.saturating_add(amount) > WINDOW {
            let started = Instant::now();
            if let Err(error) = self.guard.check_now() {
                return self.refuse(error.to_string());
            }
            window.scratch_used = match super::existing_scratch_bytes(&self.guard.scratch) {
                Ok(value) => value.saturating_sub(self.guard.initial_scratch_allocated),
                Err(error) => return self.refuse(error.to_string()),
            };
            if temp_separate {
                let temp = match filesystem(&self.guard.temp) {
                    Ok(value) => value,
                    Err(error) => return self.refuse(error.to_string()),
                };
                window.temp_used = self
                    .guard
                    .initial_temp_available
                    .saturating_sub(temp.available);
            }
            window.scratch_pending = 0;
            window.temp_pending = 0;
            window.requests_since_refresh = 0;
            window.ready = true;
            window.refreshes += 1;
            window.refresh_ns += started.elapsed().as_nanos();
        }
        let old_size = *window.file_sizes.get(&file_key).unwrap_or(&0);
        let growth = self
            .rounded(end, class)
            .saturating_sub(self.rounded(old_size, class));
        // Charge the possible new allocation before forwarding. Retaining a charge after a
        // failed underlying write is conservative; repeated writes to the same pages cost 0.
        window.file_sizes.insert(file_key, old_size.max(end));
        window.requests_since_refresh = window.requests_since_refresh.saturating_add(amount);
        if class == Class::Temp {
            let proposed = window.temp_total_requested.saturating_add(amount);
            let temp_peak = window
                .temp_used
                .saturating_add(window.temp_pending)
                .saturating_add(growth);
            let shared_peak = window
                .scratch_used
                .saturating_add(window.scratch_pending)
                .saturating_add(window.temp_pending)
                .saturating_add(growth);
            if temp_separate && temp_peak > self.guard.temp_budget {
                return self.refuse(format!("checkpoint proof SQLite temp budget refused before write: temp={} possible_peak_growth={} logical_highwater={} cumulative_requested={} budget={}", self.guard.temp.display(), temp_peak, end, window.total_requested, self.guard.temp_budget));
            }
            if !temp_separate && shared_peak > self.guard.budget {
                return self.refuse(format!("checkpoint proof scratch/temp budget refused before write: scratch={} possible_peak_growth={} logical_highwater={} cumulative_requested={} budget={}", self.guard.scratch.display(), shared_peak, end, window.total_requested, self.guard.budget));
            }
            window.temp_total_requested = proposed;
            window.temp_pending = window.temp_pending.saturating_add(growth);
        } else {
            let proposed = window
                .scratch_used
                .saturating_add(window.scratch_pending)
                .saturating_add(growth)
                .saturating_add(if temp_separate {
                    0
                } else {
                    window.temp_pending
                });
            if proposed > self.guard.budget {
                return self.refuse(format!("checkpoint proof scratch budget refused before write: scratch={} possible_peak_growth={} logical_highwater={} cumulative_requested={} budget={}", self.guard.scratch.display(), proposed, end, window.total_requested, self.guard.budget));
            }
            window.scratch_pending = window.scratch_pending.saturating_add(growth);
        }
        ffi::SQLITE_OK
    }
}

#[repr(C)]
struct WrappedFile {
    file: ffi::sqlite3_file,
    inner: *mut ffi::sqlite3_file,
    data: *mut Data,
    class: Class,
    kind: Kind,
}

unsafe fn wrapped(file: *mut ffi::sqlite3_file) -> &'static WrappedFile {
    // SAFETY: xOpen initializes this header before SQLite invokes any io method.
    unsafe { &*(file as *const WrappedFile) }
}
unsafe fn inner(file: *mut ffi::sqlite3_file) -> *mut ffi::sqlite3_file {
    unsafe { wrapped(file).inner }
}
unsafe fn methods(file: *mut ffi::sqlite3_file) -> &'static ffi::sqlite3_io_methods {
    unsafe { &*(*inner(file)).pMethods }
}
unsafe fn data(vfs: *mut ffi::sqlite3_vfs) -> &'static Data {
    unsafe { &*((*vfs).pAppData as *const Data) }
}
unsafe fn base(vfs: *mut ffi::sqlite3_vfs) -> *mut ffi::sqlite3_vfs {
    unsafe { data(vfs).base }
}

unsafe extern "C" fn open(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out: *mut c_int,
) -> c_int {
    let data = unsafe { data(vfs) };
    if flags & ffi::SQLITE_OPEN_CREATE != 0
        && data
            .guard
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    {
        return ffi::SQLITE_FULL;
    }
    let class = data.classify(name);
    let temp_flags = ffi::SQLITE_OPEN_TEMP_DB
        | ffi::SQLITE_OPEN_TRANSIENT_DB
        | ffi::SQLITE_OPEN_TEMP_JOURNAL
        | ffi::SQLITE_OPEN_SUBJOURNAL;
    let kind = if name.is_null() || flags & temp_flags != 0 {
        Kind::Temp
    } else if unsafe { CStr::from_ptr(name).to_bytes() }.ends_with(b"-journal")
        || unsafe { CStr::from_ptr(name).to_bytes() }.ends_with(b"-wal")
    {
        Kind::Journal
    } else {
        Kind::Data
    };
    if class == Class::Unknown {
        return data.refuse("checkpoint proof VFS refused unknown file path".into());
    }
    let align = align_of::<libc::max_align_t>();
    let inner_addr = (file as usize + size_of::<WrappedFile>() + align - 1) & !(align - 1);
    let inner = inner_addr as *mut ffi::sqlite3_file;
    let base = data.base;
    let Some(base_open) = (unsafe { (*base).xOpen }) else {
        return ffi::SQLITE_CANTOPEN;
    };
    // SAFETY: SQLite allocated szOsFile bytes; inner lies within that allocation and leaves
    // at least the base VFS's szOsFile bytes. The base xOpen owns only inner.
    let rc = unsafe { base_open(base, name, inner, flags, out) };
    if rc != ffi::SQLITE_OK {
        return rc;
    }
    let methods = unsafe { &*(*inner).pMethods };
    let required = methods.iVersion >= 3
        && methods.xClose.is_some()
        && methods.xRead.is_some()
        && methods.xWrite.is_some()
        && methods.xTruncate.is_some()
        && methods.xSync.is_some()
        && methods.xFileSize.is_some()
        && methods.xLock.is_some()
        && methods.xUnlock.is_some()
        && methods.xCheckReservedLock.is_some()
        && methods.xFileControl.is_some()
        && methods.xSectorSize.is_some()
        && methods.xDeviceCharacteristics.is_some();
    if !required {
        if let Some(close) = methods.xClose {
            unsafe { close(inner) };
        }
        return data.refuse("checkpoint proof VFS requires complete SQLite io methods v3".into());
    }
    let mut initial_size: ffi::sqlite3_int64 = 0;
    let size_rc = unsafe { methods.xFileSize.unwrap()(inner, &mut initial_size) };
    if size_rc != ffi::SQLITE_OK || initial_size < 0 {
        unsafe { methods.xClose.unwrap()(inner) };
        return data.refuse("checkpoint proof VFS could not measure opened file".into());
    }
    if kind == Kind::Data && class == Class::Scratch {
        let sector = unsafe { methods.xSectorSize.unwrap()(inner) };
        if sector <= 0 || sector > 65536 {
            unsafe { methods.xClose.unwrap()(inner) };
            return data.refuse(format!(
                "checkpoint proof VFS unsupported SQLite sector size {sector}"
            ));
        }
        data.sector_size.store(sector as usize, Ordering::Release);
    }
    data.window
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .file_sizes
        .insert(inner as usize, initial_size as u128);
    unsafe {
        (file as *mut WrappedFile).write(WrappedFile {
            file: ffi::sqlite3_file {
                pMethods: &IO_METHODS,
            },
            inner,
            data: data as *const Data as *mut Data,
            class,
            kind,
        });
    }
    data.files.fetch_add(1, Ordering::AcqRel);
    ffi::SQLITE_OK
}

unsafe extern "C" fn close(file: *mut ffi::sqlite3_file) -> c_int {
    let wrapped = unsafe { wrapped(file) };
    {
        let mut window = unsafe { &*wrapped.data }
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        window.file_sizes.remove(&(wrapped.inner as usize));
        window.file_sizes.remove(&((wrapped.inner as usize) ^ 1));
    }
    let rc = unsafe { ((*(*wrapped.inner).pMethods).xClose.unwrap())(wrapped.inner) };
    unsafe { &*wrapped.data }
        .files
        .fetch_sub(1, Ordering::AcqRel);
    rc
}

macro_rules! forward_file {
    ($name:ident, $field:ident, ($($arg:ident: $ty:ty),*), $ret:ty) => {
        unsafe extern "C" fn $name(file: *mut ffi::sqlite3_file, $($arg: $ty),*) -> $ret {
            match unsafe { methods(file).$field } {
                Some(call) => unsafe { call(inner(file), $($arg),*) },
                None => ffi::SQLITE_IOERR,
            }
        }
    };
}
unsafe extern "C" fn read(
    file: *mut ffi::sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    offset: ffi::sqlite3_int64,
) -> c_int {
    #[cfg(test)]
    {
        unsafe { &*wrapped(file).data }
            .read_calls
            .fetch_add(1, Ordering::Relaxed);
    }
    unsafe { methods(file).xRead.unwrap()(inner(file), buf, amt, offset) }
}
forward_file!(sync, xSync, (flags: c_int), c_int);
forward_file!(size, xFileSize, (out: *mut ffi::sqlite3_int64), c_int);
forward_file!(lock, xLock, (mode: c_int), c_int);
forward_file!(unlock, xUnlock, (mode: c_int), c_int);
forward_file!(reserved, xCheckReservedLock, (out: *mut c_int), c_int);
forward_file!(sector_size, xSectorSize, (), c_int);
forward_file!(device_characteristics, xDeviceCharacteristics, (), c_int);
forward_file!(shm_lock, xShmLock, (offset: c_int, n: c_int, flags: c_int), c_int);
forward_file!(shm_unmap, xShmUnmap, (delete: c_int), c_int);
forward_file!(unfetch, xUnfetch, (offset: ffi::sqlite3_int64, ptr: *mut c_void), c_int);

unsafe extern "C" fn fetch(
    _file: *mut ffi::sqlite3_file,
    _offset: ffi::sqlite3_int64,
    _amt: c_int,
    out: *mut *mut c_void,
) -> c_int {
    if out.is_null() {
        return ffi::SQLITE_IOERR;
    }
    // A sorter temp file is not a SQL connection, so PRAGMA mmap_size=0 on guarded
    // connections does not cover it. Never expose a mapping through the proof VFS.
    unsafe { *out = ptr::null_mut() };
    ffi::SQLITE_OK
}

unsafe extern "C" fn write(
    file: *mut ffi::sqlite3_file,
    buf: *const c_void,
    amt: c_int,
    offset: ffi::sqlite3_int64,
) -> c_int {
    let wrapped = unsafe { wrapped(file) };
    if amt < 0 || offset < 0 {
        return ffi::SQLITE_IOERR_WRITE;
    }
    let Some(end) = (offset as u128).checked_add(amt as u128) else {
        return ffi::SQLITE_FULL;
    };
    let rc = unsafe { &*wrapped.data }.before_write(
        wrapped.inner as usize,
        wrapped.class,
        wrapped.kind,
        amt as u128,
        end,
    );
    if rc != ffi::SQLITE_OK {
        return rc;
    }
    unsafe { methods(file).xWrite.unwrap()(wrapped.inner, buf, amt, offset) }
}

unsafe extern "C" fn truncate(file: *mut ffi::sqlite3_file, new_size: ffi::sqlite3_int64) -> c_int {
    let wrapped = unsafe { wrapped(file) };
    let mut old_size = 0;
    let rc = unsafe { methods(file).xFileSize.unwrap()(wrapped.inner, &mut old_size) };
    if rc != ffi::SQLITE_OK {
        return rc;
    }
    if new_size > old_size {
        let rc = unsafe { &*wrapped.data }.before_write(
            wrapped.inner as usize,
            wrapped.class,
            wrapped.kind,
            (new_size - old_size) as u128,
            new_size as u128,
        );
        if rc != ffi::SQLITE_OK {
            return rc;
        }
    }
    unsafe { methods(file).xTruncate.unwrap()(wrapped.inner, new_size) }
}

unsafe extern "C" fn file_control(
    file: *mut ffi::sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    let wrapped = unsafe { wrapped(file) };
    // SQLite asks this to choose the VFS for VACUUM INTO's output. Returning the underlying
    // pointer would silently bypass the proof guard on the largest write of the operation.
    if op == ffi::SQLITE_FCNTL_VFS_POINTER {
        if arg.is_null() {
            return ffi::SQLITE_IOERR;
        }
        unsafe {
            *(arg as *mut *mut ffi::sqlite3_vfs) = std::ptr::addr_of_mut!((*wrapped.data).vfs)
        };
        return ffi::SQLITE_OK;
    }
    if op == ffi::SQLITE_FCNTL_FILE_POINTER || op == ffi::SQLITE_FCNTL_JOURNAL_POINTER {
        return ffi::SQLITE_NOTFOUND;
    }
    if op == ffi::SQLITE_FCNTL_MMAP_SIZE {
        if arg.is_null() {
            return ffi::SQLITE_IOERR;
        }
        // SQLite's sorter requests a positive mmap limit on its own temp file,
        // even when every guarded SQL connection has mmap_size=0. Enforce zero
        // on the wrapped file instead of latching a false disk-full refusal.
        let mut zero: ffi::sqlite3_int64 = 0;
        let rc = unsafe {
            methods(file).xFileControl.unwrap()(
                wrapped.inner,
                op,
                (&mut zero as *mut ffi::sqlite3_int64).cast(),
            )
        };
        if rc != ffi::SQLITE_OK && rc != ffi::SQLITE_NOTFOUND {
            return rc;
        }
        unsafe { *(arg as *mut ffi::sqlite3_int64) = 0 };
        return ffi::SQLITE_OK;
    }
    if op == ffi::SQLITE_FCNTL_CHUNK_SIZE {
        if arg.is_null() {
            return ffi::SQLITE_IOERR;
        }
        if unsafe { *(arg as *const c_int) } > 0 {
            return unsafe { &*wrapped.data }
                .refuse("checkpoint proof VFS refused chunk preallocation".into());
        }
    }
    if op == ffi::SQLITE_FCNTL_OVERWRITE {
        return unsafe { &*wrapped.data }
            .refuse("checkpoint proof VFS refused unbounded overwrite control".into());
    }
    if op == ffi::SQLITE_FCNTL_SIZE_HINT {
        if arg.is_null() {
            return ffi::SQLITE_IOERR;
        }
        let mut old_size = 0;
        let rc = unsafe { methods(file).xFileSize.unwrap()(wrapped.inner, &mut old_size) };
        if rc != ffi::SQLITE_OK {
            return rc;
        }
        let hint = unsafe { *(arg as *const ffi::sqlite3_int64) };
        if hint > old_size {
            let rc = unsafe { &*wrapped.data }.before_write(
                wrapped.inner as usize,
                wrapped.class,
                wrapped.kind,
                (hint - old_size) as u128,
                hint as u128,
            );
            if rc != ffi::SQLITE_OK {
                return rc;
            }
        }
    }
    unsafe { methods(file).xFileControl.unwrap()(wrapped.inner, op, arg) }
}

unsafe extern "C" fn shm_map(
    file: *mut ffi::sqlite3_file,
    page: c_int,
    page_size: c_int,
    extend: c_int,
    out: *mut *mut c_void,
) -> c_int {
    let wrapped = unsafe { wrapped(file) };
    if extend != 0 {
        if wrapped.class == Class::Live {
            if let Err(error) = unsafe { &*wrapped.data }.guard.check_now() {
                return unsafe { &*wrapped.data }.refuse(error.to_string());
            }
        } else {
            let rc = unsafe { &*wrapped.data }.before_write(
                (wrapped.inner as usize) ^ 1,
                wrapped.class,
                Kind::Temp,
                page_size.max(0) as u128,
                (page.max(0) as u128 + 1) * page_size.max(0) as u128,
            );
            if rc != ffi::SQLITE_OK {
                return rc;
            }
        }
    }
    match unsafe { methods(file).xShmMap } {
        Some(call) => unsafe { call(wrapped.inner, page, page_size, extend, out) },
        None => ffi::SQLITE_IOERR,
    }
}

unsafe extern "C" fn shm_barrier(file: *mut ffi::sqlite3_file) {
    if let Some(call) = unsafe { methods(file).xShmBarrier } {
        unsafe { call(inner(file)) };
    }
}

static IO_METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
    iVersion: 3,
    xClose: Some(close),
    xRead: Some(read),
    xWrite: Some(write),
    xTruncate: Some(truncate),
    xSync: Some(sync),
    xFileSize: Some(size),
    xLock: Some(lock),
    xUnlock: Some(unlock),
    xCheckReservedLock: Some(reserved),
    xFileControl: Some(file_control),
    xSectorSize: Some(sector_size),
    xDeviceCharacteristics: Some(device_characteristics),
    xShmMap: Some(shm_map),
    xShmLock: Some(shm_lock),
    xShmBarrier: Some(shm_barrier),
    xShmUnmap: Some(shm_unmap),
    xFetch: Some(fetch),
    xUnfetch: Some(unfetch),
};

// Every process-wide VFS method receives the underlying default VFS, never our Data pointer.
macro_rules! forward_vfs {
    ($name:ident, $field:ident, ($($arg:ident: $ty:ty),*), $ret:ty) => {
        unsafe extern "C" fn $name(vfs: *mut ffi::sqlite3_vfs, $($arg: $ty),*) -> $ret {
            let base = unsafe { base(vfs) };
            let call = unsafe { (*base).$field.unwrap() };
            unsafe { call(base, $($arg),*) }
        }
    };
}
forward_vfs!(delete, xDelete, (name: *const c_char, sync: c_int), c_int);
forward_vfs!(access, xAccess, (name: *const c_char, flags: c_int, out: *mut c_int), c_int);
forward_vfs!(full_pathname, xFullPathname, (name: *const c_char, len: c_int, out: *mut c_char), c_int);
forward_vfs!(dl_open, xDlOpen, (name: *const c_char), *mut c_void);
forward_vfs!(randomness, xRandomness, (len: c_int, out: *mut c_char), c_int);
forward_vfs!(sleep, xSleep, (micros: c_int), c_int);
forward_vfs!(current_time, xCurrentTime, (out: *mut f64), c_int);
forward_vfs!(get_last_error, xGetLastError, (len: c_int, out: *mut c_char), c_int);
forward_vfs!(current_time_i64, xCurrentTimeInt64, (out: *mut ffi::sqlite3_int64), c_int);
forward_vfs!(set_system_call, xSetSystemCall, (name: *const c_char, call: ffi::sqlite3_syscall_ptr), c_int);
forward_vfs!(get_system_call, xGetSystemCall, (name: *const c_char), ffi::sqlite3_syscall_ptr);
forward_vfs!(next_system_call, xNextSystemCall, (name: *const c_char), *const c_char);

unsafe extern "C" fn dl_error(vfs: *mut ffi::sqlite3_vfs, len: c_int, out: *mut c_char) {
    let base = unsafe { base(vfs) };
    if let Some(call) = unsafe { (*base).xDlError } {
        unsafe { call(base, len, out) };
    }
}
unsafe extern "C" fn dl_close(vfs: *mut ffi::sqlite3_vfs, handle: *mut c_void) {
    let base = unsafe { base(vfs) };
    if let Some(call) = unsafe { (*base).xDlClose } {
        unsafe { call(base, handle) };
    }
}
unsafe extern "C" fn dl_sym(
    vfs: *mut ffi::sqlite3_vfs,
    handle: *mut c_void,
    name: *const c_char,
) -> Option<unsafe extern "C" fn(*mut ffi::sqlite3_vfs, *mut c_void, *const c_char)> {
    let base = unsafe { base(vfs) };
    unsafe { (*base).xDlSym.and_then(|call| call(base, handle, name)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::checkpoint_space::{allocated, sqlite_temp_dir};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    fn guard(live: &Path, scratch: &Path, budget: u128) -> SpaceGuard {
        let scratch_fs = filesystem(scratch).unwrap();
        let temp = sqlite_temp_dir().unwrap();
        let temp_fs = filesystem(&temp).unwrap();
        SpaceGuard {
            live: live.to_owned(),
            live_wal: PathBuf::from(format!("{}-wal", live.display())),
            scratch: scratch.to_owned(),
            temp,
            scratch_device: scratch_fs.device,
            temp_device: temp_fs.device,
            initial_available: 0, // fixture budget is local; parallel builds change host free bytes
            initial_scratch_allocated: super::super::existing_scratch_bytes(scratch).unwrap(),
            initial_temp_available: temp_fs.available,
            initial_wal_allocated: 0,
            page_size: 4096,
            granule: 4096,
            temp_granule: 4096,
            copy_bound: u128::MAX,
            clone_cap: u128::MAX,
            vacuum_cap: u128::MAX,
            vacuum_tree_pages: 0,
            vacuum_pointer_pages: 0,
            vacuum_wal_pages: 0,
            vacuum_source_logical: Arc::new(Mutex::new(4096)),
            snapshot_logical: 4096,
            initial_journal_pages: 1,
            budget,
            temp_budget: u128::MAX / 2,
            reserve: 0,
            wal_allowance: u128::MAX,
            shared_device: true,
            failure: Arc::new(Mutex::new(None)),
        }
    }

    #[test]
    fn proof_vfs_self_test_refuses_a_real_sqlite_write_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        fs::write(&live, [0; 4096]).unwrap();
        let scratch = dir.path().join("scratch");
        let owner = ProofOwnership::acquire(&scratch).unwrap();
        let vfs = ProofVfs::register(&guard(&live, &scratch, 1024 * 1024)).unwrap();
        let probe = scratch.join("proof-vfs-test.sqlite3");
        vfs.self_test(&owner, &probe).unwrap();
        vfs.check_closed().unwrap();
        assert!(!probe.exists());
        assert!(!PathBuf::from(format!("{}.owner", probe.display())).exists());
    }

    #[test]
    fn unavailable_write_guard_refuses_the_proof_before_copy() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        fs::write(&live, [0; 4096]).unwrap();
        let scratch = dir.path().join("scratch");
        let owner = ProofOwnership::acquire(&scratch).unwrap();
        let vfs = ProofVfs::register(&guard(&live, &scratch, 1024 * 1024)).unwrap();
        vfs.data
            .as_ref()
            .unwrap()
            .simulate_unavailable
            .store(true, Ordering::Release);
        let probe = scratch.join("proof-vfs-test.sqlite3");
        let error = vfs.self_test(&owner, &probe).unwrap_err().to_string();
        assert!(error.contains("self-test failed"), "{error}");
        assert!(!probe.exists());
        assert!(!PathBuf::from(format!("{}.owner", probe.display())).exists());
    }

    #[test]
    fn guarded_vacuum_refuses_before_spending_the_small_budget() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        let source = Connection::open(&live).unwrap();
        source
            .execute_batch("CREATE TABLE payload(bytes BLOB);")
            .unwrap();
        for _ in 0..256 {
            source
                .execute("INSERT INTO payload VALUES (zeroblob(4096))", [])
                .unwrap();
        }
        drop(source);
        let scratch = dir.path().join("scratch");
        let owner = ProofOwnership::acquire(&scratch).unwrap();
        let guard = guard(&live, &scratch, 128 * 1024);
        let vfs = ProofVfs::register(&guard).unwrap();
        vfs.self_test(&owner, &scratch.join("proof-vfs-test.sqlite3"))
            .unwrap();
        let copy = scratch.join("proof-copy.sqlite3");
        owner.mark(&copy).unwrap();
        let source = Connection::open_with_flags_and_vfs(
            &live,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
            vfs.name(),
        )
        .unwrap();
        source.execute_batch("PRAGMA mmap_size=0;").unwrap();
        let mmap: i64 = source
            .query_row("PRAGMA mmap_size", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mmap, 0);
        let error = source
            .execute("VACUUM INTO ?1", [copy.to_string_lossy()])
            .unwrap_err();
        drop(source); // Never run rollback SQL after a VFS refusal.
        let message = guard
            .explain::<()>(Err(error.into()))
            .unwrap_err()
            .to_string();
        assert!(message.contains("refused before SQLite write"), "{message}");
        assert!(
            message.contains("available=") && message.contains("reserve="),
            "{message}"
        );
        assert!(allocated(&copy).unwrap() <= 132 * 1024);
        let (calls, refreshes, max_request, refresh_ns, data_writes, journal_writes, temp_writes) =
            vfs.metrics();
        assert!(calls > 0 && refreshes > 0 && max_request > 0, "{message}");
        assert!(data_writes > 0, "VACUUM destination bypassed the proof VFS");
        eprintln!(
            "checkpoint VFS VACUUM fixture: calls={calls} refreshes={refreshes} max_request={max_request} statvfs_refresh_ns={refresh_ns} data_writes={data_writes} journal_writes={journal_writes} temp_writes={temp_writes}"
        );
        vfs.check_closed().unwrap();
        owner.finish(&copy).unwrap();
        assert!(!copy.exists());
    }

    #[test]
    fn source_growth_past_vacuum_cap_refuses_before_copy_write() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        let source = Connection::open(&live).unwrap();
        source
            .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE payload(bytes BLOB);")
            .unwrap();
        let (_, _, page_size, pages, _, btrees, _) = super::super::live_size(&live).unwrap();
        let scratch = dir.path().join("scratch");
        let owner = ProofOwnership::acquire(&scratch).unwrap();
        let mut guard = guard(&live, &scratch, 4 * 1024 * 1024);
        guard.page_size = page_size;
        guard.snapshot_logical = page_size * pages;
        guard.vacuum_tree_pages = 2 * btrees;
        guard.vacuum_wal_pages = 1;
        guard.vacuum_cap =
            guard.snapshot_logical + page_size * (guard.vacuum_tree_pages + guard.vacuum_wal_pages);
        let vfs = ProofVfs::register(&guard).unwrap();
        vfs.self_test(&owner, &scratch.join("proof-vfs-test.sqlite3"))
            .unwrap();
        source
            .execute("INSERT INTO payload VALUES (zeroblob(131072))", [])
            .unwrap();
        let copy = scratch.join("proof-copy.sqlite3");
        let writes_before = vfs.metrics().4;
        let error = vfs.vacuum_copy().unwrap_err().to_string();
        assert!(
            error.contains("source grew beyond WAL allowance"),
            "{error}"
        );
        assert!(super::super::live_size(&live).unwrap().3 * page_size > guard.vacuum_cap);
        assert_eq!(vfs.metrics().4, writes_before);
        assert!(!copy.exists());
        drop(source);
        vfs.check_closed().unwrap();
    }

    #[test]
    fn incremental_auto_vacuum_uses_pointer_map_cap() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        let source = Connection::open(&live).unwrap();
        source
            .execute_batch("PRAGMA auto_vacuum=INCREMENTAL;")
            .unwrap();
        let mut reserve_bytes: c_int = 32;
        // SQLite's RESERVE_BYTES file control persists byte 20 of the page-1 header.
        // SAFETY: the connection and mutable integer remain valid for this synchronous call.
        let rc = unsafe {
            ffi::sqlite3_file_control(
                source.handle(),
                c"main".as_ptr(),
                ffi::SQLITE_FCNTL_RESERVE_BYTES,
                (&mut reserve_bytes as *mut c_int).cast::<c_void>(),
            )
        };
        assert_eq!(rc, ffi::SQLITE_OK);
        source.execute_batch("VACUUM;").unwrap();
        source
            .execute_batch(
                "CREATE TABLE payload(bytes BLOB); INSERT INTO payload VALUES (zeroblob(32768));",
            )
            .unwrap();
        let (_, _, page_size, pages, auto_vacuum, btrees, usable_page_size) =
            super::super::live_size(&live).unwrap();
        assert_eq!(auto_vacuum, 2);
        assert_eq!(usable_page_size, page_size - 32);
        let (tree_pages, pointer_pages) =
            super::super::vacuum_structural_pages(usable_page_size, pages, auto_vacuum, btrees)
                .unwrap();
        assert!(pointer_pages > 0);
        let scratch = dir.path().join("scratch");
        let owner = ProofOwnership::acquire(&scratch).unwrap();
        let mut guard = guard(&live, &scratch, 4 * 1024 * 1024);
        guard.page_size = page_size;
        guard.snapshot_logical = page_size * pages;
        guard.vacuum_tree_pages = tree_pages;
        guard.vacuum_pointer_pages = pointer_pages;
        guard.vacuum_wal_pages = 0;
        guard.vacuum_cap = guard.snapshot_logical + page_size * (tree_pages + pointer_pages);
        let vfs = ProofVfs::register(&guard).unwrap();
        vfs.self_test(&owner, &scratch.join("proof-vfs-test.sqlite3"))
            .unwrap();
        vfs.vacuum_copy().unwrap();
        let copy = scratch.join("proof-copy.sqlite3");
        owner.mark(&copy).unwrap();
        let input = Connection::open_with_flags_and_vfs(
            &live,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
            vfs.name(),
        )
        .unwrap();
        input
            .execute("VACUUM INTO ?1", [copy.to_string_lossy()])
            .unwrap();
        drop(input);
        guard.check_copied_image(&copy, &vfs).unwrap();
        assert!(vfs.metrics().4 > 0);
        vfs.check_closed().unwrap();
        owner.finish(&copy).unwrap();
    }

    #[test]
    fn bulk_rewrite_journal_refusal_discards_the_entire_proof() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        fs::write(&live, [0; 4096]).unwrap();
        let scratch = dir.path().join("scratch");
        let owner = ProofOwnership::acquire(&scratch).unwrap();
        let copy = scratch.join("proof-bulk.sqlite3");
        {
            let db = Connection::open(&copy).unwrap();
            db.execute_batch("CREATE TABLE payload(bytes BLOB);")
                .unwrap();
            for _ in 0..256 {
                db.execute("INSERT INTO payload VALUES (zeroblob(4096))", [])
                    .unwrap();
            }
        }
        let guard = guard(&live, &scratch, 4 * 1024);
        let vfs = ProofVfs::register(&guard).unwrap();
        vfs.self_test(&owner, &scratch.join("proof-vfs-test.sqlite3"))
            .unwrap();
        owner.mark(&copy).unwrap();
        let db = Connection::open_with_flags_and_vfs(
            &copy,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
            vfs.name(),
        )
        .unwrap();
        db.execute_batch("PRAGMA mmap_size=0; PRAGMA journal_mode=DELETE;")
            .unwrap();
        let result = db.execute_batch("BEGIN; DELETE FROM payload;");
        assert!(result.is_err());
        drop(db);
        let message = guard
            .explain::<()>(result.map_err(Into::into))
            .unwrap_err()
            .to_string();
        assert!(message.contains("refused before SQLite write"), "{message}");
        let (_, _, _, _, _, journal_writes, _) = vfs.metrics();
        assert!(
            journal_writes > 0,
            "rollback journal bypassed the proof VFS"
        );
        vfs.check_closed().unwrap();
        owner.finish(&copy).unwrap();
        assert!(!copy.exists());
        assert!(!PathBuf::from(format!("{}-journal", copy.display())).exists());
    }

    #[test]
    fn canonical_renumber_and_temp_sort_write_through_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        fs::write(&live, [0; 4096]).unwrap();
        let scratch = dir.path().join("scratch");
        let owner = ProofOwnership::acquire(&scratch).unwrap();
        let copy = scratch.join("proof-renumber.sqlite3");
        {
            let db = Connection::open(&copy).unwrap();
            db.execute_batch(
                "CREATE TABLE claims(id INTEGER PRIMARY KEY, position INTEGER, body BLOB);",
            )
            .unwrap();
            db.execute_batch(
                "WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<1024)
                 INSERT INTO claims(position, body) SELECT n, zeroblob(4096) FROM seq;",
            )
            .unwrap();
        }
        // This fixture checks interception, not admission. SQLite can rewrite the temporary
        // sorter file many times while producing one sorted table.
        let guard = guard(&live, &scratch, 1024 * 1024 * 1024);
        let vfs = ProofVfs::register(&guard).unwrap();
        vfs.self_test(&owner, &scratch.join("proof-vfs-test.sqlite3"))
            .unwrap();
        owner.mark(&copy).unwrap();
        let db = Connection::open_with_flags_and_vfs(
            &copy,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
            vfs.name(),
        )
        .unwrap();
        #[cfg(target_os = "linux")]
        let observed_temp_files = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        #[cfg(target_os = "linux")]
        {
            let observed = observed_temp_files.clone();
            db.progress_handler(
                1000,
                Some(move || {
                    if let Ok(entries) = fs::read_dir("/proc/self/fd") {
                        for entry in entries.flatten() {
                            if let Ok(path) = fs::read_link(entry.path()) {
                                let path = path.to_string_lossy();
                                if path.contains("etilqs") {
                                    let mut paths = observed.lock().unwrap();
                                    if !paths.iter().any(|seen| seen == &*path) {
                                        paths.push(path.into_owned());
                                    }
                                }
                            }
                        }
                    }
                    false
                }),
            );
        }
        db.execute_batch("PRAGMA mmap_size=0; PRAGMA journal_mode=DELETE; PRAGMA temp_store=FILE; PRAGMA cache_size=1;").unwrap();
        vfs.begin_proof(&db).unwrap();
        let mmap: i64 = db
            .query_row("PRAGMA mmap_size", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mmap, 0);
        let before = vfs.metrics();
        db.execute_batch("UPDATE claims SET position=1025-position;")
            .unwrap();
        let after_renumber = vfs.metrics();
        assert!(
            after_renumber.4 > before.4,
            "renumber data writes bypassed the proof VFS"
        );
        assert!(
            after_renumber.5 > before.5,
            "renumber journal writes bypassed the proof VFS"
        );
        if let Err(error) = db
            .execute_batch("CREATE TEMP TABLE sorted AS SELECT body FROM claims ORDER BY body, id;")
        {
            eprintln!("selected SQLite temp directory: {}", guard.temp.display());
            eprintln!(
                "scratch device: {}; temp device: {}",
                guard.scratch_device, guard.temp_device
            );
            #[cfg(target_os = "linux")]
            eprintln!(
                "observed SQLite temp files: {:?}",
                observed_temp_files.lock().unwrap()
            );
            eprintln!(
                "VFS metrics before sort: {:?}; after error: {:?}",
                after_renumber,
                vfs.metrics()
            );
            let window = vfs.data.as_ref().unwrap().window.lock().unwrap();
            eprintln!(
                "VFS accounting at refusal: max_requested_iAmt={} scratch_used={} scratch_pending={} temp_used={} temp_pending={} temp_cumulative_requested={} scratch_budget={} temp_budget={} refreshes={}",
                window.max_request,
                window.scratch_used,
                window.scratch_pending,
                window.temp_used,
                window.temp_pending,
                window.temp_total_requested,
                guard.budget,
                guard.temp_budget,
                window.refreshes,
            );
            drop(window);
            eprintln!(
                "post-refusal statvfs: scratch_available={:?} temp_available={:?}",
                filesystem(&guard.scratch).map(|fs| fs.available),
                filesystem(&guard.temp).map(|fs| fs.available),
            );
            panic!(
                "sort failed: {}",
                guard.explain::<()>(Err(error.into())).unwrap_err()
            );
        }
        let after_sort = vfs.metrics();
        #[cfg(target_os = "linux")]
        eprintln!(
            "observed SQLite temp files: {:?}",
            observed_temp_files.lock().unwrap()
        );
        assert!(
            after_sort.6 > after_renumber.6,
            "SQLite temp writes bypassed the proof VFS"
        );
        drop(db);
        vfs.check_closed().unwrap();
        owner.finish(&copy).unwrap();
    }

    #[test]
    fn positive_mmap_control_is_zeroed_and_does_not_bypass_a_counted_write() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        fs::write(&live, [0; 4096]).unwrap();
        let scratch = dir.path().join("scratch");
        let owner = ProofOwnership::acquire(&scratch).unwrap();
        let guard = guard(&live, &scratch, 64 * 1024);
        let vfs = ProofVfs::register(&guard).unwrap();
        let copy = scratch.join("proof-mmap-control.sqlite3");
        owner.mark(&copy).unwrap();
        let name = CString::new(copy.as_os_str().as_bytes()).unwrap();
        let data = vfs.data.as_ref().unwrap();
        // SAFETY: SQLite's allocator supplies an aligned szOsFile allocation. The wrapped
        // xOpen initializes it; xClose runs before sqlite3_free.
        let file = unsafe { ffi::sqlite3_malloc(data.vfs.szOsFile) as *mut ffi::sqlite3_file };
        assert!(!file.is_null());
        let mut out_flags = 0;
        let rc = unsafe {
            data.vfs.xOpen.unwrap()(
                (&data.vfs as *const ffi::sqlite3_vfs).cast_mut(),
                name.as_ptr(),
                file,
                ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE | ffi::SQLITE_OPEN_MAIN_DB,
                &mut out_flags,
            )
        };
        assert_eq!(rc, ffi::SQLITE_OK);
        let io = unsafe { &*(*file).pMethods };
        let mut requested: ffi::sqlite3_int64 = 1024 * 1024 * 1024;
        let rc = unsafe {
            io.xFileControl.unwrap()(
                file,
                ffi::SQLITE_FCNTL_MMAP_SIZE,
                (&mut requested as *mut ffi::sqlite3_int64).cast(),
            )
        };
        assert_eq!(rc, ffi::SQLITE_OK);
        assert_eq!(requested, 0);
        let mut mapped = std::ptr::dangling_mut::<c_void>();
        let rc = unsafe { io.xFetch.unwrap()(file, 0, 4096, &mut mapped) };
        assert_eq!(rc, ffi::SQLITE_OK);
        assert!(mapped.is_null());
        let bytes = [7u8; 4096];
        let rc = unsafe { io.xWrite.unwrap()(file, bytes.as_ptr().cast(), 4096, 0) };
        assert_eq!(rc, ffi::SQLITE_OK, "{:?}", guard.failure.lock().unwrap());
        for _ in 0..100 {
            assert_eq!(
                unsafe { io.xWrite.unwrap()(file, bytes.as_ptr().cast(), 4096, 0) },
                ffi::SQLITE_OK
            );
        }
        let window = data.window.lock().unwrap();
        assert_eq!(window.data_writes, 101);
        assert_eq!(window.scratch_pending, 4096);
        drop(window);
        let one = [7u8; 1];
        assert_eq!(
            unsafe { io.xWrite.unwrap()(file, one.as_ptr().cast(), 1, 4096) },
            ffi::SQLITE_OK
        );
        assert_eq!(data.window.lock().unwrap().scratch_pending, 8192);
        // This request crosses a third page. Refusal precedes the underlying write.
        assert_eq!(
            unsafe { io.xWrite.unwrap()(file, one.as_ptr().cast(), 1, 64 * 1024) },
            ffi::SQLITE_FULL
        );
        assert_eq!(fs::metadata(&copy).unwrap().len(), 4097);
        assert_eq!(unsafe { io.xClose.unwrap()(file) }, ffi::SQLITE_OK);
        unsafe { ffi::sqlite3_free(file.cast()) };
        vfs.check_closed().unwrap();
        owner.finish(&copy).unwrap();
    }

    #[test]
    fn requested_mmap_reads_use_xread_after_the_guard_returns_no_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sqlite3");
        fs::write(&live, [0; 4096]).unwrap();
        let scratch = dir.path().join("scratch");
        let owner = ProofOwnership::acquire(&scratch).unwrap();
        let copy = scratch.join("proof-mmap-read.sqlite3");
        let guard = guard(&live, &scratch, 1024 * 1024);
        let vfs = ProofVfs::register(&guard).unwrap();
        owner.mark(&copy).unwrap();
        {
            let db = Connection::open_with_flags_and_vfs(
                &copy,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
                vfs.name(),
            )
            .unwrap();
            db.execute_batch(
                "CREATE TABLE payload(bytes BLOB); INSERT INTO payload VALUES (zeroblob(16384));",
            )
            .unwrap();
        }
        let db = Connection::open_with_flags_and_vfs(
            &copy,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
            vfs.name(),
        )
        .unwrap();
        db.execute_batch("PRAGMA mmap_size=1073741824;").unwrap();
        let mmap: i64 = db
            .query_row("PRAGMA mmap_size", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mmap, 0);
        let before = vfs
            .data
            .as_ref()
            .unwrap()
            .read_calls
            .load(Ordering::Relaxed);
        let hex_len: i64 = db
            .query_row("SELECT length(hex(bytes)) FROM payload", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(hex_len, 32768);
        let after = vfs
            .data
            .as_ref()
            .unwrap()
            .read_calls
            .load(Ordering::Relaxed);
        assert!(after > before, "table read bypassed the VFS xRead");
        drop(db);
        vfs.check_closed().unwrap();
        owner.finish(&copy).unwrap();
    }
}
