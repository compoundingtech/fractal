//! Conservative claim-derived failure scope; a missing bounded proof means global fencing.
use super::*;

#[cfg(test)]
thread_local! {
    pub(super) static OMIT_OTHER_CLAIMANTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn check(connection: &Connection, name: &str) -> Result<(), FractalError> {
    if IdentityKind::of(name).is_none() { return Ok(()); }
    let blocked: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate='projection:naming-operation-conflict'
         AND (COALESCE(json_extract(error_message,'$.scope'),'global')<>'scoped'
              OR EXISTS(SELECT 1 FROM meta WHERE key='operation-conflict-naming-name:' || ?1)))",
        [name], |row| row.get(0),
    ).map_err(internal)?;
    if blocked { return Err(unavailable(name, "operation-conflict-naming-refold-limit")); }
    Ok(())
}

pub(super) fn clear(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h864348ae5781 owner=fractal_store::store::identities_conflict_scope TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key GLOB 'operation-conflict-naming-name:*'", [])?;
    Ok(())
}

pub(super) fn persist(transaction: &Transaction<'_>, report: &Value) -> Result<()> {
    clear(transaction)?;
    if report["scope"] == "scoped" {
        for name in report["affected_names"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h05bf16937ee4 owner=fractal_store::store::identities_conflict_scope TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2)",
                params![format!("operation-conflict-naming-name:{name}"), "1"])?;
        }
    }
    Ok(())
}

const BY_OPERATION: &str = "SELECT subject,kind,json_extract(body,'$.fields.from'),json_extract(body,'$.fields.to')
 FROM claims INDEXED BY claims_operation_index WHERE json_extract(body,'$._operation.id')=?1 LIMIT ?2";
const BY_SUBJECT: &str = "SELECT subject,kind,json_extract(body,'$.fields.from'),json_extract(body,'$.fields.to')
 FROM claims INDEXED BY claims_subject_kind_index WHERE subject=?1
 AND kind IN ('agent.renamed','mission.renamed','intent.desired','mission.published') LIMIT ?2";
const BY_FROM: &str = "SELECT subject,kind,json_extract(body,'$.fields.from'),json_extract(body,'$.fields.to')
 FROM claims INDEXED BY claims_naming_from_index WHERE json_extract(body,'$.fields.from')=?1
 AND kind IN ('agent.renamed','mission.renamed') LIMIT ?2";
const BY_TO: &str = "SELECT subject,kind,json_extract(body,'$.fields.from'),json_extract(body,'$.fields.to')
 FROM claims INDEXED BY claims_naming_to_index WHERE json_extract(body,'$.fields.to')=?1
 AND kind IN ('agent.renamed','mission.renamed') LIMIT ?2";

fn absorb(connection: &Connection, query: &str, key: &str, remaining: &mut usize,
    visited: &BTreeSet<String>, pending: &mut BTreeSet<String>) -> Result<bool> {
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h522af0cfffad owner=fractal_store::store::identities_conflict_scope TODO=trace-statement-and-value-representation */ query)?;
    let mut rows = statement.query(params![key,*remaining + 1])?;
    while let Some(row) = rows.next()? {
        if *remaining == 0 { return Ok(false); }
        *remaining -= 1;
        let subject: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let (from, to) = if KINDS.contains(&kind.as_str()) {
            let (Some(from), Some(to)) = (row.get::<_,Option<String>>(2)?, row.get::<_,Option<String>>(3)?)
                else { return Ok(false); };
            (Some(from), Some(to))
        } else { (None, None) };
        for name in [Some(subject), from, to].into_iter().flatten() {
            if IdentityKind::of(&name).is_some() && !visited.contains(&name) { pending.insert(name); }
        }
    }
    Ok(true)
}

fn derive(connection: &Connection, operations: &BTreeSet<String>) -> Result<Option<BTreeSet<String>>> {
    let mut remaining = OPERATION_CONFLICT_NAMING_CLAIMS;
    let mut visited = BTreeSet::new();
    let mut pending = BTreeSet::new();
    for operation in operations {
        // Tombstones cannot prove the previous/destination name closure without their bodies.
        let dropped: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM checkpoint_claims WHERE operation_id=?1
             AND kind IN ('agent.renamed','mission.renamed','intent.desired','mission.published'))",
            [operation], |row| row.get(0),
        )?;
        if dropped || !absorb(connection, BY_OPERATION, operation, &mut remaining, &visited, &mut pending)? {
            return Ok(None);
        }
    }
    while let Some(name) = pending.pop_first() {
        visited.insert(name.clone());
        if !absorb(connection, BY_SUBJECT, &name, &mut remaining, &visited, &mut pending)? { return Ok(None); }
        #[cfg(test)]
        if OMIT_OTHER_CLAIMANTS.get() { continue; }
        // Follow both former names and destinations to every competing identity, then its
        // complete name history. Count repeated rows too: all indexed hydration is capped.
        for query in [BY_FROM, BY_TO] {
            if !absorb(connection, query, &name, &mut remaining, &visited, &mut pending)? { return Ok(None); }
        }
    }
    Ok(Some(visited))
}

fn enrich(connection: &Transaction<'_>, mut report: Value, operations: &BTreeSet<String>, global: bool) -> Result<Value> {
    let names = if global || operations.len() > OPERATION_CONFLICT_NAMING_CLAIMS {
        None
    } else { derive(connection, operations)? };
    report["scope"] = json!(if names.is_some() { "scoped" } else { "global" });
    report["affected_names"] = names.map_or_else(|| json!(["*"]), |names| json!(names));
    report["operation_ids"] = json!(operations.iter().take(OPERATION_CONFLICT_NAMING_CLAIMS).collect::<Vec<_>>());
    report["scope_store_index"] = json!(current_index_tx(connection)?);
    report["repair_command"] = json!("run fractal replication replay --confirm in a planned window");
    Ok(report)
}

fn saved(connection: &Connection) -> Result<Option<Value>> {
    let report: Option<String> = connection.query_row(
        "SELECT error_message FROM projection_health WHERE aggregate='projection:naming-operation-conflict'",
        [], |row| row.get(0),
    ).optional()?;
    report.map(|report| serde_json::from_str(&report).map_err(Into::into)).transpose()
}

fn operations(report: &Value) -> BTreeSet<String> {
    report["operation_ids"].as_array().into_iter().flatten()
        .filter_map(Value::as_str).map(str::to_owned).collect()
}

pub(super) fn report(connection: &Transaction<'_>, details: &serde_json::Map<String, Value>, incoming: &BTreeSet<String>) -> Result<Value> {
    let previous = saved(connection)?;
    let mut operations = previous.as_ref().map_or_else(BTreeSet::new, operations);
    operations.extend(incoming.iter().cloned());
    let global = previous.as_ref().is_some_and(|report| report["scope"] != "scoped");
    enrich(connection, json!(details), &operations, global)
}

pub(super) fn refresh(connection: &Transaction<'_>) -> Result<Option<Value>> {
    let Some(report) = saved(connection)? else { return Ok(None); };
    if report["scope"] != "scoped" || report["scope_store_index"].as_u64() == Some(current_index_tx(connection)?) {
        return Ok(None);
    }
    // A later naming claim may introduce another claimant; never trust the cached fold or
    // an old scoped proof. Re-derive at most 64 indexed rows, otherwise fence globally.
    let operations = operations(&report);
    Ok(Some(enrich(connection, report, &operations, false)?))
}
