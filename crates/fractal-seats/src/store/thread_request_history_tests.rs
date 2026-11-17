//! Integration coverage through the public missions projection and actual source writers.
use super::*;
use super::tests::fixture;
use crate::model::PersonRenameRequest;
use crate::store::thread_links::{AttachAsk, DecisionHistory, attach_ask};
use crate::store::thread_metadata::{Edit, InboxOrigin, StartFromInbox, apply, start_from_inbox};
use fractal_missions::store::missions::thread_request_history;
use fractal_missions::store::person_work_tests_shared_seats::{TEST_FLEET, receive_exchange};
use fractal_store::store::thread_metadata::request_history::{
    EpisodeState, HistorySourceSupport, RequestHistoryPage, SourceEpisode,
};

const PERSON: &str = "person/avery";
const AGENT: &str = "agent/alder.asker";
const THREAD: &str = "thread/request-history";

fn ask(store: &Store, input: &PersonAskRequest, key: &str) -> SourceEpisode {
    let mut input = input.clone();
    input.step = None;
    input.idempotency_key = key.into();
    let view = store.ask_person(&input).unwrap();
    let opening = request(&store.readers.get(), &view.subject).unwrap().unwrap();
    SourceEpisode { source_id: view.subject, episode: opening.id }
}

fn start(store: &Store, key: &SourceEpisode, thread: &str, agent: &str) {
    let message = ClaimInput {
        subject: format!("message/{}", thread.strip_prefix("thread/").unwrap()),
        kind: "message.sent".into(), actor: Some(PERSON.into()),
        fields: BTreeMap::from([
            ("from".into(), json!(PERSON)), ("to".into(), json!(agent)),
            ("content".into(), json!("Discuss this request.")), ("status".into(), json!("sent")),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    };
    let origin = InboxOrigin { source_id: key.source_id.clone(), episode: key.episode.clone() };
    start_from_inbox(store, StartFromInbox {
        thread, title: "Related requests", person: PERSON, origin: &origin,
        key: &format!("start-{thread}"), first_message: &message,
    }).unwrap();
}

fn attach(store: &Store, child: &SourceEpisode, parent: Option<&SourceEpisode>, key: &str) -> (ClaimRecord, bool) {
    attach_ask(store, AGENT, AttachAsk {
        thread: THREAD, ask: &child.source_id, episode: &child.episode, key,
        decision: parent.map(|parent| DecisionHistory {
            predecessor: &parent.source_id, episode: &parent.episode, reason: key,
        }),
    }).unwrap()
}

fn finish(store: &Store, key: &SourceEpisode, cancel: bool) {
    store.finish_person_step(&PersonStepResponse {
        delegation: None, subject: key.source_id.clone(),
        actor: if cancel { AGENT } else { PERSON }.into(),
        summary: "Independent result".into(), evidence: vec![], episode: Some(key.episode.clone()),
        idempotency_key: format!("finish-{}", key.source_id), answer: None,
    }, cancel).unwrap();
}

fn page(store: &Store) -> RequestHistoryPage {
    thread_request_history(store, PERSON, THREAD, None, 50).unwrap()
}

fn keys(page: &RequestHistoryPage) -> BTreeSet<SourceEpisode> {
    page.items.iter().map(|item| item.key.clone()).collect()
}

fn status<'a>(page: &'a RequestHistoryPage, key: &SourceEpisode) -> &'a str {
    match &page.items.iter().find(|item| &item.key == key).unwrap().state {
        EpisodeState::Available { status, .. } => status,
        EpisodeState::Unavailable => panic!("real runtime episode state unavailable"),
    }
}

