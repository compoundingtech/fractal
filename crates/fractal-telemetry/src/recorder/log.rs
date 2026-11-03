//! Finite local diagnostic history. The separate lock inode survives generation changes.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek as _, SeekFrom, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const FILE_BYTES: u64 = 64 * 1024 * 1024;
// Reserve the last 100 ms of the two-second contention budget for drop accounting.
const LOCK_WAIT: Duration = Duration::from_millis(1900);
const DROP_WAIT: Duration = Duration::from_millis(100);

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

fn regular(file: &File) -> io::Result<()> {
    if file.metadata()?.is_file() {
        Ok(())
    } else {
        Err(io::Error::other("recorder history must be a regular file"))
    }
}

fn lock(path: &Path, wait: Duration) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NONBLOCK)
        .open(sibling(path, ".lock"))?;
    acquire(file, wait)
}

fn acquire(file: File, wait: Duration) -> io::Result<File> {
    regular(&file)?;
    // SAFETY: file owns the descriptor. Closing it releases the flock, including on process exit.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(file);
    }
    let error = io::Error::last_os_error();
    if error.kind() != io::ErrorKind::WouldBlock {
        return Err(error);
    }
    // Wait in the kernel, not a retry/poll loop. A stopped holder must not hold the wrapped
    // command forever. If the receiver expires, send drops the acquired file and its lock.
    // In production the recorder exits immediately afterward, also ending a blocked waiter.
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("recorder-log-lock".into())
        .spawn(move || {
            // SAFETY: this thread owns file until it sends it or exits.
            let result = if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                Ok(file)
            } else {
                Err(io::Error::last_os_error())
            };
            let _ = send.send(result);
        })?;
    receive.recv_timeout(wait).map_err(|error| match error {
        std::sync::mpsc::RecvTimeoutError::Timeout => {
            io::Error::new(io::ErrorKind::TimedOut, "recorder log lock wait expired")
        }
        std::sync::mpsc::RecvTimeoutError::Disconnected => {
            io::Error::other("recorder log lock waiter ended")
        }
    })?
}

/// Publish only the newest whole-line tail of a legacy oversized file. The fixed staging path
/// bounds leftover scratch even if a recorder is killed during migration; normal rotation only
/// renames files. Readers with open descriptors retain their original generation.
fn trim_oversized(path: &Path, staging: &Path, limit: u64) -> io::Result<()> {
    let mut source = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    regular(&source)?;
    let length = source.metadata()?.len();
    if length <= limit {
        return Ok(());
    }
    let start = length - limit;
    source.seek(SeekFrom::Start(start - 1))?;
    let mut tail = Vec::new();
    source.take(limit + 1).read_to_end(&mut tail)?;
    let first = tail
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(tail.len(), |index| index + 1);
    let end = tail
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(first, |index| index + 1);
    let mut destination = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NONBLOCK)
        .open(staging)?;
    regular(&destination)?;
    destination.write_all(&tail[first..end])?;
    drop(destination);
    fs::rename(staging, path)
}

fn append_with(path: &Path, line: &[u8], limit: u64, wait: Duration) -> io::Result<()> {
    if line.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "recorder record exceeds a log generation",
        ));
    }
    let _guard = lock(path, wait)?;
    let archive = sibling(path, ".1");
    let staging = sibling(path, ".next");
    trim_oversized(&archive, &staging, limit)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    regular(&file)?;
    let length = file.metadata()?.len();
    if length.saturating_add(line.len() as u64) > limit {
        // Old versions could leave gigabytes here. Clip only that legacy tail, never scan the
        // whole file or carry its oversized inode into the retained archive.
        trim_oversized(path, &staging, limit)?;
        fs::rename(path, archive)?;
        file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?;
        regular(&file)?;
    }
    let original = file.metadata()?.len();
    if let Err(error) = file.write_all(line) {
        let _ = file.set_len(original);
        return Err(error);
    }
    Ok(())
}

pub(super) fn append(path: &Path, line: &[u8]) {
    if append_with(path, line, FILE_BYTES, LOCK_WAIT)
        .is_err_and(|error| error.kind() == io::ErrorKind::TimedOut)
    {
        count_drop(path);
    }
}

/// Open both generations under the writer lock, then release it before parsing any records.
/// The lengths bound each read and the descriptors survive rotation without missing or counting
/// a generation twice. Explicit --log inputs remain ordinary independent file reads.
pub(super) fn snapshot(path: &Path) -> io::Result<Vec<(PathBuf, File, u64)>> {
    let archive = sibling(path, ".1");
    if !path.try_exists()? && !archive.try_exists()? {
        return Ok(Vec::new());
    }
    let _guard = lock(path, LOCK_WAIT)?;
    let mut files = Vec::new();
    for path in [archive, path.to_owned()] {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        regular(&file)?;
        let length = file.metadata()?.len();
        files.push((path, file, length));
    }
    Ok(files)
}

/// An eight-byte native u64 counter. Use ordinary fallible I/O: mmap writes can SIGBUS on a
/// full filesystem, which would change the wrapped command's status. This independent inode
/// serializes counters even while the log lock is stuck, within the remaining contention budget.
fn count_drop(path: &Path) {
    let _ = count_drop_with(path, DROP_WAIT);
}

fn count_drop_with(path: &Path, wait: Duration) -> io::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NONBLOCK)
        .open(sibling(path, ".dropped"))?;
    let mut file = acquire(file, wait)?;
    let mut bytes = [0; 8];
    let count = match file.read_exact(&mut bytes) {
        Ok(()) => u64::from_ne_bytes(bytes),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => 0,
        Err(error) => return Err(error),
    };
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&count.saturating_add(1).to_ne_bytes())?;
    file.set_len(8)
}

