//! Sidebar rows: relationships and presentation are resolved here, not joined by clients.
use super::*;
use crate::api::waiting_on::{self, DEFAULT_QUIET_AFTER_MS};
use crate::waiting_on::{RunVerdict, RunWaitingOn};

const DAY_MS: u128 = 86_400_000;
const MAX_ROWS: usize = 100;

// A pool offer is not a personal relationship.
const RELATED: &str = r#"
WITH related(id) AS (
 SELECT id FROM mission_runs WHERE requester=?1
 UNION SELECT run_id FROM step_runs WHERE assignee=?1
 UNION SELECT run_id FROM step_runs WHERE lease_owner=?1
 UNION SELECT run_id FROM step_runs WHERE CASE WHEN json_valid(available_to) THEN json_array_length(available_to)=1 ELSE 0 END AND available_to=json_array(?1)
 UNION SELECT substr(subject,13) FROM claims
 WHERE kind IN ('mission-run.created','mission-run.report-to')
 AND json_extract(body,'$.fields.report_to')=?1
 UNION SELECT substr(json_extract(c.body,'$.fields.run'),13)
 FROM claims c JOIN step_runs s ON s.subject=c.subject
 WHERE c.kind='work.person-asked' AND json_extract(c.body,'$.fields.person')=?1
 AND s.status IN ('pending','ready')
)
SELECT 'mission-run/'||r.id, d.deadline_at_unix_ms
FROM related JOIN mission_runs r ON r.id=related.id
LEFT JOIN mission_run_deadlines d ON d.run_id=r.id
WHERE r.status NOT IN ('completed','failed','cancelled') OR CAST(r.updated_at_unix_ms AS INTEGER)>=?2
ORDER BY r.id
"#;

fn candidates(store: &Store, agent: &str, now: u128) -> anyhow::Result<Vec<(String, Option<u128>)>> {
    let connection = store.readers.get();
    let mut query = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h314a3080bc73 owner=fractal_api_core::api::client_v0::related_runs TODO=trace-statement-and-value-representation */ RELATED)?;
    let rows = query.query_map(rusqlite::params![agent, i64::try_from(now.saturating_sub(DAY_MS))?], |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?)))?
        .collect::<Result<Vec<(String, Option<String>)>, _>>()?;
    rows.into_iter().map(|(id, at)| Ok((id, at.map(|at| at.parse()).transpose()?))).collect()
}

fn row(wait: &RunWaitingOn, eta: Option<Value>, deadline: Option<u128>, ended: Option<u128>, person: Option<&str>, now: u128) -> Option<Value> {
    let folded = wait.verdict == RunVerdict::Finished;
    if folded && ended.is_none_or(|at| now.saturating_sub(at) >= DAY_MS) {
        return None;
    }
    let waiting_on_you = !folded && person.is_some_and(|person|
        wait.open.iter().any(|step| step.who.as_deref() == Some(person)));
    let late = !folded && deadline.is_some_and(|at| at < now);
    let urgency = if folded { 4 } else if waiting_on_you { 0 } else if late { 1 }
        else if wait.verdict == RunVerdict::Stalled { 2 } else { 3 };
    let done = wait.finished.iter().filter(|step| step.state == "completed").count();
    Some(json!({
        "id":wait.run, "mission_id":wait.mission, "status":wait.status,
        "status_chip":if folded {wait.status.as_str()} else if waiting_on_you {"waiting-on-you"}
            else {match wait.verdict { RunVerdict::Moving => "running", RunVerdict::Waiting => "waiting", _ => "stalled" }},
        "progress":{"done":done,"total":wait.open.len()+wait.pending.len()+wait.finished.len()},
        "eta":if waiting_on_you || folded {None} else {eta},
        "waiting_on_you":waiting_on_you, "late":late, "deadline_at":deadline,
        "current_step":wait.open.first(), "waits_on":wait.open,
        "urgency_rank":urgency, "folded":folded, "completed_at":ended
    }))
}

fn sort(rows: &mut Vec<Value>) -> usize {
    rows.sort_by(|a,b| a["urgency_rank"].as_u64().cmp(&b["urgency_rank"].as_u64())
        .then_with(|| if a["folded"] == true {
            b["completed_at"].as_u64().cmp(&a["completed_at"].as_u64())
        } else { a["deadline_at"].as_u64().unwrap_or(u64::MAX).cmp(&b["deadline_at"].as_u64().unwrap_or(u64::MAX)) })
        .then_with(|| a["id"].as_str().cmp(&b["id"].as_str())));
    let total = rows.len();
    rows.truncate(MAX_ROWS);
    total
}

