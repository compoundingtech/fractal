//! Terminal agent identity and first-send admission share the serialized writer.
use super::*;
use crate::model::{LifecycleUnavailableReason, MessageSendRefusal};

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h3c637414d605 owner=fractal_store::store::retirements TODO=chunk-or-reference-variable-columns */ "CREATE INDEX IF NOT EXISTS st3_replica_lifecycle_unavailable
    ON replica_records(subject_hint,kind_hint) WHERE state IN ('unknown','invalid');
    CREATE INDEX IF NOT EXISTS st3_lifecycle_operation_index
    ON claims(subject,json_extract(body,'$._operation.id'))
    WHERE kind IN ('agent.retired','intent.desired') AND json_extract(body,'$._operation.id') IS NOT NULL;";

pub const KIND: &str = "agent.retired";

pub fn writer_required() -> St3Error {
    St3Error::new(
        "invalid-retirement",
        "terminal retirement must be written through agent.retire",
    )
    .with_detail("applied", "none")
}

/// Every declaration selector, including mission materialization, obeys the same terminal
/// reducer. The trigger probes only the changed subject; it never scans the fleet on writes.
pub fn create_selection_triggers(connection: &Connection) -> Result<()> {
    for (name, event) in [
        ("insert", "INSERT"),
        ("update", "UPDATE OF kind,member,body"),
    ] {
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-hd26e22a8e180 owner=fractal_store::store::retirements TODO=chunk-or-reference-exact-wire-values */ "CREATE TRIGGER IF NOT EXISTS st3_retired_desired_{name} AFTER {event} ON desired
             WHEN (NEW.kind != 'stop' OR NEW.member IS NOT NULL OR json_extract(NEW.body,'$.retirement') IS NULL)
               AND EXISTS(SELECT 1 FROM claims WHERE subject=NEW.subject AND kind='agent.retired'
                 AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired'))
             BEGIN UPDATE desired SET kind='stop',member=NULL,
               body=json_object('arguments',json_array(NEW.subject),'children',json_array(), 'name','stop',
                 'properties',json_object(),'retirement',(SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
                  WHERE claims.subject=NEW.subject AND claims.kind='agent.retired'
                   AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
                  ORDER BY {CANONICAL_ORDER_DESC} LIMIT 1)) WHERE subject=NEW.subject; END;"
        ))?;
    }
    Ok(())
}

pub fn record(
    connection: &Connection,
    subject: &str,
    at: Option<u64>,
) -> Result<Option<ClaimRecord>, St3Error> {
    connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h70beb7474d50 owner=fractal_store::store::retirements TODO=trace-statement-and-value-representation */ &format!(
        "SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 AND claims.kind='agent.retired' AND (?2 IS NULL OR claims.store_index<=?2)
         AND NOT EXISTS(SELECT 1 FROM replica_records WHERE replica_records.claim_id=claims.id AND state='repaired')
         ORDER BY {CANONICAL_ORDER_DESC} LIMIT 1"
    )).map_err(internal)?.query_row(/* sqlite-large-value: allow-for-now lv-h42ae3e27a51b owner=fractal_store::store::retirements TODO=trace-statement-and-value-representation */ params![subject,at], claim_from_row).optional().map_err(internal)
}

