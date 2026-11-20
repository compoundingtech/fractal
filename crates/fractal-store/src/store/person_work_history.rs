//! Read-only retained asks. Current assignment authorizes; immutable claims supply content.
use super::*;
use crate::store::thread_metadata::request_history::{
    EpisodeState, HistoricalPayload, PayloadUnavailable, RequestHistoryItem, RequestProvenance,
    SourceEpisode,
};

const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_RESPONSES: usize = 256;

fn internal(error: impl std::fmt::Display) -> FractalError {
    FractalError::new("internal", error.to_string())
}

fn canonical_request_id(conn: &Connection, source: &str) -> Result<Option<String>, FractalError> {
    conn.query_row(/* sqlite-large-value: allow-for-now lv-hf08b64fbbc13 owner=fractal_store::store::person_work_history TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT id FROM claims WHERE subject=?1 AND kind='work.person-asked'
         ORDER BY CANONICAL_ASC(claims) LIMIT 1"), [source], |row| row.get(0))
        .optional().map_err(internal)
}


fn unavailable(key: &SourceEpisode, reason: PayloadUnavailable) -> RequestHistoryItem {
    RequestHistoryItem { key: key.clone(), payload: HistoricalPayload::Unavailable { reason },
        state: EpisodeState::Unavailable, complete: false }
}

/// Read an exact canonical runtime ask, including resolved/cancelled asks. This deliberately
/// does not call `current`, `run_live`, or join responses through today's originating attempt.
/// Authorized openings beyond the payload budget remain explicit unavailable history;
/// routing proof alone never supplies payload or authorizes Start.
pub fn retained_request_history(
    conn: &Connection, person: &str, key: &SourceEpisode, cut: u64,
) -> Result<Option<RequestHistoryItem>, FractalError> {
    let qualification = discussion_source::qualify_history(conn, person, key, now_ms())?;
    if !qualification.is_authorized() { return Ok(None); }
    if canonical_request_id(conn, &key.source_id)?.as_deref() != Some(key.episode.as_str()) {
        return Ok(None);
    }
    let cut = i64::try_from(cut).unwrap_or(i64::MAX);
    let length: Option<i64> = conn.query_row(
        "SELECT length(CAST(body AS BLOB)) FROM claims WHERE id=?1 AND store_index<=?2",
        params![key.episode, cut], |row| row.get(0),
    ).optional().map_err(internal)?;
    let Some(length) = length else {
        return Ok(Some(unavailable(key, PayloadUnavailable::PayloadUnavailable)));
    };
    if length > MAX_BODY_BYTES as i64
        || matches!(qualification, discussion_source::HistoryQualification::PayloadOverBudget)
    {
        return Ok(Some(unavailable(key, PayloadUnavailable::ReadBoundExceeded)));
    }
    let ask = conn.query_row(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
         FROM claims WHERE id=?1 AND store_index<=?2", params![key.episode, cut], claim_from_row,
    ).map_err(internal)?;
    let fields = &ask.body["fields"];
    let Some(text) = fields["reason"].as_str() else {
        return Ok(Some(unavailable(key, PayloadUnavailable::PayloadUnavailable)));
    };
    let Some(title) = fields["title"].as_str() else {
        return Ok(Some(unavailable(key, PayloadUnavailable::PayloadUnavailable)));
    };
    let payload = HistoricalPayload::Available {
        title: Some(title.into()), text: text.into(),
        request: fields.get("request").filter(|v| !v.is_null()).cloned(),
        references: Vec::new(),
        provenance: RequestProvenance::RuntimeAsk { asking_claim_id: ask.id.clone() },
    };
    let state = retained_state(conn, key, &ask, cut)?;
    let complete = matches!(state, EpisodeState::Available { .. });
    Ok(Some(RequestHistoryItem { key: key.clone(), payload, state, complete }))
}

fn retained_state(
    conn: &Connection, key: &SourceEpisode, ask: &ClaimRecord, cut: i64,
) -> Result<EpisodeState, FractalError> {
    // CASE bounds values before crossing SQLite's boundary; one extra row proves truncation.
    let mut query = conn.prepare(/* sqlite-large-value: allow-for-now lv-h86326f6e2d00 owner=fractal_store::store::person_work_history TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,
          CASE WHEN length(CAST(body AS BLOB))<=262144 THEN body ELSE NULL END,
          predecessors,accepted_at_unix_ms
         FROM claims WHERE subject=?1 AND store_index<=?2
           AND kind IN ('work.person-done','work.person-cancelled','work.person-override')
           AND (json_extract(body,'$.fields.episode')=?3
             OR json_extract(body,'$.fields.episode') IS NULL)
         ORDER BY CANONICAL_ASC(claims) LIMIT 257",
    )).map_err(internal)?;
    let mut rows = query.query(params![key.source_id, cut, key.episode]).map_err(internal)?;
    let mut responses: Vec<ClaimRecord> = Vec::new();
    let mut count = 0;
    let mut unproven_resolution = false;
    while let Some(row) = rows.next().map_err(internal)? {
        count += 1;
        if count > MAX_RESPONSES || matches!(row.get_ref(7).map_err(internal)?, rusqlite::types::ValueRef::Null) {
            return Ok(EpisodeState::Unavailable);
        }
        let response = claim_from_row(row).map_err(internal)?;
        if response.body["fields"]["attempt"].as_u64().is_some_and(|attempt|
            Some(attempt) != ask.body["fields"]["attempt"].as_u64()) {
            continue;
        }
        if response.body["fields"]["episode"].is_null()
            && (response.kind == "work.person-override" || !response.predecessors.contains(&ask.id))
        {
            unproven_resolution = true;
            continue;
        }
        if response.kind == "work.person-override" {
            let fields = &response.body["fields"];
            if response.actor.as_deref() != ask.body["fields"]["person"].as_str()
                || !fields["answer"].is_object() { continue; }
            let valid: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1 AND subject=?2
                 AND store_index<=?3 AND kind='work.person-done'
                 AND json_extract(body,'$.fields.resolved_by')='deadline'
                 AND json_extract(body,'$.fields.episode')=?4)",
                params![fields["overrides"].as_str(), key.source_id, cut, key.episode],
                |row| row.get(0),
            ).map_err(internal)?;
            if !valid || fields["key"].as_str().is_some_and(|key| responses.iter().any(|old|
                old.kind == "work.person-override" && old.body["fields"]["key"] == key)) { continue; }
        }
        responses.push(response);
    }
    let by_deadline = |claim: &ClaimRecord| claim.body["fields"]["resolved_by"] == "deadline";
    let mut deadline = ask.body["fields"]["deadline_unix_ms"].as_u64().map(u128::from);
    if deadline.is_some() && responses.iter().any(by_deadline)
        && ask.actor.as_deref() != Some("daemon/browser-pairing")
        && let Some(person) = ask.body["fields"]["person"].as_str()
    {
        // Reuse the existing clock fold, but exclude later claims from this content cut.
        let mut clock_query = conn.prepare(/* sqlite-large-value: allow-for-now lv-hca314f305a19 owner=fractal_store::store::person_work_history TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT id,store_index,batch_id,subject,kind,origin,actor,
              CASE WHEN length(CAST(body AS BLOB))<=262144 THEN body ELSE NULL END,
              predecessors,accepted_at_unix_ms FROM claims
             WHERE subject=?1 AND kind=?2 AND store_index<=?3
             ORDER BY CANONICAL_DESC(claims) LIMIT 257",
        )).map_err(internal)?;
        let mut clock_rows = clock_query.query(params![person, fractal_schema::availability::DEADLINE_CLOCK_KIND, cut]).map_err(internal)?;
        let mut clocks = Vec::new();
        while let Some(row) = clock_rows.next().map_err(internal)? {
            if clocks.len() == MAX_RESPONSES || matches!(row.get_ref(7).map_err(internal)?, rusqlite::types::ValueRef::Null) {
                return Ok(EpisodeState::Unavailable);
            }
            let clock = claim_from_row(row).map_err(internal)?;
            let stop = clock.accepted_at_unix_ms <= ask.accepted_at_unix_ms;
            clocks.push(clock);
            if stop { break; }
        }
        deadline = super::super::availability::effective_deadline(
            ask.accepted_at_unix_ms, deadline.unwrap(),
            super::super::availability::urgent_ask(&ask.body["fields"]),
            &super::super::availability::clock_holds(&clocks, ask.accepted_at_unix_ms),
        );
    }
    let (standing, _) = standing_responses(responses, by_deadline,
        |c| c.kind == "work.person-done" && !by_deadline(c),
        |c| c.kind == "work.person-override", |c| c.accepted_at_unix_ms, deadline);
    let status = standing.last().map(|claim| {
        if claim.kind == "work.person-cancelled" { "cancelled" }
        else { claim.body["fields"]["status"].as_str().unwrap_or("completed") }
    });
    let Some(status) = status else {
        if unproven_resolution { return Ok(EpisodeState::Unavailable); }
        // No resolution evidence cannot prove why an ended source ended.
        let current: String = conn.query_row("SELECT status FROM step_runs WHERE subject=?1",
            [&key.source_id], |row| row.get(0)).map_err(internal)?;
        return Ok(if matches!(current.as_str(), "pending" | "ready") {
            EpisodeState::Available { status: "ready".into(), answers: Vec::new() }
        } else { EpisodeState::Unavailable });
    };
    let status = status.to_owned();
    let mut answers = Vec::new();
    for claim in standing {
        let fields = &claim.body["fields"];
        let Some(summary) = fields["summary"].as_str() else { continue };
        let string = |name: &str| fields[name].as_str().map(str::to_owned);
        let answer = crate::model::PersonAnswerView {
            ask: key.source_id.clone(), status: string("status").unwrap_or_else(|| "completed".into()),
            summary: summary.into(), respondent: claim.actor.unwrap_or_default(),
            answered_at_unix_ms: claim.accepted_at_unix_ms,
            acted_for: string("acted_for"), sender_class: string("sender_class"),
            assistant_grant_id: string("assistant_grant_id"), assistant_owner: string("assistant_owner"),
            assistant_session_actor: string("assistant_session_actor"),
            answer: fields.get("answer").filter(|v| v.is_object()).cloned(),
            evidence: serde_json::from_value(claim.body["evidence"].clone()).unwrap_or_default(),
            resolved_by: string("resolved_by"),
        };
        answers.push(serde_json::to_value(answer).map_err(internal)?);
    }
    Ok(EpisodeState::Available { status, answers })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "step-run/history/ask";

    fn key() -> SourceEpisode {
        SourceEpisode { source_id: SOURCE.into(), episode: "opening".into() }
    }

    // Retained evidence is inserted into the actual Store schema, without exercising or
    // changing writer admission. These tests target reads of retained/damaged projections.
    fn fixture() -> Store {
        let store = Store::open_memory("history-reader").unwrap();
        {
            let conn = store.connection.write();
            conn.execute_batch(
                "INSERT INTO mission_runs(id,mission_id,initial_revision,current_generation_id,
                   root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,
                   created_at_unix_ms,updated_at_unix_ms)
                 VALUES('history','history','rev','history','rev','history',
                   'workspace','agent/requester','{}','once','completed','done','1','1');
                 INSERT INTO run_generations(id,run_id,revision,status,actor,reason,
                   created_at_unix_ms,updated_at_unix_ms)
                 VALUES('history','history','rev','completed','agent/requester',
                   'fixture','1','1');
                 INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,
                   status,attempt,assignee,title,goals,created_at_unix_ms,updated_at_unix_ms)
                 VALUES('step-run/history/ask','history','history','ask','hash',
                   'ready',9,'person/avery','POISON CURRENT TITLE','POISON CURRENT TEXT','1','1');"
            ).unwrap();
            insert(&conn, "opening", "work.person-asked", 10, json!({
                "person":"person/avery", "attempt":1, "title":"Retained title",
                "reason":"Retained question", "deadline_unix_ms":100
            }));
        }
        store
    }

    fn insert(conn: &Connection, id: &str, kind: &str, at: u64, fields: Value) -> u64 {
        conn.execute(
            "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
             VALUES(?1,'history-reader',?2,?1,?3)", params![id, at, at.to_string()],
        ).unwrap();
        conn.execute(
            "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,
               accepted_at_unix_ms) VALUES(?1,?1,?2,?3,'history-reader','person/avery',
               ?4,'[]',?5)",
            params![id, SOURCE, kind, json!({"fields":fields,"evidence":[]}).to_string(), at.to_string()],
        ).unwrap();
        conn.query_row("SELECT store_index FROM claims WHERE id=?1", [id], |row| row.get(0)).unwrap()
    }

    fn read(conn: &Connection, cut: u64) -> RequestHistoryItem {
        retained_request_history(conn, "person/avery", &key(), cut).unwrap().unwrap()
    }

    #[test]
    fn retained_runtime_current_assignment_exact_id_and_missing_source() {
        let store = fixture();
        let conn = store.connection.write();
        assert!(retained_request_history(&conn, "person/robin", &key(), u64::MAX).unwrap().is_none());
        let wrong = SourceEpisode { episode:"opening-extra".into(), ..key() };
        assert!(retained_request_history(&conn, "person/avery", &wrong, u64::MAX).unwrap().is_none());
        let duplicate = insert(&conn, "noncanonical", "work.person-asked", 20,
            json!({"reason":"not the canonical opening","person":"person/avery"}));
        let wrong = SourceEpisode { episode:"noncanonical".into(), ..key() };
        assert!(retained_request_history(&conn, "person/avery", &wrong, duplicate).unwrap().is_none());
        conn.execute("UPDATE step_runs SET assignee='person/blair' WHERE subject=?1", [SOURCE]).unwrap();
        assert!(retained_request_history(&conn, "person/avery", &key(), u64::MAX).unwrap().is_none());
        assert!(retained_request_history(&conn, "person/blair", &key(), u64::MAX).unwrap().is_none());
        conn.execute("DELETE FROM step_runs WHERE subject=?1", [SOURCE]).unwrap();
        assert!(retained_request_history(&conn, "person/blair", &key(), u64::MAX).unwrap().is_none());
    }

    #[test]
    fn retained_runtime_resolved_payload_never_uses_current_text_or_attempt() {
        let store = fixture();
        let conn = store.connection.write();
        let cut = insert(&conn, "answer", "work.person-done", 80, json!({
            "episode":"opening","attempt":1,"status":"completed","summary":"original answer",
            "answer":{"id":"yes"}
        }));
        conn.execute("UPDATE step_runs SET status='completed' WHERE subject=?1", [SOURCE]).unwrap();
        insert(&conn, "other-episode", "work.person-done", 90, json!({
            "episode":"different","attempt":9,"status":"completed","summary":"POISON"
        }));
        let item = read(&conn, cut);
        assert!(item.complete);
        match item.payload {
            HistoricalPayload::Available { title, text, .. } => {
                assert_eq!(title.as_deref(), Some("Retained title"));
                assert_eq!(text, "Retained question");
            }
            _ => panic!("retained ask must be available"),
        }
        match item.state {
            EpisodeState::Available { status, answers } => {
                assert_eq!(status, "completed");
                assert_eq!(answers.len(), 1);
                assert_eq!(answers[0]["summary"], "original answer");
            }
            _ => panic!("resolution is retained"),
        }
    }

    #[test]
    fn retained_runtime_deadline_cut_override_and_retry_dedup() {
        let store = fixture();
        let conn = store.connection.write();
        let default_cut = insert(&conn, "deadline", "work.person-done", 100, json!({
            "episode":"opening","status":"completed","summary":"default","resolved_by":"deadline"
        }));
        insert(&conn, "late", "work.person-done", 101, json!({
            "episode":"opening","status":"completed","summary":"late"
        }));
        let override_cut = insert(&conn, "override", "work.person-override", 102, json!({
            "episode":"opening","status":"completed","summary":"override",
            "answer":{"id":"yes"},"overrides":"deadline","key":"same-operation"
        }));
        insert(&conn, "retry", "work.person-override", 103, json!({
            "episode":"opening","status":"completed","summary":"POISON retry",
            "answer":{"id":"no"},"overrides":"deadline","key":"same-operation"
        }));
        let summary = |item: RequestHistoryItem| match item.state {
            EpisodeState::Available { answers, .. } => {
                assert_eq!(answers.len(), 1);
                answers[0]["summary"].as_str().unwrap().to_owned()
            }
            _ => panic!("standing answer must be available"),
        };
        assert_eq!(summary(read(&conn, default_cut)), "default");
        assert_eq!(summary(read(&conn, override_cut)), "override");
        assert_eq!(summary(read(&conn, u64::MAX)), "override");
    }

    #[test]
    fn retained_runtime_before_deadline_answer_beats_default() {
        let store = fixture();
        let conn = store.connection.write();
        insert(&conn, "on-time", "work.person-done", 99, json!({
            "episode":"opening","status":"completed","summary":"on time"
        }));
        insert(&conn, "deadline", "work.person-done", 100, json!({
            "episode":"opening","status":"completed","summary":"default","resolved_by":"deadline"
        }));
        match read(&conn, u64::MAX).state {
            EpisodeState::Available { answers, .. } => {
                assert_eq!(answers.len(), 1);
                assert_eq!(answers[0]["summary"], "on time");
            }
            _ => panic!("standing answer must be available"),
        }
    }

    #[test]
    fn retained_runtime_missing_or_oversized_payload_is_explicit() {
        let store = fixture();
        let conn = store.connection.write();
        assert!(matches!(read(&conn, 0).payload, HistoricalPayload::Unavailable { .. }));
        conn.execute("UPDATE claims SET body='{\"fields\":{\"person\":\"person/avery\",\"attempt\":1,\"reason\":\"retained reason\"}}' WHERE id='opening'", []).unwrap();
        assert!(matches!(read(&conn, u64::MAX).payload,
            HistoricalPayload::Unavailable { reason: PayloadUnavailable::PayloadUnavailable }));
        conn.execute("UPDATE claims SET body='{\"fields\":{\"person\":\"person/avery\",\"attempt\":1,\"title\":\"Retained title\"}}' WHERE id='opening'", []).unwrap();
        assert!(matches!(read(&conn, u64::MAX).payload,
            HistoricalPayload::Unavailable { reason: PayloadUnavailable::PayloadUnavailable }));
        let body = json!({"fields":{"person":"person/avery","attempt":1,
            "reason":"x".repeat(MAX_BODY_BYTES + 1)}}).to_string();
        conn.execute("UPDATE claims SET body=?1 WHERE id='opening'", [body]).unwrap();
        assert!(matches!(read(&conn, u64::MAX).payload,
            HistoricalPayload::Unavailable { reason: PayloadUnavailable::ReadBoundExceeded }));
    }

    #[test]
    fn retained_runtime_original_authority_is_required_at_every_payload_size() {
        let oversized_person = format!("person/{}", "x".repeat(MAX_BODY_BYTES + 1));
        for oversized in [false, true] {
            for original in [None, Some("person/robin"), Some(oversized_person.as_str())] {
                let store = fixture();
                let conn = store.connection.write();
                let mut fields = json!({"attempt":1,"title":"Retained title",
                    "reason":if oversized { "x".repeat(MAX_BODY_BYTES + 1) } else { "Retained question".into() }});
                if let Some(person) = original { fields["person"] = json!(person); }
                let body = json!({"fields":fields,"evidence":[]}).to_string();
                conn.execute("UPDATE claims SET body=?1 WHERE id='opening'", [body]).unwrap();
                assert!(!discussion_source::qualify_history(&conn, "person/avery", &key(), now_ms())
                    .unwrap().is_authorized());
                assert!(retained_request_history(&conn, "person/avery", &key(), u64::MAX)
                    .unwrap().is_none());
                assert!(discussion_source::qualify(&conn, "person/avery", &key(), now_ms())
                    .unwrap().is_none());
            }
        }
    }

    #[test]
    fn retained_runtime_bounded_payload_qualifies_but_wrong_current_assignee_does_not() {
        let store = fixture();
        let conn = store.connection.write();
        let fields = json!({"person":"person/avery","attempt":1,"title":"Retained title",
            "reason":"bounded retained question"});
        conn.execute("UPDATE claims SET body=?1 WHERE id='opening'",
            [json!({"fields":fields,"evidence":[]}).to_string()]).unwrap();
        assert!(matches!(discussion_source::qualify_history(&conn, "person/avery", &key(), now_ms())
            .unwrap(), discussion_source::HistoryQualification::Qualified));
        assert!(discussion_source::qualify(&conn, "person/avery", &key(), now_ms()).unwrap().is_some());
        assert!(read(&conn, u64::MAX).complete);
        conn.execute("UPDATE step_runs SET assignee='person/robin' WHERE subject=?1", [SOURCE]).unwrap();
        for person in ["person/avery", "person/robin"] {
            assert!(!discussion_source::qualify_history(&conn, person, &key(), now_ms())
                .unwrap().is_authorized());
            assert!(retained_request_history(&conn, person, &key(), u64::MAX).unwrap().is_none());
        }
    }

    #[test]
    fn retained_runtime_oversized_duplicate_wrong_key_and_repaired_opening_stay_absent() {
        let store = fixture();
        let conn = store.connection.write();
        let fields = json!({"person":"person/avery","attempt":1,"title":"Retained title",
            "reason":"x".repeat(MAX_BODY_BYTES + 1)});
        insert(&conn, "oversized-duplicate", "work.person-asked", 20, fields.clone());
        conn.execute("UPDATE claims SET body=?1 WHERE id='opening'",
            [json!({"fields":fields,"evidence":[]}).to_string()]).unwrap();
        for candidate in [
            SourceEpisode { episode:"oversized-duplicate".into(), ..key() },
            SourceEpisode { source_id:"step-run/history/other".into(), ..key() },
            SourceEpisode { episode:"missing-opening".into(), ..key() },
        ] {
            assert!(!discussion_source::qualify_history(&conn, "person/avery", &candidate, now_ms())
                .unwrap().is_authorized());
            assert!(retained_request_history(&conn, "person/avery", &candidate, u64::MAX)
                .unwrap().is_none());
        }
        assert!(matches!(discussion_source::qualify_history(&conn, "person/avery", &key(), now_ms())
            .unwrap(), discussion_source::HistoryQualification::PayloadOverBudget));
        conn.execute("INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)", ["opening"]).unwrap();
        assert!(!discussion_source::qualify_history(&conn, "person/avery", &key(), now_ms())
            .unwrap().is_authorized());
        assert!(retained_request_history(&conn, "person/avery", &key(), u64::MAX).unwrap().is_none());
    }

    #[test]
    fn retained_runtime_oversized_explicit_operation_requires_active_exact_canonical_proof() {
        let store = fixture();
        let conn = store.connection.write();
        let body = json!({"fields":{"person":"person/avery","attempt":1,
            "title":"Retained title","reason":"x".repeat(MAX_BODY_BYTES + 1)},
            "evidence":[], "_operation":{"id":"operation/history-opening","request_digest":"opening-digest"}});
        conn.execute("UPDATE claims SET body=?1 WHERE id='opening'", [body.to_string()]).unwrap();
        assert!(!discussion_source::qualify_history(&conn, "person/avery", &key(), now_ms())
            .unwrap().is_authorized());
        conn.execute("INSERT INTO operations(id,request_digest,canonical_claim_id,state)
            VALUES('operation/history-opening','opening-digest','opening','active')", []).unwrap();
        assert!(matches!(discussion_source::qualify_history(&conn, "person/avery", &key(), now_ms())
            .unwrap(), discussion_source::HistoryQualification::PayloadOverBudget));
        let item = read(&conn, u64::MAX);
        assert!(!item.complete);
        assert!(matches!(item.payload,
            HistoricalPayload::Unavailable { reason: PayloadUnavailable::ReadBoundExceeded }));
        insert(&conn, "different-opening", "work.person-asked", 20,
            json!({"person":"person/avery","attempt":1,"title":"Duplicate","reason":"Duplicate"}));
        for (state, digest, canonical) in [
            ("conflict", "opening-digest", "opening"),
            ("active", "different-digest", "opening"),
            ("active", "opening-digest", "different-opening"),
        ] {
            conn.execute("UPDATE operations SET state=?1,request_digest=?2,canonical_claim_id=?3
                WHERE id='operation/history-opening'", params![state, digest, canonical]).unwrap();
            assert!(!discussion_source::qualify_history(&conn, "person/avery", &key(), now_ms())
                .unwrap().is_authorized());
            assert!(retained_request_history(&conn, "person/avery", &key(), u64::MAX)
                .unwrap().is_none());
        }
    }

    #[test]
    fn retained_runtime_authorized_oversized_history_does_not_authorize_start() {
        let store = fixture();
        {
            let conn = store.connection.write();
            let body = json!({"fields":{"person":"person/avery","attempt":1,
                "title":"Retained title","reason":"x".repeat(MAX_BODY_BYTES + 1)}}).to_string();
            conn.execute("UPDATE claims SET body=?1 WHERE id='opening'", [body]).unwrap();
            assert!(matches!(discussion_source::qualify_history(&conn, "person/avery", &key(), now_ms())
                .unwrap(), discussion_source::HistoryQualification::PayloadOverBudget));
            let before_opening = read(&conn, 0);
            assert!(!before_opening.complete);
            assert!(matches!(before_opening.payload,
                HistoricalPayload::Unavailable { reason: PayloadUnavailable::PayloadUnavailable }));
            let item = read(&conn, u64::MAX);
            assert!(!item.complete);
            assert!(matches!(item.payload,
                HistoricalPayload::Unavailable { reason: PayloadUnavailable::ReadBoundExceeded }));
            assert!(discussion_source::qualify(&conn, "person/avery", &key(), now_ms())
                .unwrap().is_none());
        }
        let origin = thread_metadata::InboxOrigin {
            source_id: SOURCE.into(), episode:"opening".into(),
        };
        assert_eq!(thread_metadata::start_context(&store, "person/avery", &origin)
            .err().unwrap().code, "thread-unavailable");
        store.configure_thread_write_rollout([]);
        let message = ClaimInput {
            subject:"message/history-start".into(), kind:"message.sent".into(),
            actor:Some("person/avery".into()),
            fields:serde_json::from_value(json!({"from":"person/avery",
                "to":"agent/history","content":"Discuss the retained ask"})).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:None,
        };
        assert_eq!(thread_metadata::start_from_inbox(&store, thread_metadata::StartFromInbox {
            thread:"thread/history-start", title:"Retained ask", person:"person/avery",
            origin:&origin, key:"history-start", first_message:&message,
        }).err().unwrap().code, "thread-unavailable");
        let conn = store.connection.write();
        conn.execute("UPDATE step_runs SET assignee='person/robin' WHERE subject=?1", [SOURCE]).unwrap();
        for person in ["person/avery", "person/robin"] {
            assert!(!discussion_source::qualify_history(&conn, person, &key(), now_ms())
                .unwrap().is_authorized());
            assert!(retained_request_history(&conn, person, &key(), u64::MAX).unwrap().is_none());
        }
    }

    #[test]
    fn thread_history_keeps_authorized_oversized_origin_and_rejects_missing_authority() {
        let store = fixture();
        {
            let conn = store.connection.write();
            let body = json!({"fields":{"person":"person/avery","attempt":1,
                "title":"Retained title","reason":"x".repeat(MAX_BODY_BYTES + 1)}}).to_string();
            conn.execute("UPDATE claims SET body=?1 WHERE id='opening'", [body]).unwrap();
            insert(&conn, "history-thread-created", "thread.created", 20, json!({
                "person":"person/avery","agent":"agent/history","title":"History",
                "origin_source":SOURCE,"origin_episode":"opening"
            }));
            conn.execute("UPDATE claims SET subject='thread/history',
                body=json_set(body,'$._operation.id','operation/history-thread')
                WHERE id='history-thread-created'", []).unwrap();
            conn.execute("INSERT INTO operations(id,request_digest,canonical_claim_id,state)
                VALUES('operation/history-thread','history-thread','history-thread-created','active')", []).unwrap();
        }
        let history = || thread_metadata::request_history::read_with(
            &store, "person/avery", "thread/history", None, 10,
            retained_request_history, |_, _, _, _| Ok((Vec::new(), true)),
        );
        let page = history().unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].key, key());
        assert!(!page.complete);
        assert!(!page.items[0].complete);
        assert!(matches!(page.items[0].payload,
            HistoricalPayload::Unavailable { reason: PayloadUnavailable::ReadBoundExceeded }));
        for person in [None, Some("person/robin")] {
            {
                let conn = store.connection.write();
                let body = match person {
                    None => json!({"fields":{"attempt":1,"reason":"x".repeat(MAX_BODY_BYTES + 1)}}),
                    Some(person) => json!({"fields":{"person":person,"attempt":1,
                        "reason":"x".repeat(MAX_BODY_BYTES + 1)}}),
                };
                conn.execute("UPDATE claims SET body=?1 WHERE id='opening'", [body.to_string()]).unwrap();
            }
            assert_eq!(history().err().unwrap().code, "thread-unavailable");
        }
    }

    #[test]
    fn retained_runtime_cancelled_and_unprovable_ended_states() {
        let store = fixture();
        let conn = store.connection.write();
        conn.execute("UPDATE step_runs SET status='cancelled' WHERE subject=?1", [SOURCE]).unwrap();
        assert!(matches!(read(&conn, u64::MAX).state, EpisodeState::Unavailable));
        insert(&conn, "cancelled", "work.person-cancelled", 80, json!({
            "episode":"opening","status":"cancelled","summary":"ended"
        }));
        assert!(matches!(read(&conn, u64::MAX).state,
            EpisodeState::Available { status, .. } if status == "cancelled"));
    }

    #[test]
    fn retained_runtime_deadline_clock_uses_only_retained_cut() {
        let store = fixture();
        let conn = store.connection.write();
        insert(&conn, "hold", fractal_schema::availability::DEADLINE_CLOCK_KIND, 50,
            json!({"deadline_clock":"hold-all","until_unix_ms":150}));
        conn.execute("UPDATE claims SET subject='person/avery' WHERE id='hold'", []).unwrap();
        insert(&conn, "on-time-held-clock", "work.person-done", 180, json!({
            "episode":"opening","status":"completed","summary":"held clock answer"
        }));
        let cut = insert(&conn, "deadline", "work.person-done", 200, json!({
            "episode":"opening","status":"completed","summary":"default","resolved_by":"deadline"
        }));
        insert(&conn, "later-clock", fractal_schema::availability::DEADLINE_CLOCK_KIND, 60,
            json!({"deadline_clock":"run"}));
        conn.execute("UPDATE claims SET subject='person/avery' WHERE id='later-clock'", []).unwrap();
        match read(&conn, cut).state {
            EpisodeState::Available { answers, .. } => {
                assert_eq!(answers.len(), 1);
                assert_eq!(answers[0]["summary"], "held clock answer");
            }
            _ => panic!("retained deadline clock must keep the on-time answer"),
        }
    }

    #[test]
    fn authored_epoch_without_opening_recipient_never_qualifies() {
        let store = fixture();
        let conn = store.connection.write();
        conn.execute("DELETE FROM claims WHERE id='opening'", []).unwrap();
        let authored = SourceEpisode {
            source_id: SOURCE.into(), episode:"run-generation/history:1:2".into(),
        };
        assert!(discussion_source::qualify(&conn, "person/avery", &authored, now_ms()).unwrap().is_none());
        assert!(retained_request_history(&conn, "person/avery", &authored, u64::MAX).unwrap().is_none());
        for episode in ["run-generation/other:1:2", "run-generation/history:01:2",
            "run-generation/history:1:invalid"] {
            let key = SourceEpisode { episode:episode.into(), ..authored.clone() };
            assert!(discussion_source::qualify(&conn, "person/avery", &key, now_ms()).unwrap().is_none());
        }
    }

    #[test]
    fn retained_runtime_legacy_cancellation_requires_causal_fence() {
        let store = fixture();
        let conn = store.connection.write();
        insert(&conn, "legacy-cancel", "work.person-cancelled", 80, json!({
            "attempt":1,"status":"cancelled","summary":"legacy cancellation"
        }));
        assert!(matches!(read(&conn, u64::MAX).state, EpisodeState::Unavailable));
        conn.execute("UPDATE claims SET predecessors='[\"opening\"]' WHERE id='legacy-cancel'", []).unwrap();
        assert!(matches!(read(&conn, u64::MAX).state,
            EpisodeState::Available { status, .. } if status == "cancelled"));
    }

    #[test]
    fn retained_runtime_wrong_attempt_answer_cannot_replace_exact_attempt() {
        let store = fixture();
        let conn = store.connection.write();
        insert(&conn, "right-attempt", "work.person-done", 80, json!({
            "episode":"opening","attempt":1,"status":"completed","summary":"right attempt"
        }));
        insert(&conn, "wrong-attempt", "work.person-done", 90, json!({
            "episode":"opening","attempt":9,"status":"completed","summary":"POISON wrong attempt"
        }));
        match read(&conn, u64::MAX).state {
            EpisodeState::Available { answers, .. } => {
                assert_eq!(answers.len(), 1);
                assert_eq!(answers[0]["summary"], "right attempt");
            }
            _ => panic!("exact-attempt answer must be retained"),
        }
    }
}