pub async fn get(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    AxumPath(agent): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let agent = client_detail_id("agent", &agent);
    let person = person_filter(&session, None)?;
    let sources = waiting_on::sources(&state.store)?;
    let store = state.store.clone();
    blocking_store(move || {
        let now = client_now_ms();
        let candidates = candidates(&store, &agent, now)?;
        let ids = candidates.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let states = store.mission_run_states_for_runs(&ids)?;
        let mut rows = Vec::new();
        for (id, deadline) in candidates {
            let Some(steps) = store.run_step_states(&id)? else { continue };
            let ended = states.get(&id).and_then(|state| state.completed_at_unix_ms);
            if matches!(steps.status.as_str(), "completed" | "failed" | "cancelled")
                && ended.is_none_or(|at| now.saturating_sub(at) >= DAY_MS) { continue; }
            let wait = waiting_on::run_waiting_on(&steps, &sources.work, &sources.missions.rows, now, DEFAULT_QUIET_AFTER_MS);
            let eta = if wait.verdict == RunVerdict::Finished {None} else {store.mission_run_eta(&id, now)?};
            if let Some(value) = row(&wait, eta, deadline, ended, person.as_deref(), now) { rows.push(value); }
        }
        let total_rows = sort(&mut rows);
        Ok(Json(json!({"kind":"agent-related-runs", "agent_id":agent, "as_of":now,
            "work_cut":sources.work.cut, "missions_cut":sources.missions.cut,
            "total_rows":total_rows, "truncated":total_rows>rows.len(), "rows":rows})))
    }).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn relationships_include_asks_but_not_pool_offers_or_expired_history() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(r#"CREATE TABLE mission_runs(id,requester,status,updated_at_unix_ms);
            CREATE TABLE step_runs(subject,run_id,assignee,lease_owner,available_to,status);
            CREATE TABLE claims(subject,kind,body);
            CREATE TABLE mission_run_deadlines(run_id,deadline_at_unix_ms);
            INSERT INTO mission_runs VALUES
                ('owned','agent/example','running','0'),('held','other','running','0'),
                ('offered','other','running','0'),('pool','other','running','0'),
                ('supervised','other','running','0'),('asked','other','running','0'),
                ('expired','agent/example','completed','0'),('resolved','other','running','0');
            INSERT INTO step_runs VALUES
                ('held','held',NULL,'agent/example','[]','working'),
                ('offered','offered',NULL,NULL,'["agent/example"]','ready'),
                ('pool','pool',NULL,NULL,'["agent/example","agent/other"]','ready'),
                ('ask','asked','person/other',NULL,'[]','ready'),
                ('resolved','resolved','person/other',NULL,'[]','completed');"#).unwrap();
        for (subject, kind, fields) in [
            ("mission-run/supervised", "mission-run.created", json!({"report_to":"agent/example"})),
            ("ask", "work.person-asked", json!({"person":"agent/example","run":"mission-run/asked"})),
            ("resolved", "work.person-asked", json!({"person":"agent/example","run":"mission-run/resolved"})),
        ] {
            db.execute("INSERT INTO claims VALUES(?1,?2,?3)", rusqlite::params![subject,kind,json!({"fields":fields}).to_string()]).unwrap();
        }
        let mut query = db.prepare(RELATED).unwrap();
        let ids = query.query_map(rusqlite::params!["agent/example",1], |r| r.get::<_,String>(0)).unwrap()
            .collect::<Result<Vec<_>,_>>().unwrap();
        assert_eq!(ids, ["mission-run/asked","mission-run/held","mission-run/offered","mission-run/owned","mission-run/supervised"]);
    }

    use super::*;
    use crate::waiting_on::{StepWait, WaitKind};

    #[test]
    fn individual_offers_seek_the_recipient_index() {
        let store = Store::open_memory("example").unwrap();
        let db = store.readers.get();
        let mut plan = db.prepare(&format!("EXPLAIN QUERY PLAN {RELATED}")).unwrap();
        let details = plan.query_map(rusqlite::params!["agent/example",0], |r| r.get::<_,String>(3)).unwrap()
            .collect::<Result<Vec<_>,_>>().unwrap().join("\n");
        assert!(details.contains("SEARCH step_runs USING COVERING INDEX step_runs_single_agent_offer_index"), "{details}");
        assert!(!details.contains("SCAN step_runs"), "{details}");
    }

    #[test]
    fn response_bound_keeps_the_most_urgent_rows_and_reports_total() {
        let mut rows = (0..=MAX_ROWS).rev().map(|n| json!({"id":format!("run/{n:03}"),"urgency_rank":3,"folded":false})).collect();
        assert_eq!(sort(&mut rows), MAX_ROWS+1);
        assert_eq!(rows.len(), MAX_ROWS);
        assert_eq!(rows[0]["id"], "run/000");
    }

    #[tokio::test]
    async fn authenticated_session_only_marks_its_own_person_holds() {
        use crate::WorkListStore;
        use crate::api::published_lists::tests_shared_client_v0_tests::{fold, publish};
        use tower::ServiceExt as _;
        let root = tempfile::tempdir().unwrap();
        let state = crate::api::tests_shared::state(root.path());
        publish(&state.store, r#"version 2
mission "example" state="ready" report-to="agent/example" {
 goal "Review."
 step "check" { assigned-to "person/reviewer" }
}"#, "publish");
        let run = state.store.create_mission_run(&crate::model::MissionRunRequest {
            mission:"example".into(), revision:None, workspace:root.path().display().to_string(),
            requester:Some("person/operator".into()), mode:None, inputs:BTreeMap::new(), idempotency_key:"run".into(),
        }).unwrap();
        state.store.set_step_state(&run.steps[0].subject, "ready", None).unwrap();
        fold(&state.store);
        state.store.published_work_list().start();
        crate::api::published_lists::refresh_work_once(&state.store);
        for (person, expected) in [("person/reviewer",true),("person/other",false)] {
            let app = Router::new().route("/related/{*id}", axum::routing::get(super::get))
                .layer(Extension(ClientSession::local(Some(person)).unwrap())).with_state(state.clone());
            let response = app.oneshot(axum::http::Request::builder().uri("/related/agent/example")
                .body(axum::body::Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 1_000_000).await.unwrap();
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(value["rows"].as_array().unwrap().len(), 1);
            assert_eq!(value["rows"][0]["waiting_on_you"], expected);
        }
    }

    fn wait(verdict: RunVerdict) -> RunWaitingOn {
        RunWaitingOn { run:"mission-run/example".into(), mission:"mission/example".into(),
            status:if verdict == RunVerdict::Finished {"completed"} else {"running"}.into(),
            generation:"generation/example".into(), verdict, cause:None, reason:String::new(),
            open:vec![], pending:vec![], finished:vec![], quiet_after_ms:1000, work_cut:1, as_of_unix_ms:DAY_MS }
    }

    #[test]
    fn person_wait_replaces_eta_and_wins_urgency() {
        let mut wait = wait(RunVerdict::Waiting);
        wait.open.push(StepWait { step:"step-run/example/check".into(), path:"check".into(),
            title:Some("Check".into()), state:"ready".into(), kind:WaitKind::Ask,
            who:Some("person/reviewer".into()), seat_state:None, since_unix_ms:None,
            lease_expires_at_unix_ms:None, until_unix_ms:None, detail:None, wake_attempts:None });
        let own = row(&wait, Some(json!({"seconds":10})), Some(1), None, Some("person/reviewer"), DAY_MS).unwrap();
        assert_eq!(own["status_chip"], "waiting-on-you");
        assert_eq!(own["urgency_rank"], 0);
        assert!(own["eta"].is_null());
        assert_eq!(own["current_step"]["title"], "Check");
        let other = row(&wait, Some(json!({"seconds":10})), Some(1), None, Some("person/other"), DAY_MS).unwrap();
        assert_eq!(other["urgency_rank"], 1);
        assert!(other["late"].as_bool().unwrap());
        assert!(!other["eta"].is_null());
    }

    #[test]
    fn finished_rows_expire_at_24_hours_and_sort_after_active() {
        let finished = wait(RunVerdict::Finished);
        assert!(row(&finished, None, None, Some(0), None, DAY_MS).is_none());
        let recent = row(&finished, None, None, Some(1), None, DAY_MS).unwrap();
        assert_eq!(recent["folded"], true);
        let active = row(&wait(RunVerdict::Moving), None, None, None, None, DAY_MS).unwrap();
        let mut rows = vec![recent, active];
        sort(&mut rows);
        assert_eq!(rows[0]["folded"], false);
    }
}
