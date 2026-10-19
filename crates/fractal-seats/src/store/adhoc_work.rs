//! The part of `store::adhoc_work` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::MissionsStore;
use super::*;
use crate::model::{WorkAcknowledgeRequest, WorkHandoffRequest, WorkStartRequest};
pub use st3_missions::store::adhoc_work::*;
pub use st3_missions::store::adhoc_work::{enrich_handoff, is_adhoc, pending_handoff, project_start, transfer_tx, validate_recipient, validate_retry};

#[cfg(test)]
mod tests {
    use super::super::tests::{exchange_from, receive_and_project};
    use super::*;
    use st3_missions::store::adhoc_work_tests_shared_seats::*;
    use st3_missions::store::adhoc_work_tests_shared_seats::{request};

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
        }
        let mut empty = input.clone();
        empty.note = " ".into();
        assert_eq!(
            source.handoff_work(&work.subject, &empty).unwrap_err().code,
            "handoff-needs-note"
        );
        source.handoff_work(&work.subject, &input).unwrap();
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
}
