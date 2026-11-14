//! Where the person last read each conversation: the newest prose (st's `last_prose_at`) a
//! presented conversation pane showed. A conversation whose roster prose is newer than that is
//! unread, and its tab and row say so.
//!
//! st keeps the positions, one per person and conversation, merged from every device by the
//! larger time. This TUI lists them on connecting and on each invalidation from their dedicated
//! stream, and merges them in by the larger. Its own reads move a position here at once and are
//! then sent as `read-position.advance`, at most one in flight and one every [`COALESCE`] per
//! conversation, retried after a growing wait.
//!
//! `read-<scope>.json` beside the drafts is only an offline cache: the merged positions and
//! advances st has not confirmed, scoped to the same gateway/device grant and person as the
//! display snapshot. A larger position from st always wins. Against an st without read positions
//! (an older daemon) the positions stay on this device, as the file alone.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The roster stamps words when st accepts them, a moment after the harness wrote them: prose this
/// close after the position is what the person already saw. The web's `rosterSkewMs`.
pub const ROSTER_SKEW_MS: i64 = 2_000;

/// The capability a daemon grants when it keeps read positions.
pub const CAPABILITY: &str = "read-positions";

/// The least time between two sends for one conversation: reading a conversation as it streams
/// moves its position many times a second, and st needs only the latest.
pub const COALESCE: Duration = Duration::from_secs(2);

/// How long a failed send waits, doubling from [`COALESCE`] up to this.
const RETRY_CAP: Duration = Duration::from_secs(60);

/// How long a failed list waits before it is asked again.
const LIST_RETRY: Duration = Duration::from_secs(10);

/// How many conversations the file keeps; the least recently read go first. The web keeps as many.
const KEEP: usize = 500;

#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct File {
    /// The merged positions, st's and this device's.
    #[serde(default)]
    positions: BTreeMap<String, i64>,
    /// Advances st has not confirmed, by conversation: sent once st keeps positions.
    #[serde(default)]
    unsent: BTreeMap<String, i64>,
}

/// An offline cache beside the drafts, keyed by the display snapshot's endpoint-and-person scope.
/// Never import the old unscoped `read.json`: its unsent advances have no trustworthy owner.
pub fn path(snapshot: Option<&Path>) -> Option<PathBuf> {
    let scope = snapshot?.file_stem()?.to_str()?;
    super::drafts_store::path().map(|drafts| drafts.with_file_name(format!("read-{scope}.json")))
}

/// The time, in ms since the epoch, of an RFC 3339 timestamp st sent.
pub fn at_ms(at: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|at| at.timestamp_millis())
}

/// A time in ms since the epoch as st's RFC 3339 timestamp, with milliseconds.
pub fn timestamp(at_ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(at_ms)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// Whether the daemon on this connection keeps read positions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Server {
    /// Not asked yet on this connection, or offline: nothing is sent.
    #[default]
    Unknown,
    /// An older daemon: positions stay on this device.
    Absent,
    Present,
}

/// One conversation's sends: at most one in flight, the next not before `next`.
#[derive(Debug, Default)]
struct Sending {
    in_flight: bool,
    next: Option<Instant>,
    failures: u32,
}

/// When st's list is read: one read at a time, again at once when a frame came during it.
#[derive(Debug, Default)]
struct Listing {
    reading: bool,
    again: bool,
    due: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct ReadPositions {
    path: Option<PathBuf>,
    positions: BTreeMap<String, i64>,
    /// The largest position st said it keeps, per conversation.
    confirmed: BTreeMap<String, i64>,
    /// Conversations whose position here is beyond what st confirmed.
    unsent: BTreeSet<String>,
    server: Server,
    sending: BTreeMap<String, Sending>,
    listing: Listing,
}

impl ReadPositions {
    /// The positions and unsent advances a previous Fractal TUI on this device left; none without a
    /// file. Until st answers, nothing here is known to be confirmed but what the file says was.
    pub fn load(path: Option<PathBuf>) -> Self {
        let file = path.as_deref().map(read).unwrap_or_default();
        let mut positions = file.positions;
        for (conversation, at) in &file.unsent {
            let seen = positions.entry(conversation.clone()).or_insert(*at);
            *seen = (*seen).max(*at);
        }
        let unsent: BTreeSet<String> = file.unsent.into_keys().collect();
        let confirmed = positions
            .iter()
            .filter(|(conversation, _)| !unsent.contains(*conversation))
            .map(|(conversation, at)| (conversation.clone(), *at))
            .collect();
        Self { path, positions, confirmed, unsent, ..Self::default() }
    }

