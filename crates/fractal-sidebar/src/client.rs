//! Optional, unwired read/write conversions; hosts subscribe before executing page reads.
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

/// Exact v2 action payload. The actor is supplied by the authenticated client session,
/// not a wire field; hosts must execute/replay with the journal's matching actor.
/// Call only for a reducer `Send` effect, borrowing its persisted immutable request.
pub fn write_request(request: &crate::edits::Request) -> Result<fractal_client::ActionRequest, serde_json::Error> {
    use fractal_client::{ActionRequest, ArrangementEditParameters, ArrangementMembershipEditParameters, Fence};
    use crate::edits::Operations;
    let fence = Fence { snapshot_id: request.fence.snapshot.id.clone(),
        subject_revisions: [(request.fence.scope.container.clone(), request.fence.revision.clone())].into(),
        ..Fence::default() };
    let id = &request.identity;
    match &request.operations {
        Operations::Folders(operations) => ActionRequest::arrangement_edit(&id.action_id, &id.idempotency_key, fence,
            ArrangementEditParameters { subject: request.fence.scope.container.clone(), owner: request.fence.scope.owner.clone(),
                operations: operations.clone(), version: None }),
        Operations::Memberships(operations) => ActionRequest::arrangement_membership_edit(&id.action_id, &id.idempotency_key, fence,
            ArrangementMembershipEditParameters { subject: request.fence.scope.container.clone(), owner: request.fence.scope.owner.clone(),
                operations: operations.clone() }),
    }
}

