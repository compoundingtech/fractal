//! Unwired collection paging contract. Snapshot indexes are provenance, never collection clocks.
//! Subscribe before reading; hosts execute effects and persist only `complete`, never staged rows.
use std::collections::{BTreeMap, BTreeSet};
use crate::model::MembershipInput;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Scope { pub owner: String, pub container: String, pub endpoint: String, pub host: String, pub projection: String }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token { pub scope: Scope, pub generation: u64, pub attempt: u64 }
/// Epoch changes (including maintenance) are incomparable, not numerically ordered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frontier { pub epoch: String, pub changed_index: u64, pub live_count: u64 }
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Snapshot { pub id: String, pub host_id: String, pub store_index: u64, pub projection_version: String, pub created_at: String, pub published_at: Option<String> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Complete { pub scope: Scope, pub frontier: Frontier, pub snapshots: Vec<Snapshot>, pub members: Vec<MembershipInput> }
#[derive(Debug)]
pub struct Page { pub frontier: Frontier, pub snapshot: Snapshot, pub members: Vec<MembershipInput>, pub has_more: bool, pub next: Option<String> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status { Unloaded, Paging, Ready, Disconnected, Refused, Unsupported, InvalidCursor, FrontierInconsistent, Exhausted }
#[derive(Debug, PartialEq, Eq)]
pub enum Effect { SubscribeAndRead(Token), Read(Token, String), PersistBase }
pub enum Event { Refresh(Scope, u64), Page(Token, Page), Metadata(Token, Frontier), Expired(Token), Refused(Token), Unsupported(Token), InvalidCursor(Token), FrontierInconsistent(Token), Disconnect }
#[derive(Default)]
struct Staged { frontier: Option<Frontier>, rows: BTreeMap<String, MembershipInput>, cursors: BTreeSet<String>, snapshots: Vec<Snapshot> }
pub struct Paging {
    pub status: Status,
    complete: Option<Complete>,
    token: Option<Token>,
    serial: u64,
    attempts: u8,
    newest: Option<Frontier>,
    staged: Staged,
}
impl Paging {
    /// Restored data is stale until a successful refresh; refresh filters it by full scope.
    pub fn new(restored: Option<Complete>) -> Self {
        Self { status: Status::Unloaded, complete: restored, token: None, serial: 0, attempts: 0, newest: None, staged: Staged::default() }
    }
    pub fn complete(&self) -> Option<&Complete> { self.complete.as_ref() }
    pub fn stale(&self) -> bool { self.status != Status::Ready }
    fn stop(&mut self, status: Status) -> Vec<Effect> {
        self.status = status; self.token = None; self.staged = Staged::default(); Vec::new()
    }
    fn start(&mut self, mut token: Token) -> Vec<Effect> {
        self.staged = Staged::default();
        if self.attempts == 3 { return self.stop(Status::Exhausted); }
        let Some(serial) = self.serial.checked_add(1) else { return self.stop(Status::Exhausted); };
        self.serial = serial; self.attempts += 1; token.attempt = serial;
        self.token = Some(token.clone()); self.status = Status::Paging;
        vec![Effect::SubscribeAndRead(token)]
    }
    pub fn reduce(&mut self, event: Event) -> Vec<Effect> {
        let (token, event) = match event {
            Event::Refresh(scope, generation) => {
                if self.complete.as_ref().is_some_and(|base| base.scope != scope) { self.complete = None; }
                self.attempts = 0; self.newest = None;
                return self.start(Token { scope, generation, attempt: 0 });
            }
            Event::Disconnect => { self.newest = None; return self.stop(Status::Disconnected); }
            Event::Page(token, page) => (token, Ok(page)),
            Event::Metadata(token, frontier) => (token, Err(Some(frontier))),
            Event::Expired(token) => (token, Err(None)),
            Event::Refused(token) => {
                return if self.token.as_ref() == Some(&token) { self.stop(Status::Refused) } else { Vec::new() };
            }
            Event::Unsupported(token) => return if self.token.as_ref() == Some(&token) { self.stop(Status::Unsupported) } else { Vec::new() },
            Event::InvalidCursor(token) => return if self.token.as_ref() == Some(&token) { self.stop(Status::InvalidCursor) } else { Vec::new() },
            Event::FrontierInconsistent(token) => return if self.token.as_ref() == Some(&token) { self.stop(Status::FrontierInconsistent) } else { Vec::new() },
        };
        if self.token.as_ref() != Some(&token) { return Vec::new(); }
        let page = match event {
            Err(None) => return self.start(token),
            Err(Some(frontier)) => {
                let held = self.newest.as_ref().or(self.staged.frontier.as_ref()).or_else(|| self.complete.as_ref().map(|base| &base.frontier));
                if let Some(held) = held {
                    if held.epoch == frontier.epoch && held.changed_index == frontier.changed_index && held.live_count != frontier.live_count { return self.stop(Status::Refused); }
                    if held.epoch == frontier.epoch && frontier.changed_index < held.changed_index { return Vec::new(); }
                    if held == &frontier { self.newest = Some(frontier); return Vec::new(); }
                }
                let restart = self.status == Status::Ready || self.staged.frontier.as_ref().is_some_and(|held| held != &frontier);
                if self.status == Status::Ready { self.attempts = 0; }
                self.newest = Some(frontier);
                return if restart { self.start(token) } else { Vec::new() };
            }
            Ok(page) => page,
        };
        if self.status != Status::Paging { return Vec::new(); }
        if page.snapshot.host_id != token.scope.host || page.snapshot.projection_version != token.scope.projection {
            self.newest = None; return self.start(token);
        }
        // A fresh subscription must rejoin an incomparable epoch; do not pin the discarded cut.
        if self.newest.as_ref().is_some_and(|held| held.epoch != page.frontier.epoch) { self.newest = None; return self.start(token); }
        if self.staged.frontier.as_ref().is_some_and(|held| held.epoch == page.frontier.epoch && held.changed_index == page.frontier.changed_index && held.live_count != page.frontier.live_count) { return self.stop(Status::Refused); }
        if self.staged.frontier.as_ref().is_some_and(|held| held != &page.frontier) { return self.start(token); }
        if let Some(newest) = &self.newest {
            if newest.changed_index > page.frontier.changed_index { return self.start(token); }
            if newest.changed_index == page.frontier.changed_index && newest.live_count != page.frontier.live_count { return self.stop(Status::Refused); }
        }
        if page.has_more && page.members.is_empty() { return self.stop(Status::Refused); }
        if page.has_more != page.next.is_some() || page.next.as_ref().is_some_and(|cursor| cursor.is_empty() || !self.staged.cursors.insert(cursor.clone())) { return self.stop(Status::Refused); }
        for row in page.members {
            if row.container != token.scope.container || self.staged.rows.insert(row.member.clone(), row).is_some() { return self.stop(Status::Refused); }
        }
        if self.staged.rows.len() as u64 > page.frontier.live_count { return self.stop(Status::Refused); }
        self.staged.frontier = Some(page.frontier.clone()); self.staged.snapshots.push(page.snapshot);
        self.newest = Some(page.frontier.clone());
        if let Some(cursor) = page.next { return vec![Effect::Read(token, cursor)]; }
        if self.staged.rows.len() as u64 != page.frontier.live_count { return self.stop(Status::Refused); }
        self.complete = Some(Complete { scope: token.scope, frontier: page.frontier.clone(), snapshots: std::mem::take(&mut self.staged.snapshots), members: std::mem::take(&mut self.staged.rows).into_values().collect() });
        self.newest = Some(page.frontier); self.status = Status::Ready; self.staged = Staged::default();
        vec![Effect::PersistBase]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> Scope { Scope { owner: "owner-a".into(), container: "container-a".into(), endpoint: "endpoint-a".into(), host: "host-a".into(), projection: "1".into() } }
    fn frontier(index: u64, count: u64) -> Frontier { Frontier { epoch: "epoch-a".into(), changed_index: index, live_count: count } }
    fn page(index: u64, count: u64, ids: &[&str], next: Option<&str>) -> Page {
        Page { frontier: frontier(index, count), snapshot: Snapshot { id: format!("snapshot-{index}"), host_id: "host-a".into(), store_index: index, projection_version: "1".into(), created_at: "created".into(), published_at: None }, members: ids.iter().map(|id| MembershipInput { container: "container-a".into(), member: (*id).into(), revision: "revision".into(), position: fractal_schema::ordered_membership::Position { bucket: None, key: "a0".into() } }).collect(), has_more: next.is_some(), next: next.map(str::to_owned) }
    }
    fn refresh(state: &mut Paging) -> Token {
        let effects = state.reduce(Event::Refresh(scope(), 1));
        let [Effect::SubscribeAndRead(token)] = effects.as_slice() else { panic!("expected first read") }; token.clone()
    }
    fn ready() -> (Paging, Token) {
        let mut state = Paging::new(None); let token = refresh(&mut state);
        assert_eq!(state.reduce(Event::Page(token.clone(), page(1, 1, &["a"], None))), vec![Effect::PersistBase]); (state, token)
    }
    #[test]
    fn matching_refresh_metadata_guards_against_regressed_index_and_count() {
        for (index, count, status) in [(4, 1, Status::Paging), (5, 0, Status::Refused)] {
            let mut state = Paging::new(None); let token = refresh(&mut state);
            assert_eq!(state.reduce(Event::Page(token, page(5, 1, &["a"], None))), vec![Effect::PersistBase]);
            let base = state.complete().unwrap().clone(); let token = refresh(&mut state);
            assert!(state.reduce(Event::Metadata(token.clone(), frontier(5, 1))).is_empty());
            let effects = state.reduce(Event::Page(token, page(index, count, if count == 0 { &[] } else { &["b"] }, None)));
            if status == Status::Paging { assert!(matches!(effects.as_slice(), [Effect::SubscribeAndRead(_)])); } else { assert!(effects.is_empty()); }
            assert_eq!(state.status, status); assert_eq!(state.complete(), Some(&base));
        }
    }
    #[test]
    fn every_partition_publishes_only_complete_pages_despite_unrelated_writes() {
        for count in 0..12 { for width in 1..=12 {
            let mut state = Paging::new(None); let token = refresh(&mut state);
            state.reduce(Event::Metadata(token.clone(), frontier(1, count)));
            let ids: Vec<String> = (0..count).map(|i| format!("member-{i}")).collect();
            let chunks: Vec<_> = ids.chunks(width).collect();
            if count == 0 { state.reduce(Event::Page(token.clone(), page(1, 0, &[], None))); }
            for (i, chunk) in chunks.iter().enumerate() {
                let more = i + 1 < chunks.len(); let cursor = format!("cursor-{i}");
                let mut input = page(1, count, &chunk.iter().map(String::as_str).collect::<Vec<_>>(), more.then_some(cursor.as_str()));
                input.snapshot.store_index = 100 + i as u64; input.snapshot.id = format!("snapshot-{i}");
                let effects = state.reduce(Event::Page(token.clone(), input));
                if more { assert!(state.complete().is_none()); assert_eq!(effects, vec![Effect::Read(token.clone(), cursor)]); }
                else { assert_eq!(effects, vec![Effect::PersistBase]); }
            }
            assert_eq!(state.status, Status::Ready); assert_eq!(state.complete().unwrap().members.len() as u64, count);
            assert!(!state.stale()); state.reduce(Event::Metadata(token, frontier(0, 0))); assert!(!state.stale());
        } }
    }
    #[test]
    fn malformed_pages_refuse_without_erasing_last_complete() {
        for case in 0..11 {
            let (mut state, _) = ready(); let token = refresh(&mut state);
            let mut input = page(2, 2, &["b"], Some("cursor"));
            match case {
                0 => input.next = None, 1 => input.has_more = false, 2 => input.next = Some(String::new()),
                3 => input.members[0].container = "other".into(), 4 => input.members.push(input.members[0].clone()),
                5 => input.frontier.live_count = 0, 6 => { input.next = None; input.has_more = false; },
                7 => { state.reduce(Event::Page(token.clone(), input)); input = page(2, 2, &["c"], Some("cursor")); }
                8 => input.members.clear(),
                9 => { state.reduce(Event::Page(token.clone(), input)); input = page(2, 3, &["c"], None); }
                _ => { state.reduce(Event::Page(token.clone(), input)); input = page(2, 2, &["b"], None); }
            }
            assert!(state.reduce(Event::Page(token.clone(), input)).is_empty()); assert_eq!(state.status, Status::Refused);
            assert_eq!(state.complete().unwrap().members[0].member, "a");
            assert!(state.reduce(Event::Expired(token)).is_empty()); refresh(&mut state); assert_eq!(state.status, Status::Paging);
        }
    }
    #[test]
    fn restarts_are_bounded_and_late_attempts_are_inert() {
        for reason in 0..5 {
            let (mut state, _) = ready(); let old = refresh(&mut state); let mut token = old.clone();
            for attempt in 1..=3 {
                let mut first = page(attempt + 1, 2, &["b"], Some("cursor"));
                if let Some(newest) = &state.newest { first.frontier.epoch.clone_from(&newest.epoch); }
                state.reduce(Event::Page(token.clone(), first));
                let mut input = page(attempt + 2, 2, &["c"], None);
                let event = match reason {
                    0 => Event::Expired(token.clone()), 1 => Event::Metadata(token.clone(), frontier(attempt + 2, 2)),
                    2 => Event::Page(token.clone(), input),
                    3 => { input.snapshot.host_id = "other".into(); Event::Page(token.clone(), input) },
                    _ => { input.frontier.epoch = format!("epoch-{attempt}"); Event::Metadata(token.clone(), input.frontier) }
                };
                let effects = state.reduce(event);
                assert!(state.reduce(Event::Page(token, page(2, 0, &[], None))).is_empty());
                if attempt < 3 { let [Effect::SubscribeAndRead(next)] = effects.as_slice() else { panic!("restart") }; token = next.clone(); }
                else { assert!(effects.is_empty()); assert_eq!(state.status, Status::Exhausted); break; }
            }
            assert_eq!(state.complete().unwrap().members[0].member, "a"); refresh(&mut state);
            assert!(state.reduce(Event::Metadata(old, frontier(99, 0))).is_empty());
        }
    }
    #[test]
    fn metadata_join_restore_disconnect_and_scope_switch() {
        let (mut state, token) = ready();
        assert_eq!(state.reduce(Event::Metadata(token.clone(), frontier(1, 1))).len(), 0);
        let effects = state.reduce(Event::Metadata(token.clone(), frontier(2, 1)));
        assert!(matches!(effects.as_slice(), [Effect::SubscribeAndRead(_)])); assert!(state.stale());
        state.reduce(Event::Disconnect); assert_eq!(state.status, Status::Disconnected);
        assert!(state.reduce(Event::Page(token, page(99, 0, &[], None))).is_empty());
        let restored = state.complete().cloned(); let mut state = Paging::new(restored); assert!(state.stale());
        let effects = state.reduce(Event::Refresh(scope(), 2));
        let [Effect::SubscribeAndRead(token)] = effects.as_slice() else { panic!("reconnect") };
        state.reduce(Event::Metadata(token.clone(), frontier(1, 1)));
        assert_eq!(state.reduce(Event::Page(token.clone(), page(2, 0, &[], None))), vec![Effect::PersistBase]);
        state.reduce(Event::Refused(token.clone())); assert_eq!(state.status, Status::Refused);
        for field in 0..5 {
            let mut other = scope(); match field { 0 => other.owner.push('b'), 1 => other.container.push('b'), 2 => other.endpoint.push('b'), 3 => other.host.push('b'), _ => other.projection.push('b') }
            state.reduce(Event::Refresh(other, 3)); assert!(state.complete().is_none());
        }
    }
    #[test]
    fn generation_scope_and_refusal_events_cannot_cross_attempts() {
        let (mut state, old) = ready();
        let effects = state.reduce(Event::Refresh(scope(), 2));
        let [Effect::SubscribeAndRead(current)] = effects.as_slice() else { panic!("refresh") };
        for field in 0..8 {
            let mut wrong = current.clone();
            match field {
                0 => wrong.generation = old.generation, 1 => wrong.attempt = old.attempt,
                2 => wrong.scope.owner.push('b'), 3 => wrong.scope.container.push('b'),
                4 => wrong.scope.endpoint.push('b'), 5 => wrong.scope.host.push('b'),
                6 => wrong.scope.projection.push('b'), _ => wrong.attempt += 100,
            }
            assert!(state.reduce(Event::Page(wrong.clone(), page(5, 0, &[], None))).is_empty());
            assert!(state.reduce(Event::Metadata(wrong.clone(), frontier(5, 0))).is_empty());
            assert!(state.reduce(Event::Expired(wrong.clone())).is_empty());
            assert!(state.reduce(Event::Refused(wrong)).is_empty());
            assert_eq!(state.status, Status::Paging);
        }
        state.reduce(Event::Refused(current.clone())); assert_eq!(state.status, Status::Refused);
        assert!(state.stale()); assert_eq!(state.complete().unwrap().members.len(), 1);
    }
    #[test]
    fn metadata_before_pages_supersedes_and_equal_index_count_conflicts_refuse() {
        for mode in 0..4 {
            let mut state = Paging::new(None); let token = refresh(&mut state);
            state.reduce(Event::Metadata(token.clone(), frontier(5, 1)));
            let mut input = page(5, 1, &["a"], None);
            match mode {
                0 => input.frontier.changed_index = 4,
                1 => input.frontier.epoch = "other-epoch".into(),
                2 => input.frontier.live_count = 2,
                _ => input.snapshot.projection_version = "other-projection".into(),
            }
            let effects = state.reduce(Event::Page(token, input));
            assert!(state.complete().is_none());
            if mode == 2 { assert!(effects.is_empty()); assert_eq!(state.status, Status::Refused); }
            else { assert!(matches!(effects.as_slice(), [Effect::SubscribeAndRead(_)])); }
        }
    }
    #[test]
    fn old_stream_frames_cannot_invalidate_a_newer_staged_frontier() {
        let mut state = Paging::new(None); let token = refresh(&mut state);
        state.reduce(Event::Metadata(token.clone(), frontier(1, 2)));
        state.reduce(Event::Page(token.clone(), page(3, 2, &["a"], Some("cursor"))));
        assert!(state.reduce(Event::Metadata(token.clone(), frontier(2, 1))).is_empty());
        assert_eq!(state.reduce(Event::Page(token, page(3, 2, &["b"], None))), vec![Effect::PersistBase]);
        assert_eq!(state.complete().unwrap().snapshots.len(), 2);
    }
    #[test]
    fn incomparable_page_epoch_can_complete_after_a_fresh_subscription() {
        let mut state = Paging::new(None); let token = refresh(&mut state);
        state.reduce(Event::Page(token.clone(), page(2, 2, &["a"], Some("cursor"))));
        let mut changed = page(1, 0, &[], None); changed.frontier.epoch = "epoch-b".into();
        let effects = state.reduce(Event::Page(token, changed));
        let [Effect::SubscribeAndRead(next)] = effects.as_slice() else { panic!("restart") };
        let mut fresh = page(1, 0, &[], None); fresh.frontier.epoch = "epoch-b".into();
        assert_eq!(state.reduce(Event::Page(next.clone(), fresh)), vec![Effect::PersistBase]);
        assert_eq!(state.complete().unwrap().frontier.epoch, "epoch-b");
    }
    #[test]
    fn equal_frontier_metadata_count_conflicts_refuse_paging_and_ready() {
        for paging in [false, true] {
            let (mut state, mut token) = ready();
            if paging {
                token = refresh(&mut state);
                state.reduce(Event::Page(token.clone(), page(1, 2, &["b"], Some("cursor"))));
            }
            assert!(state.reduce(Event::Metadata(token.clone(), frontier(1, 3))).is_empty());
            assert_eq!(state.status, Status::Refused); assert!(state.stale());
            assert_eq!(state.complete().unwrap().members[0].member, "a");
            assert!(state.reduce(Event::Page(token, page(1, 3, &["a", "b", "c"], None))).is_empty());
            assert_eq!(state.status, Status::Refused); assert_eq!(state.complete().unwrap().members.len(), 1);
        }
    }
}