pub(super) fn dropped(path: &Path) -> u64 {
    let mut bytes = [0; 8];
    match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(sibling(path, ".dropped"))
        .and_then(|mut file| {
            regular(&file)?;
            file.read_exact(&mut bytes)
        }) {
        Ok(()) => u64::from_ne_bytes(bytes),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn rows(path: &Path) -> Vec<u64> {
        [sibling(path, ".1"), path.to_owned()]
            .into_iter()
            .flat_map(|path| {
                fs::read_to_string(path)
                    .unwrap_or_default()
                    .lines()
                    .map(|line| {
                        serde_json::from_str::<serde_json::Value>(line).unwrap()["id"]
                            .as_u64()
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn a_long_run_keeps_only_two_bounded_generations_and_the_latest_record() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("commands.jsonl");
        for id in 0..512 {
            append_with(
                &path,
                format!("{{\"id\":{id}}}\n").as_bytes(),
                128,
                LOCK_WAIT,
            )
            .unwrap();
            for path in [&path, &sibling(&path, ".1")] {
                if let Ok(metadata) = fs::metadata(path) {
                    assert!(metadata.len() <= 128);
                    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
                }
            }
        }
        let retained = rows(&path);
        assert_eq!(retained.last(), Some(&511));
        assert!(!retained.contains(&0));
        assert!(retained.windows(2).all(|pair| pair[0] + 1 == pair[1]));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }

    #[test]
    fn legacy_oversized_files_keep_only_complete_tail_lines() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("commands.jsonl");
        for path in [&path, &sibling(&path, ".1")] {
            fs::write(
                path,
                (0..100)
                    .map(|id| format!("{{\"id\":{id}}}\n"))
                    .collect::<String>()
                    + "partial",
            )
            .unwrap();
        }
        append_with(&path, b"{\"id\":100}\n", 128, LOCK_WAIT).unwrap();
        let retained = rows(&path);
        assert!(retained.len() < 20);
        assert_eq!(retained.last(), Some(&100));
        assert!(retained.windows(2).all(|pair| pair[0] + 1 == pair[1]));
        assert!(fs::metadata(&path).unwrap().len() <= 128);
        assert!(fs::metadata(sibling(&path, ".1")).unwrap().len() <= 128);
        assert!(!sibling(&path, ".next").exists());
        assert!(!sibling(&sibling(&path, ".1"), ".next").exists());
    }

    #[test]
    fn concurrent_rotation_keeps_whole_unique_records_under_the_bound() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("commands.jsonl");
        let barrier = std::sync::Barrier::new(16);
        std::thread::scope(|scope| {
            for writer in 0..16 {
                let (path, barrier) = (&path, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    for row in 0..40 {
                        let id = writer * 40 + row;
                        append_with(
                            path,
                            format!("{{\"id\":{id}}}\n").as_bytes(),
                            256,
                            LOCK_WAIT,
                        )
                        .unwrap();
                    }
                });
            }
        });
        let retained = rows(&path);
        let unique = retained
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), retained.len());
        assert!(!retained.is_empty());
        assert!(fs::metadata(&path).unwrap().len() <= 256);
        assert!(fs::metadata(sibling(&path, ".1")).unwrap().len() <= 256);
    }

    #[test]
    fn a_snapshot_survives_later_rotations_without_duplication_or_missing_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("commands.jsonl");
        for id in 0..40 {
            append_with(
                &path,
                format!("{{\"id\":{id}}}\n").as_bytes(),
                128,
                LOCK_WAIT,
            )
            .unwrap();
        }
        let expected = rows(&path);
        let snapshot = snapshot(&path).unwrap();
        for id in 40..100 {
            append_with(
                &path,
                format!("{{\"id\":{id}}}\n").as_bytes(),
                128,
                LOCK_WAIT,
            )
            .unwrap();
        }
        let mut actual = Vec::new();
        for (_, file, length) in snapshot {
            let mut bytes = String::new();
            file.take(length).read_to_string(&mut bytes).unwrap();
            actual.extend(bytes.lines().map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["id"]
                    .as_u64()
                    .unwrap()
            }));
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn a_stuck_holder_times_out_and_the_abandoned_waiter_releases_its_lock() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("commands.jsonl");
        let guard = lock(&path, LOCK_WAIT).unwrap();
        let result = append_with(&path, b"{\"id\":1}\n", 128, Duration::from_millis(20));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(!path.exists());
        drop(guard);
        append_with(&path, b"{\"id\":2}\n", 128, LOCK_WAIT).unwrap();
        assert_eq!(rows(&path), [2]);
    }

    #[test]
    fn a_stuck_counter_cannot_hold_up_a_timed_out_recorder() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("commands.jsonl");
        count_drop_with(&path, LOCK_WAIT).unwrap();
        let counter = OpenOptions::new()
            .read(true)
            .write(true)
            .open(sibling(&path, ".dropped"))
            .unwrap();
        let guard = acquire(counter, LOCK_WAIT).unwrap();
        let error = count_drop_with(&path, Duration::from_millis(20)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(guard);
        count_drop_with(&path, LOCK_WAIT).unwrap();
        assert_eq!(dropped(&path), 2);
    }

    #[test]
    fn concurrent_drop_counts_stay_in_one_fixed_size_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("commands.jsonl");
        let barrier = std::sync::Barrier::new(16);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                let (path, barrier) = (&path, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..8 {
                        count_drop_with(path, DROP_WAIT).unwrap();
                    }
                });
            }
        });
        assert_eq!(dropped(&path), 128);
        assert_eq!(fs::metadata(sibling(&path, ".dropped")).unwrap().len(), 8);
    }
}
