// These raw projection writes and schema includes are fixture-only. Keep the cfg explicit in
// this source file as well as its parent module; the source inventory does not traverse cfgs.
#[cfg(test)]
mod controls {
    use super::super::*;
use crate::api::tests_shared::state as fixture;
use crate::MissionsStore;

fn declare(state: &AppState, name: &str, timing: &str) {
    let source = if timing == "stop" { format!("version 2\nschedule \"{name}\" {{ stop }}\n") } else { format!("version 2\nschedule \"{name}\" {{ host \"local\"; {timing}; work {{ mission \"harvest\"; workspace \"/tmp/harvest\" }} }}\n") };
    let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
    let plan = state.store.mission(&intent, crate::model::IntentInput { kdl:source,source_name:None }).unwrap();
    state.store.apply(&intent,&plan.subject_tokens,&format!("declare-{name}")).unwrap();
}
fn page(state: &AppState, after: &str, limit: usize, now: u128) -> Value {
    let session = ClientSession::local(Some("person/ada")).unwrap();
    state.store.read_snapshot(|index| Ok(read_page(state,&session,index,after,limit,DEFAULT_UNHEARD_MS,now).unwrap().1)).unwrap()
}
fn stamp(value: &str) -> u128 { chrono::DateTime::parse_from_rfc3339(value).unwrap().timestamp_millis() as u128 }
fn body(state: &AppState, subject: &str, value: &Value) {
    state.store.connection.write().execute("UPDATE desired SET body=?2 WHERE subject=?1",rusqlite::params![subject,value.to_string()]).unwrap();
}

#[test]
fn declarations_show_recorded_owner_local_origin_and_future_fields_without_standby_semantics() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path());
    declare(&state,"harvest","every \"1h\"; anchor \"2026-01-01T00:00:00Z\"");
    let mut raw: Value = serde_json::from_str(&state.store.schedule_declaration_rows("",1).unwrap()[0].body.clone().unwrap()).unwrap();
    let future = json!({"name":"standby","arguments":[],"children":[
        {"name":"host","arguments":["other"],"children":[]},
        {"name":"after","arguments":["5m"],"children":[]},
        {"name":"future-mode","arguments":["opaque"],"children":[]}
    ]});
    raw["children"].as_array_mut().unwrap().push(future.clone());
    raw["future_top_level"] = json!({"kept":true});
    body(&state,"schedule/harvest",&raw);
    state.store.connection.write().execute("UPDATE desired SET owner_run='mission-run/owner',owner_generation='run-generation/owner',owner_step='step-run/owner/tick' WHERE subject='schedule/harvest'",[]).unwrap();
    let out = page(&state,"",50,stamp("2026-10-21T08:00:00Z"));
    let row = &out["items"][0];
    assert_eq!(row["owner"]["run_id"],"mission-run/owner");
    assert_eq!(row["owner"]["state"],"recorded");
    assert_eq!(row["primary_host_id"],"host/node");
    assert_eq!(row["standby_state"],"declared");
    assert_eq!(row["standby_node"],future);
    assert_eq!(row["declaration"]["future_top_level"]["kept"],true);
    assert!(row.get("run_twice_warning").is_none());
    assert!(row.get("standby").is_none(),"this read returns the raw standby node without takeover interpretation");
    assert_eq!(row["knowledge"]["basis"],"local-read");
    assert_eq!(row["knowledge"]["completeness"],"unknown");
    assert_eq!(row["next_declared_fire_at"],"2026-10-21T08:00:00.000Z");
}

#[test]
fn scheduler_rejected_standby_keeps_raw_node_and_reports_unknown_tick() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path());
    declare(&state,"harvest","every \"1h\"; anchor \"2026-01-01T00:00:00Z\"");
    let mut raw: Value = serde_json::from_str(&state.store.schedule_declaration_rows("",1).unwrap()[0].body.clone().unwrap()).unwrap();
    let invalid = json!({"name":"standby","arguments":[],"children":[{"name":"future-mode","arguments":["opaque"],"children":[]}]});
    raw["children"].as_array_mut().unwrap().push(invalid.clone());
    raw["future_top_level"] = json!({"kept":true});
    body(&state,"schedule/harvest",&raw);
    let out = page(&state,"",50,stamp("2026-10-21T08:00:00Z"));
    let row = &out["items"][0];
    assert_eq!(row["next_fire_state"],"unknown");
    assert!(row["next_declared_fire_at"].is_null());
    assert_eq!(row["state"],"unknown");
    assert_eq!(row["standby_state"],"declared");
    assert_eq!(row["standby_node"],invalid);
    assert_eq!(row["declaration"],raw);
}

