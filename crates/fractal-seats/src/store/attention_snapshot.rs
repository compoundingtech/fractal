//! The part of `store::attention_snapshot` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::MissionsStore;
use crate::PersonWorkStore;
use super::*;
use crate::model::LoopSpec;
pub use st3_missions::store::attention_snapshot::*;
pub use st3_missions::store::attention_snapshot::{CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, agent, conversation_agent, native_prompt_gone_key, person_attention_item, plan_approval, resolve_native_harness_at, screen_prompt_delivered_key, screen_prompt_gone_key, screen_prompt_undelivered_key};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClaimInput, PersonStepResponse};
    use st3_missions::store::attention_snapshot_tests_shared_seats::*;
    use st3_missions::store::attention_snapshot_tests_shared_seats::{parity, step_back_in_time};

    /// A person ask joins the collection at its asking claim's acceptance, never at the
    /// waiting time it names: a migrated ask can name a time before it was asked. Fault-only
    /// clocks (observer state, reconcile faults) never move this collection.
    #[test]
    fn person_ask_collection_changes_only_at_the_asks_admission() {
        let (store, origin, input) = super::super::person_work::tests::fixture();
        let ask = store.ask_person(&input).unwrap();
        let admitted = now_ms() + 60_000;
        store
            .connection
            .lock().expect("fixture writer")
            .execute(
                "UPDATE claims SET accepted_at_unix_ms=?2,
                        body=json_set(body, '$.fields.waiting_since', ?3)
                 WHERE subject=?1 AND kind='work.person-asked'",
                params![
                    ask.subject,
                    admitted.to_string(),
                    (admitted - 3_600_000).to_string()
                ],
            )
            .unwrap();

        assert_eq!(
            store
                .attention_collection_snapshot(Some("person/avery"), admitted - 1)
                .unwrap()
                .1,
            Some(admitted)
        );
        assert_eq!(
            store
                .attention_collection_snapshot(None, admitted - 1)
                .unwrap()
                .1,
            Some(admitted)
        );
        assert_eq!(
            store
                .attention_collection_snapshot(Some("person/robin"), admitted - 1)
                .unwrap()
                .1,
            None
        );
        parity(&store, Some("person/avery"), admitted - 1);
        parity(&store, None, admitted - 1);
        parity(&store, Some("person/robin"), admitted - 1);
        assert!(store
            .attention_snapshot(Some("person/avery"), admitted - 1)
            .unwrap()
            .iter()
            .all(|item| item.subject != ask.subject));
        let shown = store.attention_snapshot(Some("person/avery"), admitted).unwrap();
        let item = shown
            .iter()
            .find(|item| item.subject == ask.subject)
            .expect("the ask joins the collection at its admission");
        parity(&store, Some("person/avery"), admitted);

        // Fault-collection clocks are not this collection's inputs.
        for (subject, kind, fields) in [
            (
                "observer/issues",
                "observer.state",
                json!({"state": "degraded", "reason": "limited"}),
            ),
            (
                origin.subject.as_str(),
                "reconcile.fault",
                json!({"scope": "st3.test", "status": "faulted", "reason": "stuck"}),
            ),
        ] {
            store
                .connection
                .batched(|tx| -> Result<()> {
                    append_claim_tx(
                        tx,
                        "alder",
                        subject,
                        kind,
                        Some("agent/alder.asker"),
                        &json!({"fields": fields}),
                        &[],
                        None,
                    )?;
                    Ok(())
                })
                .unwrap()
                .unwrap();
        }
        assert_eq!(
            store
                .attention_collection_snapshot(Some("person/avery"), admitted - 1)
                .unwrap()
                .1,
            Some(admitted)
        );
        assert_eq!(
            store.attention_collection_snapshot(None, admitted - 1).unwrap().1,
            Some(admitted)
        );
        parity(&store, Some("person/avery"), admitted - 1);
        parity(&store, None, admitted - 1);

        step_back_in_time(&store, &ask.subject, &(now_ms() - 1_000).to_string());
        assert_eq!(
            store
                .attention_collection_snapshot(Some("person/avery"), now_ms())
                .unwrap()
                .1,
            None
        );
        parity(&store, Some("person/avery"), now_ms());
        store
            .finish_person_step(
                &PersonStepResponse {
                    delegation: None,
                    subject: ask.subject.clone(),
                    actor: "person/avery".into(),
                    summary: "Friday".into(),
                    evidence: vec![],
                    episode: Some(item.episode.clone()),
                    idempotency_key: "fence-response".into(),
                    answer: None,
                },
                false,
            )
            .unwrap();
        assert!(store
            .attention_snapshot(Some("person/avery"), now_ms())
            .unwrap()
            .iter()
            .all(|item| item.subject != ask.subject));
        assert_eq!(
            store
                .attention_collection_snapshot(Some("person/avery"), now_ms())
                .unwrap()
                .1,
            None
        );
        parity(&store, Some("person/avery"), now_ms());
    }

    /// A ready person step without an ask joins at its activation, and falls back to its
    /// creation when it was never separately activated.
    #[test]
    fn ready_person_step_joins_at_its_activation_and_falls_back_to_its_creation() {
        let (store, origin, _input) = super::super::person_work::tests::fixture();
        let review = format!(
            "step-run/{}/review",
            origin.generation.trim_start_matches("run-generation/")
        );
        store.set_step_state(&review, "ready", None).unwrap();
        let activated = now_ms() + 120_000;
        store
            .connection
            .lock().expect("fixture writer")
            .execute(
                "UPDATE step_runs SET activated_at_unix_ms=?2 WHERE subject=?1",
                params![review.as_str(), activated.to_string()],
            )
            .unwrap();
        assert_eq!(
            store
                .attention_collection_snapshot(Some("person/avery"), activated - 1)
                .unwrap()
                .1,
            Some(activated)
        );
        parity(&store, Some("person/avery"), activated - 1);
        assert!(store
            .attention_snapshot(Some("person/avery"), activated - 1)
            .unwrap()
            .iter()
            .all(|item| item.subject != review));
        assert!(
            store
                .attention_snapshot(Some("person/avery"), activated)
                .unwrap()
                .iter()
                .any(|item| item.subject == review && item.kind == "person-step")
        );
        parity(&store, Some("person/avery"), activated);
        store
            .connection
            .lock().expect("fixture writer")
            .execute(
                "UPDATE step_runs SET activated_at_unix_ms=NULL WHERE subject=?1",
                params![review.as_str()],
            )
            .unwrap();
        assert_eq!(
            store
                .attention_collection_snapshot(Some("person/avery"), now_ms())
                .unwrap()
                .1,
            None
        );
        parity(&store, Some("person/avery"), now_ms());
        assert!(
            store
                .attention_snapshot(Some("person/avery"), now_ms())
                .unwrap()
                .iter()
                .any(|item| item.subject == review)
        );
    }
}
