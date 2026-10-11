//! The host facts conditions need that the daemon does not otherwise measure: free space on each
//! local filesystem, and the physical database size. Each is read
//! from the kernel by plain code, only for a condition that asks for it.
//! Disk space is sampled every 30 seconds; physical database size is cached for an hour.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::disk::{DiskSpace, disk_space};

#[derive(Clone, Default)]
struct DiskRequests {
    all: bool,
    paths: std::collections::BTreeSet<String>,
}

#[derive(Default)]
struct DiskSnapshot {
    at: u128,
    all: BTreeMap<String, DiskSpace>,
    paths: BTreeMap<String, DiskSpace>,
    error: Option<String>,
}

#[derive(Default)]
struct DiskCache {
    requests: std::sync::Mutex<DiskRequests>,
    snapshot: std::sync::Mutex<DiskSnapshot>,
}

/// One native-filesystem worker per daemon. A stuck kernel call makes disk readings stale;
/// the evaluator and its other metrics continue, and no replacement workers accumulate.
#[derive(Clone)]
pub(crate) struct DiskSampler(std::sync::Arc<DiskCache>);

impl DiskSampler {
    pub fn spawn() -> Self {
        Self::spawn_with(|requests| {
            let mut snapshot = DiskSnapshot::default();
            if requests.all {
                snapshot.all = filesystems();
            }
            for path in &requests.paths {
                if let Some(space) = filesystem_of(path) {
                    snapshot.paths.insert(path.clone(), space);
                } else {
                    snapshot.error = Some(format!(
                        "unavailable native filesystem selector: {}",
                        path.chars().take(256).collect::<String>()
                    ));
                }
            }
            snapshot
        })
    }

    fn spawn_with(read: impl Fn(&DiskRequests) -> DiskSnapshot + Send + Sync + 'static) -> Self {
        let cache = std::sync::Arc::new(DiskCache::default());
        let weak = std::sync::Arc::downgrade(&cache);
        let read = std::sync::Arc::new(read);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Some(cache) = weak.upgrade() else { break };
                let requests = cache
                    .requests
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                if !requests.all && requests.paths.is_empty() {
                    continue;
                }
                let read = read.clone();
                let mut task = tokio::task::spawn_blocking(move || read(&requests));
                match tokio::time::timeout(std::time::Duration::from_secs(10), &mut task).await {
                    Ok(Ok(mut snapshot)) => {
                        snapshot.at = super::now_ms();
                        *cache
                            .snapshot
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = snapshot;
                    }
                    Ok(Err(error)) => {
                        cache
                            .snapshot
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .error = Some(format!("disk probe failed: {error}"))
                    }
                    Err(_) => {
                        cache
                            .snapshot
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .error = Some(
                            "disk probe exceeded 10 seconds; other condition metrics continue"
                                .into(),
                        );
                        // Discard late data: earlier mounts in that result may already be stale.
                        let _ = task.await;
                    }
                }
            }
        });
        Self(cache)
    }

    pub fn configure(&self, all: bool, paths: impl Iterator<Item = String>) {
        *self
            .0
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = DiskRequests {
            all,
            paths: paths.take(super::MAX_CONDITIONS).collect(),
        };
    }

    pub fn filesystems(&self) -> BTreeMap<String, DiskSpace> {
        self.0
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .all = true;
        let snapshot = self
            .0
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if super::now_ms() < snapshot.at
            || super::now_ms().saturating_sub(snapshot.at) > super::STALE_AFTER_MS
        {
            BTreeMap::new()
        } else {
            snapshot.all.clone()
        }
    }

    pub fn filesystem_of(&self, path: &str) -> Option<DiskSpace> {
        let mut requests = self
            .0
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if requests.paths.len() < super::MAX_CONDITIONS {
            requests.paths.insert(path.to_owned());
        }
        drop(requests);
        let snapshot = self
            .0
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (super::now_ms() >= snapshot.at
            && super::now_ms().saturating_sub(snapshot.at) <= super::STALE_AFTER_MS)
            .then(|| snapshot.paths.get(path).cloned())
            .flatten()
    }

    pub fn errors(&self) -> Vec<String> {
        let requests = self
            .0
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !requests.all && requests.paths.is_empty() {
            return Vec::new();
        }
        drop(requests);
        self.0
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .error
            .clone()
            .into_iter()
            .collect()
    }
}

