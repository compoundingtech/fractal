//! Opt-in, real HTTP/Unix daemon measurement (not a router oneshot or mocked daemon).
//! Run this same test source on 98e550cb and on PR3-B, sequentially on one host:
//! ```sh
//! ST_ADMISSION_VARIANT=baseline cargo test --release -p fractal --test integration daemon_cost::assistant_admission::authenticated_get_during_pairing -- --exact --nocapture --test-threads=1
//! ST_ADMISSION_VARIANT=indexed cargo test --release -p fractal --test integration daemon_cost::assistant_admission::authenticated_get_during_pairing -- --exact --nocapture --test-threads=1
//! ```
//! Copy only this file and its `mod assistant_admission;` registration to the A worktree.
//! The baseline must actually be built from 98e550cb: dropping an index on PR3-B does not
//! restore A's admission query. Use distinct CARGO_TARGET_DIR paths for the two worktrees:
//! Cargo freshness can otherwise reuse the baseline's artifacts for the indexed checkout.
//! ST_ADMISSION_HISTORY (default 10000) is expired grants PER principal; ST_ADMISSION_SAMPLES
//! (default 200) is measured GET/completion pairs PER read mode. Setup and proof creation are
//! outside timing. Each pair starts behind a barrier, with no sleeps, polling, retries or timers.
//! ST_ADMISSION_REPORT optionally saves the complete JSON report. Hardware is observed
//! from uname/lscpu in the report, not inferred from an environment label.
//! The index DDL timing is a synthetic startup migration probe on the populated disposable
//! real store, not total daemon startup or cold-disk startup. Latencies use the real daemon's
//! bound sockets, bearer middleware, grant lookup, projection and durable approval consumption.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::{Instant, SystemTime, UNIX_EPOCH}};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use fractal_daemon::{api::{self, AppState}, client::Client, model::ClaimInput, store::Store};
use fractal_daemon::{AttentionListStore as _, EvidenceStore as _, SeatsStore as _};
use fractal_daemon::api::client_v0::assistant::authorization::{ReadPayload, request_digest, signing_bytes, HEADER};
use ring::{rand::SystemRandom, signature::{Ed25519KeyPair, KeyPair as _}};
use serde_json::{Value, json};
use tokio::sync::Barrier;

const OWNER: &str = "person/bench-operator";
const NODE: &str = "admission-bench";
const GRANTED: &str = "custom.client.assistant.v1.granted";
const INDEX: &str = "claims_assistant_v1_principal_index";
const CREATE_INDEX: &str = "CREATE INDEX claims_assistant_v1_principal_index ON claims(kind,COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt'))";
const A_QUERY: &str = "SELECT id FROM claims WHERE kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2";
const INDEXED_QUERY: &str =
    "SELECT id FROM claims WHERE kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2
     AND (?5 IS NULL OR NOT EXISTS (SELECT 1 FROM claims AS revoked WHERE revoked.subject=claims.subject AND revoked.kind=?3))
     AND (?5 IS NULL OR COALESCE(
         (SELECT CASE WHEN json_type(policy.body,'$.fields') IS NULL THEN 1 ELSE
             COALESCE(json_type(policy.body,'$.fields.expires_at_unix_ms')='integer'
                 AND json_extract(policy.body,'$.fields.expires_at_unix_ms')>?5,0) END
          FROM claims AS policy WHERE policy.subject=claims.subject AND policy.kind=?4 ORDER BY policy.store_index DESC LIMIT 1),
         CASE WHEN json_type(body,'$.fields') IS NULL THEN 1 ELSE
             COALESCE(json_type(body,'$.fields.expires_at_unix_ms')='integer'
                 AND json_extract(body,'$.fields.expires_at_unix_ms')>?5,0) END))";
const PATH: &str = "/v1/client/assistant/summary";

fn number(name: &str, default: usize) -> usize {
    let value = std::env::var(name).map_or(default, |s| s.parse().expect("positive integer setting"));
    assert!(value > 0, "{name} must be positive");
    value
}

