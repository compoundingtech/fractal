//! Notice admission policy. The fleet's singleton `notice-owner` declaration binds one admitted
//! member incarnation; only that member's daemon, writing as `daemon/runtime`, writes snapshots.
//! Every daemon writes as `daemon/runtime`, so the actor alone proves nothing: the authority is
//! the claim's origin member and writer sequence checked against the binding. A missing,
//! conflicted, invalid or ended binding fails closed, with no election. Sources and their
//! lineage never depend on the binding, so a handover rebuilds snapshots and keeps sources.
//! Payload timestamps and declaration hashes are deliberately never authority evidence here.
use super::*;
use crate::fleet::MemberState;
use crate::notice_owner::{self, NoticeOwner};
use fractal_schema::{custom::Manifest, notice};

/// Where a fact was written: its origin member and that writer's sequence. A local write is
/// judged at the sequence its batch takes in the writing transaction.
#[derive(Clone, Copy, Debug)]
pub(super) struct Position<'a> {
    origin: &'a str,
    sequence: u64,
}

impl<'a> Position<'a> {
    pub(super) fn next(tx: &Transaction<'_>, origin: &'a str) -> Result<Self> {
        Ok(Self { origin, sequence: next_replica_sequence(tx, origin)? })
    }

    pub(super) fn written(connection: &Connection, claim: &'a ClaimRecord) -> Result<Self> {
        let sequence = connection.prepare_cached("SELECT replica_sequence FROM batches WHERE id=?1")?
            .query_row(/* sqlite-large-value: allow-for-now lv-hd59eb9d34a64 owner=fractal_store::store::notice_authority TODO=trace-statement-and-value-representation */ [&claim.batch_id], |row| row.get(0))?;
        Ok(Self { origin: &claim.origin, sequence })
    }
}

