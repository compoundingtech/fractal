//! The part of `store::adhoc_work` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::MissionsStore;
use super::*;
use crate::model::{WorkAcknowledgeRequest, WorkHandoffRequest, WorkStartRequest};
pub use fractal_missions::store::adhoc_work::*;
pub use fractal_missions::store::adhoc_work::{enrich_handoff, is_adhoc, pending_handoff, project_start, transfer_tx, validate_recipient, validate_retry};

#[cfg(test)]
mod tests {
    use super::super::tests::{exchange_from, receive_and_project};
    use super::*;
    use fractal_missions::store::adhoc_work_tests_shared_seats::*;
    use fractal_missions::store::adhoc_work_tests_shared_seats::{request};

    #[test]
    fn handoff_replicates_and_each_transfer_requires_its_own_acknowledgment() {
        let source = Store::open_memory("alder").unwrap();
        let target = Store::open_memory("birch").unwrap();
        let intent = crate::graph::parse_internal_intent("version 2\nagent \"example/alder\" { workspace \"/tmp\"; command \"true\" }\nagent \"example/birch\" { workspace \"/tmp\"; command \"true\" }", "alder").unwrap();
        source.apply_internal(&intent, "fixture-seats").unwrap();
        let start = WorkStartRequest {
            actor: "agent/example/alder".into(),
            title: "Inspect a fixture".into(),
            idempotency_key: "start-fixture".into(),
        };
        let work = source.start_work(&start).unwrap();
        source
            .work_action(
                &work.subject,
                "claim",
                &request(&start.actor, "claim-alder"),
            )
            .unwrap();
        let eta = fractal_store::work_eta::summary(
            Some("Fixture inspected"),
            "2h",
            &start.actor,
            now_ms(),
        )
        .unwrap();
        let mut progress = request(&start.actor, "progress-eta");
        progress.summary = Some(eta.clone());
        source.work_action(&work.subject, "progress", &progress).unwrap();
        let unchanged = source
            .work_action(
                &work.subject,
                "progress",
                &request(&start.actor, "progress-without-summary"),
            )
            .unwrap();
        assert_eq!(unchanged.progress_summary.as_deref(), Some(eta.as_str()));
        let input = WorkHandoffRequest {
            actor: start.actor.clone(),
            incarnation: Some(format!("{}-one", start.actor)),
            to: "agent/example/birch".into(),
            note: "Inspect the fixture next".into(),
            evidence: vec!["file:fixture".into()],
            idempotency_key: "handoff-birch".into(),
        };
        for recipient in [
            "agent/missing",
            "person/",
            "work/other",
            "agent/example/alder",
        ] {
            let mut invalid = input.clone();
            invalid.to = recipient.into();
            assert!(source.handoff_work(&work.subject, &invalid).is_err());
            assert_eq!(
                source
                    .step_run(&work.subject)
                    .unwrap()
                    .unwrap()
                    .claimant
                    .as_deref(),
                Some(start.actor.as_str())
            );
            assert!(source.messages(None, true).unwrap().is_empty());
            assert_eq!(
                source.step_run(&work.subject).unwrap().unwrap().progress_summary.as_deref(),
                Some(eta.as_str())
            );
        }
        let mut empty = input.clone();
        empty.note = " ".into();
        assert_eq!(
            source.handoff_work(&work.subject, &empty).unwrap_err().code,
            "handoff-needs-note"
        );
        assert_eq!(
            source.step_run(&work.subject).unwrap().unwrap().progress_summary.as_deref(),
            Some(eta.as_str())
        );
        let released = source.handoff_work(&work.subject, &input).unwrap();
        assert_eq!(released.progress_summary.as_deref(), Some("Fixture inspected"));
        assert_eq!(
            source
                .handoff_work(&work.subject, &input)
                .unwrap()
                .assigned_to
                .as_deref(),
            Some(input.to.as_str())
        );
        let mut conflicting = input.clone();
        conflicting.note = "Another note".into();
        assert_eq!(
            source
                .handoff_work(&work.subject, &conflicting)
                .unwrap_err()
                .code,
            "idempotency-conflict"
        );
        receive_and_project(
            &target,
            "alder",
            &exchange_from(&source, &ReplicationInventory::default()),
        );
        let transferred = target
            .mission_run(&work.run)
            .unwrap()
            .unwrap()
            .steps
            .remove(0);
        assert_eq!(transferred.assigned_to.as_deref(), Some(input.to.as_str()));
        assert!(transferred.claimant.is_none());
        assert_eq!(transferred.progress_summary.as_deref(), Some("Fixture inspected"));
        let message = target.messages(Some(&input.to), true).unwrap().remove(0);
        assert!(message.content.contains(&input.note));
        assert_eq!(
            target
                .work_action(
                    &work.subject,
                    "claim",
                    &request(&input.to, "claim-unacknowledged")
                )
                .unwrap_err()
                .code,
            "handoff-not-acknowledged"
        );
        target
            .acknowledge_work(
                &work.subject,
                &WorkAcknowledgeRequest {
                    actor: input.to.clone(),
                    message: message.subject.clone(),
                },
            )
            .unwrap();
        target
            .work_action(&work.subject, "claim", &request(&input.to, "claim-birch"))
            .unwrap();
        receive_and_project(
            &source,
            "birch",
            &exchange_from(&target, &ReplicationInventory::default()),
        );
        source.replay_replication_graph().unwrap();
        assert_eq!(
            source.mission_run(&work.run).unwrap().unwrap().steps[0]
                .claimant
                .as_deref(),
            Some(input.to.as_str())
        );
        let mut back = input.clone();
        back.actor = input.to.clone();
        back.incarnation = Some(format!("{}-one", input.to));
        back.to = start.actor.clone();
        back.idempotency_key = "handoff-alder".into();
        target.handoff_work(&work.subject, &back).unwrap();
        assert_eq!(
            target
                .acknowledge_work(
                    &work.subject,
                    &WorkAcknowledgeRequest {
                        actor: back.to.clone(),
                        message: message.subject
                    }
                )
                .unwrap_err()
                .code,
            "stale-handoff"
        );
        assert_eq!(
            target
                .work_action(
                    &work.subject,
                    "claim",
                    &request(&back.to, "claim-back-unacknowledged")
                )
                .unwrap_err()
                .code,
            "handoff-not-acknowledged"
        );
        let back_message = target.messages(Some(&back.to), true).unwrap().remove(0);
        target
            .acknowledge_work(
                &work.subject,
                &WorkAcknowledgeRequest {
                    actor: back.to.clone(),
                    message: back_message.subject,
                },
            )
            .unwrap();
        target
            .work_action(&work.subject, "claim", &request(&back.to, "claim-back"))
            .unwrap();
        receive_and_project(
            &source,
            "birch",
            &exchange_from(&target, &ReplicationInventory::default()),
        );
        source.replay_replication_graph().unwrap();
        let final_work = source
            .mission_run(&work.run)
            .unwrap()
            .unwrap()
            .steps
            .remove(0);
        assert_eq!(final_work.claimant.as_deref(), Some(back.to.as_str()));
        assert_eq!(final_work.assigned_to, final_work.claimant);
        assert_eq!(source.mission_runs().unwrap().len(), 1);
    }

