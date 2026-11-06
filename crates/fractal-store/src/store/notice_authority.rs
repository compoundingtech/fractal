//! Notice admission policy. B2 supplies replicated admission cuts and revocation ordering;
//! payload timestamps and declaration hashes are deliberately never authority evidence here.
use super::*;
use fractal_schema::{custom::Manifest, notice};

/// Watch the projection tables used by declaration_live, not only newly authored claims.
/// Retirement, owned declarations and ancestor-run liveness can change without intent.desired.
pub(super) fn create_invalidation_triggers(connection: &Connection) -> Result<()> {
    let run_id = |column: &str| format!(
        "CASE WHEN {column} LIKE 'mission-run/%' THEN substr({column},13) ELSE {column} END"
    );
    // Keep missing dependencies in the traversal: their later insertion can restore liveness.
    // BEFORE triggers retain the old chain when deleting or replacing one of its links.
    let chain = format!(
        "WITH RECURSIVE notice_runs(id) AS (
         SELECT {owner} FROM desired WHERE kind='agent' AND owner_run IS NOT NULL AND EXISTS(
          SELECT 1 FROM json_each(CASE WHEN json_valid(desired.body) THEN desired.body ELSE '{{}}' END,'$.children') WHERE json_extract(value,'$.name')='notice-runtime')
         UNION SELECT {parent} FROM notice_runs n JOIN mission_runs r ON r.id=n.id
          LEFT JOIN step_runs s ON s.subject=r.parent_step_run
          LEFT JOIN desired d ON d.subject=substr(r.parent_step_run,10)
          WHERE r.parent_step_run IS NOT NULL AND coalesce(s.run_id,d.owner_run) IS NOT NULL
         UNION SELECT {root} FROM notice_runs n JOIN mission_runs r ON r.id=n.id
          WHERE r.parent_step_run IS NULL AND r.root_run_id IS NOT NULL)",
        owner=run_id("owner_run"), parent=run_id("coalesce(s.run_id,d.owner_run)"),
        root=run_id("r.root_run_id"),
    );
    for (table, columns) in [
        ("desired", &["subject", "kind", "body", "owner_run", "owner_generation"][..]),
        ("mission_runs", &["id", "status", "current_generation_id", "parent_step_run", "root_run_id"][..]),
        ("step_runs", &["subject", "status", "run_id", "generation_id"][..]),
        ("run_generations", &["id"][..]),
    ] {
        for action in ["INSERT", "UPDATE", "DELETE"] {
            let references = match action { "INSERT" => &["NEW"][..], "DELETE" => &["OLD"][..], _ => &["OLD", "NEW"][..] };
            let relevant = references.iter().map(|row| {
                let dependency = match table {
                    "desired" => format!(
                        "EXISTS(SELECT 1 FROM json_each(CASE WHEN json_valid({row}.body) THEN {row}.body ELSE '{{}}' END,'$.children')
                          WHERE json_extract(value,'$.name')='notice-runtime')
                         OR EXISTS({chain} SELECT 1 FROM notice_runs n JOIN mission_runs r ON r.id=n.id
                          WHERE r.parent_step_run='step-run/'||{row}.subject)"
                    ),
                    "mission_runs" => format!("EXISTS({chain} SELECT 1 FROM notice_runs WHERE id={row}.id)"),
                    "step_runs" => format!(
                        "EXISTS({chain} SELECT 1 FROM notice_runs n JOIN mission_runs r ON r.id=n.id
                         WHERE r.parent_step_run={row}.subject)"
                    ),
                    _ => format!(
                        "EXISTS({chain} SELECT 1 FROM notice_runs n JOIN mission_runs r ON r.id=n.id
                         WHERE r.current_generation_id={row}.id)"
                    ),
                };
                format!("({dependency})")
            }).collect::<Vec<_>>().join(" OR ");
            let changed = if action == "UPDATE" {
                columns.iter().map(|column| format!("OLD.{column} IS NOT NEW.{column}"))
                    .collect::<Vec<_>>().join(" OR ")
            } else { "1".into() };
            let name = format!("notice_authority_{table}_{action}");
            let sql = format!(
                /* sqlite-large-value: allow-for-now lv-h6b344b76fae0 owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */
                "CREATE TRIGGER {name} BEFORE {action} ON {table}
                 WHEN ({relevant}) AND ({changed}) BEGIN
                 INSERT INTO local_custom_dirty SELECT subject FROM custom_sources
                 WHERE subject LIKE 'custom/attention/%' ON CONFLICT(subject) DO NOTHING; END"
            );
            let installed: Option<String> = connection.query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [&name], |row| row.get(0),
            ).optional()?;
            if installed.as_deref() != Some(sql.as_str()) {
                connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-ha3325e0e8f6d owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER IF EXISTS {name}; {sql};"))?;
            }
        }
    }
    Ok(())
}

