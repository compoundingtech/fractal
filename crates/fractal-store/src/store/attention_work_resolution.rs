//! Point proofs for accepted work resolutions. No current-owner or open-projection lookup.
use super::*;
use super::attention_history::SourceResolution;

fn gate_sql() -> String {
    canonical_sql(&format!(
        "WITH winner AS MATERIALIZED (
            SELECT claims.id, claims.actor, claims.accepted_at_unix_ms,
                   json_extract(request.body,'$.fields.reviewer') original_person,
                   {} answered_person
            FROM claims request
            JOIN claims INDEXED BY claims_human_gate_answer_request_index ON {}
            WHERE request.id=?3 AND request.kind='gate.requested'
              AND json_extract(request.body,'$.fields.owner')=?1
            ORDER BY CANONICAL_ASC(claims) LIMIT 1)
         SELECT CASE WHEN octet_length(id)<=4096 THEN id END,
                CASE WHEN octet_length(actor)<=4096 THEN actor END,
                CASE WHEN octet_length(accepted_at_unix_ms)<=39 THEN accepted_at_unix_ms END
         FROM winner WHERE original_person=?2 OR answered_person=?2",
        human_review_reassignment::reviewer_sql("request", Some("claims")),
        reviewer_answer_sql("claims"),
    ))
}

fn proof(row: &rusqlite::Row<'_>, person: &str) -> rusqlite::Result<Option<SourceResolution>> {
    let (Some(claim_id), Some(resolved_by), Some(at)) = (
        row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?,
        row.get::<_, Option<String>>(2)?,
    ) else { return Ok(None); };
    Ok(at.parse().ok().map(|resolved_at_unix_ms| SourceResolution {
        claim_id, resolved_by, person: person.to_owned(), resolved_at_unix_ms,
    }))
}

/// The canonical accepted answer and the immutable request authorize the original card
/// recipient, or its admitted reassignee at answer time. Later routing changes are irrelevant.
pub fn human_gate_resolution_at(
    connection: &Connection, source_id: &str, person: &str, episode: &str,
) -> Result<Option<SourceResolution>, St3Error> {
    connection.query_row(/* sqlite-large-value: allow-for-now lv-hbcfc69b2343b owner=fractal_store::store::attention_work_resolution TODO=trace-statement-and-value-representation */ &gate_sql(), params![source_id, person, episode], |row| proof(row, person))
        .optional().map(Option::flatten).map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE claims(id TEXT PRIMARY KEY, store_index INTEGER,
            batch_id TEXT, subject TEXT, kind TEXT, actor TEXT, body TEXT, accepted_at_unix_ms TEXT);
            CREATE INDEX claims_subject_kind_index ON claims(subject,kind,store_index);
            CREATE INDEX claims_batch_index ON claims(batch_id,store_index);
            CREATE INDEX claims_human_gate_answer_request_index ON claims(json_extract(body,'$.fields.request')) WHERE kind='gate.result';
            CREATE INDEX claims_human_gate_reassignment_request_index ON claims(json_extract(body,'$.fields.request')) WHERE kind='gate.reviewer-reassigned';
            CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
            CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY, position INTEGER);
            CREATE TABLE replica_records(claim_id TEXT, position INTEGER);
            CREATE INDEX replica_claim_position ON replica_records(claim_id,position);").unwrap();
        connection
    }

    pub(super) fn insert(connection: &Connection, id: &str, subject: &str, kind: &str,
        actor: &str, at: u64, fields: Value) {
        connection.execute("INSERT INTO claims VALUES(?1,?2,?1,?3,?4,?5,?6,?7)",
            params![id, at, subject, kind, actor, json!({"fields":fields}).to_string(), at.to_string()]).unwrap();
    }

    pub(super) fn indexed(connection: &Connection, sql: &str, index: &str) {
        let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let plan = statement.query_map(params!["step-run/test", "person/alice", "request"],
            |row| row.get::<_, String>(3)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert!(plan.iter().any(|line| line.contains(index)), "{plan:?}");
        for line in &plan {
            assert!(!["claims", "request", "legacy_position", "rename_receipt", "reassignment"]
                .iter().any(|alias| line.starts_with(&format!("SCAN {alias}"))), "{plan:?}");
        }
    }

    #[test]
    fn gate_exact_recipient_episode_and_canonical_actor() {
        let connection = database();
        insert(&connection, "request", "review/test", "gate.requested", "agent/requester", 1,
            json!({"owner":"step-run/test","reviewer":"person/alice"}));
        insert(&connection, "wrong", "review/test", "gate.result", "person/bob", 2,
            json!({"request":"request","verdict":"pass"}));
        insert(&connection, "winner", "review/test", "gate.result", "agent/delegate", 3,
            json!({"request":"request","verdict":"pass","acted_for":"person/alice",
                "delegation":{"episode":"request"}}));
        insert(&connection, "later", "review/test", "gate.result", "person/alice", 4,
            json!({"request":"request","verdict":"fail"}));
        let result = human_gate_resolution_at(&connection, "step-run/test", "person/alice", "request").unwrap().unwrap();
        assert_eq!((result.claim_id.as_str(), result.resolved_by.as_str()), ("winner", "agent/delegate"));
        assert!(human_gate_resolution_at(&connection, "step-run/test", "person/bob", "request").unwrap().is_none());
        assert!(human_gate_resolution_at(&connection, "step-run/test", "person/alice", "stale").unwrap().is_none());
        assert!(human_gate_resolution_at(&connection, "step-run/other", "person/alice", "request").unwrap().is_none());
        indexed(&connection, &gate_sql(), "claims_human_gate_answer_request_index");
    }
}
