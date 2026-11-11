//! Opt-in, real HTTP/Unix daemon measurement (not a router oneshot or mocked daemon).
//! Run this same source sequentially on each revision, from that worktree root.
//! Bind the build to its source revision and select the executable from Cargo JSON,
//! NEVER by searching a shared target directory for `integration-*` (stale head/base
//! binaries can coexist). `baseline` is a scenario label, NOT executable identity.
//! ```sh
//! export ST_ADMISSION_SOURCE_ROOT="$PWD"
//! export ST_ADMISSION_EXPECT_REVISION="$(git rev-parse HEAD)"
//! export ST_ADMISSION_BUILD_REVISION="$ST_ADMISSION_EXPECT_REVISION"
//! export ST_ADMISSION_BUILD_PROVENANCE="git worktree; harness backport only; cargo command below"
//! export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/tmp/867-admission/target}"
//! mkdir -p "$PWD/tmp/867-admission"
//! cargo test --release -p fractal --test integration --no-run --message-format=json > "$PWD/tmp/867-admission/build.jsonl"
//! export ST_ADMISSION_BINARY="$(jq -rs '[.[] | select(.reason == "compiler-artifact" and .target.name == "integration" and .profile.test and .executable != null) | .executable] | unique | if length == 1 then .[0] else error("expected one integration test executable") end' "$PWD/tmp/867-admission/build.jsonl")"
//! # Like-for-like counts: run N=2 AND N=5 at BOTH revisions; fresh profiles.
//! # Set variant=baseline on base, indexed on head. Do not reuse an old binary.
//! for n in 2 5; do
//!   ST3_PROFILE_DIR="$PWD/tmp/867-admission/$ST_ADMISSION_EXPECT_REVISION-n$n-profile" ST3_PROFILE_SLOW_MS=0 ST_ADMISSION_VARIANT=indexed ST_ADMISSION_LOCAL_ACTIVE="$n" ST_ADMISSION_READ_COUNTS_ONLY=1 ST_ADMISSION_WRONG_ORIGIN_OVERLAYS=0 ST_ADMISSION_FOREIGN_ACTIVE=0 ST_ADMISSION_REPORT="$PWD/tmp/867-admission/$ST_ADMISSION_EXPECT_REVISION-n$n.json" "$ST_ADMISSION_BINARY" daemon_cost::assistant_admission::authenticated_get_during_pairing --exact --nocapture --test-threads=1
//! done
//! # Head only, full extended q39: 2 local + 3 authenticated foreign grants,
//! # 10000 expired grants/principal, 3 renewals each, 1000 overlays/active grant,
//! # 200 concurrent GET/completion pairs/mode; do NOT set READ_COUNTS_ONLY.
//! ST3_PROFILE_DIR="$PWD/tmp/867-admission/$ST_ADMISSION_EXPECT_REVISION-q39-profile" ST3_PROFILE_SLOW_MS=0 ST_ADMISSION_VARIANT=indexed ST_ADMISSION_LOCAL_ACTIVE=2 ST_ADMISSION_HISTORY=10000 ST_ADMISSION_SAMPLES=200 ST_ADMISSION_FOREIGN_ACTIVE=3 ST_ADMISSION_WRONG_ORIGIN_OVERLAYS=1000 ST_ADMISSION_REPORT="$PWD/tmp/867-admission/$ST_ADMISSION_EXPECT_REVISION-q39.json" "$ST_ADMISSION_BINARY" daemon_cost::assistant_admission::authenticated_get_during_pairing --exact --nocapture --test-threads=1
//! ```
//! Backport to base d477c67ad1: copy this file, add `mod assistant_admission;` to
//! crates/fractal/tests/daemon_cost.rs, and replace ONLY `revision_api`'s body
//! with the documented base body below. Build there using the commands above;
//! change VARIANT to baseline in the counts loop. Base supports local N=2/5,
//! but requires READ_COUNTS_ONLY=1, FOREIGN_ACTIVE=0, WRONG_ORIGIN_OVERLAYS=0.
//! It has no provenance API and does not ignore foreign overlays or expose an indexed probe.
//! For a git archive (no .git), set ST_ADMISSION_SOURCE_REVISION to the archived
//! full commit ID and ST_ADMISSION_SOURCE_PROVENANCE to the exact archive command
//! and source/build evidence location; retain the build env above and set EXPECT
//! to that ID explicitly instead of git rev-parse. These are declared provenance,
//! not a claim that a reused binary was built from that archive. The executable's
//! embedded revision MUST match EXPECT; its canonical path and SHA256 are reported.
//! A worktree report records actual `git rev-parse HEAD`; archives record null for
//! that observation and separately record the explicit source revision/evidence.
//! Counts are observations, not expectations: rerun true base N=2 to establish or
//! explain the previously cited 4/12/16 (auth/caller-bound/approval-once) counts.
//! Old base 8/8/9 with identical head SQL is not evidence of base behavior without
//! executable identity. Compare direct auth separately from GET: receipt/proof work
//! and the number of live grants differ. Never relabel an existing report as base.
//! Prior run: `git archive d477c67ad1` into a base directory, harness-only
//! backport, then `cargo test --release -p fractal --test integration ...` in a
//! shared target directory previously used by head. The retained report
//! /var/tmp/867c-evidence/fix/base-counts-1.json lacked executable identity.
//! This fixture serves routers IN PROCESS: daemon_bench's standalone binary
//! discovery cannot explain its direct-auth measurement. A reused integration
//! executable/dependency artifact is a hypothesis, not an established cause.
//! Source inspection of d477c67ad1 authenticate/session implies four acquisitions
//! (digest lookup, claim_by_id, POLICY, REVOKED), independent of N. GET 12/16 must
//! be remeasured with the same route, proof, receipt flush and profiling scope;
//! they are not asserted as expected counts by this fixture. Retain reports and
//! build JSON to distinguish executable reuse from instrumentation/scope differences.
//! ST_ADMISSION_HISTORY (default 10000) is expired grants PER principal; ST_ADMISSION_SAMPLES
//! (default 200) is measured GET/completion pairs PER read mode. Setup and proof creation are
//! outside timing. Each pair starts behind a barrier, with no sleeps, polling, retries or timers.
//! ST_ADMISSION_FOREIGN_ACTIVE optionally adds verified same-profile active foreign grants
//! (default 0; use 3 for the provenance-check scenario). ST_ADMISSION_REPORT optionally
//! saves the complete JSON report. Hardware is observed from uname/lscpu, not inferred.
//! ST_ADMISSION_LOCAL_ACTIVE (default 2, minimum 2) creates that many local active
//! grants, including the two measured modes; total N is local + FOREIGN_ACTIVE.
//! Extra local grants use caller-bound-allow and remain active throughout sampling.
//! Each expired grant has three still-expired POLICY renewals. Each active session has
//! ST_ADMISSION_WRONG_ORIGIN_OVERLAYS (default 1000; 0 allowed) authenticated wrong-origin
//! overlays, alternating POLICY and REVOKED; every measured read must still succeed.
//! ST_ADMISSION_READ_COUNTS_ONLY=1 measures only read counts: it skips the
//! candidate SQL, latency pairs, the q39 gate and the release-build requirement.
//! Read counts require a fresh ST3_PROFILE_DIR and ST3_PROFILE_SLOW_MS=0. The existing
//! profile `read_acquisitions` counts pooled read-connection/snapshot acquisitions per
//! operation (not SQL rows), for direct authentication and one standalone GET per mode.
//! SQL work uses existing smallclaims sqlite::work trace counters (statements/VM steps).
//! Authentication work is thread-scoped; standalone GET work is process-global through the
//! receipt flush and can include background daemon SQL. No production code changes.
//! Current-head candidate diagnostics read the effective authority projection only;
//! the separate historical index DDL probe is not a current admission measurement.
//! After clean q39 sampling, indexed foreign-grant runs also measure authentication
//! with a matching proof recheck held queued under the Store writer guard.
//! Assertions bracket each sample; this destructive fixture runs last.
//! Latencies use the real daemon's bound sockets, bearer middleware, grant lookup,
//! projection and durable approval consumption.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::{Instant, SystemTime, UNIX_EPOCH}};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use fractal_daemon::{api::{self, AppState}, client::Client, model::ClaimInput, store::Store};
use fractal_daemon::{AttentionListStore as _, EvidenceStore as _, SeatsStore as _};
use fractal_daemon::api::client_v0::assistant::authorization::{ReadPayload, request_digest, signing_bytes, HEADER};
use ring::{rand::SystemRandom, signature::{Ed25519KeyPair, KeyPair as _}};
use serde_json::{Value, json};
use tokio::sync::Barrier;