fn value(response: Value) -> Value {
    response.get("value").cloned().unwrap_or(response)
}

fn key() -> Ed25519KeyPair {
    let encoded = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(encoded.as_ref()).unwrap()
}

async fn begin(owner: &Client, mode: &str) -> Value {
    value(owner.post("/v1/client/pairings", &json!({
        "api_version":"st3.client.v0", "device_name":"Admission measurement",
        "assistant_id":"assistant/chatgpt", "scopes":["read.assistant.summary","read.assistant.attention","read.assistant.missions","read.assistant.agents"],
        "expires_in_seconds":3600, "disclosure_consent":true,
        "consent_evidence":"Disposable synthetic benchmark owner consent", "minimal_read_mode":mode,
    })).await.unwrap())
}

async fn complete(http: &reqwest::Client, base: &str, challenge: &Value) -> Value {
    let id = challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/");
    let response = http.post(format!("{base}/v1/client/pairings/{id}/complete"))
        .json(&json!({"api_version":"st3.client.v0", "code":challenge["code"],
            "device_public_key":"BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc", "key_storage":"software"}))
        .send().await.unwrap();
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert!(status.is_success(), "completion {status}: {body}");
    value(body)
}

fn proof(paired: &Value, signer: &Ed25519KeyPair, sequence: usize) -> String {
    let parameters = BTreeMap::new();
    let payload = ReadPayload {
        issuer:"scg/admission-bench".into(), identity:"chatgpt".into(), assistant_id:"assistant/chatgpt".into(),
        grant_id:paired["grant"]["id"].as_str().unwrap().into(),
        policy_revision:paired["grant"]["policy_revision"].as_u64().unwrap(), key_id:"approval-key/admission-bench".into(),
        approval_id:format!("approval/admission-{sequence}"), execution_id:format!("execution/admission-{sequence}"),
        projection:"summary".into(), max_response_bytes:4096,
        request_digest:request_digest(PATH, &parameters).unwrap(), parameters,
        expires_at_unix_ms:u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap() + 60_000,
        nonce:uuid::Uuid::new_v4().to_string(),
    };
    let signature = URL_SAFE_NO_PAD.encode(signer.sign(signing_bytes(&payload).unwrap().as_bytes()).as_ref());
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"payload":payload,"signature":signature})).unwrap())
}

fn explain(connection: &rusqlite::Connection, query: &str) -> Vec<String> {
    let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap();
    let mut rows = if query == A_QUERY {
        statement.query(rusqlite::params![GRANTED, "assistant/chatgpt"]).unwrap()
    } else {
        statement.query(rusqlite::params![GRANTED, "assistant/chatgpt", "custom.client.assistant.v1.revoked",
            "custom.client.assistant.v1.policy",
            u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap()]).unwrap()
    };
    let mut plan = Vec::new();
    while let Some(row) = rows.next().unwrap() { plan.push(row.get::<_, String>(3).unwrap()); }
    plan
}

