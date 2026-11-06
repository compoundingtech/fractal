//! Accepted approvals for an original attention locator, not current projection state.
use super::*;
use super::attention_history::SourceResolution;

// Both families are durable. Subject/kind indexes restrict reads to one launch/proposal;
// CASE bounds JSON parsing and every value transferred to Rust. No document is returned.
const LAUNCH: &str = "SELECT
    CASE WHEN octet_length(id)<=4096 THEN id END,
    CASE WHEN octet_length(actor)<=4096 THEN actor END,
    CASE WHEN octet_length(accepted_at_unix_ms)<=40 THEN accepted_at_unix_ms END
  FROM claims INDEXED BY claims_subject_kind_index
  WHERE subject=?1 AND kind='planning-session.approved'
    AND CASE WHEN octet_length(body)<=1048576 THEN CASE WHEN json_valid(body) THEN
      json_extract(body,'$.fields.requester')=?2
      AND json_extract(body,'$.fields.preview_hash')=?3 ELSE 0 END ELSE 0 END
  ORDER BY store_index LIMIT 1";

const REVISION: &str = "SELECT
    CASE WHEN octet_length(a.id)<=4096 THEN a.id END,
    CASE WHEN octet_length(a.actor)<=4096 THEN a.actor END,
    CASE WHEN octet_length(a.accepted_at_unix_ms)<=40 THEN a.accepted_at_unix_ms END
  FROM claims a INDEXED BY claims_subject_kind_index
  WHERE a.subject=?1 AND a.kind='revision-proposal.approved' AND a.actor=?2
    AND CASE WHEN octet_length(a.body)<=1048576 THEN CASE WHEN json_valid(a.body) THEN
      json_extract(a.body,'$.fields.reviewer')=?2
      AND json_extract(a.body,'$.fields.preview_hash')=?4 ELSE 0 END ELSE 0 END
    AND EXISTS (SELECT 1 FROM claims c INDEXED BY claims_subject_kind_index
      WHERE c.subject=?1 AND c.kind='revision-proposal.created'
        AND c.store_index<a.store_index
        AND CASE WHEN octet_length(c.body)<=1048576 THEN CASE WHEN json_valid(c.body) THEN
          json_extract(c.body,'$.fields.source_generation')=?3
          AND json_extract(c.body,'$.fields.preview_hash')=?4
          AND EXISTS (SELECT 1 FROM json_each(c.body,'$.fields.reviewers') WHERE value=?2)
        ELSE 0 END ELSE 0 END)
  ORDER BY a.store_index LIMIT 1";

fn proof(
    conn: &Connection,
    sql: &str,
    args: &[&str],
    person: &str,
) -> std::result::Result<Option<SourceResolution>, St3Error> {
    let row = conn.query_row(/* sqlite-large-value: allow-for-now lv-h86dfd7785502 owner=fractal_store::store::attention_launch_resolution TODO=trace-statement-and-value-representation */ sql, rusqlite::params_from_iter(args), |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?))
    }).optional().map_err(internal)?;
    let Some((Some(claim_id), Some(resolved_by), Some(at))) = row else { return Ok(None) };
    let resolved_at_unix_ms = at.parse::<u128>().map_err(internal)?;
    Ok(Some(SourceResolution { claim_id, resolved_by, person: person.into(), resolved_at_unix_ms }))
}

/// Approval's accepted requester authorizes the original launch preview, even after cutover.
pub fn launch_resolution_at(
    conn: &Connection,
    source_id: &str,
    person: &str,
    episode: &str,
) -> std::result::Result<Option<SourceResolution>, St3Error> {
    if !source_id.starts_with("planning-session/") || [source_id, person, episode].iter().any(|v| v.is_empty() || v.len()>4096) {
        return Ok(None);
    }
    proof(conn, LAUNCH, &[source_id, person, episode], person)
}