fn rename_live(store: &Store) {
    // Establish the destination with the ordinary message writer, as the existing rename fixture does.
    store.append_claim(&ClaimInput {
        subject: "message/history-rename-destination".into(), kind: "message.sent".into(),
        actor: Some("person/robin".into()),
        fields: BTreeMap::from([
            ("from".into(), json!("person/robin")), ("to".into(), json!(AGENT)),
            ("content".into(), json!("Known destination")), ("status".into(), json!("sent")),
        ]), evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    store.rename_person(&PersonRenameRequest {
        old_person: PERSON.into(), new_person: "person/robin".into(), actor: PERSON.into(),
        idempotency_key: "history-rename".into(),
    }, &[]).unwrap();
}

#[test]
fn request_history_branch_statuses_remain_independent_after_thread_close() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let a = ask(&store, &input, "branch-a");
    start(&store, &a, THREAD, AGENT);
    let revision = crate::store::thread_metadata::summary(&store, PERSON, THREAD).unwrap().revision;
    apply(&store, PERSON, THREAD, "close-history", Edit::Close { revision: &revision }).unwrap();
    let closed_revision = store.latest_claim_id(THREAD).unwrap();
    let b = ask(&store, &input, "branch-b");
    let c = ask(&store, &input, "branch-c");
    attach(&store, &b, Some(&a), "branch-a-to-b");
    attach(&store, &c, Some(&a), "branch-a-to-c");
    finish(&store, &a, false);
    finish(&store, &b, true);
    let history = page(&store);
    assert_eq!(keys(&history), BTreeSet::from([a.clone(), b.clone(), c.clone()]));
    assert_eq!(status(&history, &a), "completed");
    assert_eq!(status(&history, &b), "cancelled");
    assert_eq!(status(&history, &c), "ready");
    assert_eq!(history.relations.len(), 2);
    for child in [&b, &c] {
        assert!(history.relations.iter().any(|edge| edge.predecessor == a && &edge.request == child));
    }
    let inbox = store.attention_items(Some(PERSON)).unwrap();
    assert!(!inbox.iter().any(|item| item.subject == a.source_id || item.subject == b.source_id));
    assert!(inbox.iter().any(|item| item.subject == c.source_id));
    assert_eq!(store.latest_claim_id(THREAD).unwrap(), closed_revision);
}

#[test]
fn request_history_same_ask_can_origin_multiple_threads_without_cross_thread_traversal() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let a = ask(&store, &input, "shared-origin");
    start(&store, &a, THREAD, AGENT);
    start(&store, &a, "thread/other-history", AGENT);
    let b = ask(&store, &input, "only-first-thread");
    attach(&store, &b, Some(&a), "first-only-reason");
    assert_eq!(keys(&page(&store)), BTreeSet::from([a.clone(), b]));
    let other = thread_request_history(&store, PERSON, "thread/other-history", None, 50).unwrap();
    assert_eq!(keys(&other), BTreeSet::from([a]));
    assert!(other.relations.is_empty());
}

#[test]
fn request_history_page_size_one_carries_predecessors_without_dangling_relations() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let a = ask(&store, &input, "boundary-a");
    start(&store, &a, THREAD, AGENT);
    let b = ask(&store, &input, "boundary-b");
    let c = ask(&store, &input, "boundary-c");
    attach(&store, &b, Some(&a), "boundary-a-to-b");
    attach(&store, &c, Some(&a), "boundary-a-to-c");
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let mut edges = 0;
    loop {
        let history = thread_request_history(&store, PERSON, THREAD, cursor.as_deref(), 1).unwrap();
        assert_eq!(history.items.len(), 1);
        assert!(seen.insert(history.items[0].key.clone()));
        for edge in &history.relations {
            assert!(history.items.iter().any(|item| item.key == edge.request));
            assert!(history.items.iter().chain(&history.boundary_items).any(|item| item.key == edge.predecessor));
            assert_eq!(edge.predecessor, a);
            assert_eq!(history.boundary_items.len(), 1);
            edges += 1;
        }
        cursor = history.next_cursor;
        if cursor.is_none() { break; }
        assert!(seen.len() < 3, "cursor must advance over primary rows, not boundary rows");
    }
    assert_eq!(seen, BTreeSet::from([a, b, c]));
    assert_eq!(edges, 2);
}

