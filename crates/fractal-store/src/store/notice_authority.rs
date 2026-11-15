//! Notice admission policy. The fleet's singleton `notice-owner` declaration binds one admitted
//! member incarnation; only that member's daemon, writing as `daemon/runtime`, writes snapshots.
//! Every daemon writes as `daemon/runtime`, so the actor alone proves nothing: the authority is
//! the claim's origin member and writer sequence checked against the binding. A missing,
//! conflicted, invalid or ended binding fails closed, with no election. Ordinary producer
//! sources never depend on the binding; daemon-owned digest sources do.
//! Digest emission history pins its exact owner declaration and checks the historical member
//! window; current ownership still fences every new local source and snapshot write.
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
        /* sqlite-large-value: allow-for-now lv-h1fd7a459c656 owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */
        "INSERT INTO local_custom_dirty SELECT subject FROM custom_sources WHERE subject LIKE '{}%'
         OR subject LIKE '{}%' ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;",
        notice::SNAPSHOT_PREFIX, notice::DIGEST_SOURCE_PREFIX,
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

/// Install only the compiled reviewed notice manifests, without borrowing a person's identity.
/// The fold's runtime registration exception admits only these exact compiled pins.
pub(super) fn ensure_digest_registrations_tx(tx: &Transaction<'_>, origin: &str) -> Result<()> {
    validate_owner_writer(tx, notice::RUNTIME_ACTOR, Some(Position::next(tx, origin)?))?;
    custom::flush(tx)?;
    for (manifest, hash) in notice::manifests().iter().zip(notice::HASHES) {
        notice::validate_registration(manifest, hash)?;
        if let Some((prior, _, state)) = custom::registration(tx, &format!("{}digest/", manifest.subject_prefix))? {
            anyhow::ensure!(prior == hash && state == "ready",
                "notices unavailable: reviewed registration conflicts with existing registration");
            continue;
        }
        let bytes = serde_json::to_vec(manifest)?;
        let name = format!("doc/custom-kinds/{}.v{}/{hash}", manifest.kind, manifest.version);
        tx.execute(/* sqlite-large-value: allow-for-now lv-h3dba2b97c423 owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO blobs(hash,bytes,size) VALUES(?1,?2,?3)",
            params![hash, bytes, bytes.len() as u64])?;
        let binding = append_claim_tx(tx, origin, &name, "doc.bound", None,
            &json!({"name":name,"hash":hash,"size":bytes.len()}), &[], None)?;
        select_replicated_document(tx, &binding, binding.store_index)?;
        append_claim_tx(tx, origin, &manifest.subject(), fractal_schema::custom::REGISTERED,
            Some(notice::RUNTIME_ACTOR), &json!({"fields":{"document":format!("{name}@{hash}")}}), &[], None)?;
        custom::flush(tx)?;
        anyhow::ensure!(custom::registration(tx, &format!("{}digest/", manifest.subject_prefix))?
            .is_some_and(|(pin, _, state)| pin == hash && state == "ready"),
            "notices unavailable: reviewed registration could not be installed");
    }
    Ok(())
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
        if subject.starts_with(notice::DIGEST_SOURCE_PREFIX) {
            validate_digest_source(connection, subject, actor, fields)?;
            return validate_digest_writer(connection, actor, fields, position);
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
    let mut source_history = custom::claims(connection, source_id)?;
    retain_authenticated_digest_sources(connection, source_id, &mut source_history)?;
    let mut creations = source_history.iter().filter(|claim| claim.kind == source_manifest.creation_kind);
    anyhow::ensure!(creations.next().is_some_and(|claim| claim.id == source.id) && creations.next().is_none(),
        "notices unavailable: conflicting or ineligible source creation");
    let recipient = source_fields.get("recipient").and_then(Value::as_str)
        .filter(|p| p.starts_with("person/"))
        .ok_or_else(|| unavailable("invalid source recipient"))?;
    let producer = source.actor.as_deref().ok_or_else(|| unavailable("missing source author"))?;
    fractal_schema::custom::validate_fields(&source_manifest.claims[&source.kind], &source_fields)?;
    if source_id.starts_with(notice::DIGEST_SOURCE_PREFIX) {
        validate_digest_source(connection, source_id, producer, &source_fields)?;
        anyhow::ensure!(authenticated_digest_source(connection, &source)?,
            "notices unavailable: unauthenticated digest source");
        anyhow::ensure!(subject.strip_prefix(notice::DIGEST_SNAPSHOT_PREFIX)
            == source_id.strip_prefix(notice::DIGEST_SOURCE_PREFIX),
            "notices unavailable: digest snapshot identity differs from its source");
    } else {
        anyhow::ensure!(!subject.starts_with(notice::DIGEST_SNAPSHOT_PREFIX),
            "notices unavailable: digest snapshots require a digest source");
        validate_source(producer, &source_fields)?;
    }
    let (owner, _) = owner(connection)?;
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
    validate_owner_writer(connection, actor, position)?;
    Ok(())
}

pub(super) fn validate_owner_writer(connection: &Connection, actor: &str, position: Option<Position<'_>>) -> Result<()> {
    anyhow::ensure!(actor == notice::RUNTIME_ACTOR,
        "notices unavailable: only {} writes daemon notices", notice::RUNTIME_ACTOR);
    let (owner, membership) = owner(connection)?;
    let position = position.ok_or_else(|| unavailable("a daemon notice needs its writer position"))?;
    anyhow::ensure!(
        position.origin == owner.member
            && membership.keys_for(position.origin, position.sequence).contains(&owner.member_key),
        "notices unavailable: only the bound notice owner's daemon writes daemon notices"
    );
    Ok(())
}

/// Bind new emissions to the selected declaration before they enter immutable history.
pub(super) fn pin_digest_owner(
    tx: &Transaction<'_>, origin: &str, fields: &mut BTreeMap<String, Value>,
) -> Result<()> {
    validate_owner_writer(tx, notice::RUNTIME_ACTOR, Some(Position::next(tx, origin)?))?;
    let declaration = current_desired_row_tx(tx, notice_owner::SUBJECT)?
        .ok_or_else(|| unavailable("missing notice-owner declaration"))?;
    let source = fields.get_mut("source").and_then(Value::as_object_mut)
        .ok_or_else(|| unavailable("missing digest source metadata"))?;
    anyhow::ensure!(source.get("owner_claim").is_none_or(|value| value.as_str() == Some(&declaration.claim_id)),
        "notices unavailable: digest pins another owner declaration");
    source.insert("owner_claim".into(), json!(declaration.claim_id));
    Ok(())
}

fn validate_digest_writer(
    connection: &Connection, actor: &str, fields: &BTreeMap<String, Value>, position: Option<Position<'_>>,
) -> Result<()> {
    let id = fields.get("source").and_then(|source| source.get("owner_claim")).and_then(Value::as_str)
        .ok_or_else(|| unavailable("only the bound notice owner's daemon writes daemon notices: missing owner claim"))?;
    let declaration = claim_by_id_tx(connection, id)?
        .filter(|claim| claim.subject == notice_owner::SUBJECT && claim.kind == "intent.desired")
        .ok_or_else(|| unavailable("owner declaration is unavailable"))?;
    let repaired: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND state='repaired')",
        [id], |row| row.get(0),
    )?;
    anyhow::ensure!(!repaired, "notices unavailable: owner declaration was repaired");
    let position = position.ok_or_else(|| unavailable("missing digest writer position"))?;
    if declaration.origin == position.origin {
        anyhow::ensure!(Position::written(connection, &declaration)?.sequence <= position.sequence,
            "notices unavailable: owner declaration follows the digest emission");
    }
    let desired: DesiredSubject = serde_json::from_value(declaration.body)?;
    anyhow::ensure!(desired.subject == notice_owner::SUBJECT && desired.kind == notice_owner::KIND,
        "notices unavailable: invalid owner declaration identity");
    let bound = notice_owner::parse(&desired.desired).map_err(anyhow::Error::new)?;
    let membership = fleet_membership_tx(connection)?;
    anyhow::ensure!(actor == notice::RUNTIME_ACTOR && position.origin == bound.member
        && membership.keys_for(position.origin, position.sequence).contains(&bound.member_key),
        "notices unavailable: only the bound notice owner's daemon writes daemon notices");
    Ok(())
}

/// Authenticate immutable emissions independently of today's owner. Raw claim existence,
/// daemon actor names and payload host strings are not emission evidence.
pub(super) fn authenticated_digest_source(connection: &Connection, claim: &ClaimRecord) -> Result<bool> {
    if !claim.subject.starts_with(notice::DIGEST_SOURCE_PREFIX)
        || claim.kind != "custom.attention.notice-created" {
        return Ok(false);
    }
    let repaired: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND state='repaired')",
        [&claim.id], |row| row.get(0),
    )?;
    if repaired { return Ok(false); }
    let fields = custom::fields(claim);
    let manifest = &notice::manifests()[0];
    if fields.get("_registration").and_then(Value::as_str) != Some(notice::HASHES[0])
        || fractal_schema::custom::validate_fields(&manifest.claims[&claim.kind], &fields).is_err()
        || validate_digest_identity(&claim.subject, claim.actor.as_deref().unwrap_or_default(), &fields).is_err() {
        return Ok(false);
    }
    let position = Position::written(connection, claim)?;
    match validate_digest_writer(connection, claim.actor.as_deref().unwrap_or_default(), &fields, Some(position)) {
        Ok(()) => Ok(true),
        Err(error) if error.downcast_ref::<rusqlite::Error>().is_some() => Err(error),
        Err(_) => Ok(false),
    }
}

pub(super) fn retain_authenticated_digest_sources(
    connection: &Connection, subject: &str, history: &mut Vec<ClaimRecord>,
) -> Result<()> {
    if subject.starts_with(notice::DIGEST_SOURCE_PREFIX) {
        let mut failure = None;
        history.retain(|claim| match authenticated_digest_source(connection, claim) {
            Ok(authenticated) => authenticated,
            Err(error) => { failure = Some(error); false }
        });
        if let Some(error) = failure { return Err(error); }
    }
    Ok(())
}

fn validate_digest_identity(subject: &str, actor: &str, fields: &BTreeMap<String, Value>) -> Result<()> {
    anyhow::ensure!(actor == notice::RUNTIME_ACTOR && !fields.contains_key("producer"),
        "notices unavailable: digest sources are daemon-owned without a producer field");
    let recipient = fields.get("recipient").and_then(Value::as_str)
        .filter(|person| person.starts_with("person/"))
        .ok_or_else(|| unavailable("digest recipient must be a person"))?;
    let suffix = subject.strip_prefix(notice::DIGEST_SOURCE_PREFIX).unwrap_or_default();
    let date = suffix.strip_prefix(recipient).and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| unavailable("digest identity must bind its recipient"))?;
    anyhow::ensure!(date.len() == 10 && date.bytes().enumerate().all(|(index, byte)|
        if index == 4 || index == 7 { byte == b'-' } else { byte.is_ascii_digit() }),
        "notices unavailable: digest identity needs a local date");
    Ok(())
}

fn validate_digest_source(connection: &Connection, subject: &str, actor: &str, fields: &BTreeMap<String, Value>) -> Result<()> {
    validate_digest_identity(subject, actor, fields)?;
    let source = fields.get("source").ok_or_else(|| unavailable("digest source needs metadata"))?;
    anyhow::ensure!(source.get("digest").is_some() || source.get("digest_document").is_some(),
        "notices unavailable: digest source needs digest rows");
    if let Some(reference) = source.get("digest_document").and_then(Value::as_str) {
        anyhow::ensure!(source.get("digest").is_none(), "notices unavailable: digest has two payloads");
        let (name, hash) = split_document_ref(reference)?;
        anyhow::ensure!(name == format!("doc/ask-digest/{hash}")
            && source.get("digest_count").and_then(Value::as_str).and_then(|count| count.parse::<u64>().ok()).is_some(),
            "notices unavailable: invalid digest document reference or count");
        let bytes: Vec<u8> = connection.query_row(
            "SELECT b.bytes FROM documents d JOIN blobs b ON b.hash=d.hash WHERE d.name=?1 AND d.hash=?2 AND length(b.bytes)<65536",
            params![name, hash], |row| row.get(0),
        )?;
        anyhow::ensure!(bytes.len() < 64 * 1024 && hex::encode(Sha256::digest(&bytes)) == hash,
            "notices unavailable: invalid digest backing root");
        // Every chunk is immutable and checked by the bounded payload reader. Walking all
        // rows here would turn final source admission into unbounded writer work.
        return Ok(());
    }
    let rows = source.get("digest").and_then(Value::as_array)
        .ok_or_else(|| unavailable("digest source needs digest rows"))?;
    anyhow::ensure!(rows.len() <= 128, "notices unavailable: inline digest exceeds row bound");
    for row in rows {
        let row = row.as_object().ok_or_else(|| unavailable("digest row must be an object"))?;
        anyhow::ensure!(row.keys().all(|key| matches!(key.as_str(),
            "resolution_id" | "title" | "requester_id" | "default_answer" | "resolved_at" | "target_id" | "episode" | "action_parameters")),
            "notices unavailable: unknown digest row field");
        for field in ["resolution_id", "title", "requester_id", "resolved_at", "target_id", "episode"] {
            anyhow::ensure!(row.get(field).and_then(Value::as_str).is_some_and(|value| !value.is_empty()),
                "notices unavailable: digest row needs {field}");
        }
        if let Some(parameters) = row.get("action_parameters") {
            anyhow::ensure!(parameters == &json!({"work.override":{
                "target_id": row["target_id"], "episode": row["episode"]
            }}), "notices unavailable: digest override must bind its row");
        }
        anyhow::ensure!(row.get("default_answer").is_none_or(Value::is_object),
            "notices unavailable: digest default answer must be an object");
    }
    Ok(())
}

/// A source is a producer's own input: an agent or person, never a daemon, and never pinning
/// the runtime, so its lineage survives an owner handover.
fn validate_source(producer: &str, fields: &BTreeMap<String, Value>) -> Result<()> {
    anyhow::ensure!(fields.get("source").and_then(|source| source.get("digest")).is_none(),
        "notices unavailable: digest rows require a digest source");
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
