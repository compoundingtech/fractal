//! Resume a reconfigured stream after its last completely delivered callback.
//!
//! FSEvents IDs belong to the shared journal, not the root list. Keeping this cursor
//! across watch/unwatch is what preserves events in roots untouched by reconfiguration.
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct Cursor(AtomicU64);
impl Cursor {
    pub(crate) fn new(current: u64) -> Self { Self(AtomicU64::new(current)) }
    pub(crate) fn since(&self) -> u64 { self.0.load(Ordering::Acquire) }
    pub(crate) fn delivered(&self, id: u64, wrapped: bool) {
        if wrapped { self.0.store(id, Ordering::Release); }
        else { self.0.fetch_max(id, Ordering::AcqRel); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_stream_restart_replays_unrelated_root_events_from_the_gap() {
        let cursor = Cursor::new(100);
        cursor.delivered(103, false);
        // Stop the shared stream to change /a. /b changes before its replacement starts.
        let journal = [(104, "/b/input"), (105, "/a/input")];
        let since = cursor.since();
        let mut deliveries = [0, 0];
        for (id, path) in journal.into_iter().filter(|(id, _)| *id > since) {
            deliveries[usize::from(path.starts_with("/a/"))] += 1;
            cursor.delivered(id, false);
        }
        assert_eq!(since, 103, "reconfiguration must not use SinceNow");
        assert_eq!(deliveries, [1, 1], "the unchanged root's gap event is delivered");
        assert_eq!(cursor.since(), 105);
        cursor.delivered(104, false);
        assert_eq!(cursor.since(), 105, "an older event cannot regress the journal cursor");
    }
    #[test]
    fn a_wrapped_journal_restarts_in_the_new_id_space() {
        let cursor = Cursor::new(u64::MAX - 1);
        cursor.delivered(2, true);
        assert_eq!(cursor.since(), 2);
        cursor.delivered(3, false);
        assert_eq!(cursor.since(), 3);
    }
}