#[test]
fn request_history_cursor_is_scoped_visible_keyset_with_fresh_status() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let a = ask(&store, &input, "cursor-a");
    start(&store, &a, THREAD, AGENT);
    start(&store, &a, "thread/cursor-other", AGENT);
    let b = ask(&store, &input, "cursor-b");
    attach(&store, &b, Some(&a), "cursor-edge");
    let cursor = thread_request_history(&store, PERSON, THREAD, None, 1).unwrap().next_cursor.unwrap();
    let first = thread_request_history(&store, PERSON, THREAD, None, 1).unwrap();
    let (_, marker) = cursor.split_once(':').unwrap();
    assert_eq!(serde_json::from_str::<SourceEpisode>(marker).unwrap(), first.items[0].key);
    let unrelated = ask(&store, &input, "unrelated-cursor-activity");
    assert!(!keys(&first).contains(&unrelated));
    assert_eq!(thread_request_history(&store, PERSON, THREAD, None, 1).unwrap().next_cursor.as_deref(), Some(cursor.as_str()));
    thread_request_history(&store, PERSON, THREAD, Some(&cursor), 1).unwrap();
    thread_request_history(&store, PERSON, THREAD, Some(&cursor), 2).unwrap();
    let mut tampered = cursor.clone();
    tampered.push('!');
    for (person, thread, token, limit, code) in [
        (PERSON, THREAD, tampered.as_str(), 1, "invalid-thread-cursor"),
        (PERSON, "thread/cursor-other", cursor.as_str(), 1, "invalid-thread-cursor"),
        (PERSON, THREAD, &format!("{}:{}", cursor.split_once(':').unwrap().0, serde_json::to_string(&unrelated).unwrap()), 1, "invalid-thread-cursor"),
        ("person/other", THREAD, cursor.as_str(), 1, "thread-unavailable"),
    ] {
        assert_eq!(thread_request_history(&store, person, thread, Some(token), limit).unwrap_err().code, code);
    }
    finish(&store, &b, false);
    thread_request_history(&store, PERSON, THREAD, Some(&cursor), 1).unwrap();
    assert_eq!(status(&page(&store), &b), "completed");
}

#[test]
fn request_history_reassigned_seed_is_unavailable_before_episode_enumeration() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let a = ask(&store, &input, "denied-seed");
    start(&store, &a, THREAD, AGENT);
    assert_eq!(thread_request_history(&store, "person/other", THREAD, None, 1).unwrap_err().code, "thread-unavailable");
    rename_live(&store);
    let error = thread_request_history(&store, PERSON, THREAD, None, 1).unwrap_err();
    assert_eq!(error.code, "thread-unavailable");
    let rendered = format!("{error:?}");
    assert!(!rendered.contains(&a.source_id));
    assert!(!rendered.contains(&a.episode));
}

#[test]
fn request_history_denied_child_hides_identity_reason_counts_and_pagination() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let a = ask(&store, &input, "visible-resolved-seed");
    start(&store, &a, THREAD, AGENT);
    finish(&store, &a, false);
    let before = serde_json::to_value(page(&store)).unwrap();
    let hidden = ask(&store, &input, "later-denied-child");
    attach(&store, &hidden, Some(&a), "private-decision-reason");
    rename_live(&store);
    let after = thread_request_history(&store, PERSON, THREAD, None, 1).unwrap();
    assert_eq!(serde_json::to_value(&after).unwrap(), before);
    assert!(after.next_cursor.is_none());
    assert!(after.complete);
    let serialized = serde_json::to_string(&after).unwrap();
    for secret in [hidden.source_id.as_str(), hidden.episode.as_str(), "private-decision-reason"] {
        assert!(!serialized.contains(secret));
    }
    assert!(after.relations.is_empty());
    assert!(after.boundary_items.is_empty());
    assert!(after.source_episodes.is_empty());
}

