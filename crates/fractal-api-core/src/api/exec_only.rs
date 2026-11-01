use super::*;
use crate::model::DesiredSubject;
use rusqlite::OptionalExtension;

pub(super) struct Card {
    pub state: &'static str,
    pub runtime_ids: Vec<String>,
    pub host: Option<String>,
    pub workspace: String,
}

// A memberless root is a declaration container, not an absent primary runtime.
// Read only the subjects named by that declaration, never historical tagged children.
pub(super) fn cards(
    store: &Store,
    roots: &[DesiredSubject],
    index: u64,
) -> anyhow::Result<BTreeMap<String, Card>> {
    let mut declarations = BTreeMap::new();
    for root in roots.iter().filter(|root| root.kind == "agent" && root.member.is_none()) {
        let Some(children) = root.desired.get("children").and_then(Value::as_array) else { continue };
        let tasks = children.iter().filter(|child| {
            matches!(child["name"].as_str(), Some("exec" | "pty"))
        }).filter_map(|child| {
            let kind = child["name"].as_str()?;
            let name = child["arguments"][0].as_str()?;
            Some((format!("{kind}/{}/{name}", root.subject.strip_prefix("agent/")?), child))
        }).collect::<Vec<_>>();
        if !tasks.is_empty() { declarations.insert(root.subject.clone(), (root, tasks)); }
    }
    let names = declarations.values().flat_map(|(_, tasks)| tasks.iter().map(|(name, _)| name.clone()))
        .collect::<BTreeSet<_>>();
    if names.is_empty() { return Ok(BTreeMap::new()); }
    let desired = store.desired_subjects_named(&names.iter().cloned().collect::<Vec<_>>())?
        .into_iter().map(|child| (child.subject.clone(), child)).collect::<BTreeMap<_, _>>();
    let statuses = store.agent_card_status_at(Some(&names), index, true)?.subjects
        .into_iter().map(|child| (child.subject.clone(), child)).collect::<BTreeMap<_, _>>();
    let connection = store.readers.get();
    let mut fresh_receipt = connection.prepare_cached(
        "SELECT actual.store_index>=desired.store_index FROM claims actual
         JOIN claims desired ON desired.id=?2 WHERE actual.id=?1",
    )?;
    let mut cards = BTreeMap::new();
    for (name, (root, tasks)) in declarations {
        let mut card = Card { state: "stopped", runtime_ids: Vec::with_capacity(tasks.len()),
            host: None, workspace: crate::graph::canonical_child_value(&root.desired, "workspace")
                .and_then(Value::as_str).unwrap_or(".").to_owned() };
        let mut severity = 0;
        let mut inherited_hosts = BTreeSet::new();
        for (name, node) in tasks {
            let child = desired.get(&name).filter(|child| {
                child.desired == *node && child.owner_run == root.owner_run
                    && child.owner_generation == root.owner_generation && child.owner_step == root.owner_step
                    && child.member.as_ref().is_some_and(|member| {
                        member.tags.get("st3.agent") == Some(&root.subject)
                    })
            });
            let member = child.and_then(|child| child.member.as_ref());
            if let Some(member) = member {
                card.runtime_ids.push(format!("runtime/{}", member.runtime_id));
                if crate::graph::canonical_child_value(node, "host").is_none() {
                    inherited_hosts.insert(member.host.clone());
                }
            }
            let status = statuses.get(&name).filter(|status| {
                child.is_some_and(|child| status.desired.as_ref() == Some(&child.desired))
            });
            let fields = status.and_then(|status| status.actual.as_ref())
                .map(|actual| actual.get("fields").unwrap_or(actual));
            // A receipt from before this selected declaration cannot prove its launch.
            let fresh = if let Some((actual, desired)) = status.and_then(|status| {
                status.actual_claim.as_deref().zip(status.desired_token.as_deref())
            }) {
                fresh_receipt.query_row([actual, desired], |row| row.get::<_, bool>(0))
                    .optional()?.unwrap_or(false)
            } else { false };
            let observed = fields.filter(|fields| fresh && member.is_some_and(|member| {
                fields["runtime_id"].as_str() == Some(member.runtime_id.as_str())
            })).and_then(|fields| fields["status"].as_str());
            let (rank, state) = match observed {
                Some("failed") => (4, "failed"),
                Some("stopped" | "exited" | "ended" | "absent") => (0, "stopped"),
                _ if status.is_none_or(|status| status.reachability != "reachable") => (3, "waiting"),
                Some("running" | "ready" | "working" | "idle") => (1, "running"),
                Some("starting" | "pending") => (2, "starting"),
                _ => (3, "waiting"),
            };
            if rank >= severity { severity = rank; card.state = state; }
        }
        card.host = crate::graph::canonical_child_value(&root.desired, "host")
            .and_then(Value::as_str).filter(|host| *host != "local").map(str::to_owned)
            .or_else(|| (inherited_hosts.len() == 1).then(|| inherited_hosts.pop_first()).flatten())
            .map(|host| client_host_id(&host));
        card.runtime_ids.sort();
        card.runtime_ids.dedup();
        cards.insert(name, card);
    }
    Ok(cards)
}