const OWNER: &str = "person/bench-operator";
const NODE: &str = "benchmark-member";
const GRANTED: &str = "custom.client.assistant.v1.granted";
const POLICY: &str = "custom.client.assistant.v1.policy";
const REVOKED: &str = "custom.client.assistant.v1.revoked";
const INDEX: &str = "claims_assistant_v1_principal_expiry_index";
const CREATE_INDEX: &str = "CREATE INDEX claims_assistant_v1_principal_expiry_index ON claims(kind,COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt'),json_extract(body,'$.fields.expires_at_unix_ms'))";
const A_QUERY: &str = "SELECT id FROM claims WHERE kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2";
const PROJECTION_INDEX: &str = "assistant_grant_effective_active_index";
const PATH: &str = "/v1/client/assistant/summary";

fn number(name: &str, default: usize) -> usize {
    let value = std::env::var(name).map_or(default, |s| s.parse().expect("positive integer setting"));
    assert!(value > 0, "{name} must be positive");
    value
}

fn count(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |s| s.parse().expect("nonnegative integer setting"))
}

/// Bind source checkout/archive, build declaration and the actual running executable.
/// Embedded env values are Cargo-tracked inputs, not runtime overrides of old artifacts.
fn execution_identity() -> Value {
    let required = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}; see harness instructions"));
    let expected = required("ST_ADMISSION_EXPECT_REVISION");
    assert_eq!(expected.len(), 40, "use a full commit ID");
    assert!(expected.bytes().all(|byte| byte.is_ascii_hexdigit()), "use a full commit ID");
    let (Some(built_revision), Some(build_provenance)) = (
        option_env!("ST_ADMISSION_BUILD_REVISION"),
        option_env!("ST_ADMISSION_BUILD_PROVENANCE"),
    ) else {
        panic!("opt-in measurement requires build revision and provenance; see harness instructions");
    };
    assert!(!build_provenance.trim().is_empty(), "build provenance must not be empty");
    assert_eq!(built_revision, expected, "the executable was built for a different source revision");
    let binary = std::env::current_exe().unwrap().canonicalize().unwrap();
    assert_eq!(binary, PathBuf::from(required("ST_ADMISSION_BINARY")).canonicalize().unwrap(),
        "execute the explicitly selected Cargo artifact");
    let source_root = PathBuf::from(required("ST_ADMISSION_SOURCE_ROOT")).canonicalize().unwrap();
    let (git_head, source_revision, source_provenance) = if source_root.join(".git").exists() {
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git").arg("-C").arg(&source_root).args(args).output().unwrap();
            assert!(output.status.success(), "git identity command failed");
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        assert_eq!(PathBuf::from(git(&["rev-parse", "--show-toplevel"])).canonicalize().unwrap(), source_root);
        let head = git(&["rev-parse", "HEAD"]);
        assert_eq!(head, expected, "source checkout and executable revision differ");
        let status = git(&["status", "--porcelain", "--untracked-files=no"]);
        (Some(head.clone()), head, format!("git worktree; tracked changes:\n{status}"))
    } else {
        let revision = required("ST_ADMISSION_SOURCE_REVISION");
        let provenance = required("ST_ADMISSION_SOURCE_PROVENANCE");
        assert_eq!(revision, expected, "archive source and executable revision differ");
        assert!(!provenance.trim().is_empty(), "archive evidence must not be empty");
        (None, revision, provenance)
    };
    use std::io::Read as _;
    let mut executable = std::fs::File::open(&binary).unwrap();
    let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = [0_u8; 65_536];
    loop {
        let bytes = executable.read(&mut buffer).unwrap();
        if bytes == 0 { break; }
        hash.update(&buffer[..bytes]);
    }
    let sha256 = hex::encode(hash.finish().as_ref());
    json!({"git_rev_parse_head":git_head,"source_revision":source_revision,
        "source_root":source_root,"source_provenance":source_provenance,
        "binary_path":binary,"binary_sha256":sha256,"build_revision":built_revision,
        "build_provenance":build_provenance,
        "identity_limit":"build/archive provenance is explicitly supplied; SHA256 identifies executed bytes, not independent source attestation"})
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