#[test]
fn request_history_duplicate_receipts_and_late_predecessor_membership_converge() {
    let (source, _, input) = fixture();
    source.configure_thread_write_rollout([]);
    let seed = ask(&source, &input, "replication-seed");
    let a = ask(&source, &input, "replication-parent");
    let b = ask(&source, &input, "replication-child");
    start(&source, &seed, THREAD, AGENT);
    let target = Store::open_memory("birch").unwrap();
    let baseline = source.export_replication_exchange_answering(TEST_FLEET, &target.replication_inventory().unwrap(), &[]).unwrap();
    receive_exchange(&target, &baseline);
    attach(&source, &a, None, "late-parent-membership");
    let parent = source.export_replication_exchange_answering(TEST_FLEET, &target.replication_inventory().unwrap(), &[]).unwrap();
    let (receipt, appended) = attach(&source, &b, Some(&a), "replicated-decision");
    assert!(appended);
    let (duplicate, appended) = attach(&source, &b, Some(&a), "replicated-decision");
    assert!(!appended);
    assert_eq!(duplicate.id, receipt.id);
    let mut child = source.export_replication_exchange_answering(TEST_FLEET, &target.replication_inventory().unwrap(), &[]).unwrap();
    child.envelopes.retain(|envelope| !parent.envelopes.iter().any(|old| old.hash == envelope.hash));
    receive_exchange(&target, &child);
    let partial = page(&target);
    assert_eq!(keys(&partial), BTreeSet::from([seed, b.clone()]));
    assert!(partial.relations.is_empty());
    assert!(partial.boundary_items.is_empty());
    assert!(!serde_json::to_string(&partial).unwrap().contains("replicated-decision"));
    receive_exchange(&target, &parent);
    receive_exchange(&target, &child);
    let converged = page(&target);
    assert_eq!(converged.relations.len(), 1);
    assert_eq!(converged.relations[0].predecessor, a);
    assert_eq!(converged.relations[0].request, b);
    assert_eq!(serde_json::to_value(converged).unwrap(), serde_json::to_value(page(&source)).unwrap());
}

#[test]
fn request_history_current_custom_origin_has_explicit_unsupported_marker() {
    let (store, _, _) = fixture();
    store.configure_thread_write_rollout([]);
    let manifest = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h9ce35b9b6628 owner=fractal_seats::store::thread_request_history_tests TODO=trace-statement-and-value-representation */ include_str!("../../../../examples/fractal/custom-review.json")).unwrap();
    store.register_custom_kind(&crate::store::custom::RegistrationRequest {
        manifest, actor: AGENT.into(),
    }).unwrap();
    let opening = store.append_claim(&ClaimInput {
        subject: "custom/garden/review/v1/history".into(),
        kind: "custom.garden.review.v1.requested".into(), actor: Some(AGENT.into()),
        fields: serde_json::from_value(json!({
            "title": "Review", "detail": "Choose the next step", "recipient": PERSON,
        })).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: Some("history-custom-open".into()),
    }).unwrap();
    let origin = SourceEpisode { source_id: opening.subject, episode: opening.id };
    start(&store, &origin, THREAD, AGENT);
    let history = page(&store);
    assert!(matches!(history.source_support, Some(HistorySourceSupport::UnsupportedSource)));
    assert!(history.items.is_empty());
    assert!(history.relations.is_empty());
    assert!(history.boundary_items.is_empty());
    assert!(history.source_episodes.is_empty());
    assert!(history.next_cursor.is_none());
    assert!(history.complete);
    assert_eq!(thread_request_history(&store, "person/other", THREAD, None, 1).unwrap_err().code, "thread-unavailable");
    store.append_claim(&ClaimInput {
        subject: origin.source_id.clone(), kind: "custom.garden.review.v1.answered".into(),
        actor: Some(PERSON.into()),
        fields: serde_json::from_value(json!({
            "request": origin.episode, "selection": "keep", "text": "Proceed.",
        })).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: Some("history-custom-close".into()),
    }).unwrap();
    let retained = thread_request_history(&store, PERSON, THREAD, None, 1).unwrap();
    assert!(matches!(retained.source_support, Some(HistorySourceSupport::UnsupportedSource)));
    assert!(retained.items.is_empty());
}

