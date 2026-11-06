//! Exact accepted native-answer receipts; absence is never evidence of resolution.
//! Native diagnostics expire after seven days. Compact durable harness-question receipts
//! contain only actor/key digests, not an attributable person, so cannot supply this proof.
use super::*;
use super::attention_history::SourceResolution;

const ANSWER: &str = "WITH winner AS MATERIALIZED (
    SELECT CASE WHEN octet_length(id)<=4096 THEN id END AS winning_id
    FROM claims INDEXED BY claims_operation_index
    WHERE json_extract(body,'$._operation.id') IS NOT NULL
      AND json_extract(body,'$._operation.id')=?1
    ORDER BY CANONICAL_DESC(claims) LIMIT 1)
  SELECT
    CASE WHEN octet_length(id)<=4096 THEN id END,
    CASE WHEN octet_length(actor)<=4096 THEN actor END,
    CASE WHEN octet_length(accepted_at_unix_ms)<=40 THEN accepted_at_unix_ms END
  FROM winner JOIN claims ON claims.id=winner.winning_id
  WHERE subject=?2 AND kind='harness.diagnostic'
    AND CASE WHEN octet_length(body)<=1048576 THEN CASE WHEN json_valid(body) THEN
      json_extract(body,'$.fields.code')='native-prompt-answered'
      AND EXISTS (SELECT 1 FROM json_each(body,'$.evidence') WHERE value=?4)
      AND (actor=?3 AND json_extract(body,'$.fields.acted_for') IS NULL
        OR json_extract(body,'$.fields.status')='dismiss'
          AND json_extract(body,'$.fields.acted_for')=?3
          AND json_extract(body,'$.fields.delegation.person')=?3
          AND json_extract(body,'$.fields.delegation.episode')=?4)
    ELSE 0 END ELSE 0 END";