#[test]
fn primary_local_is_the_declaration_origin_and_relay_only_age_is_unknown() {
    let root=tempfile::tempdir().unwrap(); let state=fixture(root.path());
    let raw=json!({"name":"schedule","arguments":["remote"],"children":[
        {"name":"host","arguments":["local"],"children":[]},
        {"name":"at","arguments":["2030-01-01T00:00:00Z"],"children":[]},
        {"name":"work","arguments":[],"children":[{"name":"mission","arguments":["harvest"],"children":[]},{"name":"workspace","arguments":["/tmp/harvest"],"children":[]}]}
    ]});
    let row=ScheduleDeclarationRow {subject:"schedule/remote".into(),revision:"rev".into(),claim:"claim/remote".into(),body:Some(raw.to_string()),body_bytes:raw.to_string().len() as u64,origin:Some("origin".into()),owner_run:None,owner_generation:None,owner_step:None};
    let shown=row_value(&row,stamp("2026-10-21T08:00:00Z"));
    assert_eq!(shown["primary_host_id"],"host/origin");
    assert_eq!(shown["owner"]["state"],"not-recorded");
    let times=BTreeMap::from([("relay".into(),Some("1000".into()))]);
    let observation=knowledge(row.origin.as_deref(),&state.node,&times,2000);
    assert_eq!(observation["basis"],"unknown"); assert!(observation["age_ms"].is_null());
    assert_eq!(shown["standby_state"],"not-declared");
}

#[test]
fn empty_listing_still_names_unheard_hosts_with_exact_threshold_and_unknown_clock() {
    let root=tempfile::tempdir().unwrap(); let mut state=fixture(root.path());
    state.configured_peers=vec!["never".into(),"equal".into(),"older".into(),"future".into()];
    let connection=state.store.connection.write();
    for (peer,time) in [("equal","700000"),("older","699999"),("future","1000001")] {
        connection.execute("INSERT INTO replication_peers(peer,status,last_success_at_unix_ms,updated_at_unix_ms) VALUES(?1,'up',?2,?2)",rusqlite::params![peer,time]).unwrap();
    }
    drop(connection);
    let shown=page(&state,"",50,1_000_000);
    assert!(shown["items"].as_array().unwrap().is_empty());
    let unheard=shown["coverage"]["unheard_hosts"].as_array().unwrap();
    assert_eq!(unheard.iter().map(|row|row["host_id"].as_str().unwrap()).collect::<Vec<_>>(),vec!["host/future","host/never","host/older"]);
    assert_eq!(unheard[0]["reason"],"clock-unknown");
    assert_eq!(unheard[1]["reason"],"never-recorded");
    assert_eq!(unheard[2]["age_ms"],300001);
    assert_eq!(shown["coverage"]["membership"],"unknown");
    assert!(knowledge(Some("future"),"node",&BTreeMap::from([("future".into(),Some("1000001".into()))]),1_000_000)["age_ms"].is_null());
}

#[test]
fn host_coverage_reports_omitted_inputs_and_never_claims_a_fleet_directory() {
    let root=tempfile::tempdir().unwrap(); let mut state=fixture(root.path());
    state.configured_peers=(0..205).map(|n|format!("peer-{n:03}")).collect();
    let shown=page(&state,"",50,1_000_000);
    assert_eq!(shown["coverage"]["scope"],"configured-peers-and-page-hosts");
    assert_eq!(shown["coverage"]["omitted_configured_peer_count"],5);
    assert_eq!(shown["coverage"]["truncated"],true);
    assert!(shown["coverage"]["unheard_hosts"].as_array().unwrap().len()<=200);
}

#[test]
fn explicit_keyset_pages_keep_oversized_and_unknown_declarations_visible() {
    let root=tempfile::tempdir().unwrap(); let state=fixture(root.path());
    for name in ["a","b","c"] { declare(&state,name,"at \"2030-01-01T00:00:00Z\""); }
    let mut oversized:Value=serde_json::from_str(&state.store.schedule_declaration_rows("",1).unwrap()[0].body.clone().unwrap()).unwrap();
    oversized["future"]=json!("x".repeat(70*1024)); body(&state,"schedule/a",&oversized);
    body(&state,"schedule/b",&json!({"future":{"unknown":true}}));
    let first=page(&state,"",1,1_000_000);
    assert_eq!(first["items"][0]["id"],"schedule/a");
    assert_eq!(first["items"][0]["state"],"unknown");
    assert!(first["items"][0]["read_problem"].as_str().unwrap().contains("64 KiB"));
    assert_eq!(first["page"]["has_more"],true);
    let second=page(&state,"schedule/a",1,1_000_000);
    assert_eq!(second["items"][0]["id"],"schedule/b");
    assert_eq!(second["items"][0]["declaration"]["future"]["unknown"],true);
    assert_eq!(second["items"][0]["standby_state"],"unknown");
    let last=page(&state,"schedule/b",1,1_000_000);
    assert_eq!(last["items"][0]["id"],"schedule/c"); assert_eq!(last["page"]["has_more"],false);
}

