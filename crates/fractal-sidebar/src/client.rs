//! Optional, unwired collection-v1 read conversions; hosts subscribe before executing page reads.
//! Decode `MembershipPage` before generated client decoding drops the extension metadata.
use fractal_client::{CollectionEvent, Envelope, OrderedMembershipPage, OrderedMembershipState};
use serde::Deserialize;
use crate::{model::MembershipInput, paging::{Event, Frontier, Page, Snapshot, Token}};

/// Daemon-provided metadata; never synthesize an epoch from a host snapshot or a local counter.
#[derive(Debug, Deserialize)]
pub struct MembershipMetadata {
    #[serde(flatten)]
    pub state: OrderedMembershipState,
    pub epoch: String,
}
/// Opt-in HTTP response value, decoded with the generated page fields intact.
#[derive(Debug, Deserialize)]
pub struct MembershipPage {
    #[serde(flatten)]
    pub page: OrderedMembershipPage,
    pub membership: MembershipMetadata,
}
fn frontier(token: &Token, state: &OrderedMembershipState, epoch: &str) -> Option<Frontier> {
    (state.container == token.scope.container && !epoch.is_empty() && state.live_count <= 4096)
        .then(|| Frontier { epoch: epoch.into(), changed_index: state.changed_index, live_count: state.live_count })
}
/// Convert a complete HTTP envelope, retaining provenance and leaving pagination checks to the reducer.
pub fn membership_page(token: Token, response: Envelope<MembershipPage>) -> Event {
    let Envelope { snapshot, value, .. } = response;
    let MembershipPage { page, membership } = value;
    if page.collection != "ordered-memberships" || page.filters.get("person") != Some(&token.scope.owner)
        || page.filters.get("subject") != Some(&token.scope.container) {
        return Event::Refused(token);
    }
    let Some(frontier) = frontier(&token, &membership.state, &membership.epoch) else { return Event::Refused(token); };
    let snapshot = Snapshot { id: snapshot.id, host_id: snapshot.host_id, store_index: snapshot.store_index,
        projection_version: snapshot.projection_version, created_at: snapshot.created_at, published_at: snapshot.published_at };
    let members = page.items.into_iter().map(|row| MembershipInput { container: row.container, member: row.member,
        revision: row.revision, position: fractal_schema::ordered_membership::Position { bucket: row.position.bucket, key: row.position.key } }).collect();
    Event::Page(token, Page { frontier, snapshot, members, has_more: page.page.has_more, next: page.page.next_cursor })
}
/// Low-level window rows are not a complete collection: both frame kinds invalidate only.
/// The generated stream type currently drops `membership.epoch`; the host must extract it from
/// this same raw frame, passing `None` for legacy frames (which refuse). Never use a cached epoch.
pub fn membership_event(token: Token, subscription: &str, event: &CollectionEvent, epoch: Option<&str>) -> Option<Event> {
    let (id, snapshot, membership) = match event {
        CollectionEvent::MembershipSnapshot { id, snapshot, membership, .. }
        | CollectionEvent::MembershipChanges { id, snapshot, membership, .. } => (id, snapshot, membership),
        CollectionEvent::Resync { id, .. } if id == subscription => return Some(Event::Expired(token)),
        CollectionEvent::Error { id, retryable, .. } if id == subscription => return Some(if *retryable == Some(true) { Event::Expired(token) } else { Event::Refused(token) }),
        _ => return None,
    };
    if id != subscription { return None; }
    if snapshot.host_id != token.scope.host || snapshot.projection_version != token.scope.projection { return Some(Event::Expired(token)); }
    Some(match epoch.and_then(|epoch| frontier(&token, membership, epoch)) {
        Some(frontier) => Event::Metadata(token, frontier), None => Event::Refused(token),
    })
}
/// Status AND code are authoritative; legacy HTTP 410 is not a collection-v1 restart.
pub fn membership_http_failure(token: Token, status: u16, code: &str) -> Event {
    match (status, code) {
        (404, _) | (422, "unsupported-membership-paging-contract") => Event::Unsupported(token),
        (422, "invalid-membership-cursor") => Event::InvalidCursor(token),
        (500, "membership-frontier-inconsistent") => Event::FrontierInconsistent(token),
        (409, "membership-frontier-reset" | "membership-collection-changed") => Event::Expired(token),
        _ => Event::Refused(token),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paging::{Effect, Paging, Scope, Status};
    use serde_json::{json, Value};
    fn refresh(state: &mut Paging) -> Token {
        let effects = state.reduce(Event::Refresh(Scope { owner: "person/test".into(), container: "arrangement/test".into(), endpoint: "endpoint/test".into(), host: "host/test".into(), projection: "1".into() }, 1));
        let [Effect::SubscribeAndRead(token)] = effects.as_slice() else { panic!("expected read") }; token.clone()
    }
    fn wire() -> Value {
        json!({"api_version":"0", "request_id":"request/test", "snapshot":{"id":"snapshot/test", "host_id":"host/test", "store_index":99, "projection_version":"1", "created_at":"created", "published_at":"published"},
            "value":{"kind":"collection", "collection":"ordered-memberships", "filters":{"person":"person/test", "subject":"arrangement/test"},
                "membership":{"container":"arrangement/test", "epoch":"epoch/test", "changed_index":2, "live_count":1},
                "items":[{"id":"agent/test", "container":"arrangement/test", "member":"agent/test", "revision":"revision/test", "position":{"bucket":null,"key":"a0"}}],
                "page":{"limit":200,"has_more":false,"next_cursor":null}}})
    }
    fn ready() -> (Paging, Token) {
        let mut state = Paging::new(None); let token = refresh(&mut state);
        assert_eq!(state.reduce(membership_page(token.clone(), serde_json::from_value(wire()).unwrap())), vec![Effect::PersistBase]);
        (state, token)
    }
    #[test]
    fn page_preserves_rows_frontier_and_snapshot_without_using_host_index() {
        let (state, _) = ready(); let base = state.complete().unwrap();
        assert_eq!(base.frontier, Frontier { epoch: "epoch/test".into(), changed_index: 2, live_count: 1 });
        assert_eq!(base.snapshots[0].store_index, 99); assert_eq!(base.snapshots[0].published_at.as_deref(), Some("published"));
        assert_eq!(base.members[0].revision, "revision/test"); assert_eq!(base.members[0].position.key, "a0");
        for path in ["person", "subject"] {
            let mut value = wire(); value["value"]["filters"][path] = json!("other");
            assert!(matches!(membership_page(ready().1, serde_json::from_value(value).unwrap()), Event::Refused(_)));
        }
        let mut legacy = wire(); legacy["value"]["membership"].as_object_mut().unwrap().remove("epoch");
        assert!(serde_json::from_value::<Envelope<MembershipPage>>(legacy).is_err());
    }
    #[test]
    fn every_status_code_pair_maps_exactly_and_terminal_failures_keep_stale_base() {
        let codes = ["not-found", "unsupported-membership-paging-contract", "invalid-membership-cursor", "membership-frontier-inconsistent", "membership-frontier-reset", "membership-collection-changed", "page-cursor-expired"];
        for status in [403, 404, 409, 410, 422, 500] { for code in codes {
            let (mut state, _) = ready(); let base = state.complete().unwrap().clone(); let token = refresh(&mut state);
            let expected = match (status, code) {
                (404, _) | (422, "unsupported-membership-paging-contract") => Status::Unsupported,
                (422, "invalid-membership-cursor") => Status::InvalidCursor,
                (500, "membership-frontier-inconsistent") => Status::FrontierInconsistent,
                (409, "membership-frontier-reset" | "membership-collection-changed") => Status::Paging,
                _ => Status::Refused,
            };
            let effects = state.reduce(membership_http_failure(token.clone(), status, code));
            assert_eq!(state.status, expected, "{status}/{code}"); assert_eq!(state.complete(), Some(&base)); assert!(state.stale());
            if expected == Status::Paging {
                let [Effect::SubscribeAndRead(next)] = effects.as_slice() else { panic!("restart from page one") };
                assert_ne!(next.attempt, token.attempt);
            } else { assert!(effects.is_empty()); }
            assert!(state.reduce(membership_http_failure(token, 500, "membership-frontier-inconsistent")).is_empty());
        } }
    }
    #[test]
    fn both_window_events_only_invalidate_and_metadata_only_changes_reread() {
        for kind in ["snapshot", "changes"] {
            let (mut state, token) = ready(); let base = state.complete().unwrap().clone();
            let page = wire(); let mut frame = json!({"kind":kind,"collection":"ordered-memberships","id":"sidebar","snapshot":page["snapshot"],"membership":page["value"]["membership"],"order":[],"has_more":false,"items":[],"upserts":[],"removes":[]});
            frame["membership"]["changed_index"] = json!(3);
            let event: CollectionEvent = serde_json::from_value(frame).unwrap();
            assert!(membership_event(token.clone(), "other", &event, Some("epoch/test")).is_none());
            assert!(matches!(membership_event(token.clone(), "sidebar", &event, None), Some(Event::Refused(_))));
            let converted = membership_event(token.clone(), "sidebar", &event, Some("epoch/test")).unwrap();
            assert!(matches!(&converted, Event::Metadata(_, f) if f.changed_index == 3));
            assert!(matches!(state.reduce(converted).as_slice(), [Effect::SubscribeAndRead(_)]));
            assert_eq!(state.complete(), Some(&base)); assert!(state.stale());
            assert!(state.reduce(membership_event(token, "sidebar", &event, Some("epoch/test")).unwrap()).is_empty());
        }
    }
}