#[test]
fn request_history_denied_predecessor_does_not_disclose_edge_or_boundary() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let seed = ask(&store, &input, "predecessor-visible-seed");
    start(&store, &seed, THREAD, AGENT);
    finish(&store, &seed, false);
    let hidden = ask(&store, &input, "predecessor-to-reassign");
    attach(&store, &hidden, None, "parent-membership");
    let child = ask(&store, &input, "predecessor-visible-child");
    attach(&store, &child, Some(&hidden), "hidden-parent-reason");
    finish(&store, &child, false);
    rename_live(&store);
    let history = page(&store);
    assert_eq!(keys(&history), BTreeSet::from([seed, child]));
    assert!(history.relations.is_empty());
    assert!(history.boundary_items.is_empty());
    assert!(history.next_cursor.is_none());
    assert!(history.complete);
    let serialized = serde_json::to_string(&history).unwrap();
    for secret in [hidden.source_id.as_str(), hidden.episode.as_str(), "hidden-parent-reason"] {
        assert!(!serialized.contains(secret));
    }
}

#[test]
fn retained_runtime_discussion_starts_after_resolution_without_reopening_attention_and_keeps_links() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let seed = ask(&store, &input, "retained-runtime-seed");
    finish(&store, &seed, false);
    start(&store, &seed, THREAD, AGENT);
    assert_eq!(status(&page(&store), &seed), "completed");
    assert!(!store.attention_items(Some(PERSON)).unwrap().iter().any(|item| item.subject == seed.source_id));
    let child = ask(&store, &input, "retained-runtime-child");
    attach(&store, &child, Some(&seed), "retained-runtime-link");
    finish(&store, &child, true);
    let history = page(&store);
    assert_eq!(keys(&history), BTreeSet::from([seed.clone(), child.clone()]));
    assert_eq!(status(&history, &seed), "completed");
    assert_eq!(status(&history, &child), "cancelled");
    assert_eq!(history.relations.len(), 1);
    assert_eq!(history.relations[0].predecessor, seed);
    assert_eq!(history.relations[0].request, child);
    assert!(!store.attention_items(Some(PERSON)).unwrap().iter()
        .any(|item| item.subject == seed.source_id || item.subject == child.source_id));
}

#[test]
fn retained_runtime_start_rejects_reassigned_away_and_never_opened_episodes() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let seed = ask(&store, &input, "retained-runtime-reassigned");
    rename_live(&store);
    let message = ClaimInput {
        subject:"message/denied-retained-runtime".into(), kind:"message.sent".into(), actor:Some(PERSON.into()),
        fields:serde_json::from_value(json!({"from":PERSON,"to":AGENT,"content":"Denied discussion","status":"sent"})).unwrap(),
        evidence:vec![], expected_subject:None, idempotency_key:None,
    };
    for (key, episode) in [("reassigned", seed.episode.as_str()), ("never-opened", "claim/never-opened")] {
        let origin = InboxOrigin { source_id:seed.source_id.clone(), episode:episode.into() };
        let error = start_from_inbox(&store, StartFromInbox {
            thread:"thread/denied-retained-runtime", title:"Denied", person:PERSON, origin:&origin,
            key, first_message:&message,
        }).unwrap_err();
        assert_eq!(error.code, "thread-unavailable");
    }
    assert!(store.claims_for("thread/denied-retained-runtime", None).unwrap().is_empty());
    assert!(store.claims_for(&message.subject, None).unwrap().is_empty());
}

