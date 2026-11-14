//! Host-owned person attention, folded with the population refresher's store snapshot.

use super::{AttentionItemView, Store};
use std::collections::HashSet;

pub const KINDS: [&str; 11] = [
    "person-step", "human-gate", "launch-approval", "unread-message", "agent-request",
    "revision-approval", "harness-login", "harness-prompt", "fault", "model-drift", "unattributed",
];

pub type Counts = [(&'static str, u64); 11];

/// Call inside the population refresher's `Store::read_snapshot`, alongside the population
/// fold. No independent cut, timeout, mailbox read or fault collection is introduced here.
/// Every source's ownership must resolve, including remote sources: an error invalidates the
/// complete refresh rather than silently treating an unowned item as zero.
pub fn summarize_with_deadline(store: &Store, now: u128) -> anyhow::Result<(Counts, Option<u128>)> {
    // Use the canonical collection's admission clock, not a periodic refresh. Runtime grace,
    // leases and ask escalation affect other projections or require a graph write; none can
    // change these source counts on their own.
    let (items, deadline) = store.attention_collection_snapshot(None, now)?;
    let mut seen = HashSet::with_capacity(items.len());
    let sources: Vec<_> = items.iter().filter(|item| {
        item.kind != "unread-message"
            && seen.insert((item.subject.as_str(), item.person.as_str(), item.episode.as_str()))
    }).collect();
    let origins = store.attention_source_origins(&sources)?;
    let origins: std::collections::HashMap<_, _> = sources.into_iter().zip(origins)
        .map(|(item, origin)| ((item.subject.as_str(), item.kind.as_str(), item.mission_run.as_deref()), origin))
        .collect();
    let counts = count_items(&items, store.origin(), |item| {
        Ok(origins[&(item.subject.as_str(), item.kind.as_str(), item.mission_run.as_deref())].clone())
    })?;
    Ok((counts, deadline))
}

fn count_items(
    items: &[AttentionItemView],
    local_origin: &str,
    mut source_origin: impl FnMut(&AttentionItemView) -> anyhow::Result<String>,
) -> anyhow::Result<Counts> {
    let mut counts = KINDS.map(|kind| (kind, 0));
    let mut seen = HashSet::with_capacity(items.len());
    for item in items {
        // Reserved, even if a future or malformed projection supplies such an item. Only an
        // approved conversation-owner projection may change this contract's explicit zero.
        if item.kind == "unread-message" {
            continue;
        }
        if !seen.insert((item.subject.as_str(), item.person.as_str(), item.episode.as_str())) {
            continue;
        }
        if source_origin(item)? != local_origin {
            continue;
        }
        let index = KINDS.iter().position(|kind| *kind == item.kind).unwrap_or(KINDS.len() - 1);
        counts[index].1 += 1;
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::{PersonWorkStore as _, SeatsStoreCore as _};

    fn store_with_asks(times: &[(u128, u128)]) -> Store {
        let store = Store::open_memory("example").unwrap();
        let intent = crate::graph::parse_intent(
            "version 2\nagent \"example.worker\" { workspace \"/tmp\"; harness \"claude\" {} }\n",
            "example",
        ).unwrap();
        store.apply_internal(&intent, "attention-deadline-fixture").unwrap();
        for (index, (accepted, waiting)) in times.iter().enumerate() {
            let ask = store.ask_person(&serde_json::from_value(json!({
                "person":"person/reviewer", "actor":"agent/example.worker",
                "title":"Choose a seed", "reason":"Review the garden plan",
                "idempotency_key":format!("attention-{index}")
            })).unwrap()).unwrap();
            // Model a replicated future-dated ask and a migrated waiting timestamp separately.
            store.connection.write().execute(
                "UPDATE claims SET accepted_at_unix_ms=?2,
                 body=json_set(body,'$.fields.waiting_since',?3)
                 WHERE subject=?1 AND kind='work.person-asked'",
                rusqlite::params![ask.subject, accepted.to_string(), waiting.to_string()],
            ).unwrap();
        }
        store
    }

    #[test]
    fn admission_deadline_opens_both_acceptance_and_waiting_gates_without_a_commit() {
        let store = store_with_asks(&[(2_000, 1_000), (1_000, 3_000)]);
        let cut = store.index().unwrap();
        for (now, expected_count, expected_deadline) in [
            (999, 0, Some(2_000)),
            (1_000, 0, Some(2_000)),
            (1_999, 0, Some(2_000)),
            (2_000, 1, Some(3_000)),
            (2_999, 1, Some(3_000)),
            (3_000, 2, None),
            (u128::MAX, 2, None),
        ] {
            let (counts, deadline) = summarize_with_deadline(&store, now).unwrap();
            assert_eq!(counts.iter().map(|(_, count)| count).sum::<u64>(), expected_count, "{now}");
            assert_eq!(deadline, expected_deadline, "{now}");
            assert_eq!(store.index().unwrap(), cut, "a clock-only fold must not write");
        }
    }

    #[test]
    fn future_admission_does_not_transfer_source_ownership_to_the_reading_host() {
        let store = store_with_asks(&[(2_000, 1_000)]);
        store.connection.write().execute(
            "UPDATE desired SET member=json_set(member,'$.host','remote')
             WHERE subject='agent/example.worker'",
            [],
        ).unwrap();
        let (before, deadline) = summarize_with_deadline(&store, 1_999).unwrap();
        assert_eq!(before, KINDS.map(|kind| (kind, 0)));
        assert_eq!(deadline, Some(2_000));
        let (after, deadline) = summarize_with_deadline(&store, 2_000).unwrap();
        assert_eq!(after, before, "the source's selected member remains its only publisher");
        assert_eq!(deadline, None);
    }

    fn item(kind: &str, source: &str, person: &str, episode: &str) -> AttentionItemView {
        serde_json::from_value(json!({
            "kind": kind, "subject": source, "person": person, "episode": episode,
            "title": "An actionable item", "detail": "Needs an answer", "requested_at_unix_ms": 1,
        })).unwrap()
    }

    #[test]
    fn empty_snapshot_emits_every_closed_kind_including_reserved_zero() {
        let counts = count_items(&[], "local", |_| unreachable!()).unwrap();
        assert_eq!(counts, KINDS.map(|kind| (kind, 0)));
    }

    #[test]
    fn deduplicates_source_person_episode_not_kind_or_agent() {
        let rows = [
            item("person-step", "source/a", "person/a", "one"),
            item("person-step", "source/a", "person/a", "one"),
            item("person-step", "source/a", "person/other", "one"),
            item("person-step", "source/a", "person/a", "two"),
            item("person-step", "source/b", "person/a", "one"),
        ];
        let mut ownership_reads = 0;
        let counts = count_items(&rows, "local", |_| {
            ownership_reads += 1;
            Ok("local".into())
        }).unwrap();
        assert_eq!(counts[0], ("person-step", 4));
        assert_eq!(ownership_reads, 4);
    }

    #[test]
    fn replicated_sources_have_exactly_one_publisher() {
        let rows = [
            item("human-gate", "mission-run/remote", "person/a", "one"),
            item("launch-approval", "planning-session/local", "person/a", "two"),
            item("custom.review", "custom/review/global", "person/a", "three"),
        ];
        let owner = |item: &AttentionItemView| -> anyhow::Result<String> {
            Ok(if item.kind == "launch-approval" { "local" } else { "remote" }.into())
        };
        let local = count_items(&rows, "local", owner).unwrap();
        let remote = count_items(&rows, "remote", owner).unwrap();
        assert_eq!(local.iter().map(|(_, count)| count).sum::<u64>(), 1);
        assert_eq!(remote.iter().map(|(_, count)| count).sum::<u64>(), 2);
        assert_eq!(local[2], ("launch-approval", 1));
        assert_eq!(remote[1], ("human-gate", 1));
        assert_eq!(remote[10], ("unattributed", 1));
    }

    #[test]
    fn maps_unknown_kinds_and_counts_only_supplied_person_faults() {
        let rows = [
            item("custom.question", "custom/question/a", "person/a", "one"),
            item("fault", "mission-run/a", "person/a", "broken-gate"),
            item("agent-request", "step-run/a", "person/a", "ask"),
            item("model-drift", "agent/a", "person/a", "drift"),
        ];
        let counts = count_items(&rows, "local", |_| Ok("local".into())).unwrap();
        assert_eq!(counts[10], ("unattributed", 1));
        assert_eq!(counts[9], ("model-drift", 1));
        assert_eq!(counts[8], ("fault", 1));
        assert_eq!(counts[4], ("agent-request", 1));
        assert_eq!(counts.iter().map(|(_, count)| count).sum::<u64>(), 4);
    }

    #[test]
    fn unread_messages_never_count_or_request_guessed_ownership() {
        let rows = [item("unread-message", "message/a", "person/a", "one")];
        let counts = count_items(&rows, "local", |_| unreachable!()).unwrap();
        assert_eq!(counts, KINDS.map(|kind| (kind, 0)));
    }

    #[test]
    fn unresolved_ownership_invalidates_the_whole_aggregate() {
        let rows = [
            item("human-gate", "mission-run/known", "person/a", "one"),
            item("custom.question", "custom/question/unowned", "person/a", "two"),
        ];
        let result = count_items(&rows, "local", |item| {
            if item.kind == "human-gate" { Ok("local".into()) }
            else { anyhow::bail!("source owner is unavailable") }
        });
        assert!(result.is_err());
    }
}