#[test]
fn response_byte_budget_continues_at_the_actual_last_returned_declaration() {
    let root=tempfile::tempdir().unwrap(); let state=fixture(root.path());
    for n in 0usize..24 {
        let name=format!("schedule-{n:02}"); declare(&state,&name,"at \"2030-01-01T00:00:00Z\"");
        let subject=format!("schedule/{name}");
        let row=state.store.schedule_declaration_rows("",200).unwrap();
        let mut raw:Value=serde_json::from_str(&row.iter().find(|r|r.subject==subject).unwrap().body.clone().unwrap()).unwrap();
        raw["future_payload"]=json!("x".repeat(48*1024)); body(&state,&subject,&raw);
    }
    let now=1_000_000; let session=ClientSession::local(Some("person/ada")).unwrap();
    let first=page(&state,"",200,now);
    assert!(first["items"].as_array().unwrap().len()<24);
    assert_eq!(first["page"]["has_more"],true);
    assert!(serde_json::to_vec(&first).unwrap().len()<CLIENT_MAX_RESPONSE_BYTES-4096);
    let position=decode(first["page"]["next_cursor"].as_str().unwrap(),&state,&session,now).unwrap();
    assert_eq!(position.after,first["items"].as_array().unwrap().last().unwrap()["id"].as_str().unwrap());
    let second=page(&state,&position.after,200,now);
    assert_eq!(first["items"].as_array().unwrap().len()+second["items"].as_array().unwrap().len(),24);
}

#[test]
fn cursor_binds_session_instance_parameters_and_expiry_and_rejects_tampering() {
    let root=tempfile::tempdir().unwrap(); let state=fixture(root.path());
    let session=ClientSession::local(Some("person/ada")).unwrap();
    let cursor=encode(Position {instance:instance(&state),actor:session.actor.clone(),authority:session.authority_actor.clone(),grant:session.pairing_grant.clone(),after:"schedule/a".into(),limit:50,unheard_after_ms:300000,expires_at:1000}).unwrap();
    assert_eq!(decode(&cursor,&state,&session,999).unwrap().after,"schedule/a");
    assert!(decode(&cursor,&state,&session,1000).is_err());
    let mut other=session.clone();other.actor="person/other".into(); assert!(decode(&cursor,&state,&other,999).is_err());
    let another_root=tempfile::tempdir().unwrap(); let another=fixture(another_root.path());assert!(decode(&cursor,&another,&session,999).is_err());
    let mut tampered=cursor.into_bytes();let last=tampered.len()-1;tampered[last]=if tampered[last]==b'A' {b'B'} else {b'A'};
    assert!(decode(std::str::from_utf8(&tampered).unwrap(),&state,&session,999).is_err());
}

#[tokio::test]
async fn listing_accepts_declaration_only_and_both_scope_sessions() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path());
    declare(&state, "harvest", "at \"2030-01-01T00:00:00Z\"");
    for scopes in [
        &["read.declarations"][..],
        &["read.declarations", "read.projections"][..],
    ] {
        let mut session = ClientSession::local(Some("person/ada")).unwrap();
        session.scopes = scopes.iter().map(|scope| (*scope).to_owned()).collect();
        let (_, Json(page)) = list(
            State(state.clone()), Extension(session), Query(SchedulesQuery::default()),
        ).await.unwrap();
        assert_eq!(page["collection"], "schedules");
        assert_eq!(page["items"][0]["id"], "schedule/harvest");
    }
}

#[tokio::test]
async fn listing_rejects_projection_only_and_unscoped_sessions_with_declaration_scope_error() {
    let root = tempfile::tempdir().unwrap();
    let state = fixture(root.path());
    for scopes in [&["read.projections"][..], &[][..]] {
        let mut session = ClientSession::local(Some("person/ada")).unwrap();
        session.scopes = scopes.iter().map(|scope| (*scope).to_owned()).collect();
        let error = list(
            State(state.clone()), Extension(session), Query(SchedulesQuery::default()),
        ).await.unwrap_err();
        assert_eq!(error.status, axum::http::StatusCode::FORBIDDEN);
        assert_eq!(error.code, "forbidden");
        assert_eq!(error.message, "the authenticated client session is not granted `read.declarations`");
    }
}