#[test]
fn request_history_concurrent_cross_thread_attachments_have_one_canonical_membership() {
    let (baseline, _, input) = fixture();
    baseline.configure_thread_write_rollout([]);
    let other = "thread/concurrent-history";
    let mut expected = BTreeMap::new();
    for (index, thread) in [THREAD, other].into_iter().enumerate() {
        let seed = ask(&baseline, &input, &format!("concurrent-seed-{index}"));
        start(&baseline, &seed, thread, AGENT);
        let mut members = BTreeSet::from([seed]);
        for member in 0..3 {
            let child = ask(&baseline, &input, &format!("concurrent-member-{index}-{member}"));
            attach_ask(&baseline, AGENT, AttachAsk {
                thread, ask: &child.source_id, episode: &child.episode,
                key: &format!("concurrent-link-{index}-{member}"), decision: None,
            }).unwrap();
            members.insert(child);
        }
        expected.insert(thread, members);
    }
    let shared = ask(&baseline, &input, "concurrent-unattached");
    let left = Store::open_memory("birch").unwrap();
    let right = Store::open_memory("cedar").unwrap();
    let forward = Store::open_memory("dogwood").unwrap();
    let reverse = Store::open_memory("elm").unwrap();
    for replica in [&left, &right, &forward, &reverse] {
        replica.configure_thread_write_rollout([]);
        let exchange = baseline.export_replication_exchange_answering(
            TEST_FLEET, &replica.replication_inventory().unwrap(), &[],
        ).unwrap();
        receive_exchange(replica, &exchange);
    }
    let (left_receipt, left_appended) = attach_ask(&left, AGENT, AttachAsk {
        thread: THREAD, ask: &shared.source_id, episode: &shared.episode,
        key: "concurrent-attach-left", decision: None,
    }).unwrap();
    let (right_receipt, right_appended) = attach_ask(&right, AGENT, AttachAsk {
        thread: other, ask: &shared.source_id, episode: &shared.episode,
        key: "concurrent-attach-right", decision: None,
    }).unwrap();
    assert!(left_appended && right_appended);
    assert_ne!(left_receipt.id, right_receipt.id);
    assert_eq!(left_receipt.subject, right_receipt.subject);
    let left_exchange = left.export_replication_exchange_answering(
        TEST_FLEET, &forward.replication_inventory().unwrap(), &[],
    ).unwrap();
    let right_exchange = right.export_replication_exchange_answering(
        TEST_FLEET, &reverse.replication_inventory().unwrap(), &[],
    ).unwrap();
    receive_exchange(&forward, &left_exchange);
    receive_exchange(&forward, &right_exchange);
    receive_exchange(&reverse, &right_exchange);
    receive_exchange(&reverse, &left_exchange);
    receive_exchange(&left, &right_exchange);
    receive_exchange(&right, &left_exchange);

    let canonical = forward.latest_claim(&left_receipt.subject, Some("thread.ask-linked"))
        .unwrap().unwrap();
    let winner = canonical.body["fields"]["thread_id"].as_str().unwrap();
    assert!([THREAD, other].contains(&winner));
    expected.get_mut(winner).unwrap().insert(shared.clone());
    for replica in [&left, &right, &forward, &reverse] {
        assert_eq!(
            replica.latest_claim(&left_receipt.subject, Some("thread.ask-linked"))
                .unwrap().unwrap().id,
            canonical.id,
        );
        for thread in [THREAD, other] {
            let unpaged = thread_request_history(replica, PERSON, thread, None, 50).unwrap();
            assert_eq!(keys(&unpaged), expected[thread]);
            assert_eq!(keys(&unpaged).contains(&shared), thread == winner);
            assert_eq!(
                serde_json::to_value(&unpaged).unwrap(),
                serde_json::to_value(thread_request_history(&forward, PERSON, thread, None, 50).unwrap()).unwrap(),
            );
            let mut cursor = None;
            let mut traversed = Vec::new();
            loop {
                let history = thread_request_history(replica, PERSON, thread, cursor.as_deref(), 1).unwrap();
                assert_eq!(history.items.len(), 1);
                traversed.push(history.items[0].key.clone());
                assert!(history.complete);
                cursor = history.next_cursor;
                if cursor.is_none() { break; }
                assert!(traversed.len() < unpaged.items.len(), "every emitted cursor must advance");
            }
            assert_eq!(traversed, unpaged.items.iter().map(|item| item.key.clone()).collect::<Vec<_>>());
        }
    }
}