    /// Whether `conversation` has prose newer than where the person last read it. A conversation
    /// never read anywhere has nothing new, and without st's `last_prose_at` (an older st) nothing
    /// is known to be new.
    pub fn unread(&self, conversation: &str, last_prose_ms: Option<i64>) -> bool {
        matches!(
            (self.positions.get(conversation), last_prose_ms),
            (Some(seen), Some(prose)) if prose > seen.saturating_add(ROSTER_SKEW_MS)
        )
    }

    /// Move the position to `at_ms`, never back; whether it moved. Kept in the file at once and
    /// sent to st by [`Self::due`].
    pub fn advance(&mut self, conversation: &str, at_ms: i64) -> bool {
        if self.positions.get(conversation).is_some_and(|seen| *seen >= at_ms) {
            return false;
        }
        self.positions.insert(conversation.to_owned(), at_ms);
        if self.confirmed.get(conversation).is_none_or(|kept| *kept < at_ms) {
            self.unsent.insert(conversation.to_owned());
        }
        self.save();
        true
    }

    /// A new connection: whether its daemon keeps positions is asked again, and its list read now.
    /// An answer from the connection before is not waited for.
    pub fn connected(&mut self, now: Instant) {
        self.server = Server::Unknown;
        self.listing = Listing { due: Some(now), ..Listing::default() };
        for sending in self.sending.values_mut() {
            sending.in_flight = false;
            sending.next = None;
        }
    }

    /// The stream of st's positions said something changed: read the list again.
    pub fn stream_frame(&mut self, now: Instant) {
        if self.listing.reading {
            self.listing.again = true;
        } else {
            self.listing.due = Some(now);
        }
    }


    /// Whether st's list should be read now; then whether the daemon's capability must be asked
    /// first. Never against a daemon without read positions.
    pub fn list_due(&mut self, now: Instant) -> Option<bool> {
        if self.server == Server::Absent || self.listing.reading || self.listing.due.is_none_or(|due| due > now) {
            return None;
        }
        self.listing.reading = true;
        self.listing.due = None;
        self.listing.again = false;
        Some(self.server == Server::Unknown)
    }

    /// st answered a list: its positions (`None` from a daemon without them), or why it could not.
    /// Whether any position moved.
    pub fn listed(&mut self, outcome: Result<Option<Vec<(String, i64)>>, fractal_client::ClientError>, now: Instant) -> bool {
        self.listing.reading = false;
        match outcome {
            Ok(None) => {
                self.server = Server::Absent;
                self.listing.due = None;
                false
            }
            Ok(Some(positions)) => {
                self.server = Server::Present;
                if self.listing.again {
                    self.listing.again = false;
                    self.listing.due = Some(now);
                }
                self.merge(positions)
            }
            Err(error) => {
                if matches!(error, fractal_client::ClientError::Transport(_) | fractal_client::ClientError::Unreachable(_)
                    | fractal_client::ClientError::Api(fractal_client::ErrorCode::RemoteUnavailable, _, _)) {
                    self.listing.due = Some(now + LIST_RETRY);
                }
                false
            }
        }
    }

    /// Merge positions st keeps, by the larger: a larger one from st always wins here. Whether any
    /// position moved.
    pub fn merge(&mut self, positions: impl IntoIterator<Item = (String, i64)>) -> bool {
        let mut moved = false;
        let mut kept = false;
        for (conversation, at) in positions {
            let confirmed = self.confirmed.entry(conversation.clone()).or_insert(at);
            *confirmed = (*confirmed).max(at);
            let confirmed = *confirmed;
            let seen = self.positions.entry(conversation.clone()).or_insert(i64::MIN);
            if *seen < at {
                *seen = at;
                moved = true;
            }
            if *seen <= confirmed {
                kept |= self.unsent.remove(&conversation);
            }
        }
        if moved || kept {
            self.save();
        }
        moved
    }

    /// The advances to send now, each marked in flight: only to a daemon that keeps positions, on a
    /// live connection, one in flight and one every [`COALESCE`] per conversation.
    pub fn due(&mut self, now: Instant, live: bool) -> Vec<(String, i64)> {
        if !live || self.server != Server::Present {
            return Vec::new();
        }
        let mut due = Vec::new();
        for conversation in &self.unsent {
            let sending = self.sending.entry(conversation.clone()).or_default();
            if sending.in_flight || sending.next.is_some_and(|next| next > now) {
                continue;
            }
            let Some(at) = self.positions.get(conversation) else { continue };
            sending.in_flight = true;
            sending.next = Some(now + COALESCE);
            due.push((conversation.clone(), *at));
        }
        due
    }

