//! Indexed field/source selection for roster presentation, not public full status responses.
use super::*;

pub(super) const FIELDS: [&str; 9] = [
    "status", "incarnation_id", "runtime_id", "reachability", "reason", "restarting_from", "mode",
    "exit_code", "exit_signal",
];
const NATIVE: &str = "claims.kind!='custom.attention' AND claims.kind NOT GLOB 'custom.attention.*'
    AND claims.kind!='attention.notice' AND claims.kind NOT GLOB 'attention.notice-*'";
const NONTERMINAL: &str = "COALESCE(json_extract(claims.body,
    CASE WHEN json_type(claims.body,'$.fields') IS NULL THEN '$.status' ELSE '$.fields.status' END)
    IN ('stopped','absent','exited','vanished'),0)=0";
static ELIGIBLE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| format!(
    "{ACTUAL_STATE_CLAIM} AND claims.kind NOT LIKE 'harness.%' AND {NATIVE}"
));

pub(super) fn indexes() -> String {
    let mut sql = String::new();
    for field in FIELDS {
        let predicate = identity_field_predicate(field);
        sql.push_str(&format!(
            /* sqlite-large-value: allow-for-now lv-h746c7066eb56 owner=fractal_store::store::roster_actual TODO=chunk-or-reference-variable-columns */
            "CREATE INDEX IF NOT EXISTS claims_roster_{field}_accepted_index
             ON claims(subject,length(accepted_at_unix_ms),accepted_at_unix_ms)
             WHERE {} AND claims.kind!='vcs.item.state' AND {predicate};", ELIGIBLE.as_str(),
        ));
    }
    sql.push_str(&format!(
        /* sqlite-large-value: allow-for-now lv-h6512f9758bfe owner=fractal_store::store::roster_actual TODO=chunk-or-reference-variable-columns */
        "CREATE INDEX IF NOT EXISTS claims_roster_actual_accepted_index
         ON claims(subject,length(accepted_at_unix_ms),accepted_at_unix_ms) WHERE {};
         CREATE INDEX IF NOT EXISTS claims_roster_nonterminal_accepted_index
         ON claims(subject,length(accepted_at_unix_ms),accepted_at_unix_ms)
         WHERE kind='runtime.observed' AND {NONTERMINAL};", ELIGIBLE.as_str(),
    ));
    sql
}

static FIELD_QUERIES: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| FIELDS.iter().map(|field| {
    let predicate = identity_field_predicate(field);
    format!(
        "SELECT claims.body -> CASE WHEN json_type(claims.body,'$.fields') IS NULL
             THEN '$.{field}' ELSE '$.fields.{field}' END
         FROM claims INDEXED BY claims_roster_{field}_accepted_index
         JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 AND +claims.store_index<=?2 AND {}
           AND claims.kind!='vcs.item.state' AND {predicate} AND {UNREPAIRED_CLAIM}
         ORDER BY {CANONICAL_ORDER_DESC} LIMIT 1", ELIGIBLE.as_str(),
    )
}).collect());

static EXISTS_QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| format!(
    "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_roster_actual_accepted_index
     WHERE claims.subject=?1 AND +claims.store_index<=?2 AND {} AND {UNREPAIRED_CLAIM})",
    ELIGIBLE.as_str(),
));

/// Missing fields terminate with an empty index seek, not a walk over every heartbeat.
/// Explicit JSON null is a value; an omitted transition field clears the previous value.
pub(super) fn fields(connection: &Connection, subject: &str, cut: u64) -> Result<Option<Value>> {
    let exists = connection.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-h2580ede53fb1 owner=fractal_store::store::roster_actual TODO=trace-statement-and-value-representation */
        &EXISTS_QUERY)?
        .query_row(
            /* sqlite-large-value: allow-for-now lv-hc8d90bf7dbe9 owner=fractal_store::store::roster_actual TODO=trace-statement-and-value-representation */
            params![subject,cut], |row| row.get::<_,bool>(0))?;
    if !exists { return Ok(None); }
    let mut merged = serde_json::Map::new();
    for (field, query) in FIELDS.iter().zip(FIELD_QUERIES.iter()) {
        let value = connection.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h58533854a830 owner=fractal_store::store::roster_actual TODO=trace-statement-and-value-representation */
            query)?.query_row(
                /* sqlite-large-value: allow-for-now lv-h1a646458bfff owner=fractal_store::store::roster_actual TODO=trace-statement-and-value-representation */
                params![subject,cut], |row| row.get::<_,Option<String>>(0)).optional()?.flatten();
        if let Some(value) = value {
            merged.insert((*field).into(),serde_json::from_str(&value)?);
        }
    }
    Ok(Some(Value::Object(merged)))
}