/// Terminal identities for a page of agents, using the same admitted lifecycle records as
/// `record` and first-send admission. Read once per page rather than probing each card.
pub fn retired_subjects_at(
    connection: &Connection,
    subjects: &[String],
    at: u64,
) -> Result<BTreeSet<String>> {
    if subjects.is_empty() {
        return Ok(BTreeSet::new());
    }
    let subjects = serde_json::to_string(subjects)?;
    let mut statement = connection.prepare_cached(
        "SELECT DISTINCT claims.subject FROM json_each(?1) subjects
         CROSS JOIN claims ON claims.subject=subjects.value
         WHERE claims.kind='agent.retired' AND claims.store_index<=?2
           AND NOT EXISTS(SELECT 1 FROM replica_records
             WHERE replica_records.claim_id=claims.id AND state='repaired')",
    )?;
    Ok(statement
        .query_map(params![subjects, at], |row| row.get(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?)
}

fn retired(recipient: &str, claim: &ClaimRecord) -> St3Error {
    MessageSendRefusal::Retired {
        recipient: recipient.into(),
        successor: claim.body["fields"]["successor"]
            .as_str()
            .map(str::to_owned),
        transfer: claim.body["fields"]["transfer"].as_str().map(str::to_owned),
    }
    .into_error()
}

pub fn refuse_declaration(
    connection: &Connection,
    desired: &DesiredSubject,
) -> Result<(), St3Error> {
    if desired.kind == "agent"
        && let Some(claim) = record(connection, &desired.subject, None)?
    {
        return Err(retired(&desired.subject, &claim));
    }
    Ok(())
}

/// Terminal identity supersedes owned membership too. An effect prepared from the old
/// declaration must fail its authority check; only the canonical terminal stop is current.
pub fn terminal_selection(
    connection: &Connection,
    desired: &DesiredSubject,
) -> Result<Option<bool>, St3Error> {
    if !desired.subject.starts_with("agent/") {
        return Ok(None);
    }
    Ok(record(connection, &desired.subject, None)?.map(|claim| {
        desired.kind == "stop"
            && desired.member.is_none()
            && desired.desired
                == json!({"name":"stop", "arguments":[desired.subject],
                "properties":{}, "children":[], "retirement":claim.id})
    }))
}

pub fn admit_recipient(
    connection: &Connection,
    recipient: Option<&str>,
) -> Result<(), St3Error> {
    let Some(recipient) = recipient.filter(|to| to.starts_with("agent/")) else {
        return Ok(());
    };
    let pending: bool = connection.prepare_cached("SELECT EXISTS(SELECT 1 FROM replica_records INDEXED BY st3_replica_lifecycle_unavailable
        WHERE subject_hint=?1 AND state IN ('unknown','invalid') AND (kind_hint IN ('agent.retired','intent.desired') OR kind_hint IS NULL))")
        .map_err(internal)?.query_row(/* sqlite-large-value: allow-for-now lv-h4eb2f4d0f48a owner=fractal_store::store::retirements TODO=trace-statement-and-value-representation */ [recipient], |row| row.get(0)).map_err(internal)?;
    if pending {
        return Err(unavailable(
            recipient,
            LifecycleUnavailableReason::Unavailable,
        ));
    }
    let conflict: bool = connection.prepare_cached("SELECT EXISTS(SELECT 1 FROM claims INDEXED BY st3_lifecycle_operation_index
        CROSS JOIN operations ON operations.id=json_extract(claims.body,'$._operation.id')
        WHERE claims.subject=?1 AND claims.kind IN ('agent.retired','intent.desired')
          AND json_extract(claims.body,'$._operation.id') IS NOT NULL AND operations.state='conflict'
          AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired'))")
        .map_err(internal)?.query_row(/* sqlite-large-value: allow-for-now lv-hc6278276aa34 owner=fractal_store::store::retirements TODO=trace-statement-and-value-representation */ [recipient], |row| row.get(0)).map_err(internal)?;
    if conflict {
        return Err(unavailable(recipient, LifecycleUnavailableReason::Conflict));
    }
    if let Some(claim) = record(connection, recipient, None)? {
        let another: bool = connection.prepare_cached("SELECT EXISTS(SELECT 1 FROM claims
            WHERE subject=?1 AND kind='agent.retired' AND id<>?2
              AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired'))")
            .map_err(internal)?.query_row(/* sqlite-large-value: allow-for-now lv-ha00b1a8a0c42 owner=fractal_store::store::retirements TODO=trace-statement-and-value-representation */ params![recipient,claim.id],|row|row.get(0)).map_err(internal)?;
        if another {
            return Err(unavailable(recipient, LifecycleUnavailableReason::Conflict));
        }
        let token = claim.body["fields"]["declaration"]
            .as_str()
            .ok_or_else(|| unavailable(recipient, LifecycleUnavailableReason::Conflict))?;
        let declared = declaration(connection, recipient, token)?;
        if declared
            .member
            .as_ref()
            .is_none_or(|member| member.host != claim.origin)
            || claim.body["fields"]["owner_host"].as_str() != Some(&claim.origin)
        {
            return Err(unavailable(recipient, LifecycleUnavailableReason::Conflict));
        }
        return Err(retired(recipient, &claim));
    }
    Ok(())
}

/// Runtime selection lowers terminal identity to a stop, retaining the declaration token and
/// ownership history. The terminal claim remains separate and can never be overwritten by KDL.
pub fn project_subject(connection: &Connection, subject: &str) -> Result<(), St3Error> {
    if let Some(claim) = record(connection, subject, None)? {
        let body = json!({"name":"stop", "arguments":[subject], "properties":{}, "children":[], "retirement":claim.id});
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h0a4f7456ebe4 owner=fractal_store::store::retirements TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET kind='stop',member=NULL,body=?2 WHERE subject=?1",
                params![subject, canonical_json_text(&body).map_err(internal)?],
            )
            .map_err(internal)?;
    }
    Ok(())
}

pub fn overlay_row(
    connection: &Connection,
    subject: &str,
    at: Option<u64>,
    mut row: Option<DesiredRow>,
) -> Result<Option<DesiredRow>> {
    if let Some(retirement) = record(connection, subject, at).map_err(anyhow::Error::new)?
        && let Some(row) = row.as_mut()
    {
        row.kind = "stop".into();
        row.member = None;
        row.body = canonical_json_text(
            &json!({"name":"stop", "arguments":[subject], "properties":{}, "children":[], "retirement":retirement.id}),
        )?;
    }
    Ok(row)
}

pub fn unavailable(subject: &str, reason: LifecycleUnavailableReason) -> St3Error {
    MessageSendRefusal::LifecycleUnavailable {
        recipient: subject.into(),
        reason,
    }
    .into_error()
}

/// A stop preserves its original declaration through causal predecessors, including a seat
/// stopped before retirement. Ambiguous ownership is never guessed from a runtime observation.
pub fn declaration(
    connection: &Connection,
    subject: &str,
    token: &str,
) -> Result<DesiredSubject, St3Error> {
    let mut claim = claim_by_id_tx(connection, token)
        .map_err(internal)?
        .ok_or_else(|| unavailable(subject, LifecycleUnavailableReason::Unavailable))?;
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(claim.id.clone()) {
            return Err(unavailable(subject, LifecycleUnavailableReason::Conflict));
        }
        if claim.subject != subject || claim.kind != "intent.desired" {
            return Err(unavailable(subject, LifecycleUnavailableReason::Conflict));
        }
        let desired: DesiredSubject =
            serde_json::from_value(claim.body.clone()).map_err(internal)?;
        if desired.kind == "agent" {
            return Ok(desired);
        }
        if desired.kind != "stop" || claim.predecessors.len() != 1 {
            return Err(unavailable(subject, LifecycleUnavailableReason::Conflict));
        }
        claim = claim_by_id_tx(connection, &claim.predecessors[0])
            .map_err(internal)?
            .ok_or_else(|| unavailable(subject, LifecycleUnavailableReason::Unavailable))?;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_producer_inventory_cannot_call_the_admitted_primitive() {
        fn scan(path: &std::path::Path, calls: &mut Vec<String>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    scan(&path, calls);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    let source = std::fs::read_to_string(&path).unwrap();
                    // The primitive is private. Only the common wrapper and the commit-context
                    // writer may call it; a newly introduced direct producer fails this guard.
                    for _ in source.match_indices("append_admitted_claim_tx(") {
                        calls.push(path.file_name().unwrap().to_str().unwrap().into());
                    }
                }
            }
        }
        let root = std::path::Path::new(test_env!("CARGO_MANIFEST_DIR")).join("src");
        let mut calls = Vec::new();
        scan(&root, &mut calls);
        // One occurrence in this guard itself, and three in store.rs (definition + two seams).
        calls.sort();
        assert_eq!(
            calls,
            vec!["retirements.rs", "store.rs", "store.rs", "store.rs"]
        );
    }
}