#[tokio::test]
async fn listing_requires_existing_declaration_scope_and_validates_page_and_cursor_parameters() {
    let root=tempfile::tempdir().unwrap();let state=fixture(root.path());
    assert!(list(State(state.clone()),Extension(ClientSession::local(None).unwrap()),Query(SchedulesQuery::default())).await.is_err());
    let mut limited=ClientSession::local(Some("person/ada")).unwrap();limited.scopes=["read.projections".into()].into_iter().collect();
    assert!(list(State(state.clone()),Extension(limited),Query(SchedulesQuery::default())).await.is_err());
    for limit in [0,201] {
        assert!(list(State(state.clone()),Extension(ClientSession::local(Some("person/ada")).unwrap()),Query(SchedulesQuery{limit:Some(limit),..Default::default()})).await.is_err());
    }
    let too_large=list(State(state.clone()),Extension(ClientSession::local(Some("person/ada")).unwrap()),Query(SchedulesQuery{unheard_after_ms:Some(9_007_199_254_740_992),..Default::default()})).await.unwrap_err();
    assert_eq!(too_large.code,"validation-failed");
    assert_eq!(too_large.status,axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    assert!(too_large.message.contains("9007199254740991"));
    declare(&state,"a","at \"2030-01-01T00:00:00Z\"");declare(&state,"b","at \"2030-01-01T00:00:00Z\"");
    let (_,Json(first))=list(State(state.clone()),Extension(ClientSession::local(Some("person/ada")).unwrap()),Query(SchedulesQuery{limit:Some(1),..Default::default()})).await.unwrap();
    let cursor=first["page"]["next_cursor"].as_str().unwrap().to_owned();
    assert!(list(State(state),Extension(ClientSession::local(Some("person/ada")).unwrap()),Query(SchedulesQuery{limit:Some(2),cursor:Some(cursor),..Default::default()})).await.is_err());
}

#[test]
fn nominal_next_tick_never_infers_completion_or_catchup_and_unknown_mode_stays_unknown() {
    let root=tempfile::tempdir().unwrap();let state=fixture(root.path());
    declare(&state,"at","at \"2026-10-21T08:00:00Z\"");
    declare(&state,"every","every \"1h\"; anchor \"2026-01-01T00:00:00Z\"; catch-up \"all\"; max-catch-up 1");
    let shown=page(&state,"",50,stamp("2026-10-21T08:00:00.001Z"));
    assert_eq!(shown["items"][0]["next_fire_state"],"past-one-shot");
    assert_eq!(shown["items"][1]["next_declared_fire_at"],"2026-10-21T09:00:00.000Z");
    let mut raw:Value=serde_json::from_str(&state.store.schedule_declaration_rows("schedule/at",1).unwrap()[0].body.clone().unwrap()).unwrap();
    raw["children"].as_array_mut().unwrap().push(json!({"name":"future-clock","arguments":["opaque"]}));body(&state,"schedule/every",&raw);
    let changed=page(&state,"schedule/at",1,stamp("2026-10-21T08:00:00Z"));
    assert_eq!(changed["items"][0]["next_fire_state"],"unknown");
    assert!(changed["items"][0]["declaration"]["children"].as_array().unwrap().iter().any(|node|node["name"]=="future-clock"));
}

#[test]
fn stopped_declarations_remain_visible_without_a_future_tick() {
    let root=tempfile::tempdir().unwrap(); let state=fixture(root.path());
    declare(&state,"stopped","stop");
    let shown=page(&state,"",50,1_000_000);
    let row=&shown["items"][0];
    assert_eq!(row["id"],"schedule/stopped");
    assert_eq!(row["state"],"stopped");
    assert_eq!(row["next_fire_state"],"stopped");
    assert!(row["next_declared_fire_at"].is_null());
    assert!(row["primary_host_id"].is_null());
}


#[tokio::test]
async fn schedules_structured_response_matches_the_additive_public_schema() {
    let root=tempfile::tempdir().unwrap(); let state=fixture(root.path());
    declare(&state,"harvest","at \"2030-01-01T00:00:00Z\"");
    let (Extension(snapshot),Json(value))=list(State(state),Extension(ClientSession::local(Some("person/ada")).unwrap()),Query(SchedulesQuery::default())).await.unwrap();
    let schema:Value=serde_json::from_str(include_str!("../../../../../../docs/st3/client-v0/schemas/client-v0.schema.json")).unwrap();
    let validator=jsonschema::options().with_draft(jsonschema::Draft::Draft202012).build(&schema).unwrap();
    let response=json!({"api_version":"st3.client.v0","request_id":"request/schedules","snapshot":snapshot,"value":value});
    let errors=validator.iter_errors(&response).map(|error|error.to_string()).collect::<Vec<_>>();
    assert!(errors.is_empty(),"{errors:#?}");
}

}
