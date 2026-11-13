use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use serde_json::json;

use crate::objective_faults::{ObjectiveResponse, check_with_response};
use crate::store::{AGENTS_REQUEST_SLO_MISSED, Store};

// Matches the objective emitted by Factory’s compiled response intent.
const OBJECTIVE: &str = "fractal-agents-success-fast";

pub(super) struct PendingFault {
    pub(super) id: i64,
    pub(super) reviewer: String,
    pub(super) status: String,
    pub(super) current: String,
    pub(super) through_ms: u64,
}

pub(super) fn route(store: &Store, reviewer: &str, cut: &PendingFault) -> Result<()> {
    ensure!(
        cut.reviewer == reviewer,
        "pending agent-list SLO cut belongs to another reviewer"
    );
    ensure!(
        store.fault_reviewer_is_declared(reviewer)?,
        "agent-list SLO reviewer is not an active declaration"
    );
    let row = json!({
        "id": OBJECTIVE,
        "status": cut.status,
        "current": cut.current,
        "target": "95% within 100 ms",
        "explain": format!("Seven-day local GET agents request window through Unix ms {}.", cut.through_ms),
    });
    let result = check_with_response(
        store,
        &[row],
        &BTreeSet::from([OBJECTIVE.to_owned()]),
        ObjectiveResponse {
            reviewer,
            condition: AGENTS_REQUEST_SLO_MISSED,
            title: "GET agents performance",
        },
    )
    .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    ensure!(
        result.unowned == 0,
        "agent-list SLO reviewer became unavailable"
    );
    if cut.status == "missed" {
        // A queued recovery must not hide this failure from the native sender.
        // Keep the cut pending if delivery fails; the sender's episode key makes
        // a retry after delivery but before the ledger acknowledgement harmless.
        let episode = crate::objective_faults::episode(store.origin(), OBJECTIVE);
        let failure = store
            .operational_failure(&episode)?
            .ok_or_else(|| anyhow::anyhow!("agent-list SLO failure is unavailable for delivery"))?;
        let fault = store
            .fault_snapshot(crate::gate_check::now_ms())?
            .into_iter()
            .find(|fault| fault.item.episode == failure.request)
            .ok_or_else(|| anyhow::anyhow!("agent-list SLO failure is absent from native delivery"))?;
        ensure!(
            fault.owner.as_deref() == Some(failure.reviewer.as_str()),
            "agent-list SLO failure has no native reviewer"
        );
        crate::reconcile::append_fault_message(store, &failure.reviewer, &fault.item)?;
    }
    Ok(())
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use crate::model::IntentInput;

    #[test]
    fn native_route_binds_reviewer_and_retries_are_idempotent() {
        let scratch = tempfile::tempdir().unwrap();
        let store = Store::open(&scratch.path().join("graph.db"), "node").unwrap();
        let source = "version 2\nagent \"node.ops\" { command \"true\" }\nagent \"node.backup\" { command \"true\"; handles-faults }\n";
        let intent = crate::graph::parse_intent(source, "node").unwrap();
        let preview = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store
            .apply(
                &intent,
                &preview.subject_tokens,
                "agents-request-slo-reviewer",
            )
            .unwrap();
        assert_eq!(
            store.fleet_fault_agent().unwrap().as_deref(),
            Some("agent/node.backup")
        );

        // Exercise the same native snapshot owner and sender as fault delivery,
        // without starting the unrelated reconciliation stages.
        let deliver = |failure: &str| {
            let faults = store
                .fault_snapshot(crate::gate_check::now_ms() + 1)
                .unwrap();
            let mut sent = 0;
            for fault in faults {
                if fault.item.subject == "daemon/node"
                    && fault.item.episode == failure
                {
                    assert_eq!(fault.owner.as_deref(), Some("agent/node.ops"));
                    crate::reconcile::append_fault_message(
                        &store,
                        fault.owner.as_deref().unwrap(),
                        &fault.item,
                    )
                    .unwrap();
                    sent += 1;
                }
            }
            sent
        };
        let assert_mailbox = |notice: &str| {
            let messages = store.messages(Some("agent/node.ops"), false).unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].subject, notice);
            assert_eq!(messages[0].from, "daemon/runtime");
            assert!(crate::silent::always_wakes(
                &messages[0].from,
                &messages[0].tags
            ));
            assert_eq!(
                store.claims_for(notice, Some("message.sent")).unwrap().len(),
                1
            );
            assert!(
                store
                    .messages(Some("agent/node.backup"), false)
                    .unwrap()
                    .is_empty()
            );
        };

        let mut cut = PendingFault {
            id: 1,
            reviewer: "agent/node.ops".into(),
            status: "missed".into(),
            current: "1/20 fast".into(),
            through_ms: 60_000,
        };
        let reviewer = "agent/node.ops";
        let episode = crate::objective_faults::episode(store.origin(), OBJECTIVE);
        assert!(route(&store, "agent/node.backup", &cut).is_err());
        assert!(store.operational_failure(&episode).unwrap().is_none());
        assert!(store.messages(Some("agent/node.ops"), false).unwrap().is_empty());
        assert!(store.messages(Some("agent/node.backup"), false).unwrap().is_empty());
        route(&store, reviewer, &cut).unwrap();
        let first = store.operational_failure(&episode).unwrap().unwrap();
        assert_eq!(first.reviewer, reviewer);
        assert_eq!(first.status, "pending");
        assert_eq!(deliver(&first.request), 1);
        let notice = store.messages(Some(reviewer), false).unwrap()[0]
            .subject
            .clone();
        assert_mailbox(&notice);
        let index = store.index().unwrap();
        route(&store, reviewer, &cut).unwrap();
        assert_eq!(deliver(&first.request), 1);
        assert_mailbox(&notice);
        assert_eq!(store.index().unwrap(), index);

        cut.reviewer = "agent/node.backup".into();
        route(&store, "agent/node.backup", &cut).unwrap();
        assert_eq!(deliver(&first.request), 1);
        assert_mailbox(&notice);
        let unchanged = store.operational_failure(&episode).unwrap().unwrap();
        assert_eq!(unchanged.request, first.request);
        assert_eq!(unchanged.reviewer, reviewer);
        assert_eq!(store.index().unwrap(), index);
        cut.reviewer = reviewer.into();

        let source = format!("daemon/{}", store.origin());
        let failures = store
            .claims_for(&source, Some("operational.failure"))
            .unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0].body["fields"]["condition"],
            AGENTS_REQUEST_SLO_MISSED
        );

        cut.status = "met".into();
        route(&store, reviewer, &cut).unwrap();
        assert_eq!(deliver(&first.request), 0);
        assert_mailbox(&notice);
        let resolved = store.operational_failure(&episode).unwrap().unwrap();
        assert_eq!(resolved.request, first.request);
        assert_eq!(resolved.status, "resolved");
        let recovered = store
            .claims_for(&source, Some("operational.recovered"))
            .unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].body["fields"]["failure"], first.request);
        assert_eq!(recovered[0].body["fields"]["episode"], episode);
        let index = store.index().unwrap();
        route(&store, reviewer, &cut).unwrap();
        assert_eq!(deliver(&first.request), 0);
        assert_mailbox(&notice);
        assert_eq!(
            store
                .claims_for(&source, Some("operational.recovered"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(store.index().unwrap(), index);
    }
}
