//! Bounded native child diagnostics and one shared detached-child reaper.

use std::io::{Read as _, Seek as _};
use std::process::Child;

/// Per-stream cap for captured child diagnostics. Tail-preserving: when output exceeds the cap,
/// the LAST [`CAPTURE_CAP_BYTES`] bytes are kept — recent output is what a failure message needs,
/// and an uncapped capture lets one chatty child balloon sidecar memory without bound.
pub(crate) const CAPTURE_CAP_BYTES: usize = 256 * 1024;

/// One captured child stream capped to [`CAPTURE_CAP_BYTES`], keeping the tail.
pub(crate) struct BoundedStream {
    pub bytes: Vec<u8>, // last <= cap bytes
    pub total: u64,     // complete stream size before capping
}

impl BoundedStream {
    pub fn truncated(&self) -> bool {
        self.total as usize > self.bytes.len()
    }
}

/// Read back at most `cap` bytes of a temp-file capture, preserving the tail. The file is stat'ed
/// and seek'ed straight to `len - cap`, so the cost is O(cap) no matter how much the child wrote.
pub(crate) fn read_bounded_tail(
    file: &mut std::fs::File,
    cap: usize,
) -> std::io::Result<BoundedStream> {
    let total = file.metadata()?.len();
    let skip = total.saturating_sub(cap as u64);
    file.seek(std::io::SeekFrom::Start(skip))?;
    let mut bytes = Vec::with_capacity((total - skip) as usize);
    file.take(cap as u64).read_to_end(&mut bytes)?;
    Ok(BoundedStream { bytes, total })
}

/// Send an already-killed child to ONE shared reaper thread instead of spawning a detached thread
/// per timed-out child: under a timeout storm one-thread-per-child accumulates without bound.
/// The thread starts lazily on first use.
pub(crate) fn reap_detached(child: std::process::Child) {
    static REAPER: std::sync::LazyLock<std::sync::mpsc::Sender<Child>> =
        std::sync::LazyLock::new(|| {
            let (sender, receiver) = std::sync::mpsc::channel::<Child>();
            // Thread-spawn exhaustion is the only failure mode; panicking here surfaces it at the
            // call site instead of silently leaking unreaped children.
            std::thread::Builder::new()
                .name("st-child-reaper".to_string())
                .spawn(move || {
                    for mut child in receiver {
                        let _ = child.wait();
                    }
                })
                .expect("spawn shared child reaper thread");
            sender
        });
    let _ = REAPER.send(child);
}