/// Filesystem types that hold data a host can run out of room on. Everything else in the mount
/// table (proc, sysfs, cgroups, tmpfs, overlays, network filesystems) is skipped.
const LOCAL_FILESYSTEMS: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "zfs", "f2fs", "bcachefs", "jfs", "reiserfs", "vfat",
    "exfat", "ntfs", "ntfs3", "apfs", "hfs",
];

/// The mount points of local filesystems, from `/proc/self/mountinfo` text. A mount point that
/// is the same filesystem mounted again (a bind mount, a btrfs subvolume) is kept here and
/// counted once when its space is read.
pub fn mount_points(mountinfo: &str) -> Vec<String> {
    let mut mounts = Vec::new();
    for line in mountinfo.lines() {
        // `36 35 98:0 /root /mnt rw,noatime master:1 - ext3 /dev/root rw,errors=continue`
        let Some((before, after)) = line.split_once(" - ") else {
            continue;
        };
        let Some(mount) = before.split(' ').nth(4) else {
            continue;
        };
        let Some(kind) = after.split(' ').next() else {
            continue;
        };
        if LOCAL_FILESYSTEMS.contains(&kind) {
            mounts.push(unescape(mount));
        }
    }
    mounts
}

/// The kernel writes a space in a path as `\040`, and likewise tab, newline and backslash.
fn unescape(path: &str) -> String {
    let mut output = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..index + 4].iter().all(u8::is_ascii_digit)
            && let Ok(code) = u8::from_str_radix(&path[index + 1..index + 4], 8)
        {
            output.push(code);
            index += 4;
            continue;
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).unwrap_or_else(|_| path.to_owned())
}

/// Free space on each local filesystem, keyed by its first mount point. A host without a mount
/// table produces no disk reading.
pub(crate) fn filesystems() -> BTreeMap<String, DiskSpace> {
    #[cfg(not(target_os = "macos"))]
    let mounts = std::fs::read_to_string("/proc/self/mountinfo")
        .map(|text| mount_points(&text))
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    let mounts = apple_mount_points();
    let mut mounts = mounts;
    mounts.sort();
    let mut seen = std::collections::BTreeSet::new();
    let mut output = BTreeMap::new();
    for mount in mounts.into_iter().take(256) {
        if let Ok(space) = disk_space(Path::new(&mount))
            && space.total > 0
            && seen.insert(space.filesystem)
        {
            output.insert(mount, space);
        }
    }
    output
}

