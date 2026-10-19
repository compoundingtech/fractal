#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_missions::{CARRY_SOURCE, carry_request, listed, misproject_a_step, publish_carry, request_fault};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{TAKEOVER_SOURCE, TREE_SHAPE_RUNS, TreeShapes, full_canonical_claim_id_oracle, graph_digest_of, insert_raw_claims, local_observation_for_test};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{local_timeline_observation_for_test, observe_runtime, projection_replayed, publish_mission, publish_takeover, query_plan, replicate_fully, roster_cache_store, simple};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{exchange_of, harness_state, takeover_step, timeline_observation};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use crate::DeliveryStore;
use super::*;
use crate::graph::parse_test_intent as parse_intent;
use proptest::prelude::*;


#[test]
fn subscription_retry_queries_use_history_indexes() {
    let store = Store::open_memory("node").unwrap();
    let subject = "subscription/reviews";
    let first = now_ms() as u64 + 10_000;
    let second = first + 20_000;
    for (request, deadline) in [
        ("request-a", first),
        ("request-b", first),
        ("request-a", second),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "subscription.mission-deferred".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("request".into(), Value::String(request.into())),
                    ("not_before_unix_ms".into(), Value::from(deadline)),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    assert_eq!(
        store
            .subscription_mission_deferral(subject, "request-a")
            .unwrap(),
        Some((u128::from(second), 2)),
    );
    assert_eq!(
        store
            .subscription_mission_deferral(subject, "request-b")
            .unwrap(),
        Some((u128::from(first), 1)),
    );
    assert_eq!(
        store
            .subscription_mission_deferral(subject, "missing")
            .unwrap(),
        None,
    );
    let before = store.index().unwrap();
    let third = second + 30_000;
    store
        .record_subscription_mission_deferral(subject, "request-a", third.into(), 3)
        .unwrap();
    assert_eq!(store.index().unwrap(), before);
    assert_eq!(
        store
            .subscription_mission_deferral(subject, "request-a")
            .unwrap(),
        Some((third.into(), 3)),
    );
    assert_eq!(
        store.next_subscription_mission_retry_deadline().unwrap(),
        Some(first.into()),
    );
    let connection = store.readers.get();
    let request_plan: String = connection
        .query_row(
            "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM claims
                 WHERE subject=?1 AND kind='subscription.mission-deferred'
                   AND json_extract(body, '$.fields.request')=?2",
            params![subject, "request-a"],
            |row| row.get(3),
        )
        .unwrap();
    assert!(
        request_plan.contains("claims_subscription_deferred_request_index"),
        "{request_plan}"
    );
    let deadline_plan: String = connection
        .query_row(
            "EXPLAIN QUERY PLAN SELECT MIN(CAST(json_extract(body, '$.fields.not_before_unix_ms') AS INTEGER))
                 FROM claims INDEXED BY claims_subscription_deferred_deadline_index
                 WHERE kind='subscription.mission-deferred'
                   AND CAST(json_extract(body, '$.fields.not_before_unix_ms') AS INTEGER)>?1",
            [now_ms() as i64],
            |row| row.get(3),
        )
        .unwrap();
    assert!(
        deadline_plan.contains("claims_subscription_deferred_deadline_index"),
        "{deadline_plan}"
    );
}

#[test]
fn local_subscription_deferral_survives_a_daemon_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite3");
    let deadline = now_ms() + 30_000;
    {
        let store = Store::open(&path, "node").unwrap();
        let before = store.index().unwrap();
        store
            .record_subscription_mission_deferral(
                "subscription/reviews",
                "request-a",
                deadline,
                7,
            )
            .unwrap();
        assert_eq!(store.index().unwrap(), before);
    }
    let reopened = Store::open(&path, "node").unwrap();
    assert_eq!(
        reopened
            .subscription_mission_deferral("subscription/reviews", "request-a")
            .unwrap(),
        Some((deadline, 7)),
    );
    assert_eq!(
        reopened.next_subscription_mission_retry_deadline().unwrap(),
        Some(deadline),
    );
}