static SOURCE_QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| format!(
    "WITH RECURSIVE origins(origin) AS (
        SELECT (SELECT origin FROM claims INDEXED BY claims_runtime_origin_accepted_index
                WHERE subject=?1 AND kind='runtime.observed' ORDER BY origin LIMIT 1)
        UNION ALL
        SELECT (SELECT origin FROM claims INDEXED BY claims_runtime_origin_accepted_index
                WHERE subject=?1 AND kind='runtime.observed' AND origin>origins.origin
                ORDER BY origin LIMIT 1)
        FROM origins WHERE origin IS NOT NULL
     ), candidates(id) AS (
        SELECT (SELECT claims.id FROM claims INDEXED BY claims_runtime_origin_accepted_index
                JOIN batches ON batches.id=claims.batch_id
                WHERE claims.subject=?1 AND claims.origin=origins.origin
                  AND claims.kind='runtime.observed' AND +claims.store_index<=?2 AND {UNREPAIRED_CLAIM}
                ORDER BY {CANONICAL_ORDER_DESC} LIMIT 1)
        FROM origins WHERE origin IS NOT NULL
        UNION
        SELECT (SELECT claims.id FROM claims INDEXED BY claims_roster_nonterminal_accepted_index
                JOIN batches ON batches.id=claims.batch_id
                WHERE claims.subject=?1 AND claims.kind='runtime.observed' AND {NONTERMINAL}
                  AND +claims.store_index<=?2 AND {UNREPAIRED_CLAIM}
                ORDER BY {CANONICAL_ORDER_DESC} LIMIT 1)
     ) SELECT claims.id,claims.kind,claims.origin,claims.body FROM claims
       JOIN batches ON batches.id=claims.batch_id
       WHERE claims.id IN (SELECT id FROM candidates) ORDER BY {CANONICAL_ORDER}"
));

/// Preserve the full selector's older nonterminal candidate as well as newest-per-origin rivals.
/// A non-owner's terminal receipt can be ignored after the declared owner observed the seat.
pub(super) fn source(
    connection: &Connection, subject: &str, cut: u64, host: Option<&str>,
) -> Result<Option<(String,String,Vec<String>)>> {
    let rows = connection.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-he9d77ba77efb owner=fractal_store::store::roster_actual TODO=trace-statement-and-value-representation */
        &SOURCE_QUERY)?.query_map(params![subject,cut], |row| {
        Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        return connection.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h069071af96ea owner=fractal_store::store::roster_actual TODO=trace-statement-and-value-representation */
            &format!(
            "SELECT claims.id,claims.origin FROM claims INDEXED BY claims_actual_accepted_index
             JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject=?1 AND +claims.store_index<=?2
               AND {ACTUAL_STATE_CLAIM} AND {NATIVE} AND {UNREPAIRED_CLAIM}
             ORDER BY {CANONICAL_ORDER_DESC} LIMIT 1",
        ))?.query_row(
            /* sqlite-large-value: allow-for-now lv-hdee7cb0b4029 owner=fractal_store::store::roster_actual TODO=trace-statement-and-value-representation */
            params![subject,cut], |row| Ok((row.get(0)?,row.get(1)?,Vec::new())))
            .optional().map_err(Into::into);
    }
    let parsed = rows.into_iter().map(|(id,kind,origin,body)| {
        (id,kind,origin,serde_json::from_str::<Value>(&body).unwrap_or(Value::Null))
    }).collect::<Vec<_>>();
    let borrowed = parsed.iter().map(|(id,kind,origin,body)|
        (id.as_str(),kind.as_str(),origin.as_str(),body)).collect::<Vec<_>>();
    Ok(select_actual_source(&borrowed,host))
}