fn p95(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[(samples.len() * 95).div_ceil(100) - 1]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn authenticated_get_during_pairing() {
    let Ok(variant) = std::env::var("ST_ADMISSION_VARIANT") else { return };
    assert!(matches!(variant.as_str(), "baseline" | "indexed"));
    assert!(fractal_daemon::test_support::optimized_build(), "use --release");
    let indexed = variant == "indexed";
    let history = number("ST_ADMISSION_HISTORY", 10_000);
    let samples = number("ST_ADMISSION_SAMPLES", 200);
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("admission.sqlite");
    let store = Arc::new(Store::open(&database, NODE).unwrap());
    store.bind_fleet(super::super::daemon_bench::FLEET).unwrap();
    store.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
    let state = AppState {
        browser_pairing_login:None,
        seat_admission:Default::default(), owning_person:Some(OWNER.into()), store:store.clone(),
        notify:Arc::new(tokio::sync::Notify::new()), event_notify:tokio::sync::watch::channel(0).0,
        node:NODE.into(), api_socket: root.path().join("api.sock"),state_dir:root.path().into(), pty_root:root.path().join("pty"), pty_binary:PathBuf::from("pty"),
        fleet_id:None, configured_peers:vec![], client_relay:None, native_session_home:None,
        planner_default:Default::default(), harness_question_sink:None, assistant_receipts:Default::default(), private_notes:Default::default(),
    };
    api::start_operation_report(&state);
    // Match run_up/daemon_load: the generic lists do not own the independent
    // attention/summary and agent-roster publication workers.
    let mut roster_publications = store.subscribe_agent_roster();
    let mut attention_publications = store.subscribe_attention_list();
    api::start_agent_roster(&state);
    api::start_attention_list(&state);
    api::start_published_lists(&state);
    // Observe the workers' real publication watches, not an HTTP retry or a
    // synchronous test-only fold. Subscribe before starting to avoid lost wakes.
    roster_publications.wait_for(|_| store.published_agent_roster(u64::MAX, false).is_some())
        .await.expect("the daemon publishes its initial agent roster");
    attention_publications.wait_for(|_| store.published_attention_list().is_some())
        .await.expect("the daemon publishes its initial attention view");
    let socket = root.path().join("owner.sock");
    let reserved = api::reserve_unix_socket(&socket, None).await.unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let local_app = api::router(state.clone());
    let local = tokio::spawn(async move {
        api::serve_reserved_unix_with_ready(reserved, local_app, None, None, || { ready_tx.send(()).unwrap(); }).await.unwrap();
    });
    ready_rx.await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fabric_app = api::fabric_router(state.clone());
    let fabric = tokio::spawn(async move { axum::serve(listener, fabric_app).await.unwrap(); });
    let owner = Client::unix_as(&socket, OWNER).unwrap();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let signer = key();
    let mut sessions = Vec::new();
    for mode in ["approval-once", "caller-bound-allow"] {
        let mut paired = complete(&http, &base, &begin(&owner, mode).await).await;
        if mode == "approval-once" {
            let id = paired["grant"]["id"].as_str().unwrap().trim_start_matches("assistant-grant/");
            let trust = value(owner.post(&format!("/v1/client/assistant/grants/{id}/read-trust"), &json!({
                "api_version":"st3.client.v0", "issuer":"scg/admission-bench", "key_id":"approval-key/admission-bench",
                "attestor_public_key":URL_SAFE_NO_PAD.encode(signer.public_key().as_ref()),
                "shim_public_key":URL_SAFE_NO_PAD.encode(key().public_key().as_ref()),
                "default_response_bytes":4096, "max_response_bytes":16_384,
            })).await.unwrap());
            paired["grant"] = trust;
        }
        sessions.push((mode, paired));
    }
    // Synthetic expired grants retain valid production field shapes, native Store envelopes,
    // origin/actor and per-subject indexes. They are never service-profile fixtures and never
    // authorize a request. Grant history is deliberately nonempty for BOTH principals.
    let template_subject = format!("custom/client/assistant-v1-{}", sessions[0].1["grant"]["id"].as_str().unwrap().trim_start_matches("assistant-grant/"));
    let template = store.claims_for_subject_kind_at(&template_subject, GRANTED, None, true, 1).unwrap().claims.pop().unwrap();
    let fixture_store = store.clone();
    tokio::task::spawn_blocking(move || {
        for principal in ["assistant/chatgpt", "assistant/mcp"] {
            for row in 0..history {
                let id = format!("history-{}-{row}", principal.trim_start_matches("assistant/"));
                let mut fields = template.body["fields"].clone();
                fields["principal"] = json!(principal);
                fields["grant_id"] = json!(format!("assistant-grant/{id}"));
                fields["session_actor"] = json!(format!("{principal}/session/assistant-grant/{id}"));
                fields["expires_at_unix_ms"] = json!(1);
                fields["credential_hash"] = json!(hex::encode(ring::digest::digest(&ring::digest::SHA256, id.as_bytes()).as_ref()));
                fixture_store.append_claim(&ClaimInput { subject:format!("custom/client/assistant-v1-{id}"), kind:GRANTED.into(),
                    actor:Some(OWNER.into()), fields:serde_json::from_value(fields).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None }).unwrap();
            }
        }
    }).await.unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    let query = if indexed { INDEXED_QUERY } else { A_QUERY };
    let index_exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)", [INDEX], |r| r.get(0)).unwrap();
    assert_eq!(index_exists, indexed, "build baseline on 98e550cb; indexed on PR3-B");
    let (before, after, build_ms) = if indexed {
        connection.execute_batch("DROP INDEX claims_assistant_v1_principal_index").unwrap();
        let before = explain(&connection, query);
        let started = Instant::now();
        connection.execute_batch(CREATE_INDEX).unwrap();
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let after = explain(&connection, query);
        assert!(after.iter().any(|line| line.contains(INDEX)), "{after:?}");
        (before, after, Some(elapsed))
    } else { (explain(&connection, query), Vec::new(), None) };
    let total_rows: u64 = connection.query_row("SELECT count(*) FROM claims", [], |r| r.get(0)).unwrap();
    let mut principal_rows = BTreeMap::new();
    for principal in ["assistant/chatgpt", "assistant/mcp"] {
        let count: u64 = connection.query_row("SELECT count(*) FROM claims WHERE kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2", rusqlite::params![GRANTED,principal], |r| r.get(0)).unwrap();
        principal_rows.insert(principal, count);
    }
    drop(connection);
    api::wait_published_lists_at_least(&store, store.index().unwrap()).await.unwrap();
    let mut measurements = Vec::new();
    for (mode, paired) in sessions {
        let credential = paired["credential"].as_str().unwrap();
        let mut get_ms = Vec::with_capacity(samples);
        let mut completion_ms = Vec::with_capacity(samples);
        // One unmeasured warm GET, with a distinct one-use proof.
        let warm = http.get(format!("{base}{PATH}")).bearer_auth(credential);
        let warm = if mode == "approval-once" { warm.header(HEADER, proof(&paired, &signer, samples + 1)) } else { warm };
        let response = warm.send().await.unwrap();
        assert!(response.status().is_success(), "warm GET: {}", response.text().await.unwrap());
        for sequence in 0..samples {
            let challenge = begin(&owner, mode).await;
            let get = http.get(format!("{base}{PATH}")).bearer_auth(credential);
            let get = if mode == "approval-once" { get.header(HEADER, proof(&paired, &signer, sequence)) } else { get };
            let barrier = Barrier::new(2);
            let read = async {
                barrier.wait().await;
                let started = Instant::now();
                let response = get.send().await.unwrap();
                let status = response.status();
                let body = response.bytes().await.unwrap();
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                assert!(status.is_success(), "GET {status}: {}", String::from_utf8_lossy(&body));
                elapsed
            };
            let pairing = async {
                barrier.wait().await;
                let started = Instant::now();
                let completed = complete(&http, &base, &challenge).await;
                (started.elapsed().as_secs_f64() * 1000.0, completed)
            };
            let (read_ms, (pair_ms, completed)) = tokio::join!(read, pairing);
            get_ms.push(read_ms);
            completion_ms.push(pair_ms);
            // Keep live-grant count stable; revocation is owner HTTP, outside both samples.
            let id = completed["grant"]["id"].as_str().unwrap().trim_start_matches("assistant-grant/");
            let _: Value = owner.post(&format!("/v1/client/assistant/grants/{id}/revoke"), &json!({})).await.unwrap();
        }
        state.assistant_receipts.flush().await.unwrap();
        let get_p95 = p95(&mut get_ms);
        let completion_p95 = p95(&mut completion_ms);
        measurements.push(json!({"mode":mode,"get_samples":get_ms.len(),"completion_samples":completion_ms.len(),
            "get_p95_ms":get_p95,"completion_p95_ms":completion_p95,"q39_budget_ms":100,"q39_pass":get_p95 <= 100.0,
            "get_ms":get_ms,"completion_ms":completion_ms}));
    }
    let hardware = |program: &str, args: &[&str]| {
        let output = std::process::Command::new(program).args(args).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    let final_connection = rusqlite::Connection::open(&database).unwrap();
    let final_rows: u64 = final_connection.query_row("SELECT count(*) FROM claims", [], |r| r.get(0)).unwrap();
    let report = json!({"variant":variant,"baseline_revision":"98e550cb75", "hardware_uname":hardware("uname", &["-a"]),
        "hardware_lscpu":hardware("lscpu", &["-J"]),"hardware_hostname":hardware("hostname", &[]),
        "expected_host":"same otherwise-idle host for both variants (verify observed hardware)",
        "history_grants_per_principal":history,"initial_total_claim_rows":total_rows,"initial_granted_rows_by_principal":principal_rows,
        "final_total_claim_rows":final_rows,"admission_query":query,
        "explain_without_index":before,"explain_with_index":after,"synthetic_startup_index_build_ms":build_ms,
        "measurements":measurements});
    println!("ADMISSION_MEASUREMENT {}", serde_json::to_string_pretty(&report).unwrap());
    if let Some(path) = std::env::var_os("ST_ADMISSION_REPORT") { std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap(); }
    local.abort();
    fabric.abort();
    if indexed {
        assert!(report["measurements"].as_array().unwrap().iter().all(|row| row["q39_pass"] == true), "q39: authenticated GET p95 must be <=100ms; see report");
    }
}

/// Synthetic large-store index build, independent of the HTTP latency fixture:
/// ```sh
/// ST_ADMISSION_INDEX_BUILD=1 cargo test --release -p fractal --test integration daemon_cost::assistant_admission::synthetic_large_store_index_build -- --exact --nocapture --test-threads=1
/// ```
/// ST_ADMISSION_INDEX_ROWS defaults to 1,000,000 total claims. ST_ADMISSION_INDEX_REPORT
/// optionally saves JSON. Run on the indexed branch. This measures the exact startup index
/// CREATE DDL, not full Store::open, cold-disk startup, admission, or signature verification.
#[test]
fn synthetic_large_store_index_build() {
    if std::env::var_os("ST_ADMISSION_INDEX_BUILD").is_none() { return; }
    assert!(fractal_daemon::test_support::optimized_build(), "use --release");
    let rows = number("ST_ADMISSION_INDEX_ROWS", 1_000_000);
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("synthetic-index.sqlite");
    let store = Store::open(&database, NODE).unwrap();
    // Obtain a real Store-authored envelope and the real migrated schema. The SQL copies
    // below are intentionally synthetic, not independently signed or admission-authoritative.
    // Never reopen this fixture as a source of replicated/authenticated claims.
    let template = store.append_claim(&ClaimInput {
        subject:"custom/admission-index/template".into(),
        kind:"custom.admission-index.fixture".into(), actor:Some(OWNER.into()),
        fields:serde_json::from_value(json!({"principal":"assistant/chatgpt",
            "expires_at_unix_ms":1,"fixture_payload":"synthetic large-store index build only"})).unwrap(),
        evidence:vec![], expected_subject:None, idempotency_key:None,
    }).unwrap();
    drop(store);
    let mut connection = rusqlite::Connection::open(&database).unwrap();
    // Native claims triggers retain their real digest implementation during the
    // synthetic population, just as on production/checkpoint writer connections.
    smallclaims::store::configure_projection_writer(&connection).unwrap();
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
        [INDEX], |row| row.get(0),
    ).unwrap();
    assert!(installed, "run the large-store index probe on the indexed branch");
    connection.execute_batch("DROP INDEX claims_assistant_v1_principal_index").unwrap();
    // Copy every native required column rather than depending on a second claims schema.
    // The native autoincrement store_index, claim ID and subject must remain unique.
    let columns = {
        let mut statement = connection.prepare("PRAGMA table_info(claims)").unwrap();
        statement.query_map([], |row| row.get::<_, String>(1)).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap()
    };
    let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    let copied = columns.iter().filter(|name| name.as_str() != "store_index").collect::<Vec<_>>();
    let destinations = copied.iter().map(|name| quote(name)).collect::<Vec<_>>().join(",");
    let expressions = copied.iter().map(|name| match name.as_str() {
        "id" => "'synthetic-index-claim/' || sequence.n".to_owned(),
        "subject" => "'custom/admission-index/row-' || sequence.n".to_owned(),
        "kind" => format!("'{GRANTED}'"),
        "body" => "json_set(template.body,'$.subject','custom/admission-index/row-' || sequence.n,\
            '$.kind','custom.client.assistant.v1.granted','$.fields.principal',\
            CASE WHEN sequence.n % 2=0 THEN 'assistant/chatgpt' ELSE 'assistant/mcp' END)".to_owned(),
        _ => format!("template.{}", quote(name)),
    }).collect::<Vec<_>>().join(",");
    let transaction = connection.transaction().unwrap();
    if rows > 1 {
        transaction.execute(&format!(
            "WITH RECURSIVE sequence(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM sequence WHERE n<?1)
             INSERT INTO claims({destinations}) SELECT {expressions}
             FROM sequence CROSS JOIN claims AS template WHERE template.id=?2"
        ), rusqlite::params![u64::try_from(rows - 1).unwrap(),template.id]).unwrap();
    }
    transaction.commit().unwrap();
    let actual_rows: u64 = connection.query_row("SELECT count(*) FROM claims", [], |row| row.get(0)).unwrap();
    assert_eq!(actual_rows, u64::try_from(rows).unwrap());
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let before = explain(&connection, INDEXED_QUERY);
    let main_bytes_before = std::fs::metadata(&database).unwrap().len();
    let started = Instant::now();
    connection.execute_batch(CREATE_INDEX).unwrap();
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after = explain(&connection, INDEXED_QUERY);
    assert!(after.iter().any(|line| line.contains(INDEX)), "{after:?}");
    let pages: u64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0)).unwrap();
    let page_size: u64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0)).unwrap();
    let wal_path = database.with_extension("sqlite-wal");
    let wal_bytes = std::fs::metadata(&wal_path).map_or(0, |metadata| metadata.len());
    let hardware = |program: &str, args: &[&str]| {
        let output = std::process::Command::new(program).args(args).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    let report = json!({
        "measurement":"synthetic startup index CREATE DDL only; not full startup",
        "claims_rows":actual_rows,"synthetic_copied_rows":rows - 1,
        "fixture_authority":"one native template; SQL copies are not signature-authoritative",
        "index_name":INDEX,"index_create_sql":CREATE_INDEX,"index_build_ms":elapsed_ms,
        "main_db_bytes_before":main_bytes_before,
        "main_db_bytes_after":std::fs::metadata(&database).unwrap().len(),
        "wal_bytes_after":wal_bytes,"logical_db_bytes_after":pages * page_size,
        "explain_without_index":before,"explain_with_index":after,
        "hardware_uname":hardware("uname", &["-a"]),
        "hardware_hostname":hardware("hostname", &[]),"hardware_lscpu":hardware("lscpu", &["-J"]),
        "expected_host":"same otherwise-idle host for both variants (verify observed hardware)",
    });
    println!("ADMISSION_INDEX_BUILD {}", serde_json::to_string_pretty(&report).unwrap());
    if let Some(path) = std::env::var_os("ST_ADMISSION_INDEX_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