#[test]
fn quiet_claim_detection_requires_only_usage_or_renewal_since_cursor() {
    let store = Store::open_memory("node").unwrap();
    let before = store.index().unwrap();
    assert!(!store.claims_since_only_quiet_notifications(before).unwrap());
    let append = |kind| {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let fields = if kind == "harness.usage" {
            json!({"driver": "codex", "incarnation_id": "current", "semantics": "response", "total_tokens": 1})
        } else if kind == "harness.observed" {
            json!({"driver": "codex", "incarnation_id": "current", "state": "ready"})
        } else {
            json!({
                "status": "claimed", "attempt": 1, "readiness_epoch": 1,
                "claimant": "agent/node.test", "claim_incarnation": "current",
                "claim_expires_at_unix_ms": now_ms().saturating_add(60_000),
                "worker_reported": false
            })
        };
        append_claim_tx(
            &transaction,
            &store.origin,
            if kind == "work.renewed" {
                "step-run/test/work"
            } else {
                "agent/node.test"
            },
            kind,
            Some("agent/node.test"),
            &json!({"fields": fields}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    };
    append("harness.usage");
    append("work.renewed");
    assert!(store.claims_since_only_quiet_notifications(before).unwrap());
    let after_quiet = store.index().unwrap();
    append("harness.observed");
    assert!(!store.claims_since_only_quiet_notifications(before).unwrap());
    assert!(
        !store
            .claims_since_only_quiet_notifications(after_quiet)
            .unwrap()
    );
}

#[test]
fn only_work_wake_messages_count_as_reconciler_changes() {
    let store = Store::open_memory("node").unwrap();
    let append = |subject: &str, kind: &str, fields: Value| {
        let mut connection = store.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        append_claim_tx(
            &transaction,
            &store.origin,
            subject,
            kind,
            Some("agent/node.test"),
            &json!({"fields": fields}),
            &[],
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
    };
    let sent = |tags: Value| {
        json!({"from": "person/example", "to": "agent/node.test", "content": "A note.",
               "status": "sent", "tags": tags})
    };
    // A conversation message and its lifecycle never change what the reconciler decides.
    let before = store.index().unwrap();
    append("message/talk", "message.sent", sent(json!(["chat"])));
    for status in ["delivered", "read", "closed"] {
        append(
            "message/talk",
            &format!("message.{status}"),
            json!({"status": status}),
        );
    }
    assert!(store.claims_since_only_quiet_notifications(before).unwrap());
    // A work wake does, and so does the lifecycle of a message this store has not seen sent.
    let before = store.index().unwrap();
    append(
        "message/wake",
        "message.sent",
        sent(json!(["st3-work:step-run/test/work"])),
    );
    assert!(!store.claims_since_only_quiet_notifications(before).unwrap());
    let before = store.index().unwrap();
    append("message/wake", "message.read", json!({"status": "read"}));
    assert!(!store.claims_since_only_quiet_notifications(before).unwrap());
    let before = store.index().unwrap();
    append("message/unseen", "message.read", json!({"status": "read"}));
    assert!(!store.claims_since_only_quiet_notifications(before).unwrap());
}

#[test]
fn native_mailbox_pages_use_exact_recipient_and_stable_created_cursors() {
    let store = Store::open_memory("node").unwrap();
    for (id, to) in [
        ("first", "agent/worker"),
        ("other", "agent/other"),
        ("second", "agent/worker"),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: format!("message/{id}"),
                kind: "message.sent".into(),
                actor: Some("person/test".into()),
                fields: BTreeMap::from([
                    ("from".into(), Value::String("person/test".into())),
                    ("to".into(), Value::String(to.into())),
                    ("content".into(), Value::String(id.into())),
                    ("status".into(), Value::String("sent".into())),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("mailbox-page-{id}")),
            })
            .unwrap();
    }
    let through = store.index().unwrap();
    let (first, cursor) = store
        .messages_page(Some("agent/worker"), true, None, through, 1)
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].subject, "message/first");
    let (second, next) = store
        .messages_page(Some("agent/worker"), true, cursor, through, 1)
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].subject, "message/second");
    assert!(next.is_none());
    assert_eq!(store.messages(Some("agent/other"), true).unwrap().len(), 1);

    for (kind, status) in [
        ("message.staged", "staged"),
        ("message.delivered", "delivered"),
        ("message.read", "read"),
        ("message.closed", "closed"),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: "message/first".into(),
                kind: kind.into(),
                actor: Some("agent/worker".into()),
                fields: BTreeMap::from([("status".into(), Value::String(status.into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("mailbox-first-{status}")),
            })
            .unwrap();
    }
    let (open, next) = store
        .messages_page(Some("agent/worker"), false, None, store.index().unwrap(), 1)
        .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].subject, "message/second");
    assert!(next.is_none());
    assert_eq!(
        store.message("message/first").unwrap().unwrap().status,
        "closed"
    );
    let (closed, _) = store
        .messages_page(Some("agent/worker"), true, None, store.index().unwrap(), 1)
        .unwrap();
    assert_eq!(closed[0].status, "closed");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn filtered_message_pages_match_views_across_sender_and_lifecycle_boundaries(
        entries in proptest::collection::vec((0usize..4, 0usize..3, 0usize..5, any::<bool>(), 0usize..3), 24..72),
        page_size in 1usize..8,
        selected_sender in 0usize..4,
        include_closed in any::<bool>(),
    ) {
        let store = Store::open_memory("node").unwrap();
        let titles = ["topic", "topic_%*", "topic_%* child", "other", r"topic_\%*"];
        let mut declarations = String::from(
            "version 2\nagent \"worker\" {\n workspace \"/work\"\n command \"sleep 60\"\n restart \"never\"\n}\n",
        );
        for sender in 0..4 {
            declarations.push_str(&format!(
                "agent \"sender-{sender}\" {{\n workspace \"/work\"\n command \"sleep 60\"\n restart \"never\"\n}}\n",
            ));
        }
        for (index, (sender, _, title, _, flavor)) in entries.iter().enumerate() {
            if *flavor == 2 {
                declarations.push_str(&format!(
                    "message \"filter-{index}\" {{\n from \"agent/sender-{sender}\"\n to \"worker\"\n title {}\n content \"example\"\n}}\n",
                    serde_json::to_string(titles[*title]).unwrap(),
                ));
            }
        }
        let intent = crate::graph::parse_test_intent(&declarations, "local").unwrap();
        let mission = store.mission(&intent, IntentInput {
            kdl: declarations.clone(), source_name: None,
        }).unwrap();
        store.apply(&intent, &mission.subject_tokens, "message").unwrap();
        for (index, (sender, lifecycle, title, archived, flavor)) in entries.iter().enumerate() {
            let resource = format!("message/filter-{index}");
            if *flavor != 2 {
                store.append_claim(&ClaimInput {
                    subject: resource.clone(),
                    kind: "message.sent".into(),
                    actor: Some(format!("agent/sender-{sender}")),
                    fields: BTreeMap::from([
                        ("from".into(), json!(format!("agent/sender-{sender}"))),
                        ("to".into(), json!("agent/worker")),
                        ("title".into(), json!(titles[*title])),
                        ("content".into(), json!("example")),
                        ("status".into(), json!("sent")),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("filter-send-{index}")),
                }).unwrap();
                if *flavor == 1 {
                    // Legacy imported sends store their fields at the body root.
                    store.connection.lock().unwrap().execute(
                        /* sqlite-large-value: allow-for-now lv-h7a48e60364ca owner=fractal_delivery::store::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_extract(body,'$.fields')
                         WHERE subject=?1 AND kind='message.sent'",
                        [&resource],
                    ).unwrap();
                }
            }
            let target = if *archived || *lifecycle == 1 {
                "closed"
            } else if *lifecycle == 2 {
                "read"
            } else {
                "sent"
            };
            let mut current = store.message(&resource).unwrap().expect("generated message");
            prop_assert_eq!(current.status.as_str(), "sent");
            while current.status != target {
                // All sources follow the same admitted transport lifecycle. Do not skip
                // delivery for agent recipients, even when the generated target is read.
                let next = match current.status.as_str() {
                    "sent" | "staged" => "delivered",
                    "delivered" => "read",
                    "read" => "closed",
                    status => panic!("unexpected generated message status {status}"),
                };
                store.append_claim(&ClaimInput {
                    subject: resource.clone(),
                    kind: format!("message.{next}"),
                    actor: Some("agent/worker".into()),
                    fields: BTreeMap::from([("status".into(), json!(next))]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("filter-state-{index}-{next}")),
                }).unwrap();
                current = store.message(&resource).unwrap().expect("transitioned message");
                prop_assert_eq!(current.status.as_str(), next);
            }
            prop_assert_eq!(current.status.as_str(), target);
        }
        let through = store.index().unwrap();
        let sender = format!("sender-{selected_sender}");
        let normalized_sender = format!("agent/{sender}");
        for recipient in [Some("worker"), None] {
            // Keep the existing listing as the oracle for each discovery domain.
            let all = store.messages(recipient, true).unwrap();
            for (index, (_, _, _, _, flavor)) in entries.iter().enumerate() {
                if *flavor == 1 {
                    let resource = format!("message/filter-{index}");
                    prop_assert_eq!(
                        all.iter().any(|message| message.subject == resource),
                        recipient.is_none(),
                    );
                }
            }
            for (exact, prefix) in [
                (None, None),
                (Some("topic_%*"), None),
                (None, Some("topic_%*")),
                (Some("topic"), None),
                (None, Some("")),
                (None, Some("TOPIC")),
                (Some(r"topic_\%*"), None),
                (None, Some(r"topic_\")),
            ] {
                let mut expected: Vec<_> = all.iter().filter(|message| {
                    message.from == normalized_sender
                        && (include_closed || message.status != "closed")
                        && exact.is_none_or(|title| message.title.as_deref() == Some(title))
                        && prefix.is_none_or(|prefix| message.title.as_deref().is_some_and(|title| title.starts_with(prefix)))
                }).map(|message| (message.created_index, message.subject.clone())).collect();
                expected.sort();
                let mut actual = Vec::new();
                let mut after = None;
                loop {
                    let (page, next) = store.messages_page_filtered(
                        recipient, Some(&sender), exact, prefix, include_closed, after, through, page_size,
                    ).unwrap();
                    prop_assert!(page.len() <= page_size);
                    if next.is_some() {
                        prop_assert_eq!(page.len(), page_size);
                        prop_assert_eq!(next, page.last().map(|message| message.created_index));
                        prop_assert!(next > after);
                    }
                    actual.extend(page.into_iter().map(|message| (message.created_index, message.subject)));
                    match next {
                        Some(next) => after = Some(next),
                        None => break,
                    }
                    prop_assert!(actual.len() <= expected.len());
                }
                prop_assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn reconciler_work_mailbox_keeps_closed_attempts_without_unrelated_messages() {
    let store = Store::open_memory("node").unwrap();
    for (id, to, content, tags) in [
        (
            "wake-closed",
            "agent/worker",
            "ready",
            vec!["st3-work:step-run/one@1@1@worker"],
        ),
        (
            "ordinary",
            "agent/worker",
            "mentions st3-work: but has no work tag",
            vec![],
        ),
        (
            "wake-open",
            "agent/worker",
            "ready again",
            vec!["st3-work:step-run/two@1@1@worker"],
        ),
        (
            "other-agent",
            "agent/other",
            "ready",
            vec!["st3-work:step-run/other@1@1@other"],
        ),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: format!("message/{id}"),
                kind: "message.sent".into(),
                actor: Some("daemon/runtime".into()),
                fields: BTreeMap::from([
                    ("from".into(), Value::String("daemon/runtime".into())),
                    ("to".into(), Value::String(to.into())),
                    ("content".into(), Value::String(content.into())),
                    ("status".into(), Value::String("sent".into())),
                    (
                        "tags".into(),
                        Value::Array(
                            tags.into_iter()
                                .map(|tag| Value::String(tag.into()))
                                .collect(),
                        ),
                    ),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("work-mailbox-{id}")),
            })
            .unwrap();
    }
    for (kind, status) in [
        ("message.staged", "staged"),
        ("message.delivered", "delivered"),
        ("message.read", "read"),
        ("message.closed", "closed"),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: "message/wake-closed".into(),
                kind: kind.into(),
                actor: Some("agent/worker".into()),
                fields: BTreeMap::from([("status".into(), Value::String(status.into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("closed-work-mailbox-{status}")),
            })
            .unwrap();
    }

    let expected = store
        .messages(Some("agent/worker"), true)
        .unwrap()
        .into_iter()
        .filter(|message| message.tags.iter().any(|tag| tag.starts_with("st3-work:")))
        .map(|message| (message.subject, message.status))
        .collect::<Vec<_>>();
    let actual = store
        .work_wake_messages_for_reconcile("agent/worker")
        .unwrap()
        .into_iter()
        .map(|message| (message.subject, message.status))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 2);
    assert_eq!(actual[0].1, "closed");
    assert_eq!(actual[1].1, "sent");
}

#[test]
fn desired_messages_use_canonical_agent_parties_in_views() {
    let store = Store::open_memory("node").unwrap();
    let intent = crate::graph::parse_test_intent(
        r#"
version 2

  agent "mix.sup" {
    workspace "/work"
    command "sleep 60"
    restart "never"
  }
  message "kickoff" {
    from "requester"
    to "mix.sup"
    content "work"
    title "topic_%* child"
  }

"#,
        "local",
    )
    .unwrap();
    let mission = store
        .mission(
            &intent,
            IntentInput {
                kdl: "message".into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &mission.subject_tokens, "message")
        .unwrap();

    let messages = store.messages(Some("agent/mix.sup"), true).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].from, "requester");
    assert_eq!(messages[0].to, "agent/mix.sup");
    let (page, next) = store
        .messages_page(
            Some("agent/mix.sup"),
            false,
            None,
            store.index().unwrap(),
            1,
        )
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].subject, messages[0].subject);
    assert!(next.is_none());
    for (exact, prefix) in [
        (Some("topic_%* child"), None),
        (None, Some("topic_%*")),
        (None, Some("")),
    ] {
        let (filtered, next) = store.messages_page_filtered(
            Some("mix.sup"), Some("requester"), exact, prefix, false,
            None, store.index().unwrap(), 1,
        ).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].subject, messages[0].subject);
        assert!(next.is_none());
    }
    let error = store.messages_page_filtered(
        Some("mix.sup"), None, Some("topic_%* child"), None, false,
        None, store.index().unwrap(), 1,
    ).unwrap_err();
    assert_eq!(error.to_string(), "subject filters need a sender filter");
}

mod actual_purity_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-h383e93839ceb owner=fractal_delivery::store::tests TODO=trace-statement-and-value-representation */ include!("actual_purity_tests.rs");
}

mod projection_divergence_tests {
    use super::*;

    /* sqlite-large-value: allow-for-now lv-hd836545d0082 owner=fractal_delivery::store::tests TODO=trace-statement-and-value-representation */ include!("projection_divergence_tests.rs");
}
