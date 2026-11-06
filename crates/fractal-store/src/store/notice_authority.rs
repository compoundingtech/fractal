//! Notice admission policy. B2 supplies replicated admission cuts and revocation ordering;
//! payload timestamps and declaration hashes are deliberately never authority evidence here.
use super::*;
use fractal_schema::{custom::Manifest, notice};

fn unavailable(reason: impl Into<String>) -> FractalError {
    FractalError::new("notices-unavailable", format!("notices unavailable: {}", reason.into()))
}

fn holders(connection: &Connection) -> Result<Vec<String>> {
    let mut query = connection.prepare(
        "SELECT subject FROM desired WHERE kind='agent' AND EXISTS (
         SELECT 1 FROM json_each(desired.body, '$.children')
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
    anyhow::ensure!(custom::claims(connection, source_id)?.iter()
        .filter(|claim| claim.kind == source_manifest.creation_kind).count() == 1,
        "notices unavailable: conflicting source creations");
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