fn explain(connection: &rusqlite::Connection, query: &str, parameters: &[&dyn rusqlite::ToSql]) -> Vec<String> {
    let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap();
    let mut rows = statement.query(parameters).unwrap();
    let mut plan = Vec::new();
    while let Some(row) = rows.next().unwrap() { plan.push(row.get::<_, String>(3).unwrap()); }
    plan
}

fn p95(samples: &mut [f64]) -> Option<f64> {
    samples.sort_by(f64::total_cmp);
    (!samples.is_empty()).then(|| samples[(samples.len() * 95).div_ceil(100) - 1])
}

fn sql_work(work: smallclaims::sqlite::work::SqliteWork) -> Value {
    assert!(work.statements > 0, "SQL work instrumentation must be enabled");
    json!({"statements":work.statements,"vm_steps":work.vm_steps,
        "fullscan_steps":work.fullscan_steps,"sorts":work.sorts,"autoindex_rows":work.autoindex_rows})
}

fn profile_lines(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path).map_or(0, |log| log.lines().count())
}

/// The read acquisitions of the single `label` operation recorded after line `from`.
fn profile_reads(path: &std::path::Path, from: usize, label: &str) -> u64 {
    let log = std::fs::read_to_string(path).expect("profile slow.jsonl must be readable");
    let rows = log.lines().skip(from).map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|row| row["label"] == label).collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "expected exactly one new {label} profile row; set ST3_PROFILE_SLOW_MS=0");
    rows[0]["read_acquisitions"].as_u64().expect("profile rows report read_acquisitions")
}