/// Revalidate snapshots when the owner declaration, a rival leaf of it, or the fleet membership
/// it binds against changes. Membership already counts every change in `fleet_generation`.
pub(super) fn create_invalidation_triggers(connection: &Connection) -> Result<()> {
    let dirty = format!(
        /* sqlite-large-value: allow-for-now lv-hfd8be8e12073 owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */
        "INSERT INTO local_custom_dirty SELECT subject FROM custom_sources WHERE subject LIKE '{}%'
         ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;",
        notice::SNAPSHOT_PREFIX,
    );
    let owner = notice_owner::SUBJECT;
    let mut triggers = Vec::new();
    for (table, action, rows) in [
        ("desired", "INSERT", &["NEW"][..]),
        ("desired", "UPDATE", &["OLD", "NEW"][..]),
        ("desired", "DELETE", &["OLD"][..]),
        ("claims", "INSERT", &["NEW"][..]),
        ("claims", "DELETE", &["OLD"][..]),
    ] {
        let when = rows.iter().map(|row| format!("{row}.subject='{owner}'")).collect::<Vec<_>>().join(" OR ");
        triggers.push((format!("notice_owner_{table}_{action}"), format!("{action} ON {table} WHEN {when}")));
    }
    triggers.push(("notice_owner_fleet_generation".into(), "UPDATE OF value ON fleet_generation".into()));
    for (name, event) in triggers {
        let sql = format!(/* sqlite-large-value: allow-for-now lv-h83dafab234e8 owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER {name} AFTER {event} BEGIN {dirty} END");
        let installed: Option<String> = connection.query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
            [&name], |row| row.get(0),
        ).optional()?;
        if installed.as_deref() != Some(sql.as_str()) {
            connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-ha3325e0e8f6d owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER IF EXISTS {name}; {sql};"))?;
        }
    }
    // The agent notice-runtime designation is gone: drop its liveness triggers and cached outcome.
    for table in ["desired", "mission_runs", "step_runs", "run_generations"] {
        for action in ["INSERT", "UPDATE", "DELETE"] {
            connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-ha5e41f8312ee owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER IF EXISTS notice_authority_{table}_{action};"))?;
        }
    }
    connection.execute(/* sqlite-large-value: allow-for-now lv-h53d4e0f7d3be owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='notice_runtime_binding'", [])?;
    Ok(())
}

/// Authority dependencies are invalidation edges, not optional projection `_basis` inputs.
/// Retain links from all facts, including unpinned, pending and invalid facts, so a later
/// source arrival, repair or competing creation revalidates every linked row.
pub(super) fn record_dependencies(tx: &Transaction<'_>, subject: &str, history: &[ClaimRecord]) -> Result<()> {
    if !subject.starts_with(notice::SNAPSHOT_PREFIX) {
        return Ok(());
    }
    let mut insert = tx.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-h2d6ca7ff29ef owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */
        "INSERT OR IGNORE INTO custom_dependencies VALUES(?1,?2,'[]','')",
    )?;
    for claim in history {
        let fields = &claim.body["fields"];
        for field in ["source_id", "creation_claim"] {
            if let Some(dependency) = fields.get(field).and_then(Value::as_str) {
                insert.execute(/* sqlite-large-value: allow-for-now lv-hc61106882558 owner=fractal_store::store::notice_authority TODO=trace-statement-and-value-representation */ params![subject, dependency])?;
            }
        }
    }
    Ok(())
}

fn unavailable(reason: impl Into<String>) -> FractalError {
    FractalError::new("notices-unavailable", format!("notices unavailable: {}", reason.into()))
}

/// The bound notice owner, with the membership it was resolved against.
pub(super) fn owner(connection: &Connection) -> Result<(NoticeOwner, crate::fleet::Membership), FractalError> {
    let desired = current_desired_row_tx(connection, notice_owner::SUBJECT).map_err(internal)?
        .ok_or_else(|| unavailable("missing notice-owner declaration"))?;
    if !desired_conflicts_at(connection, notice_owner::SUBJECT, Some(&desired.claim_id), None).map_err(internal)?.is_empty() {
        return Err(unavailable("conflicting notice-owner declarations"));
    }
    let declared = serde_json::from_str(&desired.body).map_err(|error| FractalError::new("invalid-notice-owner", error.to_string()))
        .and_then(|body| notice_owner::parse(&body))
        .map_err(|error| unavailable(format!("invalid notice-owner declaration: {}", error.message)))?;
    let membership = fleet_membership_tx(connection).map_err(internal)?;
    match membership.state(&declared.member) {
        MemberState::Current(incarnation) if incarnation.member_key == declared.member_key => {}
        MemberState::Current(_) | MemberState::Ended(_) | MemberState::LegacyRemoved(_) => {
            return Err(unavailable("the notice owner's bound incarnation has ended"));
        }
        MemberState::Conflicted(_) => return Err(unavailable("the notice owner's membership is conflicted")),
        MemberState::NotMember => return Err(unavailable("the notice owner is not an admitted fleet member")),
    }
    Ok((declared, membership))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate(
    connection: &Connection, subject: &str, manifest: &Manifest, kind: &str,
    actor: &str, fields: &BTreeMap<String, Value>, creation: Option<&ClaimRecord>, position: Option<Position<'_>>,
) -> Result<()> {
    if !notice::is_reserved_subject(subject) && !notice::is_reserved_kind(kind) { return Ok(()); }
    let hash = fields.get("_registration").and_then(Value::as_str)
        .ok_or_else(|| unavailable("missing registration pin"))?;
    notice::validate_registration(manifest, hash)?;
    anyhow::ensure!(subject.starts_with(&manifest.subject_prefix), "notices unavailable: claim/prefix mismatch");
    if subject.starts_with(notice::SOURCE_PREFIX) {
        anyhow::ensure!(kind == manifest.creation_kind, "notices unavailable: producer input is immutable");
        if let Some(prior) = creation {
            anyhow::ensure!(prior.actor.as_deref() == Some(actor) && custom::fields(prior) == *fields,
                "notices unavailable: source creation conflict");
        }
        return validate_source(actor, fields);
    }
    anyhow::ensure!(subject.starts_with(notice::SNAPSHOT_PREFIX), "notices unavailable: not a notice family");
    let source_id = fields.get("source_id").and_then(Value::as_str)
        .filter(|s| s.starts_with(notice::SOURCE_PREFIX))
        .ok_or_else(|| unavailable("missing source link"))?;
    let claim_id = fields.get("creation_claim").and_then(Value::as_str)
        .ok_or_else(|| unavailable("missing pinned source creation"))?;
    let source = claim_by_id_tx(connection, claim_id)?
        .filter(|claim| claim.subject == source_id && claim.kind == "custom.attention.notice-created")
        .ok_or_else(|| unavailable("source creation is not available"))?;
    let source_fields = custom::fields(&source);
    let (source_hash, source_manifest, state) = custom::registration(connection, source_id)?
        .ok_or_else(|| unavailable("source registration is not available"))?;
    anyhow::ensure!(state == "ready", "notices unavailable: source registration conflict");
    notice::validate_registration(&source_manifest, &source_hash)?;
    anyhow::ensure!(source_fields.get("_registration").and_then(Value::as_str) == Some(source_hash.as_str()),
        "notices unavailable: source registration mismatch");
    let source_history = custom::claims(connection, source_id)?;
    let mut creations = source_history.iter().filter(|claim| claim.kind == source_manifest.creation_kind);
    anyhow::ensure!(creations.next().is_some_and(|claim| claim.id == source.id) && creations.next().is_none(),
        "notices unavailable: conflicting or ineligible source creation");
    let recipient = source_fields.get("recipient").and_then(Value::as_str)
        .filter(|p| p.starts_with("person/"))
        .ok_or_else(|| unavailable("invalid source recipient"))?;
    let producer = source.actor.as_deref().ok_or_else(|| unavailable("missing source author"))?;
    fractal_schema::custom::validate_fields(&source_manifest.claims[&source.kind], &source_fields)?;
    validate_source(producer, &source_fields)?;
    let (owner, membership) = owner(connection)?;
    let runtime = format!("host/{}", owner.member);
    for (field, expected) in [("recipient", recipient), ("producer", producer), ("runtime", runtime.as_str())] {
        anyhow::ensure!(fields.get(field).and_then(Value::as_str) == Some(expected),
            "notices unavailable: forged {field}");
    }
    if let Some(prior) = creation {
        let prior_fields = custom::fields(prior);
        for field in ["source_id", "creation_claim", "recipient", "producer", "runtime"] {
            anyhow::ensure!(prior_fields.get(field) == fields.get(field), "notices unavailable: immutable {field} changed");
        }
    }
    anyhow::ensure!(actor == notice::RUNTIME_ACTOR,
        "notices unavailable: only {} writes snapshots", notice::RUNTIME_ACTOR);
    let position = position.ok_or_else(|| unavailable("a snapshot needs its writer position"))?;
    anyhow::ensure!(
        position.origin == owner.member
            && membership.keys_for(position.origin, position.sequence).contains(&owner.member_key),
        "notices unavailable: only the bound notice owner's daemon writes snapshots"
    );
    Ok(())
}

/// A source is a producer's own input: an agent or person, never a daemon, and never pinning
/// the runtime, so its lineage survives an owner handover.
fn validate_source(producer: &str, fields: &BTreeMap<String, Value>) -> Result<()> {
    anyhow::ensure!(producer.starts_with("agent/") || producer.starts_with("person/"),
        "notices unavailable: a source producer is an agent or person");
    anyhow::ensure!(fields.get("producer").is_none_or(|value| value.as_str() == Some(producer)),
        "notices unavailable: forged producer");
    anyhow::ensure!(fields.get("recipient").and_then(Value::as_str).is_some_and(|p| p.starts_with("person/")),
        "notices unavailable: source recipient must be a person");
    Ok(())
}

#[cfg(test)]
#[path = "notice_authority_tests.rs"]
mod tests;