/// Authority dependencies are invalidation edges, not optional projection `_basis` inputs.
/// Retain links from all facts, including unpinned, pending and invalid facts, so a later
/// source arrival, repair or competing creation revalidates every linked row.
pub(super) fn record_dependencies(tx: &Transaction<'_>, subject: &str, history: &[ClaimRecord]) -> Result<()> {
    if !subject.starts_with(notice::SNAPSHOT_PREFIX) && !subject.starts_with(notice::CONTROL_PREFIX) {
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

fn holders(connection: &Connection) -> Result<Vec<String>> {
    let mut query = connection.prepare(
        "SELECT subject FROM desired WHERE kind='agent' AND EXISTS (
         SELECT 1 FROM json_each(CASE WHEN json_valid(desired.body) THEN desired.body ELSE '{}' END, '$.children')
         WHERE json_extract(value, '$.name')='notice-runtime') ORDER BY subject",
    )?;
    let candidates = query.query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut live = Vec::new();
    for candidate in candidates {
        if person_work::declaration_live(connection, &candidate)? { live.push(candidate); }
    }
    Ok(live)
}

fn runtime(connection: &Connection) -> Result<String, FractalError> {
    let live = holders(connection).map_err(internal)?;
    match live.as_slice() {
        [actor] => Ok(actor.clone()),
        [] => Err(unavailable("missing notice-runtime designation")),
        _ => Err(unavailable("duplicate notice-runtime designations")),
    }
}

/// Compare the resolver's actual outcome, not a duplicated identity/lifecycle watch list.
/// Identity heads and pending records can fence a holder without changing desired at all.
pub(super) fn refresh_binding(tx: &Transaction<'_>) -> Result<()> {
    let outcome = match runtime(tx) {
        Ok(actor) => json!({"runtime":actor}),
        Err(error) => json!({"unavailable":{"code":error.code,"message":error.message,"details":error.details}}),
    };
    let current = canonical_json_text(&outcome)?;
    let previous: Option<String> = tx.query_row(
        "SELECT value FROM meta WHERE key='notice_runtime_binding'", [], |row| row.get(0),
    ).optional()?;
    if previous.as_deref() != Some(current.as_str()) {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h80b0a7485e7e owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_custom_dirty SELECT subject FROM custom_sources
            WHERE subject LIKE 'custom/attention/%' ON CONFLICT(subject) DO NOTHING", [])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h9d1a487a758c owner=fractal_store::store::notice_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('notice_runtime_binding',?1)", [&current])?;
    }
    Ok(())
}

pub(super) fn refuse_second_holder(
    connection: &Connection, intent: &crate::model::NormalizedIntent,
) -> Result<(), FractalError> {
    let declares = |desired: &crate::model::DesiredSubject| {
        let children = desired.desired["children"].as_array().map(Vec::as_slice).unwrap_or_default();
        desired.kind == "agent" && !matches!(children, [only] if only["name"] == "stop")
            && children.iter().any(|child| child["name"] == "notice-runtime")
    };
    let declared = intent.subjects.values().filter(|d| declares(d)).count();
    if declared == 0 { return Ok(()); }
    let retained = holders(connection).map_err(internal)?.iter()
        .filter(|holder| !intent.subjects.contains_key(*holder)).count();
    if declared + retained > 1 {
        return Err(unavailable("only one live agent may declare notice-runtime"));
    }
    Ok(())
}

pub(super) fn validate(
    connection: &Connection, subject: &str, manifest: &Manifest, kind: &str,
    actor: &str, fields: &BTreeMap<String, Value>, creation: Option<&ClaimRecord>,
) -> Result<()> {
    if !notice::is_reserved_subject(subject) && !notice::is_reserved_kind(kind) { return Ok(()); }
    let hash = fields.get("_registration").and_then(Value::as_str)
        .ok_or_else(|| unavailable("missing registration pin"))?;
    notice::validate_registration(manifest, hash)?;
    anyhow::ensure!(subject.starts_with(&manifest.subject_prefix), "notices unavailable: claim/prefix mismatch");
    let runtime = runtime(connection)?;
    if subject.starts_with("custom/attention/source/") {
        anyhow::ensure!(kind == manifest.creation_kind, "notices unavailable: producer input is immutable");
        if let Some(prior) = creation {
            anyhow::ensure!(prior.actor.as_deref() == Some(actor) && custom::fields(prior) == *fields,
                "notices unavailable: source creation conflict");
        }
        for (field, expected) in [("producer", actor), ("runtime", runtime.as_str())] {
            anyhow::ensure!(fields.get(field).is_none_or(|value| value.as_str() == Some(expected)),
                "notices unavailable: forged {field}");
        }
        anyhow::ensure!(fields.get("recipient").and_then(Value::as_str).is_some_and(|p| p.starts_with("person/")),
            "notices unavailable: source recipient must be a person");
        return Ok(());
    }
    let source_id = fields.get("source_id").and_then(Value::as_str)
        .filter(|s| s.starts_with("custom/attention/source/"))
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
    validate(connection, source_id, &source_manifest, &source.kind, producer, &source_fields, None)?;
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
    if subject.starts_with("custom/attention/snapshot/") {
        anyhow::ensure!(actor == runtime, "notices unavailable: only the designated runtime may write snapshots");
    } else {
        anyhow::ensure!(subject.starts_with("custom/attention/control/") && actor.starts_with("person/") && actor == recipient,
            "notices unavailable: only the source recipient may write person controls");
    }
    Ok(())
}

#[cfg(test)]
#[path = "notice_authority_tests.rs"]
mod tests;