/// One reviewer's accepted approval, not all-approved or a later proposal's current owner.
pub fn revision_resolution_at(
    conn: &Connection,
    source_id: &str,
    person: &str,
    episode: &str,
) -> std::result::Result<Option<SourceResolution>, St3Error> {
    if !source_id.starts_with("revision-proposal/") || [source_id, person, episode].iter().any(|v| v.is_empty() || v.len()>4096) {
        return Ok(None);
    }
    let Some((generation, preview)) = episode.rsplit_once(':') else { return Ok(None) };
    proof(conn, REVISION, &[source_id, person, generation, preview], person)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE claims(id TEXT PRIMARY KEY,store_index INTEGER,subject TEXT,kind TEXT,actor TEXT,body TEXT,accepted_at_unix_ms TEXT);
            CREATE INDEX claims_subject_kind_index ON claims(subject,kind);").unwrap();
        conn
    }

    fn claim(conn: &Connection, id: &str, subject: &str, kind: &str, actor: &str, fields: Value) {
        conn.execute("INSERT INTO claims VALUES(?1,(SELECT count(*)+1 FROM claims),?2,?3,?4,?5,'123')",
            params![id, subject, kind, actor, json!({"fields":fields}).to_string()]).unwrap();
    }

    #[test]
    fn launch_proves_original_requester_preview_and_actual_actor() {
        let conn = connection();
        let source = "planning-session/launch-1";
        assert!(launch_resolution_at(&conn, source, "person/ada", "preview-1").unwrap().is_none());
        claim(&conn, "approved", source, "planning-session.approved", "person/ada", json!({"requester":"person/ada","preview_hash":"preview-1"}));
        // A different current projection cannot affect this accepted receipt.
        conn.execute_batch("CREATE TABLE planning_sessions(requester TEXT,status TEXT); INSERT INTO planning_sessions VALUES('person/other','cancelled');").unwrap();
        let proof = launch_resolution_at(&conn, source, "person/ada", "preview-1").unwrap().unwrap();
        assert_eq!((proof.claim_id.as_str(), proof.resolved_by.as_str(), proof.person.as_str(), proof.resolved_at_unix_ms), ("approved", "person/ada", "person/ada", 123));
        for (source, person, episode) in [(source,"person/other","preview-1"),(source,"person/ada","stale"),("planning-session/other","person/ada","preview-1")] {
            assert!(launch_resolution_at(&conn, source, person, episode).unwrap().is_none());
        }
    }

    #[test]
    fn revision_requires_original_reviewer_generation_and_accepted_preview() {
        let conn = connection();
        let source = "revision-proposal/p1";
        claim(&conn, "created", source, "revision-proposal.created", "person/requester", json!({"source_generation":"run-generation/g1","preview_hash":"p1","reviewers":["person/ada","person/bob"]}));
        claim(&conn, "approved", source, "revision-proposal.approved", "person/ada", json!({"reviewer":"person/ada","preview_hash":"p1","all_approved":false}));
        let proof = revision_resolution_at(&conn, source, "person/ada", "run-generation/g1:p1").unwrap().unwrap();
        assert_eq!((proof.claim_id.as_str(), proof.resolved_by.as_str()), ("approved","person/ada"));
        for (person, episode) in [("person/bob","run-generation/g1:p1"),("person/foreign","run-generation/g1:p1"),("person/ada","run-generation/g2:p1"),("person/ada","run-generation/g1:stale")] {
            assert!(revision_resolution_at(&conn, source, person, episode).unwrap().is_none());
        }
        assert!(revision_resolution_at(&conn, "revision-proposal/foreign", "person/ada", "run-generation/g1:p1").unwrap().is_none());
    }


    #[test]
    fn launch_lookup_uses_subject_kind_index() {
        let conn = connection();
        let plan = conn.prepare(&format!("EXPLAIN QUERY PLAN {LAUNCH}")).unwrap()
            .query_map(["planning-session/l1", "person/ada", "p1"], |row| row.get::<_, String>(3)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
        assert!(plan.contains("SEARCH claims USING INDEX claims_subject_kind_index"), "{plan}");
        assert!(!plan.contains("SCAN claims"), "{plan}");
    }

    #[test]
    fn revision_lookup_uses_subject_kind_indexes() {
        let conn = connection();
        let plan = conn.prepare(&format!("EXPLAIN QUERY PLAN {REVISION}")).unwrap()
            .query_map(["revision-proposal/p1", "person/ada", "run-generation/g1", "p1"], |row| row.get::<_, String>(3)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
        for alias in ["a", "c"] {
            assert!(plan.contains(&format!("SEARCH {alias} USING INDEX claims_subject_kind_index")), "{plan}");
            assert!(!plan.contains(&format!("SCAN {alias} ")), "{plan}");
        }
    }
    #[test]
    fn bounded_documents_and_missing_actor_do_not_become_proofs() {
        let conn = connection();
        let source = "planning-session/wide";
        claim(&conn, "wide", source, "planning-session.approved", "person/ada", json!({"requester":"person/ada","preview_hash":"p","markdown":"x".repeat(1048576)}));
        assert!(launch_resolution_at(&conn, source, "person/ada", "p").unwrap().is_none());
        conn.execute("UPDATE claims SET body=?1,actor=NULL", [json!({"fields":{"requester":"person/ada","preview_hash":"p"}}).to_string()]).unwrap();
        assert!(launch_resolution_at(&conn, source, "person/ada", "p").unwrap().is_none());
    }
}