/// Receipts acknowledge delivery, never authoritative observation. A lost, malformed or
/// mismatched response cannot prove non-delivery; replay retains the original request.
/// Pass the provenance from `Effect::Send` unchanged; replay errors reject only this
/// attempt, not an earlier uncertain delivery. Hosts never infer it from HTTP errors.
pub fn write_result(request: &crate::edits::Request, delivery: crate::edits::Delivery, result: Result<Envelope<fractal_client::ActionResult>, fractal_client::ClientError>) -> crate::edits::Event {
    use fractal_client::{ActionStatus, ClientError, ErrorCode};
    use crate::edits::{Event, Refusal};
    let id = request.identity.clone();
    match result {
        Ok(response) if response.value.action_id == id.action_id => match response.value.status {
            ActionStatus::Accepted | ActionStatus::Completed => Event::Ack(id),
            ActionStatus::Rejected => Event::Refused(id, Refusal::Conflict),
            ActionStatus::Unknown => Event::Unknown(id),
        },
        Err(_) if delivery == crate::edits::Delivery::Replay => Event::Unknown(id),
        Err(error) if error.applied() == Some("unknown") => Event::Unknown(id),
        // These arrangement-specific validation/precondition codes precede the write:
        // api/client_v0/arrangements.rs:44-48,89-106; store/arrangements.rs:328-339,363-369.
        // Receipt persistence runs AFTER commit (api/client_v0.rs:9586-9588,9625-9665).
        // In particular idempotency-conflict can occur there and is not proof of refusal.
        Err(ClientError::Api(code, _, envelope)) => {
            let reason = match code {
                ErrorCode::StaleFence => Some(Refusal::StaleFence),
                ErrorCode::Forbidden | ErrorCode::ArrangementOwnerForbidden | ErrorCode::MembershipOwnerForbidden => Some(Refusal::Unauthorized),
                ErrorCode::ArrangementFolderDeleted | ErrorCode::ArrangementRetired | ErrorCode::NotFound => Some(Refusal::DestinationMissing),
                ErrorCode::InvalidMembershipOperations => Some(Refusal::InvalidPlan),
                ErrorCode::IdempotencyConflict if envelope.details.get("applied").and_then(serde_json::Value::as_str) == Some("none") => Some(Refusal::Conflict),
                _ if envelope.details.get("applied").and_then(serde_json::Value::as_str) == Some("none") => Some(Refusal::InvalidPlan),
                _ => None,
            };
            match reason { Some(reason) => Event::Refused(id, reason), None => Event::Unknown(id) }
        },
        _ => Event::Unknown(id),
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

#[cfg(test)]
mod write_tests {
    use super::*;
    use crate::{edits::*, model::{NodeRef, Parent}, paging::Scope, plan::{MoveIntent, V2Plan}};
    use fractal_client::{ActionResult, ClientError, ErrorCode, ErrorEnvelope};
    use fractal_schema::{arrangements::Operation as Folder, ordered_membership::Operation as Member};
    use serde_json::{json, Value};
    const FOLDER: &str = "01900000-0000-7000-8000-000000000001";
    const OTHER_FOLDER: &str = "01900000-0000-7000-8000-000000000002";
    const CONTAINER: &str = "arrangement/person/test/01900000-0000-7000-8000-000000000001";
    fn base(n: u64) -> Base {
        Base { scope: Scope { owner: "person/test".into(), container: CONTAINER.into(), endpoint: "endpoint/test".into(), host: "host/test".into(), projection: "1".into() },
            snapshot: Snapshot { id: format!("snapshot/{n}"), host_id: "host/test".into(), store_index: n,
                projection_version: "1".into(), created_at: "created".into(), published_at: None },
            revision: format!("revision/{n}"), ready: true }
    }
    fn id(n: u64) -> Identity {
        Identity { actor: "person/test".into(), action_id: format!("action/{n}"), idempotency_key: format!("key/{n}") }
    }
    fn members() -> Vec<Member> {
        vec![Member::Place { member: "agent/fleet/fixture-sidebar/test".into(), bucket: None, key: "a0".into() },
            Member::Place { member: "agent/fleet/fixture-sidebar/other".into(), bucket: Some(FOLDER.into()), key: "a1".into() }]
    }
    fn folders() -> Vec<Folder> {
        vec![Folder::FolderCreate { id: "01900000-0000-7000-8000-000000000003".into(), name: "Test".into(), parent: None, key: "a0".into() },
            Folder::FolderRename { id: FOLDER.into(), name: "Renamed".into() },
            Folder::FolderMove { id: FOLDER.into(), parent: Some(OTHER_FOLDER.into()), key: "a1".into() },
            Folder::FolderDelete { id: OTHER_FOLDER.into() }]
    }
    fn staged(compound: bool) -> Edits {
        let mut state = Edits::new(base(1).scope, MoveIntent { node: NodeRef::Subject { subject: "agent/test".into() },
            parent: Parent::Root {}, before: None });
        assert_eq!(state.reduce(crate::edits::Event::Plan { base: base(1),
            plan: V2Plan { folders: if compound { folders() } else { vec![] }, memberships: members() },
            folder: id(1), membership: id(2) }), vec![Effect::PersistPending]);
        state
    }
    fn wire(state: &Edits) -> Value { serde_json::to_value(write_request(state.request().unwrap()).unwrap()).unwrap() }
    fn receipt(id: &str, status: &str) -> Envelope<ActionResult> {
        serde_json::from_value(json!({"api_version":"0","request_id":"request/test",
            "snapshot":{"id":"snapshot/2","host_id":"host/test","store_index":2,"projection_version":"1","created_at":"created"},
            "value":{"kind":"action-result","action_id":id,"operation_id":"operation/test","status":status,"affected_ids":[],"snapshot_id":"snapshot/2"}})).unwrap()
    }
    fn failure(code: ErrorCode, applied: Option<&str>) -> ClientError {
        ClientError::Api(code.clone(), "refusal".into(), Box::new(ErrorEnvelope { api_version: "0".into(), error_version: "1".into(),
            request_id: "request/test".into(), code, message: "refusal".into(), retryable: true, retry_after_ms: None,
            details: applied.map(|s| [("applied".into(), json!(s))].into()).unwrap_or_default() }))
    }
    #[test]
    fn exact_folder_and_place_payloads_keep_all_operations_and_fences() {
        for compound in [false, true] {
            let state = staged(compound); let request = wire(&state);
            assert_eq!(request["id"], if compound { "action/1" } else { "action/2" });
            assert_eq!(request["idempotency_key"], if compound { "key/1" } else { "key/2" });
            assert_eq!(request["fence"]["snapshot_id"], "snapshot/1");
            assert_eq!(request["fence"]["subject_revisions"], json!({(CONTAINER):"revision/1"}));
            let expected = json!({"subject":CONTAINER,"owner":"person/test",
                "operations": if compound { serde_json::to_value(folders()).unwrap() } else { serde_json::to_value(members()).unwrap() }});
            assert_eq!(request["parameters"], expected);
            assert_eq!(request["type"], if compound { "arrangement.edit" } else { "arrangement.membership.edit" });
            assert!(request.get("actor").is_none());
        }
    }
    #[test]
    fn acknowledgements_refusals_and_ambiguous_delivery_are_distinct() {
        let state = staged(false); let request = state.request().unwrap();
        for status in ["accepted", "completed"] {
            for delivery in [Delivery::First, Delivery::Replay] {
                assert!(matches!(write_result(request, delivery, Ok(receipt("action/2", status))), crate::edits::Event::Ack(i) if i == id(2)));
            }
        }
        for (code, reason) in [(ErrorCode::StaleFence, Refusal::StaleFence), (ErrorCode::Forbidden, Refusal::Unauthorized),
            (ErrorCode::ArrangementOwnerForbidden, Refusal::Unauthorized), (ErrorCode::MembershipOwnerForbidden, Refusal::Unauthorized),
            (ErrorCode::ArrangementFolderDeleted, Refusal::DestinationMissing), (ErrorCode::ArrangementRetired, Refusal::DestinationMissing),
            (ErrorCode::NotFound, Refusal::DestinationMissing), (ErrorCode::InvalidMembershipOperations, Refusal::InvalidPlan)] {
            assert!(matches!(write_result(request, Delivery::First, Err(failure(code.clone(), None))), crate::edits::Event::Refused(i, r) if i == id(2) && r == reason));
            for applied in [None, Some("none")] {
                assert!(matches!(write_result(request, Delivery::Replay, Err(failure(code.clone(), applied))), crate::edits::Event::Unknown(i) if i == id(2)));
            }
        }
        assert!(matches!(write_result(request, Delivery::First, Err(failure(ErrorCode::IdempotencyConflict, Some("none")))), crate::edits::Event::Refused(_, Refusal::Conflict)));
        assert!(matches!(write_result(request, Delivery::First, Err(failure(ErrorCode::Internal, Some("none")))), crate::edits::Event::Refused(_, Refusal::InvalidPlan)));
        assert!(matches!(write_result(request, Delivery::First, Ok(receipt("action/2", "rejected"))), crate::edits::Event::Refused(_, Refusal::Conflict)));
        for result in [Err(ClientError::Transport("timeout".into())), Err(ClientError::Protocol("truncated".into())),
            Err(failure(ErrorCode::StaleFence, Some("unknown"))), Err(failure(ErrorCode::Internal, None)),
            Err(failure(ErrorCode::Unknown, None)), Err(failure(ErrorCode::IdempotencyConflict, None)),
            Ok(receipt("action/other", "completed")), Ok(receipt("action/2", "future"))] {
            assert!(matches!(write_result(request, Delivery::First, result), crate::edits::Event::Unknown(i) if i == id(2)));
        }
    }
    #[test]
    fn restore_replays_exact_payload_and_compound_step_refreshes_both_fences() {
        let mut state = staged(true); let first = wire(&state);
        assert_eq!(state.reduce(crate::edits::Event::Persisted(id(1))), vec![Effect::Send(Delivery::First)]);
        // HTTP 500/internal after the layout commit cannot prove non-application.
        state.reduce(write_result(state.request().unwrap(), Delivery::First, Err(failure(ErrorCode::Internal, None))));
        assert_eq!(state.status, Status::UnknownDelivery);
        assert!(state.reduce(crate::edits::Event::Retry).is_empty());
        let (mut state, effects) = Edits::restore(&state.journal().unwrap(), &base(1).scope).unwrap();
        assert_eq!(effects, vec![Effect::PersistPending]); assert_eq!(wire(&state), first);
        let sends = state.reduce(crate::edits::Event::Persisted(id(1)));
        let [Effect::Send(delivery)] = sends.as_slice() else { panic!("expected replay") };
        assert_eq!(*delivery, Delivery::Replay);
        assert_eq!(wire(&state), first);
        assert_eq!(state.reduce(write_result(state.request().unwrap(), *delivery, Err(failure(ErrorCode::Forbidden, None)))), vec![Effect::PersistPending]);
        assert_eq!(state.status, Status::UnknownDelivery); assert_eq!(wire(&state), first);
        // No automatic send: persistence acknowledgements cannot replay UnknownDelivery.
        assert!(state.reduce(crate::edits::Event::Persisted(id(1))).is_empty());
        assert!(state.reduce(crate::edits::Event::Retry).is_empty());
        assert_eq!(state.reduce(crate::edits::Event::Replay), vec![Effect::Send(Delivery::Replay)]);
        state.reduce(write_result(state.request().unwrap(), Delivery::Replay, Ok(receipt("action/1", "completed"))));
        assert_eq!(state.status, Status::AwaitingObservation);
        assert!(state.reduce(crate::edits::Event::Fresh(base(2), Ok(members()))).is_empty());
        state.reduce(crate::edits::Event::Observed(id(1)));
        assert_eq!(state.reduce(crate::edits::Event::CompletionPersisted(id(1))), vec![Effect::ReadBase]);
        assert_eq!(state.reduce(crate::edits::Event::Fresh(base(1), Ok(members()))), vec![Effect::ReadBase]);
        assert_eq!(state.reduce(crate::edits::Event::Fresh(base(2), Ok(members()))), vec![Effect::PersistPending]);
        let second = wire(&state); assert_eq!(second["type"], "arrangement.membership.edit");
        assert_eq!(second["id"], "action/2"); assert_eq!(second["fence"]["snapshot_id"], "snapshot/2");
        assert_eq!(second["fence"]["subject_revisions"], json!({(CONTAINER):"revision/2"}));
        assert_eq!(state.reduce(crate::edits::Event::Persisted(id(2))), vec![Effect::Send(Delivery::First)]);
        let (restored, _) = Edits::restore(&state.journal().unwrap(), &base(1).scope).unwrap();
        assert_eq!(wire(&restored), second);
    }
}
