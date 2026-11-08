//! Where the person last read each conversation on this device, as the web and the phone keep it:
//! the newest prose (st's `last_prose_at`) a presented conversation pane showed. A conversation
//! whose roster prose is newer than that is unread, and its tab and row say so. One file beside the
//! drafts, never synced through st. Positions only move forward, and a write keeps the newer of
//! this TUI's and the file's, so two TUIs on one device never move each other back.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The roster stamps words when st accepts them, a moment after the harness wrote them: prose this
/// close after the position is what the person already saw. The web's `rosterSkewMs`.
pub const ROSTER_SKEW_MS: i64 = 2_000;

/// How many conversations keep a position; the least recently read go first. The web keeps as many.
const KEEP: usize = 500;

type File = BTreeMap<String, i64>;

/// `read.json` beside the drafts: the cache, for the same reason (a device that only pairs keeps no
/// st state).
pub fn path() -> Option<PathBuf> {
    super::drafts_store::path().map(|drafts| drafts.with_file_name("read.json"))
}

/// The time, in ms since the epoch, of an RFC 3339 timestamp st sent.
pub fn at_ms(at: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|at| at.timestamp_millis())
}

#[derive(Debug, Default)]
pub struct ReadPositions {
    path: Option<PathBuf>,
    positions: File,
}

impl ReadPositions {
    /// The positions a previous Fractal TUI on this device left; none without a file.
    pub fn load(path: Option<PathBuf>) -> Self {
        let positions = path.as_deref().map(read).unwrap_or_default();
        Self { path, positions }
    }

    /// Whether `conversation` has prose newer than where the person last read it here. A
    /// conversation never read on this device has nothing new, and without st's `last_prose_at`
    /// (an older st) nothing is known to be new.
    pub fn unread(&self, conversation: &str, last_prose_ms: Option<i64>) -> bool {
        matches!(
            (self.positions.get(conversation), last_prose_ms),
            (Some(seen), Some(prose)) if prose > seen.saturating_add(ROSTER_SKEW_MS)
        )
    }

    /// Move the position to `at_ms`, never back; whether it moved. Written at once.
    pub fn advance(&mut self, conversation: &str, at_ms: i64) -> bool {
        if self.positions.get(conversation).is_some_and(|seen| *seen >= at_ms) {
            return false;
        }
        self.positions.insert(conversation.to_owned(), at_ms);
        if let Some(path) = &self.path {
            // A position that fails to write is still kept for this run.
            let _ = save(path, conversation, at_ms);
        }
        true
    }
}

fn read(path: &Path) -> File {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(path: &Path, conversation: &str, at_ms: i64) -> std::io::Result<()> {
    let mut file = read(path);
    let seen = file.entry(conversation.to_owned()).or_insert(at_ms);
    *seen = (*seen).max(at_ms);
    if file.len() > KEEP {
        let mut oldest: Vec<_> = file.iter().map(|(key, at)| (*at, key.clone())).collect();
        oldest.sort_unstable();
        for (_, key) in oldest.into_iter().take(file.len() - KEEP) {
            file.remove(&key);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension(format!("json.{}.partial", std::process::id()));
    std::fs::write(&partial, serde_json::to_vec(&file)?)?;
    std::fs::rename(partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_never_read_here_has_nothing_new() {
        let positions = ReadPositions::default();
        assert!(!positions.unread("agent/a", Some(10_000)));
    }

    #[test]
    fn prose_newer_than_the_position_is_unread_past_the_roster_skew() {
        let mut positions = ReadPositions::default();
        positions.advance("agent/a", 10_000);
        assert!(!positions.unread("agent/a", Some(10_000)), "what was read");
        assert!(!positions.unread("agent/a", Some(10_000 + ROSTER_SKEW_MS)), "the same words, stamped later");
        assert!(positions.unread("agent/a", Some(10_001 + ROSTER_SKEW_MS)), "newer words");
        assert!(!positions.unread("agent/a", None), "an older st says nothing about prose");
    }

    #[test]
    fn positions_only_move_forward_and_survive_a_restart_merged_with_other_tuis() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stui").join("read.json");
        let mut one = ReadPositions::load(Some(path.clone()));
        let mut two = ReadPositions::load(Some(path.clone()));
        assert!(one.advance("agent/a", 20_000));
        assert!(!one.advance("agent/a", 10_000), "never back");
        assert!(two.advance("agent/a", 15_000), "two had not seen one's position");
        assert!(two.advance("agent/b", 5_000));
        let again = ReadPositions::load(Some(path));
        assert!(!again.unread("agent/a", Some(20_000)), "the newer position stays: {:?}", again.positions);
        assert!(again.unread("agent/a", Some(30_000)));
        assert!(again.unread("agent/b", Some(10_000)));
    }

    #[test]
    fn rfc3339_times_compare_as_ms() {
        assert_eq!(at_ms("1970-01-01T00:00:01.500Z"), Some(1_500));
        assert_eq!(at_ms("not a time"), None);
    }
}