/// Caller-owned mount table: unlike getmntinfo, getfsstat does not return shared static storage.
#[cfg(target_os = "macos")]
fn apple_mounts() -> Vec<(String, String)> {
    let mut mounts = Vec::<libc::statfs>::with_capacity(256);
    let bytes = mounts.capacity() * std::mem::size_of::<libc::statfs>();
    // SAFETY: the allocation can hold 256 statfs records. MNT_NOWAIT uses cached kernel facts.
    let count =
        unsafe { libc::getfsstat(mounts.as_mut_ptr(), bytes as libc::c_int, libc::MNT_NOWAIT) };
    if count <= 0 {
        return Vec::new();
    }
    // SAFETY: getfsstat initialized the returned records, bounded by the supplied buffer.
    unsafe {
        mounts.set_len((count as usize).min(mounts.capacity()));
    }
    mounts
        .iter()
        .filter_map(|mount| {
            // SAFETY: the kernel's fixed-size filesystem and mount names are NUL-terminated.
            let kind = unsafe { std::ffi::CStr::from_ptr(mount.f_fstypename.as_ptr()) }
                .to_str()
                .ok()?;
            Some((
                unsafe { std::ffi::CStr::from_ptr(mount.f_mntonname.as_ptr()) }
                    .to_str()
                    .ok()?
                    .to_owned(),
                kind.to_owned(),
            ))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn apple_mount_points() -> Vec<String> {
    apple_mounts()
        .into_iter()
        .filter(|(_, kind)| LOCAL_FILESYSTEMS.contains(&kind.as_str()))
        .map(|(mount, _)| mount)
        .collect()
}

/// Free space on the filesystem holding `path`.
/// Select the enclosing native mount from kernel facts, then stat the mount itself.
/// Never stat an authored path: it might traverse a symlink, autofs or a network mount.
pub(crate) fn filesystem_of(path: &str) -> Option<DiskSpace> {
    if !Path::new(path).is_absolute()
        || Path::new(path)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    #[cfg(target_os = "linux")]
    let mount = local_mount_for(path, &std::fs::read_to_string("/proc/self/mountinfo").ok()?)?;
    #[cfg(target_os = "macos")]
    let mount = apple_mounts()
        .into_iter()
        .filter(|(mount, _)| Path::new(path).starts_with(mount))
        .max_by_key(|(mount, _)| mount.len())
        .filter(|(_, kind)| LOCAL_FILESYSTEMS.contains(&kind.as_str()))?
        .0;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return None;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let mut prefix = PathBuf::new();
        for component in Path::new(path).components() {
            prefix.push(component.as_os_str());
            match std::fs::symlink_metadata(&prefix) {
                Ok(metadata) if metadata.file_type().is_symlink() => return None,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    disk_space(Path::new(&mount))
        .ok()
        .filter(|space| space.total > 0)
}

/// An explicit selector cannot traverse a more-specific foreign mount.
#[cfg(any(target_os = "linux", test))]
fn local_mount_for(path: &str, mountinfo: &str) -> Option<String> {
    let path = Path::new(path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    let (mount, kind) = mountinfo
        .lines()
        .filter_map(|line| {
            let (before, after) = line.split_once(" - ")?;
            let mount = unescape(before.split(' ').nth(4)?);
            let kind = after.split(' ').next()?;
            path.starts_with(&mount).then_some((mount, kind))
        })
        .max_by_key(|(mount, _)| mount.len())?;
    LOCAL_FILESYSTEMS.contains(&kind).then_some(mount)
}

pub(crate) fn free_percent(space: &DiskSpace) -> f64 {
    space.available as f64 * 100.0 / space.total as f64
}

/// The physical database and WAL bytes, sampled in the background.
pub fn database_bytes(database: &Path) -> Option<f64> {
    let main = std::fs::metadata(database).ok()?.len();
    let mut wal = database.as_os_str().to_owned();
    wal.push("-wal");
    let wal = std::fs::metadata(PathBuf::from(wal))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    Some((main + wal) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_filesystems_come_from_the_mount_table() {
        let mountinfo = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 0:21 / /proc rw,nosuid shared:12 - proc proc rw
24 22 0:5 / /dev rw,nosuid shared:2 - devtmpfs devtmpfs rw,size=4096k
25 22 259:3 / /srv/data\\040disk rw,relatime shared:3 - xfs /dev/nvme1n1 rw
26 22 0:30 / /run rw,nosuid shared:5 - tmpfs tmpfs rw
27 22 0:31 /@home /home rw,relatime shared:6 - btrfs /dev/sda1 rw
";
        assert_eq!(mount_points(mountinfo), ["/", "/srv/data disk", "/home"]);
    }

    #[test]
    fn authored_paths_do_not_select_foreign_mounts_or_parent_traversals() {
        let mounts = "1 0 8:1 / / rw - ext4 /dev/a rw\n2 1 0:2 / /net rw - nfs server:/a rw\n3 1 0:3 / /auto rw - autofs auto rw\n4 1 0:4 / /fuse rw - fuse.sshfs remote rw\n";
        assert_eq!(local_mount_for("/srv/data", mounts).as_deref(), Some("/"));
        for path in [
            "/net/data",
            "/auto/data",
            "/fuse/data",
            "/srv/../net",
            "relative",
        ] {
            assert_eq!(local_mount_for(path, mounts), None, "{path}");
        }
    }

    #[tokio::test]
    async fn a_blocked_disk_worker_does_not_block_cached_reads() {
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let entered = std::sync::Mutex::new(Some(entered_tx));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release = std::sync::Mutex::new(release_rx);
        let sampler = DiskSampler::spawn_with(move |_| {
            if let Some(sender) = entered.lock().unwrap().take() {
                let _ = sender.send(());
            }
            let _ = release.lock().unwrap().recv();
            DiskSnapshot::default()
        });
        sampler.configure(true, std::iter::empty());
        let started = tokio::time::timeout(std::time::Duration::from_secs(2), entered_rx).await;
        let reads = sampler.filesystems();
        let errors = sampler.errors();
        release_tx.send(()).unwrap();
        assert!(started.is_ok());
        assert!(reads.is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn the_database_counts_its_write_ahead_log() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("claims.sqlite3");
        std::fs::write(&database, vec![0; 100]).unwrap();
        assert_eq!(database_bytes(&database), Some(100.0));
        std::fs::write(root.path().join("claims.sqlite3-wal"), vec![0; 20]).unwrap();
        assert_eq!(database_bytes(&database), Some(120.0));
        assert_eq!(database_bytes(&root.path().join("missing")), None);
    }
}
