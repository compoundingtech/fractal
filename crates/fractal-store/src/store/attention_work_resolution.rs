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
    /* sqlite-large-value: allow-for-now lv-hbcfc69b2343b owner=fractal_store::store::attention_work_resolution TODO=trace-statement-and-value-representation */
    connection.query_row(&gate_sql(), params![source_id, person, episode], |row| proof(row, person))
        .optional().map(Option::flatten).map_err(internal)
}

fn person_sql() -> String {
    canonical_sql(
        "WITH original_request AS MATERIALIZED (
            SELECT id, json_extract(body,'$.fields.person') person,
                   CAST(json_extract(body,'$.fields.deadline_unix_ms') AS TEXT) deadline
            FROM claims WHERE id=?3 AND subject=?1 AND kind='work.person-asked'),
         responses AS MATERIALIZED (
            SELECT CASE WHEN octet_length(claims.id)<=4096 THEN claims.id END id,
                   CASE WHEN octet_length(claims.actor)<=4096 THEN claims.actor END actor,
                   CASE WHEN octet_length(claims.accepted_at_unix_ms)<=39
                        THEN claims.accepted_at_unix_ms END at,
                   coalesce(json_extract(claims.body,'$.fields.resolved_by')='deadline',0) by_deadline,
                   claims.kind='work.person-done'
                     AND coalesce(json_extract(claims.body,'$.fields.resolved_by')!='deadline',1) person_answer,
                   CASE WHEN claims.actor LIKE 'person/%' THEN claims.actor
                        WHEN claims.actor LIKE 'agent/%'
                          AND json_extract(claims.body,'$.fields.delegation.episode')=?3
                          AND json_extract(claims.body,'$.fields.acted_for')
                              =json_extract(claims.body,'$.fields.delegation.person')
                        THEN json_extract(claims.body,'$.fields.acted_for')
                        WHEN claims.actor='assistant/chatgpt'
                          AND json_extract(claims.body,'$.fields.sender_class')='external-assistant'
                          AND json_extract(claims.body,'$.fields.assistant_grant_id') IS NOT NULL
                        THEN json_extract(claims.body,'$.fields.assistant_owner') END answered_person,
                   row_number() OVER (ORDER BY CANONICAL_ASC(claims)) position
            FROM claims INDEXED BY claims_subject_kind_index
            WHERE claims.subject=?1 AND claims.kind IN ('work.person-done','work.person-cancelled')
              AND (json_extract(claims.body,'$.fields.episode')=?3
                OR (claims.kind='work.person-cancelled'
                  AND json_extract(claims.body,'$.fields.episode') IS NULL
                  AND json_extract(claims.body,'$.fields.key')='person-owner-ended:' || ?3
                  AND EXISTS(SELECT 1 FROM original_request)))),
         cutoff AS MATERIALIZED (
            SELECT coalesce((SELECT deadline FROM original_request),at) at
            FROM responses WHERE by_deadline ORDER BY position LIMIT 1),
         timely AS MATERIALIZED (
            SELECT 1 FROM responses,cutoff WHERE person_answer
              AND (length(responses.at)<length(cutoff.at)
                OR (length(responses.at)=length(cutoff.at) AND responses.at<cutoff.at)) LIMIT 1),
         winner AS MATERIALIZED (
            SELECT * FROM responses
            WHERE (NOT by_deadline OR NOT EXISTS(SELECT 1 FROM timely))
              AND (NOT person_answer OR NOT EXISTS(SELECT 1 FROM cutoff)
                OR EXISTS(SELECT 1 FROM cutoff WHERE length(responses.at)<length(cutoff.at)
                  OR (length(responses.at)=length(cutoff.at) AND responses.at<cutoff.at)))
            ORDER BY position LIMIT 1)
         SELECT id,actor,at FROM winner
         WHERE EXISTS(SELECT 1 FROM original_request WHERE person=?2)
            OR (NOT EXISTS(SELECT 1 FROM original_request) AND ?3 LIKE '%:%:%' AND answered_person=?2)"
    )
}

