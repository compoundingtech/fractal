//! Deterministic naming projection shared by durable stores and authenticated offline readers.
use std::collections::{BTreeMap, BTreeSet};

/// Immutable runtime admission checks, before a naming fact can reserve or fence names.
/// Declaration availability and predecessor conflicts belong to the projection fold.
pub fn admits_replicated(claim: &crate::claim::ClaimRecord) -> anyhow::Result<bool> {
    let prefix = match claim.kind.as_str() {
        "agent.renamed" => "agent/", "mission.renamed" => "mission/", _ => return Ok(true),
    };
    let fields = &claim.body["fields"];
    let request = &fields["request"];
    let [Some(from), Some(to), Some(actor), Some(reason), Some(key)] =
        ["from", "to", "actor", "reason", "idempotency_key"].map(|name| request[name].as_str())
    else { return Ok(false); };
    for name in ["expected_declaration", "expected_naming_revision"] {
        if request.get(name).is_some_and(|value| !value.is_null() && !value.is_string()) { return Ok(false); }
    }
    if !claim.subject.starts_with(prefix) || !from.starts_with(prefix) || !to.starts_with(prefix)
        || reason.trim().is_empty()
        || fields["to"].as_str() != Some(to)
        || fields["reason"].as_str() != Some(reason)
        || fields["owner_host"].as_str() != Some(claim.origin.as_str())
        || claim.actor.is_none()
        || (!actor.starts_with("agent/") && claim.actor.as_deref() != Some(actor))
        || request["expected_declaration"].as_str().is_some_and(|expected| fields["declaration"].as_str() != Some(expected))
        || request["expected_naming_revision"].as_str().is_some_and(|expected| fields["previous_revision"].as_str() != Some(expected))
    { return Ok(false); }
    let Some((operation, digest)) = crate::store::operation_parts(&claim.body) else { return Ok(false); };
    Ok(operation == crate::store::operation_id_for_key(key)
        && digest == crate::hash::canonical_hash(&("st3.identity-rename-request.v1", claim.kind.as_str(), crate::hash::canonical_json_value(request)))?)
}

pub struct Fact {
    pub id: String, pub identity: String, pub from: String, pub to: String,
    pub previous: Option<String>, pub repaired: bool,
    /// Adapter-validated declaration, ownership and operation evidence.
    pub unavailable: Option<&'static str>,
}
pub struct Head<K> {
    pub kind: K, pub canonical: String, pub revision: Option<String>, pub blocked: Option<&'static str>,
}
pub type Names = BTreeMap<String, (String, Option<String>)>;
pub struct Projection<K> {
    pub heads: BTreeMap<String, Head<K>>, pub names: Names, pub accepted: bool,
}

/// Inputs include every retained naming fact and every reserved storage declaration.
/// Facts are ordered by claim ID here, never by arrival or canonical timestamp.
pub fn reduce<K>(mut heads: BTreeMap<String, Head<K>>, mut names: Names, mut facts: Vec<Fact>, proving: Option<&str>) -> Projection<K> {
    facts.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    let mut winners = BTreeMap::<String,String>::new();
    for fact in &facts {
        if !names.contains_key(&fact.to) { winners.entry(fact.to.clone()).or_insert_with(|| fact.id.clone()); }
    }
    let mut branches = BTreeMap::<(String,Option<String>),Vec<String>>::new();
    for fact in facts.iter().filter(|fact| !fact.repaired) { branches.entry((fact.identity.clone(),fact.previous.clone())).or_default().push(fact.id.clone()); }
    let mut done = BTreeSet::new();
    let mut accepted = false;
    loop {
        let mut advanced = false;
        for fact in &facts {
            if done.contains(&fact.id) { continue; }
            if fact.repaired { done.insert(fact.id.clone()); continue; }
            let head = heads.get(&fact.identity).expect("fact head");
            if fact.previous.is_some() && fact.previous != head.revision { continue; }
            if fact.previous.is_none() && head.revision.is_some() { continue; }
            let mut reason = fact.unavailable;
            if branches[&(fact.identity.clone(),fact.previous.clone())].len() > 1 { reason = Some("conflict"); }
            if winners.get(&fact.to) != Some(&fact.id) || fact.from != head.canonical { reason = Some("conflict"); }
            let head = heads.get_mut(&fact.identity).expect("fact head");
            if let Some(reason) = reason { head.blocked = Some(reason); }
            else if head.blocked != Some("conflict") {
                head.blocked = None; head.canonical.clone_from(&fact.to); head.revision = Some(fact.id.clone());
                accepted |= proving == Some(fact.id.as_str());
            }
            done.insert(fact.id.clone()); advanced = true;
        }
        if !advanced { break; }
    }
    let by_id = facts.iter().map(|fact| (fact.id.as_str(),fact)).collect::<BTreeMap<_,_>>();
    for fact in &facts {
        if !done.contains(&fact.id) {
            let head = heads.get_mut(&fact.identity).expect("fact head");
            if head.blocked.is_none() {
                let mut visited = BTreeSet::new();
                let mut previous = fact.previous.as_deref();
                let mut reason = "conflict";
                while let Some(id) = previous {
                    if !visited.insert(id) { break; }
                    let Some(predecessor) = by_id.get(id) else { reason = "pending-replication"; break; };
                    if predecessor.identity != fact.identity || predecessor.repaired { break; }
                    previous = predecessor.previous.as_deref();
                }
                head.blocked = Some(reason);
            }
        }
        if winners.get(&fact.to) == Some(&fact.id) { names.insert(fact.to.clone(),(fact.identity.clone(),Some(fact.id.clone()))); }
    }
    for fact in &facts {
        names.entry(fact.from.clone()).or_insert_with(|| (fact.identity.clone(),Some(fact.id.clone())));
    }
    Projection { heads, names, accepted }
}

#[cfg(test)]
#[path = "naming_tests.rs"]
mod tests;