#[test]
fn request_history_denied_heavy_keyset_pages_match_visible_only_metadata() {
    let (store, _, input) = fixture();
    store.configure_thread_write_rollout([]);
    let seed = ask(&store, &input, "dense-visible-seed");
    start(&store, &seed, THREAD, AGENT);
    finish(&store, &seed, false);
    let mut candidates = (0..256)
        .map(|index| ask(&store, &input, &format!("dense-candidate-{index}")))
        .collect::<Vec<_>>();
    candidates.sort();
    let mut visible = BTreeSet::from([seed]);
    // Keep visible rows on both sides of long denied runs, including late pages.
    for index in [0, 51, 102, 153, 204, 255] {
        let key = &candidates[index];
        attach(&store, key, None, &format!("dense-visible-link-{index}"));
        finish(&store, key, false);
        visible.insert(key.clone());
    }
    let control = Store::open_memory("birch").unwrap();
    let baseline = store.export_replication_exchange_answering(
        TEST_FLEET, &control.replication_inventory().unwrap(), &[],
    ).unwrap();
    receive_exchange(&control, &baseline);
    let hidden = candidates.iter().filter(|key| !visible.contains(*key)).collect::<Vec<_>>();
    assert_eq!(hidden.len(), 250);
    for (index, key) in hidden.iter().enumerate() {
        attach(&store, key, None, &format!("dense-hidden-link-{index}"));
    }
    // Reassignment denies only the still-live episodes, not the completed visible rows.
    rename_live(&store);
    rename_live(&control);
    let unpaged = page(&store);
    assert_eq!(keys(&unpaged), visible);
    assert_eq!(serde_json::to_value(&unpaged).unwrap(), serde_json::to_value(page(&control)).unwrap());
    for limit in [1, 2, 3] {
        let mut cursor = None;
        let mut traversed = Vec::new();
        loop {
            let history = thread_request_history(&store, PERSON, THREAD, cursor.as_deref(), limit).unwrap();
            let reference = thread_request_history(&control, PERSON, THREAD, cursor.as_deref(), limit).unwrap();
            assert_eq!(serde_json::to_value(&history).unwrap(), serde_json::to_value(reference).unwrap());
            assert!(!history.items.is_empty());
            assert!(history.complete);
            let serialized = serde_json::to_string(&history).unwrap();
            for key in &hidden {
                assert!(!serialized.contains(&key.source_id));
                assert!(!serialized.contains(&key.episode));
            }
            traversed.extend(history.items.iter().map(|item| item.key.clone()));
            cursor = history.next_cursor;
            if cursor.is_none() { break; }
            assert!(traversed.len() < visible.len(), "denied rows must not create extra pages");
        }
        assert_eq!(traversed, unpaged.items.iter().map(|item| item.key.clone()).collect::<Vec<_>>());
    }

    let connection = store.readers.get();
    let mut statement = connection.prepare(
        "EXPLAIN QUERY PLAN
         SELECT json_extract(body, '$.fields.ask'), json_extract(body, '$.fields.ask_episode')
         FROM claims INDEXED BY claims_thread_history_link_index
         WHERE kind='thread.ask-linked' AND json_extract(body, '$.fields.thread_id')=?1
           AND (json_extract(body, '$.fields.ask'), json_extract(body, '$.fields.ask_episode'))>(?2,?3)
         ORDER BY json_extract(body, '$.fields.ask'), json_extract(body, '$.fields.ask_episode')",
    ).unwrap();
    let anchor = &candidates[153];
    let plan = statement.query_map(params![THREAD, anchor.source_id, anchor.episode], |row| row.get::<_, String>(3))
        .unwrap().collect::<Result<Vec<_>, _>>().unwrap().join("\n");
    assert!(plan.contains("SEARCH claims USING INDEX claims_thread_history_link_index"), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    assert!(!plan.contains("UNION"), "{plan}");
}