/// Direct native Store receipt and handler authentication, not worker self-sync:
/// the worker suppresses its own origin. A fresh same-origin receiver has no
/// durable local append, so only the exact replicated proof can authorize it.
fn queued_subject_authentication(state: &AppState, history: &rusqlite::Connection, profile_log: &std::path::Path) -> Value {
    use smallclaims::store::assistant_grant_authority::{self as authority, SUBJECT_PENDING_QUERY};
    const ORIGIN: &str = "benchmark-queued-issuer";
    const ID: &str = "ffffffffffffffffffffffff";
    let fleet = super::super::daemon_bench::FLEET;
    let subject = format!("custom/client/assistant-v1-{ID}");
    let digest = hex::encode(ring::digest::digest(&ring::digest::SHA256,
        b"q39/dedicated-same-origin-replicated-credential").as_ref());
    let collisions: u64 = history.query_row(
        "SELECT count(*) FROM claims WHERE kind=?1 AND
            (subject=?2 OR json_extract(body,'$.fields.credential_hash')=?3)",
        rusqlite::params![GRANTED,subject,digest], |row| row.get(0),
    ).unwrap();
    assert_eq!(collisions,0,"queued fixture identity must not collide with clean grants or expired history");
    let root = tempfile::tempdir().unwrap();
    let issuer = Store::open_memory(ORIGIN).unwrap();
    issuer.bind_fleet(fleet).unwrap();
    let signer = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
    issuer.pin_fleet_anchor(signer.public()).unwrap();
    issuer.set_member_key(Some(signer.clone())).unwrap();
    issuer.admit_fleet_anchor(fleet,signer.public(),"listening").unwrap();
    let grant = issuer.append_claim(&ClaimInput {
        subject:subject.clone(), kind:GRANTED.into(), actor:Some(OWNER.into()),
        fields:serde_json::from_value(json!({
            "grant_id":ID,"session_actor":format!("assistant/chatgpt/session/{ID}"),
            "issuer":OWNER,"principal":"assistant/chatgpt","profile":"assistant",
            "expires_at_unix_ms":u64::MAX,"credential_hash":digest,"policy_revision":1,
            "scopes":["read.assistant.summary"],"minimal_read_mode":"caller-bound-allow",
            "disclosure_consent":true
        })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
    }).unwrap();
    issuer.seal_local_batches().unwrap();
    let database = root.path().join("queued.sqlite");
    let receiver = Arc::new(Store::open(&database,ORIGIN).unwrap());
    receiver.bind_fleet(fleet).unwrap();
    receiver.pin_fleet_anchor(signer.public()).unwrap();
    // Never install an own-origin signer on the receiver or export from it.
    let exchange = issuer.export_replication_exchange_answering(
        fleet,&receiver.replication_inventory().unwrap(),&receiver.replication_signature_requests().unwrap(),
    ).unwrap();
    receiver.receive_replication_exchange(ORIGIN,fleet,&exchange).unwrap();
    receiver.validate_replication_backlog().unwrap();
    receiver.project_replication_backlog().unwrap();
    let mut complete = false;
    const MAX_QUEUED_FIXTURE_AUTHORITY_SLICES: usize = 1024;
    for _ in 0..MAX_QUEUED_FIXTURE_AUTHORITY_SLICES {
        if receiver.advance_assistant_grant_authority_backfill().unwrap() {
            complete = true;
            break;
        }
    }
    assert!(complete,
        "q39 queued fixture authority drain did not complete within {MAX_QUEUED_FIXTURE_AUTHORITY_SLICES} slices");
    assert!(!receiver.assistant_grant_authority_maintenance_pending().unwrap());
    assert!(receiver.member_public_key().is_none(),"queued receiver must never sign its own origin");
    assert!(receiver.claim_has_authenticated_member_writer(&grant.id).unwrap(),
        "queued target must have an admitted native signed envelope and authorized membership window");
    let connection = rusqlite::Connection::open(&database).unwrap();
    let local_appends: u64 = connection.query_row(
        "SELECT count(*) FROM assistant_grant_local_appends WHERE claim_id=?1",
        [&grant.id], |row| row.get(0),
    ).unwrap();
    assert_eq!(local_appends,0,"queued target must not have durable local-append provenance");
    let digest_rows: u64 = connection.query_row(
        "SELECT count(*) FROM claims WHERE kind=?1 AND json_extract(body,'$.fields.credential_hash')=?2",
        rusqlite::params![GRANTED,digest], |row| row.get(0),
    ).unwrap();
    assert_eq!(digest_rows,1,"queued credential must identify exactly one grant");
    assert_eq!(authority::by_credential(&connection,&digest).unwrap().unwrap().id,grant.id);
    let (member_key,sequence,envelope_hash,position): (String,u64,String,u64) = connection.query_row(
        "SELECT auth_member_key,auth_sequence,auth_envelope_hash,auth_record_position
         FROM assistant_grant_pending WHERE claim_id=?1 AND authenticated=1",
        [&grant.id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).expect("queued target must retain a nonnull actual replicated proof tuple");
    assert_eq!(member_key,signer.public());
    let intact_records = || connection.query_row(
        "SELECT count(*) FROM replica_records WHERE claim_id=?1 AND state='valid'",
        [&grant.id], |row| row.get::<_,u64>(0),
    ).unwrap();
    assert_eq!(intact_records(),1,"queued target must have exactly one intact proof route");
    let mut state = state.clone();
    state.store = receiver;
    state.node = ORIGIN.into();
    state.state_dir = root.path().into();
    assert!(api::client_v0::external_grants::authenticate(&state,&digest,"http").unwrap().is_some());
    let claim_id = grant.id.clone();
    let mut writer = state.store.graph.connection.write();
    let transaction = writer.transaction().unwrap();
    // Withdraw exactly the record referenced by the authenticated tuple. Its
    // production trigger queues the claim recheck directly, unlike envelope
    // invalidation which first queues a separate invalidation obligation.
    assert_eq!(transaction.execute(
        "UPDATE replica_records SET state='invalid'
         WHERE claim_id=?1 AND writer=?2 AND sequence=?3 AND envelope_hash=?4 AND position=?5 AND state='valid'",
        rusqlite::params![claim_id,ORIGIN,sequence,envelope_hash,position],
    ).unwrap(),1,"queued fixture must withdraw its exact authenticated replica record");
    transaction.commit().unwrap();
    assert_eq!(intact_records(),0,"no alternative intact proof route may survive withdrawal");
    let matching_rechecks = || {
        let mut query = connection.prepare(
            "SELECT work.claim_id FROM assistant_grant_proof_rechecks AS work
             JOIN assistant_grant_pending AS pending ON pending.claim_id=work.claim_id
             WHERE pending.grant_subject=?1 AND pending.claim_id=?2 ORDER BY work.claim_id",
        ).unwrap();
        query.query_map(rusqlite::params![subject,claim_id], |row| row.get::<_,String>(0))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    let queue_counts = || connection.query_row(
        "SELECT (SELECT count(*) FROM assistant_grant_proof_rechecks),
                (SELECT count(*) FROM assistant_grant_proof_invalidations),
                (SELECT count(*) FROM assistant_grant_dirty),
                (SELECT count(*) FROM assistant_grant_auth_cursor)",
        [], |row| Ok([row.get::<_,u64>(0)?,row.get(1)?,row.get(2)?,row.get(3)?]),
    ).unwrap();
    let queued = matching_rechecks();
    assert_eq!(queued,vec![claim_id.clone()],"proof withdrawal must leave the exact matching committed recheck");
    let queues = queue_counts();
    let mut samples = Vec::new();
    for _ in 0..8 {
        assert_eq!(matching_rechecks(),queued,"matching obligations must remain queued at measurement");
        assert_eq!(queue_counts(),queues,"all authentication queue counts must remain fixed");
        let mut pending = connection.prepare(SUBJECT_PENDING_QUERY).unwrap();
        assert!(pending.query_row([&subject],|row| row.get::<_,bool>(0)).unwrap());
        let subject_pending_sql_work = sql_work(smallclaims::sqlite::work::SqliteWork {
            statements:1,
            vm_steps:u64::try_from(pending.get_status(rusqlite::StatementStatus::VmStep)).unwrap(),
            fullscan_steps:u64::try_from(pending.get_status(rusqlite::StatementStatus::FullscanStep)).unwrap(),
            sorts:u64::try_from(pending.get_status(rusqlite::StatementStatus::Sort)).unwrap(),
            autoindex_rows:u64::try_from(pending.get_status(rusqlite::StatementStatus::AutoIndex)).unwrap(),
        });
        let from = profile_lines(profile_log);
        let started = Instant::now();
        let operation = smallclaims::profile::Op::start("admission-bench/authenticate-queued",None).unwrap();
        let entered = operation.enter();
        let work = smallclaims::sqlite::work::SqliteWorkScope::start();
        let authenticated = api::client_v0::external_grants::authenticate(&state,&digest,"http").unwrap();
        let authentication_sql_work = sql_work(work.finish());
        drop(entered);
        operation.finish();
        let authentication_ms = started.elapsed().as_secs_f64()*1000.0;
        assert!(authenticated.is_none(),"queued proof work must deny authentication");
        let authentication_read_acquisitions = profile_reads(profile_log,from,"admission-bench/authenticate-queued");
        assert_eq!(matching_rechecks(),queued,"authentication must not consume queued work");
        assert_eq!(queue_counts(),queues,"authentication must not alter queue counts");
        assert_eq!(intact_records(),0,"measurement must not restore an intact proof route");
        samples.push(json!({"subject_pending_sql_work":subject_pending_sql_work,
            "authentication_sql_work":authentication_sql_work,
            "authentication_read_acquisitions":authentication_read_acquisitions,
            "authentication_ms":authentication_ms,"authenticated":false,
            "matching_queued_rechecks_before":queued,"matching_queued_rechecks_after":matching_rechecks()}));
    }
    drop(writer);
    json!({"subject":subject,"claim_id":claim_id,"obligation_kind":"proof-recheck",
        "fixture":"direct native same-origin Store receipt and handler; not worker self-sync",
        "proof":{"writer":ORIGIN,"member_key":member_key,"sequence":sequence,
            "envelope_hash":envelope_hash,"record_position":position},
        "queue_counts":queues,
        "queue_hold":"unsigned receiver Store writer retained across committed exact-record withdrawal and all samples","samples":samples})
}

// Real signed membership and replication, outside measured requests. Merely
// copying origin text would not exercise the authentication provenance check.
fn foreign_same_profile(store: &Store, anchor: Arc<smallclaims::fleet::MemberKey>, template: &Value, count: usize, active: &[(String, Value)], overlays: usize) {
    let fleet = super::super::daemon_bench::FLEET;
    store.pin_fleet_anchor(anchor.public()).unwrap();
    store.set_member_key(Some(anchor.clone())).unwrap();
    store.admit_fleet_anchor(fleet, anchor.public(), "listening").unwrap();
    let peer = Store::open_memory("benchmark-peer").unwrap();
    peer.bind_fleet(fleet).unwrap();
    let key = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
    peer.pin_fleet_anchor(anchor.public()).unwrap();
    peer.set_member_key(Some(key.clone())).unwrap();
    store.append_claim(&ClaimInput {
        subject:"host/benchmark-peer".into(), kind:"fleet.member-admitted".into(),
        actor:Some(OWNER.into()), fields:serde_json::from_value(json!({
            "fleet_id":fleet,"member_key":key.public(),"via":"invite","mode":"listening"
        })).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None,
    }).unwrap();
    let sync = |from: &Store, to: &Store| {
        let exchange = from.export_replication_exchange_answering(
            fleet, &to.replication_inventory().unwrap(), &to.replication_signature_requests().unwrap(),
        ).unwrap();
        to.receive_replication_exchange(from.origin(), fleet, &exchange).unwrap();
        to.validate_replication_backlog().unwrap();
        to.project_replication_backlog().unwrap();
    };
    sync(store, &peer);
    let mut ids = Vec::new();
    for row in 0..count {
        let id = format!("{:024x}", row + 1);
        let mut fields = template.clone();
        fields["grant_id"] = json!(id);
        fields["session_actor"] = json!(format!("assistant/chatgpt/session/{id}"));
        fields["expires_at_unix_ms"] = json!(u64::MAX);
        fields["credential_hash"] = json!(hex::encode(ring::digest::digest(&ring::digest::SHA256, id.as_bytes()).as_ref()));
        let grant = peer.append_claim(&ClaimInput {
            subject:format!("custom/client/assistant-v1-{id}"), kind:GRANTED.into(),
            actor:Some(OWNER.into()), fields:serde_json::from_value(fields).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:None,
        }).unwrap();
        ids.push(grant.id);
    }
    for (subject, template) in active {
        for row in 0..overlays {
            let mut fields = template.clone();
            fields["expires_at_unix_ms"] = json!(1);
            fields["policy_revision"] = json!(row + 100);
            let overlay = peer.append_claim(&ClaimInput {
                subject:subject.clone(), kind:if row % 2 == 0 { POLICY } else { REVOKED }.into(),
                actor:Some(OWNER.into()), fields:serde_json::from_value(fields).unwrap(),
                evidence:vec![], expected_subject:None, idempotency_key:None,
            }).unwrap();
            ids.push(overlay.id);
        }
    }
    // Exchanges are cut to a body budget, so overlay histories need several rounds.
    peer.seal_local_batches().unwrap();
    for _ in 0..256 {
        sync(&peer, store);
        if ids.iter().all(|id| revision_api::verified_writer(store, id)) { return; }
    }
    panic!("the foreign-grant scenario must exercise verified provenance");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn authenticated_get_during_pairing() {
    let Ok(variant) = std::env::var("ST_ADMISSION_VARIANT") else { return };
    assert!(matches!(variant.as_str(), "baseline" | "indexed"));
    let identity = execution_identity();
    assert_eq!(variant, revision_api::VARIANT, "scenario variant must match the compiled revision API");
    let counts_only = std::env::var_os("ST_ADMISSION_READ_COUNTS_ONLY").is_some();
    assert!(counts_only || fractal_daemon::test_support::optimized_build(), "use --release");
    assert_eq!(std::env::var("ST3_PROFILE_SLOW_MS").as_deref(), Ok("0"),
        "set ST3_PROFILE_SLOW_MS=0 to capture every measured operation");
    let profile_log = PathBuf::from(std::env::var_os("ST3_PROFILE_DIR")
        .expect("set a fresh ST3_PROFILE_DIR for each revision")).join("slow.jsonl");
    smallclaims::profile::init_from_env();
    assert!(smallclaims::profile::enabled(), "profiling must initialize before Store::open");
    let indexed = variant == "indexed";
    let history = number("ST_ADMISSION_HISTORY", 10_000);
    let policy_renewals = count("ST_ADMISSION_RENEWALS", 3);
    let invalid_history_ids = std::env::var_os("ST_ADMISSION_INVALID_HISTORY_IDS").is_some();
    let samples = if counts_only { 0 } else { number("ST_ADMISSION_SAMPLES", 200) };
    let foreign_active = count("ST_ADMISSION_FOREIGN_ACTIVE", 0);
    let overlays = count("ST_ADMISSION_WRONG_ORIGIN_OVERLAYS", 1000);
    let local_active = number("ST_ADMISSION_LOCAL_ACTIVE", 2);
    assert!(local_active >= 2, "the fixture measures two distinct read modes");
    if variant == "baseline" {
        assert!(counts_only && foreign_active == 0 && overlays == 0,
            "base requires READ_COUNTS_ONLY=1, FOREIGN_ACTIVE=0, WRONG_ORIGIN_OVERLAYS=0");
    }
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("admission.sqlite");
    let store = Arc::new(Store::open(&database, NODE).unwrap());
    store.bind_fleet(super::super::daemon_bench::FLEET).unwrap();
    let node_key = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
    store.set_node_key(node_key.clone()).unwrap();
    let state = AppState {
        browser_pairing_login:None,
        seat_admission:Default::default(), owning_person:Some(OWNER.into()), store:store.clone(),
        notify:Arc::new(tokio::sync::Notify::new()), event_notify:tokio::sync::watch::channel(0).0,
        node:NODE.into(), state_dir:root.path().into(), pty_root:root.path().join("pty"), pty_binary:PathBuf::from("pty"),
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
    let mut extra_sessions = Vec::new();
    for _ in 2..local_active {
        extra_sessions.push(complete(&http, &base, &begin(&owner, "caller-bound-allow").await).await);
    }
    // Synthetic expired grants retain valid production field shapes, native Store envelopes,
    // origin/actor and per-subject indexes. They are never service-profile fixtures and never
    // authorize a request. Grant history is deliberately nonempty for BOTH principals.
    let template_subject = format!("custom/client/assistant-v1-{}", sessions[0].1["grant"]["id"].as_str().unwrap().trim_start_matches("assistant-grant/"));
    let template = store.claims_for_subject_kind_at(&template_subject, GRANTED, None, true, 1).unwrap().claims.pop().unwrap();
    let active = sessions.iter().map(|(_, paired)| paired).chain(extra_sessions.iter()).map(|paired| {
        let subject = format!("custom/client/assistant-v1-{}", paired["grant"]["id"].as_str().unwrap().trim_start_matches("assistant-grant/"));
        let grant = store.claims_for_subject_kind_at(&subject, GRANTED, None, true, 1).unwrap().claims.pop().unwrap();
        (subject, grant.body["fields"].clone())
    }).collect::<Vec<_>>();
    if foreign_active > 0 || overlays > 0 {
        foreign_same_profile(&store, node_key, &template.body["fields"], foreign_active, &active, overlays);
    }
    let fixture_store = store.clone();
    tokio::task::spawn_blocking(move || {
        for principal in ["assistant/chatgpt", "assistant/mcp"] {
            for row in 0..history {
                let id = if invalid_history_ids {
                    format!("history-{}-{row}", principal.trim_start_matches("assistant/"))
                } else {
                    format!("{:024x}", row * 2 + usize::from(principal == "assistant/mcp"))
                };
                let mut fields = template.body["fields"].clone();
                fields["principal"] = json!(principal);
                fields["grant_id"] = json!(format!("assistant-grant/{id}"));
                fields["session_actor"] = json!(format!("{principal}/session/assistant-grant/{id}"));
                fields["expires_at_unix_ms"] = json!(1);
                fields["credential_hash"] = json!(hex::encode(ring::digest::digest(&ring::digest::SHA256, id.as_bytes()).as_ref()));
                let subject = format!("custom/client/assistant-v1-{id}");
                fixture_store.append_claim(&ClaimInput { subject:subject.clone(), kind:GRANTED.into(),
                    actor:Some(OWNER.into()), fields:serde_json::from_value(fields.clone()).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None }).unwrap();
                for revision in 0..policy_renewals {
                    fields["policy_revision"] = json!(revision + 2);
                    fields["expires_at_unix_ms"] = json!(revision + 2);
                    fixture_store.append_claim(&ClaimInput { subject:subject.clone(), kind:POLICY.into(),
                        actor:Some(OWNER.into()), fields:serde_json::from_value(fields.clone()).unwrap(),
                        evidence:vec![], expected_subject:None, idempotency_key:None }).unwrap();
                }
            }
        }
    }).await.unwrap();
    if indexed {
        // Replication provenance does not materialize effective authority. The
        // socket fixture does not start the daemon's authority worker, so settle
        // real membership/authentication/attachment work before clean sampling.
        let fixture_store = store.clone();
        tokio::task::spawn_blocking(move || {
            const MAX_FIXTURE_AUTHORITY_SLICES: usize = 100_000;
            let mut complete = false;
            for _ in 0..MAX_FIXTURE_AUTHORITY_SLICES {
                if fixture_store.advance_assistant_grant_authority_backfill().unwrap() {
                    complete = true;
                    break;
                }
            }
            assert!(complete,
                "q39 fixture authority drain did not complete within {MAX_FIXTURE_AUTHORITY_SLICES} slices");
            assert!(!fixture_store.assistant_grant_authority_maintenance_pending().unwrap(),
                "q39 clean sampling must begin with authority maintenance idle");
        }).await.unwrap();
    }
    let connection = rusqlite::Connection::open(&database).unwrap();
    let candidate_query = revision_api::candidate_query();
    let query = candidate_query.as_ref().map_or(A_QUERY, |(query, _)| *query);
    let after = if counts_only {
        Vec::new()
    } else if let Some((query, expiry)) = &candidate_query {
        let plan = explain(&connection, query, rusqlite::params![
            "assistant/chatgpt", OWNER, expiry, Option::<String>::None, Option::<String>::None
        ]);
        assert!(plan.iter().any(|line| line.contains(PROJECTION_INDEX)), "{plan:?}");
        assert!(!plan.iter().any(|line| line.contains("claims")), "{plan:?}");
        plan
    } else {
        explain(&connection, A_QUERY, rusqlite::params![GRANTED, "assistant/chatgpt"])
    };
    let total_rows: u64 = connection.query_row("SELECT count(*) FROM claims", [], |r| r.get(0)).unwrap();
    let mut principal_rows = BTreeMap::new();
    for principal in ["assistant/chatgpt", "assistant/mcp"] {
        let count: u64 = connection.query_row("SELECT count(*) FROM claims WHERE kind=?1 AND COALESCE(json_extract(body,'$.fields.principal'),'assistant/chatgpt')=?2", rusqlite::params![GRANTED,principal], |r| r.get(0)).unwrap();
        principal_rows.insert(principal, count);
    }
    // Isolate the production effective-authority query from provenance and locks.
    // Diagnostic work stays outside HTTP samples; no raw-history admission alias.
    let mut diagnostics = Vec::new();
    for _ in 0..8 {
        let candidate = candidate_query.as_ref().filter(|_| !counts_only).map(|(query, expiry)| {
            let started = Instant::now();
            let mut statement = connection.prepare(query).unwrap();
            let grant_records = statement.query_map(rusqlite::params![
                "assistant/chatgpt", OWNER, expiry, Option::<String>::None, Option::<String>::None
            ], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
            // This independent diagnostic connection is not a traced Store connection.
            let status = |counter| u64::try_from(statement.get_status(counter)).unwrap();
            let work = smallclaims::sqlite::work::SqliteWork {
                statements: 1,
                vm_steps: status(rusqlite::StatementStatus::VmStep),
                fullscan_steps: status(rusqlite::StatementStatus::FullscanStep),
                sorts: status(rusqlite::StatementStatus::Sort),
                autoindex_rows: status(rusqlite::StatementStatus::AutoIndex),
            };
            assert_eq!(grant_records.len(), local_active + foreign_active,
                "expired history and wrong-origin overlays must not change active N");
            assert_eq!(work.fullscan_steps, 0, "candidate reads must not scan history");
            assert!(work.vm_steps <= 512 + 128 * u64::try_from(local_active + foreign_active).unwrap(),
                "candidate VM work must scale with active N, not expired history or overlay floods: {work:?}");
            for record in &grant_records {
                let _: fractal_daemon::model::ClaimRecord = serde_json::from_str(record).unwrap();
            }
            (grant_records.len(), started.elapsed().as_secs_f64() * 1000.0, sql_work(work))
        });
        let credential = sessions[0].1["credential"].as_str().unwrap();
        let digest = hex::encode(ring::digest::digest(&ring::digest::SHA256, credential.as_bytes()).as_ref());
        let profile_from = profile_lines(&profile_log);
        let started = Instant::now();
        let authentication_profile = smallclaims::profile::Op::start("admission-bench/authenticate", None).unwrap();
        let entered = authentication_profile.enter();
        let work_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        assert!(api::client_v0::external_grants::authenticate(&state, &digest, "http").unwrap().is_some());
        let authentication_sql_work = sql_work(work_scope.finish());
        drop(entered);
        authentication_profile.finish();
        let direct_auth_ms = started.elapsed().as_secs_f64()*1000.0;
        let authentication_read_acquisitions = profile_reads(&profile_log, profile_from, "admission-bench/authenticate");
        diagnostics.push(json!({"candidate_count":candidate.as_ref().map(|(count, _, _)| count),
            "candidate_sql_ms":candidate.as_ref().map(|(_, ms, _)| ms),
            "candidate_sql_work":candidate.as_ref().map(|(_, _, work)| work),
            "direct_auth_ms":direct_auth_ms,"authentication_sql_work":authentication_sql_work,
            "authentication_read_acquisitions":authentication_read_acquisitions}));
    }
    let inspected = store.claims_for_subject_kind_at(&template_subject, GRANTED, None, true, 1).unwrap().claims.pop().unwrap();
    let started = Instant::now();
    api::client_v0::external_grants::projection(&state, &inspected).unwrap();
    let legacy_projection_ms = started.elapsed().as_secs_f64()*1000.0;
    let witness_projection_ms = revision_api::witness_projection_ms(&state, &inspected);
    let plan = after.clone();
    println!("ADMISSION_DIAGNOSTICS {}", json!({"plan":plan,"samples":diagnostics,
        "legacy_projection_ms":legacy_projection_ms,"witness_projection_ms":witness_projection_ms}));
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
        // A separate unpaired read work window avoids attributing pairing completion
        // SQL to the GET. Global counters can still include background daemon work.
        state.assistant_receipts.flush().await.unwrap();
        api::wait_published_lists_at_least(&store, store.index().unwrap()).await.unwrap();
        let measured_read = http.get(format!("{base}{PATH}")).bearer_auth(credential);
        let measured_read = if mode == "approval-once" {
            measured_read.header(HEADER, proof(&paired, &signer, samples + 2))
        } else { measured_read };
        let before_read = smallclaims::sqlite::work::total();
        let profile_from = profile_lines(&profile_log);
        let response = measured_read.send().await.unwrap();
        let status = response.status();
        let body = response.bytes().await.unwrap();
        state.assistant_receipts.flush().await.unwrap();
        let standalone_read_sql_work = sql_work(smallclaims::sqlite::work::total() - before_read);
        assert!(status.is_success(), "standalone GET {status}: {}", String::from_utf8_lossy(&body));
        let standalone_read_acquisitions = profile_reads(&profile_log, profile_from, &format!("GET {PATH}"));
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
            "get_p95_ms":get_p95,"completion_p95_ms":completion_p95,"q39_budget_ms":100,
            "q39_pass":get_p95.map(|p95| p95 <= 100.0),
            "standalone_read_sql_work":standalone_read_sql_work,
            "standalone_read_acquisitions":standalone_read_acquisitions,
            "get_ms":get_ms,"completion_ms":completion_ms}));
    }
    let hardware = |program: &str, args: &[&str]| {
        let output = std::process::Command::new(program).args(args).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    let queued_subject_authentication = if indexed && foreign_active>0 && !counts_only {
        Some(queued_subject_authentication(&state,&rusqlite::Connection::open(&database).unwrap(),&profile_log))
    } else { None };
    let final_connection = rusqlite::Connection::open(&database).unwrap();
    let final_rows: u64 = final_connection.query_row("SELECT count(*) FROM claims", [], |r| r.get(0)).unwrap();
    let report = json!({"variant":variant,"read_counts_only":counts_only,"revision_api":revision_api::REVISION,
        "execution_identity":identity,"local_active_grants":local_active,"active_grants_n":local_active + foreign_active,
        "hardware_uname":hardware("uname", &["-a"]),
        "hardware_lscpu":hardware("lscpu", &["-J"]),"hardware_hostname":hardware("hostname", &[]),
        "host_description":"Observed local benchmark host; no prescribed hardware identity",
        "fixture_person_ids":[OWNER],"verified_foreign_same_profile_active_grants":foreign_active,
        "history_grants_per_principal":history,"initial_total_claim_rows":total_rows,"initial_granted_rows_by_principal":principal_rows,
        "policy_renewals_per_expired_grant":policy_renewals,
        "invalid_expired_history_ids":invalid_history_ids,
        "wrong_origin_overlays_per_active_grant":overlays,
        "wrong_origin_overlay_active_grants":active.len(),
        "sql_work_scope":{"authentication":"current-thread completed traced SQL statements",
            "standalone_read":"process-global, unpaired HTTP GET through receipt flush; may include background daemon SQL",
            "read_count_limit":"statement/VM work counters, not logical rows read or storage IO",
            "read_acquisitions":"existing profile read_acquisitions: pooled read-connection/snapshot acquisitions per operation"},
        "final_total_claim_rows":final_rows,"admission_query":query,
        "conflict_fold_diagnostics":diagnostics,
        "queued_subject_authentication":queued_subject_authentication,
        "legacy_projection_ms":legacy_projection_ms,"witness_projection_ms":witness_projection_ms,
        "explain_with_index":after,
        "measurements":measurements});
    println!("ADMISSION_MEASUREMENT {}", serde_json::to_string_pretty(&report).unwrap());
    if let Some(path) = std::env::var_os("ST_ADMISSION_REPORT") { std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap(); }
    local.abort();
    fabric.abort();
    if indexed && !counts_only {
        assert!(report["measurements"].as_array().unwrap().iter().all(|row| row["q39_pass"] == true), "q39: authenticated GET p95 must be <=100ms; see report");
    }
}

/// Synthetic large-store index build, independent of the HTTP latency fixture:
/// ```sh
/// ST_ADMISSION_INDEX_BUILD=1 "$ST_ADMISSION_BINARY" daemon_cost::assistant_admission::synthetic_large_store_index_build --exact --nocapture --test-threads=1
/// ```
/// ST_ADMISSION_INDEX_ROWS defaults to 1,000,000 total claims. ST_ADMISSION_INDEX_REPORT
/// optionally saves JSON. This explicitly historical claims-index measurement is
/// independent of the current effective-authority query. It measures CREATE DDL,
/// not full Store::open, cold-disk startup, admission, or signature verification.
/// Use the same identity-bound build/export commands above (including SOURCE_ROOT,
/// EXPECT_REVISION and BINARY). Every index report carries the same execution identity.
#[test]
fn synthetic_large_store_index_build() {
    if std::env::var_os("ST_ADMISSION_INDEX_BUILD").is_none() { return; }
    let identity = execution_identity();
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
    connection.execute_batch("DROP INDEX IF EXISTS claims_assistant_v1_principal_expiry_index").unwrap();
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
    // Independent historical A-query evidence, not the current authority query.
    let before = explain(&connection, A_QUERY, rusqlite::params![GRANTED, "assistant/chatgpt"]);
    let main_bytes_before = std::fs::metadata(&database).unwrap().len();
    let started = Instant::now();
    connection.execute_batch(CREATE_INDEX).unwrap();
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after = explain(&connection, A_QUERY, rusqlite::params![GRANTED, "assistant/chatgpt"]);
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
        "measurement":"historical claims principal/expiry index CREATE DDL only; not current admission or full startup",
        "execution_identity":identity,
        "claims_rows":actual_rows,"synthetic_copied_rows":rows - 1,
        "fixture_authority":"one native template; SQL copies are not signature-authoritative",
        "index_name":INDEX,"index_create_sql":CREATE_INDEX,"index_build_ms":elapsed_ms,
        "historical_query":A_QUERY,
        "main_db_bytes_before":main_bytes_before,
        "main_db_bytes_after":std::fs::metadata(&database).unwrap().len(),
        "wal_bytes_after":wal_bytes,"logical_db_bytes_after":pages * page_size,
        "explain_without_index":before,"explain_with_index":after,
        "hardware_uname":hardware("uname", &["-a"]),
        "hardware_hostname":hardware("hostname", &[]),"hardware_lscpu":hardware("lscpu", &["-J"]),
        "host_description":"Observed local benchmark host; no prescribed hardware identity",
        "fixture_person_ids":[OWNER],
    });
    println!("ADMISSION_INDEX_BUILD {}", serde_json::to_string_pretty(&report).unwrap());
    if let Some(path) = std::env::var_os("ST_ADMISSION_INDEX_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}

/// The only head-specific API surface. To backport the harness to origin/main
/// (d477c67ad1), replace this module's body with exactly:
/// ```ignore
/// use super::*;
/// pub const REVISION: &str = "origin/main";
/// pub const VARIANT: &str = "baseline";
/// // origin/main admission is a credential-hash lookup with no candidate query.
/// pub fn candidate_query() -> Option<(&'static str, String)> { None }
/// pub fn verified_writer(_: &Store, _: &str) -> bool {
///     panic!("origin/main has no provenance check; use FOREIGN_ACTIVE=0 and WRONG_ORIGIN_OVERLAYS=0")
/// }
/// pub fn witness_projection_ms(_: &AppState, _: &fractal_daemon::model::ClaimRecord) -> Option<f64> { None }
/// ```
mod revision_api {
    use super::*;
    pub const REVISION: &str = "fixed-head";
    pub const VARIANT: &str = "indexed";
    pub fn candidate_query() -> Option<(&'static str, String)> {
        let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap();
        Some((smallclaims::store::assistant_grant_authority::CANDIDATES_QUERY,
            smallclaims::store::assistant_grant_authority::expiry_key(now)))
    }
    pub fn verified_writer(store: &Store, id: &str) -> bool {
        store.claim_has_authenticated_member_writer(id).unwrap()
    }
    pub fn witness_projection_ms(state: &AppState, inspected: &fractal_daemon::model::ClaimRecord) -> Option<f64> {
        let started = Instant::now();
        api::client_v0::external_grants::projection_with_witness(state, inspected).unwrap();
        Some(started.elapsed().as_secs_f64() * 1000.0)
    }
}