/// The actor authorized at accepted answer admission (or admitted delegated person).
/// No current ownership, terminal state, expiry or disappearance can fabricate a receipt.
pub fn prompt_resolution_at(
    conn: &Connection,
    source_id: &str,
    person: &str,
    episode: &str,
) -> std::result::Result<Option<SourceResolution>, St3Error> {
    if !source_id.starts_with("agent/") || [source_id, person, episode].iter().any(|v| v.is_empty() || v.len()>4096) {
        return Ok(None);
    }
    let operation = smallclaims::store::operation_id_for_key(&format!("native-prompt-answer:{episode}"));
    let row = conn.query_row(/* sqlite-large-value: allow-for-now lv-hf6eaa22251c7 owner=fractal_store::store::attention_prompt_resolution TODO=trace-statement-and-value-representation */ &canonical_sql(ANSWER), params![operation, source_id, person, episode], |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?))
    }).optional().map_err(internal)?;
    let Some((Some(claim_id), Some(resolved_by), Some(at))) = row else { return Ok(None) };
    Ok(Some(SourceResolution { claim_id, resolved_by, person: person.into(),
        resolved_at_unix_ms: at.parse::<u128>().map_err(internal)? }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE claims(id TEXT PRIMARY KEY,store_index INTEGER,subject TEXT,kind TEXT,actor TEXT,body TEXT,accepted_at_unix_ms TEXT,batch_id TEXT DEFAULT 'batch');
            CREATE INDEX claims_batch_index ON claims(batch_id,store_index);
            CREATE TABLE batches(id TEXT PRIMARY KEY,origin TEXT,replica_sequence INTEGER);
            INSERT INTO batches VALUES('batch','origin',1);
            CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY,position INTEGER);
            CREATE TABLE replica_records(claim_id TEXT,position INTEGER);
            CREATE INDEX replica_records_claim ON replica_records(claim_id);
            CREATE INDEX claims_operation_index ON claims(json_extract(body,'$._operation.id')) WHERE json_extract(body,'$._operation.id') IS NOT NULL;").unwrap();
        conn
    }

    fn answer(conn: &Connection, actor: &str, episode: &str, fields: Value) {
        let operation = smallclaims::store::operation_id_for_key(&format!("native-prompt-answer:{episode}"));
        conn.execute("INSERT INTO claims(id,store_index,subject,kind,actor,body,accepted_at_unix_ms) VALUES('answer',1,'agent/seat','harness.diagnostic',?1,?2,'456')",
            params![actor, json!({"fields":fields,"evidence":[episode],"_operation":{"id":operation}}).to_string()]).unwrap();
    }

    #[test]
    fn native_permission_answer_proves_exact_person_source_and_episode() {
        let conn = connection();
        answer(&conn, "person/ada", "observed-1", json!({"code":"native-prompt-answered","status":"allow","driver":"claude"}));
        // Original observations may already be reclaimed; the accepted receipt binds them.
        let proof = prompt_resolution_at(&conn, "agent/seat", "person/ada", "observed-1").unwrap().unwrap();
        assert_eq!((proof.claim_id.as_str(),proof.resolved_by.as_str(),proof.person.as_str(),proof.resolved_at_unix_ms), ("answer","person/ada","person/ada",456));
        for (source,person,episode) in [("agent/foreign","person/ada","observed-1"),("agent/seat","person/foreign","observed-1"),("agent/seat","person/ada","observed-stale")] {
            assert!(prompt_resolution_at(&conn, source, person, episode).unwrap().is_none());
        }
    }

    #[test]
    fn delegated_denial_keeps_actual_actor_and_original_addressed_person() {
        let conn = connection();
        answer(&conn, "agent/helper", "denial-1", json!({"code":"native-prompt-answered","status":"dismiss","acted_for":"person/ada","delegation":{"person":"person/ada","episode":"denial-1"}}));
        let proof = prompt_resolution_at(&conn, "agent/seat", "person/ada", "denial-1").unwrap().unwrap();
        assert_eq!((proof.resolved_by.as_str(),proof.person.as_str()), ("agent/helper","person/ada"));
        assert!(prompt_resolution_at(&conn, "agent/seat", "person/other", "denial-1").unwrap().is_none());
        conn.execute("UPDATE claims SET body=json_set(body,'$.fields.delegation.episode','wrong')", []).unwrap();
        assert!(prompt_resolution_at(&conn, "agent/seat", "person/ada", "denial-1").unwrap().is_none());
    }

    #[test]
    fn stale_or_unrelated_receipts_and_retention_absence_are_not_resolution() {
        let conn = connection();
        assert!(prompt_resolution_at(&conn, "agent/seat", "person/ada", "gone").unwrap().is_none());
        answer(&conn, "person/ada", "observed-1", json!({"code":"native-prompt-answered","status":"deny"}));
        conn.execute("UPDATE claims SET body=json_set(body,'$.evidence',json('[\"other\"]'))", []).unwrap();
        assert!(prompt_resolution_at(&conn, "agent/seat", "person/ada", "observed-1").unwrap().is_none());
        conn.execute("UPDATE claims SET body=json_set(body,'$.evidence',json('[\"observed-1\"]'),'$.fields.code','unrelated')", []).unwrap();
        assert!(prompt_resolution_at(&conn, "agent/seat", "person/ada", "observed-1").unwrap().is_none());
        conn.execute("UPDATE claims SET body=json_set(body,'$.fields.code','native-prompt-answered','$.fields.reason',?1)", ["x".repeat(1048576)]).unwrap();
        assert!(prompt_resolution_at(&conn, "agent/seat", "person/ada", "observed-1").unwrap().is_none());
    }

    #[test]
    fn canonical_winner_precedes_person_filter_even_when_arrival_order_reverses() {
        let conn = connection();
        answer(&conn, "person/ada", "observed-1", json!({"code":"native-prompt-answered","status":"deny"}));
        // Bob's newer accepted answer arrives first; Ada's older answer arrives later.
        conn.execute("INSERT INTO claims SELECT 'bob',0,subject,kind,'person/bob',body,'457',batch_id FROM claims WHERE id='answer'", []).unwrap();
        assert!(prompt_resolution_at(&conn, "agent/seat", "person/ada", "observed-1").unwrap().is_none());
        let proof = prompt_resolution_at(&conn, "agent/seat", "person/bob", "observed-1").unwrap().unwrap();
        assert_eq!((proof.claim_id.as_str(),proof.resolved_by.as_str()), ("bob","person/bob"));
        conn.execute("UPDATE claims SET store_index=10-store_index", []).unwrap();
        assert!(prompt_resolution_at(&conn, "agent/seat", "person/ada", "observed-1").unwrap().is_none());
        assert_eq!(prompt_resolution_at(&conn, "agent/seat", "person/bob", "observed-1").unwrap().unwrap().claim_id, "bob");
    }

    #[test]
    fn prompt_lookup_uses_exact_operation_index() {
        let conn = connection();
        let plan = conn.prepare(&format!("EXPLAIN QUERY PLAN {}", canonical_sql(ANSWER))).unwrap()
            .query_map(["operation", "agent/seat", "person/ada", "episode"], |row| row.get::<_, String>(3)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
        assert!(plan.contains("SEARCH claims USING INDEX claims_operation_index"), "{plan}");
        assert!(!plan.contains("SCAN claims"), "{plan}");
    }
}
