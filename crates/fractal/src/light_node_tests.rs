//! A real Bun program effect crosses the guarded native Unix boundary and authenticated
//! outbound session. No substitute issuer, request verifier, graph effect or executor.
use super::*;
use std::io::Write as _;
use fractal_daemon::{MissionsStore as _, parse_intent};
use fractal_daemon::api::{AppState, reserve_unix_socket, router, serve_reserved_unix_with_ready};
use fractal_daemon::model::{IntentInput, MissionRunRequest};
use fractal_daemon::reconcile::{NativeRuntime, program::JOURNAL_KIND};
use fractal_daemon::store::Store;
use fractal_daemon::light_node::journal::{OperationKind, Scope};
use tokio::sync::{Notify, oneshot};

const PROGRAM: &str = r#"
export default {
  [Symbol.for('fractal.program.v1')]: async (ctx) => {
    const secret = (await ctx.spawn(['sh', '-c', 'printf %s "$ST_RUN_CREDENTIAL"'])).stdout
    await ctx.record({ key: 'credential', status: 'completed', value: secret, ms: 0 })
    const script = `
      const claim = {subject:'custom/example/session-effect',kind:'custom.example.recorded',actor:null,
        fields:{value:'actual-program-effect'},evidence:[],expected_subject:null,idempotency_key:null};
      const post = async (request_id, claim) => {
        const response = await fetch('http://localhost/v1/internal/native/run-request/claim', {
          unix:process.env.TEST_SOCKET,method:'POST',headers:{'content-type':'application/json'},
          body:JSON.stringify({request_id,claim})
        });
        return {ok:response.ok,body:await response.json()};
      };
      const first = await post('program-session-effect',claim);
      const replay = await post('program-session-effect',claim);
      if (!first.ok || !replay.ok || JSON.stringify(first.body) !== JSON.stringify(replay.body))
        throw new Error('retry changed its receipt');
      const blocked = await post('program-capacity-probe',{...claim,subject:'custom/example/capacity-effect'});
      if (blocked.ok || !JSON.stringify(blocked.body).includes('backpressure'))
        throw new Error('unacknowledged result did not retain bounded custody');
      console.log(JSON.stringify(first.body));
    `
    const result = await ctx.spawn(['bun', '-e', script], {env:{TEST_SOCKET:ctx.payload.socket}})
    // Acceptance is durable before any ACK is possible. The fixture verifies retained
    // custody here, then releases the real FIFO to permit the explicit acknowledgement.
    await ctx.record({ key: 'forwarded', status: 'completed', value: result.stdout, ms: 0 })
    await ctx.spawn(['sh', '-c', 'read line < "$TEST_GATE"'], {env:{TEST_GATE:ctx.payload.gate}})
    const acknowledge = `
      const claim = {subject:'custom/example/session-effect',kind:'custom.example.recorded',actor:null,
        fields:{value:'actual-program-effect'},evidence:[],expected_subject:null,idempotency_key:null};
      const accepted = JSON.parse(process.env.TEST_ACCEPTED);
      const post = async (path, body) => {
        const response = await fetch('http://localhost'+path,{unix:process.env.TEST_SOCKET,
          method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});
        return {ok:response.ok,body:await response.json()};
      };
      const ack = (claim) => post('/v1/internal/native/run-request/acknowledge',{
        request:{request_id:'program-session-effect',claim},
        accepted:{request_id:'program-session-effect',result:{Ok:accepted}}});
      if ((await ack({...claim,fields:{value:'not-the-original-request'}})).ok)
        throw new Error('ACK admitted changed immutable request contents');
      const first = await ack(claim); const retry = await ack(claim);
      if (!first.ok || !retry.ok || JSON.stringify(first.body) !== JSON.stringify(retry.body))
        throw new Error('explicit ACK was not retryable');
      const expired = await post('/v1/internal/native/run-request/claim',{request_id:'program-session-effect',claim});
      if (expired.ok || !JSON.stringify(expired.body).includes('result-expired'))
        throw new Error('retired request identity was executed again');
      const probe = await post('/v1/internal/native/run-request/claim',{
        request_id:'program-capacity-probe',claim:{...claim,subject:'custom/example/capacity-effect'}});
      if (!probe.ok) throw new Error('explicit ACK did not release bounded request capacity');
      console.log(JSON.stringify(probe.body));
    `
    const acknowledged = await ctx.spawn(['bun','-e',acknowledge],
      {env:{TEST_SOCKET:ctx.payload.socket,TEST_ACCEPTED:result.stdout}})
    await ctx.record({ key: 'acknowledged', status: 'completed', value: acknowledged.stdout, ms: 0 })
    const finish = `
      const claim = {subject:'custom/example/capacity-effect',kind:'custom.example.recorded',actor:null,
        fields:{value:'actual-program-effect'},evidence:[],expected_subject:null,idempotency_key:null};
      const response = await fetch('http://localhost/v1/internal/native/run-request/acknowledge',{
        unix:process.env.TEST_SOCKET,method:'POST',headers:{'content-type':'application/json'},
        body:JSON.stringify({request:{request_id:'program-capacity-probe',claim},
          accepted:{request_id:'program-capacity-probe',result:{Ok:JSON.parse(process.env.TEST_ACCEPTED)}}})});
      if (!response.ok) throw new Error(JSON.stringify(await response.json()));
    `
    await ctx.spawn(['bun','-e',finish],{env:{TEST_SOCKET:ctx.payload.socket,TEST_ACCEPTED:acknowledged.stdout}})
    await ctx.record({ key: 'capacity-released', status: 'completed', value: true, ms: 0 })
    await ctx.spawn(['sh', '-c', 'read line < "$TEST_GATE"'], {env:{TEST_GATE:ctx.payload.gate}})
    return {_tag:'Success',value:true}
  }
}
"#;