    /// A handoff note that is a stream item is acknowledged as a graph note is, but its read is
    /// this member's record in the messages shard: no lifecycle claim reaches the graph, and the
    /// acknowledgment cites the receipt that names the read.
    #[test]
    fn a_stream_handoff_note_is_read_in_the_shard_when_acknowledged() {
        let root = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
        let shard = crate::store::messages_shard::open(&store, root.path()).unwrap();
        let intent = crate::graph::parse_internal_intent("version 2\nagent \"example/alder\" { workspace \"/tmp\"; command \"true\" }\nagent \"example/birch\" { workspace \"/tmp\"; command \"true\" }", "alder").unwrap();
        store.apply_internal(&intent, "fixture-seats").unwrap();
        let start = WorkStartRequest {
            actor: "agent/example/alder".into(),
            title: "Inspect a stream fixture".into(),
            idempotency_key: "start-stream-fixture".into(),
        };
        let work = store.start_work(&start).unwrap();
        store.work_action(&work.subject, "claim", &request(&start.actor, "claim-alder")).unwrap();
        let to = "agent/example/birch";
        store
            .handoff_work(&work.subject, &WorkHandoffRequest {
                actor: start.actor.clone(),
                incarnation: Some(format!("{}-one", start.actor)),
                to: to.into(),
                note: "Inspect the stream fixture next".into(),
                evidence: Vec::new(),
                idempotency_key: "handoff-stream".into(),
            })
            .unwrap();
        // A handoff writes its note to the graph today; point the release at a stream item.
        let note = "message/00000000000000c1";
        // Other mail to the recipient, which is not the handoff's note.
        let other = "message/00000000000000c2";
        for subject in [note, other] {
            shard
                .append_claim(&ClaimInput {
                    subject: subject.into(),
                    kind: "message.sent".into(),
                    actor: Some(start.actor.clone()),
                    fields: serde_json::from_value(json!({
                        "from": start.actor, "to": to, "content": "Inspect the stream fixture next", "status": "sent"
                    }))
                    .unwrap(),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
        store
            .connection
            .write()
            .execute(
                "UPDATE claims SET body=json_set(body,'$.fields.handoff_message',?2) WHERE subject=?1 AND kind='work.released'",
                params![work.subject, note],
            )
            .unwrap();
        // Acknowledging with other mail is refused and leaves that mail unread.
        let wrong = WorkAcknowledgeRequest { actor: to.into(), message: other.into() };
        assert_eq!(store.acknowledge_work(&work.subject, &wrong).unwrap_err().code, "stale-handoff");
        assert!(crate::store::messages_shard::mailbox_reads::lifecycle_step(&store, other, "message.read").unwrap().is_none());
        let acknowledge = WorkAcknowledgeRequest { actor: to.into(), message: note.into() };
        store.acknowledge_work(&work.subject, &acknowledge).unwrap();
        assert_eq!(store.message(note).unwrap().unwrap().status, "read");
        assert_eq!(
            crate::store::messages_shard::mailbox_reads::lifecycle_step(&store, note, "message.read")
                .unwrap()
                .map(|(actor, _)| actor),
            Some(to.to_owned())
        );
        assert!(store.claims_for(note, None).unwrap().is_empty(), "the read reached the graph");
        let receipt = crate::store::messages_shard::lifecycle_receipt(
            "alder", note, "message.read", Some(to), &BTreeMap::from([("status".to_owned(), json!("read"))]),
        );
        let progress = store
            .claims_for(&work.subject, Some("work.progress"))
            .unwrap()
            .into_iter()
            .find(|claim| claim.body["fields"]["handoff_acknowledged"] == note)
            .unwrap();
        assert_eq!(progress.body["evidence"], json!([receipt.id]));
        // A repeat finds the acknowledgment; the acknowledged recipient then claims the work.
        store.acknowledge_work(&work.subject, &acknowledge).unwrap();
        store.work_action(&work.subject, "claim", &request(to, "claim-birch")).unwrap();
        assert!(store.claims_for(note, None).unwrap().is_empty());
    }

    /// An acknowledgment the step refuses leaves a stream note unread, and the one it accepts
    /// replicates: on another member, whose graph has no read of the note, the handoff is no
    /// longer pending, so neither a claim nor a close waits on it.
    #[test]
    fn a_refused_stream_acknowledgment_reads_nothing_and_an_accepted_one_replicates() {
        let root = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
        let shard = crate::store::messages_shard::open(&store, root.path()).unwrap();
        let other = Store::open_memory("birch").unwrap();
        let intent = crate::graph::parse_internal_intent("version 2\nagent \"example/alder\" { workspace \"/tmp\"; command \"true\" }\nagent \"example/birch\" { workspace \"/tmp\"; command \"true\" }", "alder").unwrap();
        store.apply_internal(&intent, "fixture-seats").unwrap();
        let start = WorkStartRequest {
            actor: "agent/example/alder".into(),
            title: "Inspect a refused stream fixture".into(),
            idempotency_key: "start-refused-stream".into(),
        };
        let work = store.start_work(&start).unwrap();
        store.work_action(&work.subject, "claim", &request(&start.actor, "claim-alder")).unwrap();
        let to = "agent/example/birch";
        store
            .handoff_work(&work.subject, &WorkHandoffRequest {
                actor: start.actor.clone(),
                incarnation: Some(format!("{}-one", start.actor)),
                to: to.into(),
                note: "Inspect the refused stream fixture next".into(),
                evidence: Vec::new(),
                idempotency_key: "handoff-refused-stream".into(),
            })
            .unwrap();
        let note = "message/00000000000000d1";
        shard
            .append_claim(&ClaimInput {
                subject: note.into(),
                kind: "message.sent".into(),
                actor: Some(start.actor.clone()),
                fields: serde_json::from_value(json!({
                    "from": start.actor, "to": to, "content": "Inspect the refused stream fixture next", "status": "sent"
                }))
                .unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        store
            .connection
            .write()
            .execute(
                "UPDATE claims SET body=json_set(body,'$.fields.handoff_message',?2) WHERE subject=?1 AND kind='work.released'",
                params![work.subject, note],
            )
            .unwrap();
        let acknowledge = WorkAcknowledgeRequest { actor: to.into(), message: note.into() };
        let read = || crate::store::messages_shard::mailbox_reads::lifecycle_step(&store, note, "message.read").unwrap();
        // The note is the current handoff to its reader, but the step is no longer waiting for it.
        let set_status = |status: &str| {
            store.connection.write().execute("UPDATE step_runs SET status=?2 WHERE subject=?1", params![work.subject, status]).unwrap();
        };
        set_status("blocked");
        assert_eq!(store.acknowledge_work(&work.subject, &acknowledge).unwrap_err().code, "stale-handoff");
        assert!(read().is_none(), "a refused acknowledgment read the note");
        set_status("ready");
        store.acknowledge_work(&work.subject, &acknowledge).unwrap();
        assert_eq!(read().map(|(actor, _)| actor), Some(to.to_owned()));
        // A read lost after the acknowledgment committed is recorded by the repeat its error asks for.
        shard.connection.write().execute("DELETE FROM stream_lifecycle WHERE subject=?1", [note]).unwrap();
        assert!(read().is_none());
        store.acknowledge_work(&work.subject, &acknowledge).unwrap();
        assert_eq!(read().map(|(actor, _)| actor), Some(to.to_owned()));
        // The acknowledgment is a graph claim; the read is not. Another member needs only the first.
        receive_and_project(&other, "alder", &exchange_from(&store, &ReplicationInventory::default()));
        assert!(other.claims_for(note, None).unwrap().is_empty());
        let replicated = other.step_run(&work.subject).unwrap().unwrap();
        assert!(!crate::store::adhoc_work::pending_handoff(&other.readers.get(), &replicated).unwrap(), "the handoff stays pending elsewhere");
    }
}