    /// st answered the send of `at_ms` for `conversation`: the position it keeps now (`None` when
    /// it did not say, which means at least `at_ms`), or why it failed, to be sent again after a
    /// growing wait. The next send still waits out its [`COALESCE`]. Whether the position here
    /// moved.
    pub fn sent(&mut self, conversation: &str, at_ms: i64, outcome: Result<Option<i64>, String>, now: Instant) -> bool {
        let sending = self.sending.entry(conversation.to_owned()).or_default();
        sending.in_flight = false;
        match outcome {
            Ok(kept) => {
                sending.failures = 0;
                self.merge([(conversation.to_owned(), kept.unwrap_or(at_ms).max(at_ms))])
            }
            Err(_) => {
                sending.failures = sending.failures.saturating_add(1);
                let wait = COALESCE.saturating_mul(1 << sending.failures.min(6).saturating_sub(1)).min(RETRY_CAP);
                sending.next = Some(now + wait);
                false
            }
        }
    }

    fn save(&self) {
        if let Some(path) = &self.path {
            // A position that fails to write is still kept for this run.
            let _ = save(path, &self.positions, &self.confirmed, &self.unsent);
        }
    }
}

fn read(path: &Path) -> File {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Write the positions merged with what other TUIs on this device wrote, by the larger. Another
/// TUI's unsent advance stays unless this one saw st keep it.
fn save(
    path: &Path,
    positions: &BTreeMap<String, i64>,
    confirmed: &BTreeMap<String, i64>,
    unsent: &BTreeSet<String>,
) -> std::io::Result<()> {
    let mut file = read(path);
    for (conversation, at) in positions {
        let seen = file.positions.entry(conversation.clone()).or_insert(*at);
        *seen = (*seen).max(*at);
    }
    file.unsent.retain(|conversation, at| confirmed.get(conversation).is_none_or(|kept| *kept < *at));
    for conversation in unsent {
        if let Some(at) = positions.get(conversation) {
            let seen = file.unsent.entry(conversation.clone()).or_insert(*at);
            *seen = (*seen).max(*at);
        }
    }
    for (conversation, at) in &file.unsent {
        let seen = file.positions.entry(conversation.clone()).or_insert(*at);
        *seen = (*seen).max(*at);
    }
    // The least recently read go first; an unsent advance only once every sent one has gone.
    if file.positions.len() > KEEP {
        let mut oldest: Vec<_> = file
            .positions
            .iter()
            .map(|(key, at)| (file.unsent.contains_key(key), *at, key.clone()))
            .collect();
        oldest.sort_unstable();
        for (_, _, key) in oldest.into_iter().take(file.positions.len() - KEEP) {
            file.positions.remove(&key);
            file.unsent.remove(&key);
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

    /// A store whose daemon keeps positions, as after its first list.
    fn present(path: Option<PathBuf>, now: Instant) -> ReadPositions {
        let mut positions = ReadPositions::load(path);
        positions.connected(now);
        assert_eq!(positions.list_due(now), Some(true), "the first list asks the capability");
        positions.listed(Ok(Some(Vec::new())), now);
        positions
    }

    #[test]
    fn cache_paths_separate_people_and_gateways_without_importing_unscoped_advances() {
        let dir = tempfile::tempdir().unwrap();
        let scoped = |gateway: &str, person: &str| {
            let snapshot = crate::cache::path(Path::new(gateway), person).unwrap();
            let scoped = path(Some(&snapshot)).unwrap();
            dir.path().join(scoped.file_name().unwrap())
        };
        let ada = scoped("/gateway/one", "person/ada");
        let robin = scoped("/gateway/one", "person/robin");
        let other = scoped("/gateway/two", "person/ada");
        assert_ne!(ada, robin);
        assert_ne!(ada, other);
        assert_eq!(path(None), None);
        let mut unscoped = ReadPositions::load(Some(dir.path().join("read.json")));
        unscoped.advance("agent/legacy", 20_000);
        let now = Instant::now();
        let mut first = present(Some(ada.clone()), now);
        assert!(first.due(now, true).is_empty(), "an unowned advance is never imported");
        first.advance("agent/a", 10_000);
        let mut same = present(Some(ada), now);
        assert_eq!(same.due(now, true), [("agent/a".to_owned(), 10_000)]);
        for other_scope in [robin, other] {
            let mut isolated = present(Some(other_scope), now);
            assert!(isolated.due(now, true).is_empty(), "another scope cannot replay this person's advances");
        }
    }

    #[test]
    fn a_conversation_never_read_has_nothing_new() {
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
    fn st_and_this_device_merge_by_the_larger() {
        let now = Instant::now();
        let mut positions = present(None, now);
        positions.advance("agent/a", 20_000);
        positions.advance("agent/b", 20_000);
        let moved = positions.merge([("agent/a".to_owned(), 10_000), ("agent/b".to_owned(), 30_000), ("agent/c".to_owned(), 5_000)]);
        assert!(moved);
        assert_eq!(positions.positions.get("agent/a"), Some(&20_000), "an older position from st does not move it back");
        assert_eq!(positions.positions.get("agent/b"), Some(&30_000), "a larger one from st wins");
        assert_eq!(positions.positions.get("agent/c"), Some(&5_000), "one read on another device is known here");
        assert!(!positions.merge([("agent/b".to_owned(), 25_000)]), "an older list moves nothing");
        assert_eq!(positions.unsent, BTreeSet::from(["agent/a".to_owned()]), "only what st has not got is sent");
    }

    #[test]
    fn a_larger_position_from_st_wins_over_the_file_and_clears_its_unsent_advance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("read.json");
        let now = Instant::now();
        let mut offline = ReadPositions::load(Some(path.clone()));
        offline.advance("agent/a", 10_000);
        let mut positions = ReadPositions::load(Some(path.clone()));
        assert!(positions.unread("agent/a", Some(20_000)), "the cache shows before st answers");
        positions.connected(now);
        positions.list_due(now);
        assert!(positions.listed(Ok(Some(vec![("agent/a".to_owned(), 40_000)])), now));
        assert!(!positions.unread("agent/a", Some(20_000)), "read further on another device");
        assert!(positions.due(now, true).is_empty(), "nothing st does not already have");
        let file = read(&path);
        assert_eq!(file.positions.get("agent/a"), Some(&40_000));
        assert!(file.unsent.is_empty(), "{file:?}");
    }

    #[test]
    fn advances_are_coalesced_one_in_flight_per_conversation_and_retried_after_a_wait() {
        let now = Instant::now();
        let mut positions = present(None, now);
        positions.advance("agent/a", 10_000);
        assert!(positions.due(now, false).is_empty(), "nothing goes offline");
        assert_eq!(positions.due(now, true), [("agent/a".to_owned(), 10_000)]);
        positions.advance("agent/a", 11_000);
        positions.advance("agent/a", 12_000);
        assert!(positions.due(now + COALESCE * 5, true).is_empty(), "one in flight");
        positions.sent("agent/a", 10_000, Ok(Some(10_000)), now + Duration::from_millis(100));
        assert!(positions.due(now + Duration::from_millis(500), true).is_empty(), "one every COALESCE");
        assert_eq!(positions.due(now + COALESCE, true), [("agent/a".to_owned(), 12_000)], "the latest only");
        let failed = now + COALESCE + Duration::from_millis(100);
        positions.sent("agent/a", 12_000, Err("unreachable".into()), failed);
        assert!(positions.due(failed + COALESCE - Duration::from_millis(1), true).is_empty(), "waits before trying again");
        assert_eq!(positions.due(failed + COALESCE, true), [("agent/a".to_owned(), 12_000)]);
        positions.sent("agent/a", 12_000, Err("unreachable".into()), failed + COALESCE);
        assert!(positions.due(failed + COALESCE * 2, true).is_empty(), "the wait grows");
        assert_eq!(positions.due(failed + COALESCE * 3, true).len(), 1);
        positions.sent("agent/a", 12_000, Ok(Some(12_000)), failed + COALESCE * 3);
        assert!(positions.unsent.is_empty());
        assert!(positions.due(failed + COALESCE * 10, true).is_empty(), "confirmed, nothing left");
    }

    #[test]
    fn st_answering_a_larger_position_than_sent_moves_it_here() {
        let now = Instant::now();
        let mut positions = present(None, now);
        positions.advance("agent/a", 10_000);
        positions.due(now, true);
        assert!(positions.sent("agent/a", 10_000, Ok(Some(50_000)), now));
        assert!(!positions.unread("agent/a", Some(50_000)));
        assert!(positions.unsent.is_empty());
    }

    #[test]
    fn unsent_advances_survive_a_restart_and_are_sent_once_st_keeps_positions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("read.json");
        let now = Instant::now();
        let mut before = present(Some(path.clone()), now);
        before.advance("agent/a", 10_000);
        before.advance("agent/b", 20_000);
        before.due(now, true);
        before.sent("agent/b", 20_000, Ok(None), now);
        drop(before);
        let mut after = ReadPositions::load(Some(path.clone()));
        assert!(after.due(now, true).is_empty(), "not before st says it keeps positions");
        after.connected(now);
        assert_eq!(after.list_due(now), Some(true));
        after.listed(Ok(Some(vec![("agent/b".to_owned(), 20_000)])), now);
        assert_eq!(after.due(now, true), [("agent/a".to_owned(), 10_000)], "only the advance st never confirmed");
        after.sent("agent/a", 10_000, Ok(None), now);
        assert!(read(&path).unsent.is_empty());
    }

    #[test]
    fn without_the_capability_positions_stay_on_this_device() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("read.json");
        let now = Instant::now();
        let mut positions = ReadPositions::load(Some(path.clone()));
        positions.connected(now);
        assert_eq!(positions.list_due(now), Some(true));
        assert!(!positions.listed(Ok(None), now));
        assert!(positions.advance("agent/a", 10_000));
        assert!(positions.unread("agent/a", Some(20_000)));
        assert!(!positions.unread("agent/a", Some(10_000)));
        assert!(positions.due(now + RETRY_CAP, true).is_empty(), "nothing is sent to an older daemon");
        assert_eq!(positions.list_due(now + RETRY_CAP * 10), None, "nor its list asked again");
        assert_eq!(read(&path).unsent.get("agent/a"), Some(&10_000), "kept for a daemon that keeps positions");
        positions.stream_frame(now);
        assert_eq!(positions.list_due(now), None, "a frame does not list an older daemon either");
    }

    #[test]
    fn the_list_is_read_once_at_a_time_only_after_invalidations_or_network_failure() {
        let now = Instant::now();
        let mut positions = ReadPositions::default();
        positions.connected(now);
        assert_eq!(positions.list_due(now), Some(true));
        positions.stream_frame(now);
        assert_eq!(positions.list_due(now), None, "one read at a time");
        positions.listed(Ok(Some(Vec::new())), now);
        assert_eq!(positions.list_due(now), Some(false), "a frame during the read reads again");
        positions.listed(Ok(Some(Vec::new())), now);
        assert_eq!(positions.list_due(now + Duration::from_secs(3600)), None, "no periodic listing");
        positions.stream_frame(now);
        assert_eq!(positions.list_due(now), Some(false));
        positions.listed(Err(fractal_client::ClientError::Transport("unreachable".into())), now);
        assert_eq!(positions.list_due(now + LIST_RETRY), Some(false), "a failed network read is tried again");
    }

    #[test]
    fn a_refused_list_does_not_start_a_retry_loop() {
        let now = Instant::now();
        let mut positions = present(None, now);
        positions.stream_frame(now);
        assert_eq!(positions.list_due(now), Some(false));
        positions.listed(Err(fractal_client::ClientError::Protocol("invalid response".into())), now);
        assert_eq!(positions.list_due(now + Duration::from_secs(3600)), None);
    }

    #[test]
    fn a_new_connection_asks_again_and_drops_what_was_in_flight() {
        let now = Instant::now();
        let mut positions = present(None, now);
        positions.advance("agent/a", 10_000);
        assert_eq!(positions.due(now, true).len(), 1);
        positions.connected(now);
        assert!(positions.due(now, true).is_empty(), "the daemon is asked again first");
        assert_eq!(positions.list_due(now), Some(true));
        positions.listed(Ok(Some(Vec::new())), now);
        assert_eq!(positions.due(now, true), [("agent/a".to_owned(), 10_000)], "sent again on the new connection");
    }

    #[test]
    fn rfc3339_times_compare_as_ms() {
        assert_eq!(at_ms("1970-01-01T00:00:01.500Z"), Some(1_500));
        assert_eq!(at_ms("not a time"), None);
        assert_eq!(timestamp(1_500).as_deref(), Some("1970-01-01T00:00:01.500Z"));
        assert_eq!(timestamp(1_500).as_deref().and_then(at_ms), Some(1_500));
    }
}
