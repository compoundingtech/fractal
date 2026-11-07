//! Host-owned person attention, folded with the population refresher's store snapshot.

use super::{AttentionItemView, Store};
use std::collections::HashSet;

pub const KINDS: [&str; 10] = [
    "person-step", "human-gate", "launch-approval", "unread-message", "agent-request",
    "revision-approval", "harness-login", "harness-prompt", "fault", "unattributed",
];

pub type Counts = [(&'static str, u64); 10];

/// Call inside the population refresher's `Store::read_snapshot`, alongside the population
/// fold. No independent cut, timeout, mailbox read or fault collection is introduced here.
/// Every source's ownership must resolve, including remote sources: an error invalidates the
/// complete refresh rather than silently treating an unowned item as zero.
pub fn summarize(store: &Store, now: u128) -> anyhow::Result<Counts> {
    let items = store.attention_snapshot(None, now)?;
    count_items(&items, store.origin(), |item| store.attention_source_origin(item))
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
        assert_eq!(remote[9], ("unattributed", 1));
    }

    #[test]
    fn maps_unknown_kinds_and_counts_only_supplied_person_faults() {
        let rows = [
            item("custom.question", "custom/question/a", "person/a", "one"),
            item("fault", "mission-run/a", "person/a", "broken-gate"),
            item("agent-request", "step-run/a", "person/a", "ask"),
        ];
        let counts = count_items(&rows, "local", |_| Ok("local".into())).unwrap();
        assert_eq!(counts[9], ("unattributed", 1));
        assert_eq!(counts[8], ("fault", 1));
        assert_eq!(counts[4], ("agent-request", 1));
        assert_eq!(counts.iter().map(|(_, count)| count).sum::<u64>(), 3);
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