/// Ask recipients come from the immutable ask. Authored person steps have a fenced
/// generation:attempt:epoch episode; their accepted answer carries admission-validated
/// person/delegation/assistant authority. Neither path reads today's assignment.
/// Like standing_responses, only an on-time person answer can replace a deadline default.
pub fn person_step_resolution_at(
    connection: &Connection, source_id: &str, person: &str, episode: &str,
) -> Result<Option<SourceResolution>, St3Error> {
    /* sqlite-large-value: allow-for-now lv-he8a3ddb37275 owner=fractal_store::store::attention_work_resolution TODO=trace-statement-and-value-representation */
    connection.query_row(&person_sql(), params![source_id, person, episode], |row| proof(row, person))
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

#[cfg(test)]
mod person_tests {
    use super::*;
    use super::tests::{database, indexed, insert};

    fn ask(connection: &Connection, deadline: Option<u64>) {
        insert(connection, "request", "step-run/test", "work.person-asked", "agent/requester", 1,
            json!({"person":"person/alice","deadline_unix_ms":deadline}));
    }

    fn answer(connection: &Connection, id: &str, actor: &str, at: u64, extra: Value) {
        let mut fields = json!({"episode":"request","attempt":1,"summary":"Ship the reviewed option",
            "status":"completed","answer":{"id":"ship"}});
        fields.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        insert(connection, id, "step-run/test", "work.person-done", actor, at, fields);
    }

    fn resolved(connection: &Connection) -> SourceResolution {
        person_step_resolution_at(connection, "step-run/test", "person/alice", "request")
            .unwrap().unwrap()
    }

    #[test]
    fn ask_proof_binds_original_recipient_episode_and_delegated_actor() {
        let connection = database();
        ask(&connection, None);
        answer(&connection, "stale", "person/alice", 2, json!({"episode":"older-request"}));
        assert!(person_step_resolution_at(&connection, "step-run/test", "person/alice", "request").unwrap().is_none());
        answer(&connection, "delegated", "agent/reviewer", 3,
            json!({"acted_for":"person/alice","delegation":{"person":"person/alice","episode":"request"}}));
        assert_eq!(resolved(&connection).resolved_by, "agent/reviewer");
        assert!(person_step_resolution_at(&connection, "step-run/test", "person/bob", "request").unwrap().is_none());
        assert!(person_step_resolution_at(&connection, "step-run/other", "person/alice", "request").unwrap().is_none());
        indexed(&connection, &person_sql(), "claims_subject_kind_index");
    }

    #[test]
    fn deadline_default_rejects_late_answer_but_yields_to_accepted_timely_answer() {
        let connection = database();
        ask(&connection, Some(100));
        answer(&connection, "default", "daemon/runtime", 110, json!({"resolved_by":"deadline"}));
        answer(&connection, "late", "person/alice", 100, json!({}));
        assert_eq!(resolved(&connection).claim_id, "default");
        answer(&connection, "timely", "agent/reviewer", 99,
            json!({"acted_for":"person/alice","delegation":{"person":"person/alice","episode":"request"}}));
        assert_eq!(resolved(&connection).claim_id, "timely");
        assert_eq!(resolved(&connection).resolved_by, "agent/reviewer");
    }

    #[test]
    fn legacy_deadline_uses_default_acceptance_as_cutoff() {
        let connection = database();
        ask(&connection, None);
        answer(&connection, "default", "daemon/runtime", 10, json!({"resolved_by":"deadline"}));
        answer(&connection, "late", "person/alice", 11, json!({}));
        assert_eq!(resolved(&connection).claim_id, "default");
        answer(&connection, "timely", "person/alice", 9, json!({}));
        assert_eq!(resolved(&connection).claim_id, "timely");
    }

    #[test]
    fn authored_episode_uses_admitted_person_metadata_not_current_owner() {
        for (actor, extra) in [
            ("person/alice", json!({})),
            ("agent/reviewer", json!({"acted_for":"person/alice",
                "delegation":{"person":"person/alice","episode":"generation:1:2"}})),
            ("assistant/chatgpt", json!({"sender_class":"external-assistant",
                "assistant_owner":"person/alice","assistant_grant_id":"grant-accepted"})),
        ] {
            let connection = database();
            let mut fields = json!({"episode":"generation:1:2","attempt":1,"summary":"Finished"});
            fields.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            insert(&connection, "accepted", "step-run/test", "work.person-done", actor, 3, fields);
            let winner = person_step_resolution_at(&connection, "step-run/test", "person/alice", "generation:1:2")
                .unwrap().unwrap();
            assert_eq!(winner.resolved_by, actor);
            assert!(person_step_resolution_at(&connection, "step-run/test", "person/bob", "generation:1:2").unwrap().is_none());
            assert!(person_step_resolution_at(&connection, "step-run/test", "person/alice", "generation:1:3").unwrap().is_none());
        }
    }
}

#[cfg(test)]
mod ended_ask_tests {
    use super::*;
    use super::tests::{database, insert};

    #[test]
    fn owner_ended_cancellation_binds_ask_through_immutable_key() {
        let connection = database();
        insert(&connection, "request", "step-run/test", "work.person-asked", "agent/requester", 1,
            json!({"person":"person/alice"}));
        insert(&connection, "cancelled", "step-run/test", "work.person-cancelled", "daemon/runtime", 2,
            json!({"attempt":1,"status":"cancelled","summary":"owning run ended","key":"person-owner-ended:request"}));
        let result = person_step_resolution_at(&connection, "step-run/test", "person/alice", "request")
            .unwrap().unwrap();
        assert_eq!(result.resolved_by, "daemon/runtime");
        assert!(person_step_resolution_at(&connection, "step-run/test", "person/alice", "older").unwrap().is_none());
    }
}
