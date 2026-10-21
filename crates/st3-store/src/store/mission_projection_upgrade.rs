//! Forward repair of persisted mission projections from their own canonical claims.
//! A corrected reducer alone does not replace rows retained by older store opens.
use super::*;

/// A carry references the source step at the carry's canonical cut. Its current
/// projection can already be cancelled by a later supersession, so never read it
/// to recover the carried reason. Existing wire claims need no new fields.
pub(super) fn carried_reason(
    transaction: &Transaction<'_>,
    carried: &str,
) -> Result<Option<String>, St3Error> {
    let mut id = carried.to_owned();
    loop {
        let claim = claim_by_id_tx(transaction, &id)
            .map_err(internal)?
            .context("carried reason source disappeared")
            .map_err(internal)?;
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        if claim.kind != "step-run.carried" {
            return Ok(fields
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned));
        }
        let Some(source) = fields.get("source_step_run").and_then(Value::as_str) else {
            return Ok(None);
        };
        // Every iteration follows a strictly earlier canonical claim, including
        // ties in accepted time. No arrival index or member-local clock is used.
        let query = format!(
            "SELECT claims.id FROM claims JOIN claims carried ON carried.id=?2
             WHERE claims.subject=?1 AND claims.kind IN
               ('step-run.state','step-run.carried','step-run.retried','work.claimed',
                'work.renewed','work.progress','work.submitted','work.failed','work.released','work.extended')
             AND (claims.kind='step-run.retried' OR json_type(claims.body,'$.fields.status')='text')
             AND (claims.kind NOT LIKE 'work.%' OR COALESCE(json_extract(claims.body,'$.fields.attempt'),1)=?3)
             AND {} AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims repaired WHERE repaired.id=claims.id)
             ORDER BY {} LIMIT 1",
            canonical::after_sql("carried", "claims"),
            canonical::order_sql("claims", true),
        );
        let previous = transaction
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h197d9d7f204f owner=st3_store::store::mission_projection_upgrade TODO=trace-statement-and-value-representation */ &query)
            .map_err(internal)?
            .query_row(
                /* sqlite-large-value: allow-for-now lv-hbf902e5959bb owner=st3_store::store::mission_projection_upgrade TODO=trace-statement-and-value-representation */ params![
                    source,
                    id,
                    fields.get("attempt").and_then(Value::as_u64).unwrap_or(1)
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(internal)?;
        let Some(previous) = previous else {
            return Ok(None);
        };
        id = previous;
    }
}

/// Restore terminal dates written by the old parent cascade and carried reasons
/// inferred from already-terminal predecessor projections. Source and frontier
/// stay unchanged; repeated opens do no repair work.
pub(super) fn migrate(transaction: &Transaction<'_>) -> Result<()> {
    if transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='mission_projection_source_rules' AND value='1')",
        [], |row| row.get::<_, bool>(0),
    )? { return Ok(()) }
    for (table, prefix, kind) in [
        ("mission_runs", "mission-run/", "mission-run.state"),
        ("run_generations", "run-generation/", "run-generation.state"),
    ] {
        let rows = transaction
            .prepare(/* sqlite-large-value: allow-for-now lv-hbd2c4bbffab2 owner=st3_store::store::mission_projection_upgrade TODO=trace-statement-and-value-representation */ &format!(
                "SELECT id,status,updated_at_unix_ms FROM {table}
             WHERE status IN ('completed','failed','cancelled','superseded')"
            ))?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let query = format!(
            "SELECT claims.accepted_at_unix_ms,json_extract(claims.body,'$.fields.status') FROM claims
             WHERE claims.subject=?1 AND claims.kind=?2
             AND json_extract(claims.body,'$.fields.status') IN ('completed','failed','cancelled','superseded')
             AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims repaired WHERE repaired.id=claims.id)
             ORDER BY {} LIMIT 1", canonical::order_sql("claims", true)
        );
        for (id, status, old) in rows {
            let source = transaction
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-h13fd16ed5349 owner=st3_store::store::mission_projection_upgrade TODO=trace-statement-and-value-representation */ &query)?
                .query_row(/* sqlite-large-value: allow-for-now lv-he4193c2dc947 owner=st3_store::store::mission_projection_upgrade TODO=trace-statement-and-value-representation */ params![format!("{prefix}{id}"), kind], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .optional()?;
            if let Some((at, source_status)) = source
                && source_status == status
                && at != old
            {
                transaction.execute(
                    &format!(/* sqlite-large-value: allow-for-now lv-h33428c0fa506 owner=st3_store::store::mission_projection_upgrade TODO=bound-or-reference-values-and-reclamation */ "UPDATE {table} SET updated_at_unix_ms=?2 WHERE id=?1"),
                    params![id, at],
                )?;
            }
        }
    }
    let carried = transaction
        .prepare("SELECT id FROM claims WHERE kind='step-run.carried'")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in carried {
        let claim = claim_by_id_tx(transaction, &id)?
            .context("carried claim disappeared during upgrade")?;
        reconcile_carried_step_tx(transaction, &claim).map_err(anyhow::Error::new)?;
    }
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h6e78404ecec8 owner=st3_store::store::mission_projection_upgrade TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('mission_projection_source_rules','1')",
        [],
    )?;
    Ok(())
}