struct Tasks(Vec<tokio::task::JoinHandle<()>>);
impl Drop for Tasks { fn drop(&mut self) { for task in &self.0 { task.abort(); } } }

async fn until(reconciler: &Reconciler, notify: &Notify, done: impl Fn() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let changed = notify.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            reconciler.reconcile_once().unwrap();
            if done() { return; }
            changed.await;
        }
    }).await.expect("actual program/session event must wake the reconciler");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn program_waits_for_home_ack_forwards_one_real_effect_and_revokes_on_end() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("seat-graph.sqlite3");
    let store = Arc::new(Store::open(&database, "example-seat").unwrap());
    let source_file = root.path().join("program.mjs");
    std::fs::write(&source_file, PROGRAM).unwrap();
    let hash = store.put_blob(PROGRAM.as_bytes()).unwrap();
    let source = format!(r#"version 2
mission "program-session" state="ready" {{
  goal "Forward exactly one authorized program effect."
  input "socket" kind="text"
  input "gate" kind="text"
  step "program" {{ program "{}" sha256="{hash}" }}
}}"#, source_file.display());
    let intent = parse_intent(&source, "example-seat").unwrap();
    let plan = store.mission(&intent, IntentInput { kdl: source, source_name: None }).unwrap();
    store.apply(&intent, &plan.subject_tokens, "program-session-definition").unwrap();
    let socket = root.path().join("native.sock");
    let gate = root.path().join("gate");
    use std::os::unix::ffi::OsStrExt as _;
    let gate_name = std::ffi::CString::new(gate.as_os_str().as_bytes()).unwrap();
    // SAFETY: the NUL-terminated path remains alive and names only this fixture's FIFO.
    assert_eq!(unsafe { libc::mkfifo(gate_name.as_ptr(), 0o600) }, 0);
    let mut gate_writer = OpenOptions::new().read(true).write(true).open(&gate).unwrap();
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "program-session".into(), revision: None, workspace: root.path().display().to_string(),
        requester: None, mode: None,
        inputs: BTreeMap::from([("socket".into(), socket.display().to_string()), ("gate".into(), gate.display().to_string())]),
        idempotency_key: "program-session-run".into(),
    }).unwrap();
    let step = run.steps.iter().find(|step| step.step == "program").unwrap().subject.clone();
    let home_key = Arc::new(MemberKey::generate().unwrap().0);
    let seat_key = Arc::new(MemberKey::generate().unwrap().0);
    let selection = Selection { home: "example-home".into(), home_key: home_key.public().into(),
        host: "example-seat".into(), host_key: seat_key.public().into(), epoch: 1,
        grant: Scope { subjects: vec![step.clone()], operations: vec![OperationKind::Observe, OperationKind::Stop] } };
    let journal = Journal::open(&root.path().join("seat-session.sqlite3"),
        Limits { max_commands: 1, ..Limits::default() }).unwrap();
    let home_journal = Journal::open(&root.path().join("home-session.sqlite3"), Limits::default()).unwrap();
    journal.install_epoch(&selection).unwrap(); home_journal.install_epoch(&selection).unwrap();
    let notify = Arc::new(Notify::new());
    let runtime = Arc::new(NativeRuntime::new(root.path(), Some(&root.path().join("pty")), Path::new("unused-pty")));
    let reconciler = Arc::new(Reconciler::new(store.clone(), runtime, selection.host.clone(), notify.clone()));
    reconciler.install_home_session_journal(journal.clone()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}{}", listener.local_addr().unwrap(), fractal_daemon::config::LIGHT_NODE_SESSION_PATH);
    let seat_auth = FleetAuth::test("program-session-tests", &[7; 32]).with_member_key(Some(seat_key.clone()));
    let client = SeatClient::new(journal.clone(), seat_auth, selection.host.clone(), endpoint, Arc::new(NativeSessionExecutor::new(reconciler.clone()))).unwrap();
    let authority = Arc::new(NativeRunAuthority { journal: journal.clone(), selection: selection.clone(), key: seat_key, requester: client.requester() });
    reconciler.install_program_run_authority(authority.clone()).unwrap();
    reconciler.reconcile_once().unwrap();
    assert!(store.claims_for(&step, Some(JOURNAL_KIND)).unwrap().is_empty(), "no program effect or trace may run before its own admission ACK");
    assert!(journal.reported_executions().unwrap().iter().any(|binding| binding.run.as_ref().is_some_and(|bound| bound.step_run == step)));
    // A real full-node graph snapshot provides the exact canonical run/attempt revisions;
    // the two session journals, host signing keys and effect writer remain separate.
    let home_database = root.path().join("home-graph.sqlite3");
    rusqlite::Connection::open(&database).unwrap().execute(/* sqlite-large-value: allow-for-now lv-h76fffc4c4dab owner=fractal::light_node_tests TODO=trace-statement-and-value-representation */ "VACUUM INTO ?1", [home_database.to_str().unwrap()]).unwrap();
    let graph = Arc::new(Store::open(&home_database, selection.home.clone()).unwrap());
    let home_auth = FleetAuth::test("program-session-tests", &[7; 32]).with_member_key(Some(home_key));
    let home = Home::new(vec![home_journal], home_auth, selection.home.clone(), Arc::new(StoreRunEffects::new(graph.clone()))).unwrap();
    let mut tasks = Tasks(Vec::new());
    let app = home.router();
    tasks.0.push(tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); }));
    let state = AppState {
        owning_person: None, browser_pairing_login: None, store: store.clone(), seat_admission: Arc::default(),
        notify: notify.clone(), event_notify: watch::channel(0).0, node: selection.host.clone(),
        api_socket: socket.clone(), state_dir: root.path().into(), pty_root: root.path().join("pty"),
        pty_binary: Path::new("unused-pty").into(), fleet_id: None, configured_peers: Default::default(),
        client_relay: None, native_session_home: None, planner_default: Default::default(), harness_question_sink: None,
        assistant_receipts: Default::default(), private_notes: Arc::new(fractal_daemon::private_notes::Authority { catalogs: Default::default() }),
    };
    let reserved = reserve_unix_socket(&socket, None).await.unwrap();
    let (ready, ready_rx) = oneshot::channel();
    let app = router(state);
    let bound_store = store.clone();
    let bound_authority = authority.clone();
    tasks.0.push(tokio::spawn(async move {
        serve_reserved_unix_with_ready(reserved, app, Some(bound_store), None, Some(bound_authority), || { let _ = ready.send(()); }).await.unwrap();
    }));
    ready_rx.await.unwrap();
    let (stop, stopped) = watch::channel(false);
    tasks.0.push(tokio::spawn(async move { client.run(stopped).await.unwrap(); }));
    let changed_journal = journal.clone();
    let changed_reconciler = reconciler.clone();
    tasks.0.push(tokio::spawn(async move {
        loop {
            let changed = changed_journal.authority_changed.notified();
            tokio::pin!(changed); changed.as_mut().enable();
            changed_reconciler.session_authority_changed().unwrap();
            changed.await;
        }
    }));
    until(&reconciler, &notify, || store.claims_for(&step, Some(JOURNAL_KIND)).unwrap().iter()
        .any(|claim| claim.body["fields"]["key"] == "forwarded" && claim.body["fields"]["status"] == "completed")).await;
    assert!(journal.run_result("program-session-effect").unwrap().is_some(),
        "sending the HTTP response and durably accepting it must not implicitly acknowledge custody");
    writeln!(gate_writer, "acknowledge").unwrap();
    until(&reconciler, &notify, || store.claims_for(&step, Some(JOURNAL_KIND)).unwrap().iter()
        .any(|claim| claim.body["fields"]["key"] == "capacity-released" && claim.body["fields"]["status"] == "completed")).await;
    assert!(journal.run_result("program-session-effect").unwrap_err().to_string().contains("result-expired"));
    assert!(journal.run_result("program-capacity-probe").unwrap_err().to_string().contains("result-expired"));
    assert_eq!(graph.claims_for("custom/example/capacity-effect", Some("custom.example.recorded")).unwrap().len(), 1);
    let effects = graph.claims_for("custom/example/session-effect", Some("custom.example.recorded")).unwrap();
    assert_eq!(effects.len(), 1, "same signed request identity must recover its real effect receipt");
    assert_eq!(effects[0].actor.as_deref(), Some(run.subject.as_str()));
    let credential = store.claims_for(&step, Some(JOURNAL_KIND)).unwrap().into_iter()
        .find(|claim| claim.body["fields"]["key"] == "credential").unwrap().body["fields"]["value"].as_str().unwrap().to_owned();
    assert_eq!(authority.resolve(&credential).unwrap().run.unwrap().step_run, step);
    writeln!(gate_writer, "complete").unwrap();
    until(&reconciler, &notify, || store.step_run(&step).unwrap().unwrap().status == "completed").await;
    assert!(authority.resolve(&credential).is_err(), "native end revokes permission before its VM is forgotten");
    stop.send_replace(true);
    home.shutdown();
}
