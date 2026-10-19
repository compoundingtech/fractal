//! The part of `store::agent_messages` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use serde::Deserialize;
pub use st3_store::store::agent_messages::*;
pub use st3_store::store::agent_messages::{DAY_MS, DailyUsage, create_schema, flush, open, synthetic, windows};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_usage_matches_separate_periods_including_baselines_and_restarts() {
        let store = Store::open_memory("node").unwrap();
        for (i, (at, tokens, cost, unpriced)) in [
            (DAY_MS, 100, 100_000, 0),
            (DAY_MS + 1, 120, 150_000, 0),
            (2 * DAY_MS, 200, 250_000, 10),
            (2 * DAY_MS + 1, 10, 30_000, 0),
            (3 * DAY_MS, 50, 80_000, 0),
        ]
        .into_iter()
        .enumerate()
        {
            store
                .append_claim(&ClaimInput {
                    subject: "agent/alder".into(),
                    kind: "harness.usage".into(),
                    actor: None,
                    fields: serde_json::from_value(json!({"driver":"codex","incarnation_id":"one",
                    "semantics":"response_rollup","observed_at_unix_ms":at,"total_tokens":tokens,
                    "cost_microusd":cost,"unpriced_tokens":unpriced,"model":"test-model"}))
                    .unwrap(),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: Some(format!("usage-{i}")),
                })
                .unwrap();
        }
        let (_, estimate) = store.usage_period_report(DAY_MS, 3 * DAY_MS).unwrap();
        for day in estimate["days"].as_array().unwrap() {
            let rows = store
                .usage_period_rows(
                    day["since_ms"].as_u64().unwrap(),
                    day["until_ms"].as_u64().unwrap(),
                )
                .unwrap();
            assert_eq!(
                day["usage_cost_microusd"].as_u64().unwrap(),
                rows.iter()
                    .map(|r| r["cost_microusd"].as_u64().unwrap())
                    .sum::<u64>()
            );
            assert_eq!(
                day["unpriced_tokens"].as_u64().unwrap(),
                rows.iter()
                    .map(|r| r["unpriced_tokens"].as_u64().unwrap())
                    .sum::<u64>()
            );
        }
        assert!(windows(1, 1).is_empty());
        assert_eq!(windows(0, 100 * DAY_MS).len(), 31);
    }

    #[test]
    fn replicated_sends_are_counted_once_on_each_node() {
        let source = Store::open_memory("node").unwrap();
        source.append_claim(&ClaimInput {
            subject:"message/replicated".into(),kind:"message.sent".into(),actor:None,
            fields:serde_json::from_value(json!({"from":"agent/alder","to":"agent/birch","title":"Update","status":"sent"})).unwrap(),
            evidence:vec![],expected_subject:None,idempotency_key:None,
        }).unwrap();
        let peer = Store::open_memory("peer").unwrap();
        let exchange =
            crate::store::tests_shared::exchange_from(&source, &peer.replication_inventory().unwrap());
        crate::store::tests_shared::receive_and_project(&peer, "node", &exchange);
        crate::store::tests_shared::receive_and_project(&peer, "node", &exchange);
        let until = now_ms() as u64 + 1;
        let (_, expected) = source.usage_period_report(0, until).unwrap();
        assert_eq!(peer.usage_period_report(0, until).unwrap().1, expected);
        assert_eq!(
            expected["days"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d["messages"].as_u64().unwrap())
                .sum::<u64>(),
            1
        );
    }
}
