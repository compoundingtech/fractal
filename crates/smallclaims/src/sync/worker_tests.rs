//! Tests of the sync worker on stores opened with the plain runtime.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::Mutex;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use tower::ServiceExt as _;

use super::*;
use crate::claim::{ClaimInput, ReplicaEnvelopeId};
use crate::fleet::handshake::JoinRequest;
use crate::replication::{ReplicationExportResponse, ReplicationReceiveResponse};
use crate::store::runtime::Plain;
use crate::store::{CheckpointAction, CheckpointManifestNeed};
use crate::store::{Runtime as _, Store};
use crate::sync::Local;

#[tokio::test]
async fn leave_snapshot_is_read_only_and_reports_expiry_confirmation_and_persistence_failure() {
    let fixture = leave_fixture();
    let fleet = &fixture.leaving_fleet;
    let key = fixture.key.public().to_owned();
    let fleet_id = Some(fixture.leaving_auth.fleet_id().to_owned());
    let initial = fleet.leave_snapshot(fleet_id.clone()).await;
    assert_eq!(initial.members.len(), 1);
    assert_eq!(initial.members[0].state, FleetLeaveStatus::Unconfirmed);
    assert_eq!(initial.members[0].grace_deadline_unix_ms, None);
    assert!(
        fleet.leave.lock().await.awaiting.is_empty(),
        "GET cannot start grace"
    );
    {
        let mut leave = fleet.leave.lock().await;
        leave.awaiting.insert(key.clone());
        leave
            .receiver_deadlines_unix_ms
            .insert(key.clone(), u128::MAX);
    }
    let before = serde_json::to_vec(&*fleet.leave.lock().await).unwrap();
    for _ in 0..2 {
        let snapshot = fleet.leave_snapshot(fleet_id.clone()).await;
        assert_eq!(snapshot.membership, initial.membership);
        assert_eq!(snapshot.own_member_key.as_deref(), Some(key.as_str()));
        assert_eq!(
            snapshot.members[0].state,
            FleetLeaveStatus::AwaitingConfirmation
        );
        assert_eq!(snapshot.members[0].grace_deadline_unix_ms, Some(u128::MAX));
    }
    assert_eq!(
        serde_json::to_vec(&*fleet.leave.lock().await).unwrap(),
        before
    );
    {
        let mut leave = fleet.leave.lock().await;
        leave.receiver_deadlines_unix_ms.insert(key.clone(), 1);
    }
    let expired = fleet.leave_snapshot(fleet_id.clone()).await;
    assert_eq!(expired.members[0].state, FleetLeaveStatus::Unconfirmed);
    assert_eq!(expired.members[0].grace_deadline_unix_ms, Some(1));
    {
        let mut leave = fleet.leave.lock().await;
        leave
            .receiver_deadlines_unix_ms
            .insert(key.clone(), u128::MAX);
        leave.persistence_error = Some("journal unavailable".into());
    }
    let failed = fleet.leave_snapshot(fleet_id.clone()).await;
    assert_eq!(failed.members[0].state, FleetLeaveStatus::Unconfirmed);
    assert_eq!(
        failed.persistence_error.as_deref(),
        Some("journal unavailable")
    );
    {
        let mut leave = fleet.leave.lock().await;
        leave.persistence_error = None;
        leave.completed.insert(key);
    }
    assert_eq!(
        fleet.leave_snapshot(fleet_id).await.members[0].state,
        FleetLeaveStatus::Confirmed
    );
}

#[tokio::test]
async fn leave_authority_socket_is_private_and_reserves_its_address_until_exit() {
    let root = tempfile::tempdir().unwrap();
    let (reservation, listener) = bind_leave_state_socket(root.path()).unwrap();
    let path = leave_state_socket(root.path());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(bind_leave_state_socket(root.path()).is_err());
    assert!(tokio::net::UnixStream::connect(&path).await.is_ok());
    drop(listener);
    assert!(
        bind_leave_state_socket(root.path()).is_err(),
        "startup reservation outlives listener"
    );
    drop(reservation);
    let (_reservation, _listener) = bind_leave_state_socket(root.path()).unwrap();
}

/// A store on the plain runtime.
fn plain_memory(origin: &str) -> anyhow::Result<Store> {
    Store::open_memory(origin, Arc::new(Plain))
}

fn plain_open(path: &Path, origin: &str) -> anyhow::Result<Store> {
    Store::open(path, origin, Arc::new(Plain))
}

#[tokio::test]
async fn signed_default_answer_cannot_draw_envelopes_from_a_named_shard() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let root = tempfile::tempdir().unwrap();
    let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[7; 32]);
    let shard = Arc::new(plain_open(&root.path().join("github.sqlite3"), "birch").unwrap());
    shard
        .bind_shard(&crate::shards::ShardId::parse("github").unwrap())
        .unwrap();
    shard.bind_fleet(auth.fleet_id()).unwrap();
    shard
        .append_claim(&ClaimInput {
            subject: "note/github".into(),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let default = plain_open(&root.path().join("default.sqlite3"), "cedar").unwrap();
    default.bind_fleet(auth.fleet_id()).unwrap();
    let answer = default.export_replication_summary(auth.fleet_id()).unwrap();
    assert_eq!(answer.shard_id, None);
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let signing = auth.clone();
    let app = Router::new().route(
        EXCHANGE_PATH,
        post(move |body: Bytes| {
            let count = count.clone();
            let signing = signing.clone();
            let answer = answer.clone();
            async move {
                count.fetch_add(1, Ordering::Relaxed);
                let query: ReplicationExchange = serde_json::from_slice(&body).unwrap();
                assert_eq!(query.shard_id.as_deref(), Some("github"));
                assert!(
                    query.envelopes.is_empty(),
                    "scope check precedes payload push"
                );
                let envelope = PeerResponse::new("cedar", 0, answer);
                let bytes = serde_json::to_vec(&envelope).unwrap();
                let mut response = (StatusCode::OK, bytes.clone()).into_response();
                response.headers_mut().extend(
                    signing
                        .response_headers_for(
                            EXCHANGE_PATH,
                            "cedar",
                            &bytes,
                            &FleetAuth::body_digest(&body),
                        )
                        .unwrap(),
                );
                response
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "cedar".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let fleet = FleetContext::legacy(BTreeSet::from(["cedar".into()]));
    let error = exchange(
        &replication_http_client(),
        &Local(shard.clone()),
        "birch",
        &peer,
        &auth,
        &fleet,
    )
    .await
    .unwrap_err();
    let mismatch = error.downcast_ref::<PeerShardMismatch>().unwrap();
    assert_eq!(mismatch.expected.as_deref(), Some("github"));
    assert_eq!(mismatch.actual, None);
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    assert!(
        default
            .replication_inventory()
            .unwrap()
            .envelopes
            .is_empty()
    );
    server.abort();
}

/// A peer whose build has shards but not this one refuses the exchange before it stores
/// anything. The dialer must treat that like an answer for the wrong store, so the route
/// parks instead of retrying the same refusal.
#[tokio::test]
async fn a_peer_refusing_the_shard_is_a_shard_mismatch() {
    let root = tempfile::tempdir().unwrap();
    let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[7; 32]);
    let shard = Arc::new(plain_open(&root.path().join("messages.sqlite3"), "birch").unwrap());
    shard
        .bind_shard(&crate::shards::ShardId::parse("messages").unwrap())
        .unwrap();
    shard.bind_fleet(auth.fleet_id()).unwrap();
    let default = Arc::new(plain_open(&root.path().join("default.sqlite3"), "cedar").unwrap());
    default.bind_fleet(auth.fleet_id()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "cedar".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let state = PeerState {
        backend: Local(default.clone()),
        node: "cedar".into(),
        auth: auth.clone(),
        fleet: FleetContext::legacy(BTreeSet::from(["birch".into()])),
        outbound_notify: watch::channel(0_u64).0,
        shards: Default::default(),
    };
    let server = tokio::spawn(async move {
        axum::serve(listener, peer_router(state, Router::new()))
            .await
            .unwrap();
    });
    let fleet = FleetContext::legacy(BTreeSet::from(["cedar".into()]));
    let error = exchange(
        &replication_http_client(),
        &Local(shard.clone()),
        "birch",
        &peer,
        &auth,
        &fleet,
    )
    .await
    .unwrap_err();
    let mismatch = error
        .downcast_ref::<PeerShardMismatch>()
        .unwrap_or_else(|| panic!("not a shard mismatch: {error:#}"));
    assert_eq!(mismatch.refused.as_deref(), Some("shard-mismatch"));
    assert!(default.replication_inventory().unwrap().envelopes.is_empty());
    server.abort();
}

/// One listener answers the default store and the shards it holds. An exchange naming a held
/// shard reaches that shard's store and nothing else; the default store's exchange to the
/// same peer is a separate job.
#[tokio::test]
async fn one_listener_answers_each_held_shard_from_its_own_store() {
    let root = tempfile::tempdir().unwrap();
    let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[7; 32]);
    let messages = crate::shards::ShardId::parse("messages").unwrap();
    let open_shard = |path: &str, origin: &str| {
        let store = Arc::new(plain_open(&root.path().join(path), origin).unwrap());
        store.bind_shard(&messages).unwrap();
        store.bind_fleet(auth.fleet_id()).unwrap();
        store
    };
    let source = open_shard("source-messages.sqlite3", "birch");
    source
        .append_claim(&ClaimInput {
            subject: "note/stream".into(),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let target_shard = open_shard("target-messages.sqlite3", "cedar");
    let target_default = Arc::new(plain_open(&root.path().join("target.sqlite3"), "cedar").unwrap());
    target_default.bind_fleet(auth.fleet_id()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "cedar".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let state = PeerState::new(
        Local(target_default.clone()),
        "cedar".into(),
        auth.clone(),
        FleetContext::legacy(BTreeSet::from(["birch".into()])),
    )
    .with_shards(BTreeMap::from([("messages".into(), Local(target_shard.clone()))]));
    let server = tokio::spawn(async move {
        axum::serve(listener, peer_router(state, Router::new()))
            .await
            .unwrap();
    });
    let fleet = FleetContext::legacy(BTreeSet::from(["cedar".into()]));
    exchange(
        &replication_http_client(),
        &Local(source.clone()),
        "birch",
        &peer,
        &auth,
        &fleet,
    )
    .await
    .unwrap();
    target_shard.validate_replication_backlog().unwrap();
    target_shard.project_replication_backlog().unwrap();
    assert_eq!(target_shard.claims_for("note/stream", None).unwrap().len(), 1);
    assert!(target_default.replication_inventory().unwrap().envelopes.is_empty());
    assert!(target_default.claims_for("note/stream", None).unwrap().is_empty());
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn overload_poll_exhaustion_retains_authenticated_response_details() {
    let fleet_id = "1f91ca65-7793-48cc-866e-ac15690130e1";
    let auth = FleetAuth::test(fleet_id, &[7; 32]);
    let signing = auth.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(EXCHANGE_PATH, post(move |request: Bytes| {
        let auth = signing.clone();
        async move {
            let body = serde_json::to_vec(&PeerResponse::new(
                "cedar", 0, serde_json::json!({
                    "code": "replication-overloaded", "retry_after_ms": 5000,
                    "message": "private backend prose is not diagnostic metadata"
                }),
            )).unwrap();
            let headers = auth.response_headers_for(
                EXCHANGE_PATH, "cedar", &body, &FleetAuth::body_digest(&request),
            ).unwrap();
            let mut response = (StatusCode::SERVICE_UNAVAILABLE, body).into_response();
            response.headers_mut().extend(headers);
            response
        }
    }));
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    // Keep paused time from advancing ahead of real socket work.
    let awake = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    let local = Arc::new(plain_memory("birch").unwrap());
    local.bind_fleet(fleet_id).unwrap();
    let backend = Local(local.clone());
    let query = local.export_replication_summary(fleet_id).unwrap();
    let poller = tokio::spawn(async move {
        post_signed(
            &replication_http_client(), &backend,
            &PeerConfig { name: "cedar".into(), url: format!("http://{address}") },
            "birch", &auth, &FleetContext::legacy(BTreeSet::from(["cedar".into()])),
            &query, false,
        ).await
    });
    let started = std::time::Instant::now();
    loop {
        let overloaded = local.replication_status(true, Some(fleet_id), &["cedar".into()])
            .unwrap().peers[0].worker.as_ref()
            .is_some_and(|worker| worker.phase == "overload");
        if overloaded {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "signed overload was not observed");
        tokio::task::yield_now().await;
    }
    tokio::time::advance(EXCHANGE_POLL_BUDGET).await;
    let error = poller.await.unwrap().unwrap_err();
    server.abort();
    awake.abort();
    let _ = server.await;
    let _ = awake.await;
    assert!(error.is::<PeerOverloaded>());
    let inner_elapsed = error.downcast_ref::<ReplicationExchangeFailure>().unwrap().elapsed_ms;
    let failure = ReplicationExchangeFailure::from_error(
        &error, ReplicationFailurePhase::Verify, Duration::from_secs(300), None,
    );
    assert_eq!(failure.code, "replication-overloaded");
    assert_eq!(failure.phase, ReplicationFailurePhase::ResponseBody);
    assert_eq!(failure.http_status, Some(503));
    assert_eq!(failure.elapsed_ms, inner_elapsed);
    assert!(failure.preparation_ms.is_some());
    assert!(!failure.cause.contains("private"));
}

/// Different checkpoint lineages leave payloadless ranges that can never settle through
/// envelope transport. Both live writers already have a shared range, so a summary alone
/// cannot prove either of their later gaps. The live ranges are listed ahead of the payloadless
/// ones, so one compact round settles them, and no request ever lists a whole inventory.
#[tokio::test]
async fn checkpoint_tombstones_do_not_starve_later_live_ranges() {
    let root = tempfile::tempdir().unwrap();
    let fleet = "1f91ca65-7793-48cc-866e-ac15690130e1";
    let auth = FleetAuth::test(fleet, &[7; 32]);
    let left = Arc::new(plain_open(&root.path().join("left.sqlite3"), "birch").unwrap());
    let right = Arc::new(plain_open(&root.path().join("right.sqlite3"), "cedar").unwrap());
    let note = |store: &Store, text: &str| {
        store
            .append_claim(&ClaimInput {
                subject: format!("note/{text}"),
                kind: "example.note".into(),
                actor: None,
                fields: BTreeMap::from([("text".into(), Value::String(text.into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    };
    for store in [&left, &right] {
        store.bind_fleet(fleet).unwrap();
        note(store, &format!("{}-shared", store.origin));
    }
    for (from, to) in [(&left, &right), (&right, &left)] {
        let shared = from
            .export_replication_exchange(fleet, &ReplicationInventory::default())
            .unwrap();
        to.receive_replication_exchange(&from.origin, fleet, &shared)
            .unwrap();
    }
    for store in [&left, &right] {
        for index in 0..20 {
            note(store, &format!("{}-private-{index}", store.origin));
        }
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        for id in crate::store::test_envelope_ids("alder-retired", 0..768, &store.origin) {
            transaction
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h67d08b9ee5cc owner=smallclaims::sync::worker_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes VALUES (?1, ?2, ?3, 0, ?4)",
                    rusqlite::params![
                        id.writer,
                        id.sequence,
                        id.hash,
                        format!("checkpoint/{}", store.origin)
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        store.replica_rows_changed();
    }

    let state = PeerState {
        backend: Local(right.clone()),
        node: right.origin.clone(),
        auth: auth.clone(),
        fleet: FleetContext::legacy(BTreeSet::from([left.origin.clone()])),
        outbound_notify: watch::channel(0_u64).0,
        shards: Default::default(),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: right.origin.clone(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let router = peer_router(state, Router::new()).layer(axum::middleware::from_fn(
        move |request: Request<Body>, next: axum::middleware::Next| {
            let observed = observed.clone();
            async move {
                let (parts, body) = request.into_parts();
                let bytes = to_bytes(body, MAX_EXCHANGE_BYTES).await.unwrap();
                let decoded = if deflated(&parts.headers) {
                    inflate(&bytes, "test").unwrap()
                } else {
                    bytes.to_vec()
                };
                let query: ReplicationExchange = serde_json::from_slice(&decoded).unwrap();
                assert!(query.envelopes.len() <= crate::store::REPLICATION_PAGE_LIMIT as usize);
                assert!(
                    !query.inventory.digest.is_empty(),
                    "a request keeps its digest"
                );
                observed.lock().unwrap().push((
                    query.inventory.buckets.is_empty(),
                    query.inventory.envelopes.len(),
                    query.envelopes.len(),
                ));
                next.run(Request::from_parts(parts, Body::from(bytes)))
                    .await
            }
        },
    ));
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    let http = replication_http_client();
    let fleet_context = FleetContext::legacy(BTreeSet::from([right.origin.clone()]));
    let result = exchange(
        &http,
        &Local(left.clone()),
        &left.origin,
        &peer,
        &auth,
        &fleet_context,
    )
    .await
    .unwrap();
    assert!(
        result,
        "a divergent compact exchange must reach live ranges beyond the tombstones"
    );
    {
        let requests = requests.lock().unwrap();
        assert!(
            requests.len() <= 2,
            "one compact round, never a full-inventory round or an unbounded retry"
        );
        assert!(
            requests
                .iter()
                .all(|(bare, listed, _)| !bare && *listed <= 512),
            "every request carries range digests and a bounded listing: {requests:?}"
        );
    }
    for store in [&left, &right] {
        store.validate_replication_backlog().unwrap();
        store.project_replication_backlog().unwrap();
        for writer in ["birch", "cedar"] {
            for index in 0..20 {
                let subject = format!("note/{writer}-private-{index}");
                assert_eq!(
                    store
                        .claims_for(&subject, Some("example.note"))
                        .unwrap()
                        .len(),
                    1,
                    "both directions must receive every later live note: {subject}"
                );
            }
        }
        assert_eq!(
            store.checkpointed_envelopes().unwrap(),
            768,
            "tombstones are not transported"
        );
    }
    assert_ne!(
        left.replication_inventory().unwrap().digest,
        right.replication_inventory().unwrap().digest,
        "checkpoint-lineage reconciliation remains separate"
    );
    assert!(
        !exchange(
            &http,
            &Local(left.clone()),
            &left.origin,
            &peer,
            &auth,
            &fleet_context
        )
        .await
        .unwrap(),
        "tombstone-only divergence must let the worker rest"
    );
    note(&left, "published-after-quiet-comparison");
    assert!(
        exchange(
            &http,
            &Local(left.clone()),
            &left.origin,
            &peer,
            &auth,
            &fleet_context
        )
        .await
        .unwrap()
    );
    assert!(
        right
            .latest_claim(
                "note/published-after-quiet-comparison",
                Some("example.note")
            )
            .unwrap()
            .is_some(),
        "a later write must still cross the unresolved tombstone prefix"
    );

    server.abort();
}

/// Measure the exceptional full-inventory path at the reported fleet size, on an isolated
/// disk-backed store. Run explicitly with --ignored --nocapture; no live database is opened.
#[test]
#[ignore = "fleet-size inventory cost measurement"]
fn fleet_size_full_inventory_fallback_cost() {
    let root = tempfile::tempdir().unwrap();
    let store = plain_open(&root.path().join("store.sqlite3"), "birch").unwrap();
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    for id in crate::store::test_envelope_ids("alder-retired", 0..70_000, "lineage") {
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-hd2b7ba01e655 owner=smallclaims::sync::worker_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes VALUES (?1, ?2, ?3, 0, 'checkpoint/example')",
                rusqlite::params![id.writer, id.sequence, id.hash],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    drop(connection);
    store.replica_rows_changed();
    let started = std::time::Instant::now();
    let summary = store.export_replication_summary("fleet/example").unwrap();
    let snapshot_ms = started.elapsed().as_secs_f64() * 1000.;
    let mut divergent = summary.inventory.clone();
    divergent.digest = "different".into();
    for bucket in divergent.buckets.iter_mut().take(18) {
        bucket.digest = "00".repeat(32);
    }
    let started = std::time::Instant::now();
    let compact = store
        .export_replication_exchange("fleet/example", &divergent)
        .unwrap();
    let compact_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_eq!(compact.inventory.envelopes.len(), 512);
    let started = std::time::Instant::now();
    let full = store
        .export_replication_exchange(
            "fleet/example",
            &ReplicationInventory {
                digest: "different".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let export_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = std::time::Instant::now();
    let bytes = serde_json::to_vec(&full).unwrap();
    let json_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = std::time::Instant::now();
    let compressed = deflate(&bytes).unwrap();
    let deflate_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_eq!(full.inventory.envelopes.len(), 70_000);
    assert!(full.envelopes.is_empty());
    assert!(bytes.len() < MAX_EXCHANGE_BYTES);
    eprintln!(
        "identities=70000 summary_bytes={} compact_bytes={} full_bytes={} deflated_bytes={} snapshot_ms={snapshot_ms:.2} compact_ms={compact_ms:.2} export_ms={export_ms:.2} json_ms={json_ms:.2} deflate_ms={deflate_ms:.2}",
        serde_json::to_vec(&summary).unwrap().len(),
        serde_json::to_vec(&compact).unwrap().len(),
        bytes.len(),
        compressed.len()
    );
}

#[tokio::test]
async fn stalled_replication_http_stream_is_bounded() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stalled = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let _socket = socket;
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    let request = replication_http_client()
        .get(format!("http://{address}/stalled"))
        .send();
    let result = tokio::time::timeout(Duration::from_secs(25), request).await;
    stalled.abort();
    assert!(matches!(result, Ok(Err(error)) if error.is_timeout()));
}

#[test]
fn signed_messages_detect_tampering_and_wrong_fleets() {
    let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[7; 32]);
    let body = br#"{"hello":"fleet"}"#;
    let headers = auth.request_headers("node-a", body).unwrap();
    assert!(
        auth.verify(&headers, "POST", EXCHANGE_PATH, body, Some("node-a"), None)
            .is_ok()
    );
    assert!(
        auth.verify(
            &headers,
            "POST",
            EXCHANGE_PATH,
            br#"{"hello":"other"}"#,
            Some("node-a"),
            None
        )
        .is_err()
    );
    let other = FleetAuth::test("48608b46-bf75-442a-a462-787085dc574e", &[7; 32]);
    assert!(
        other
            .verify(&headers, "POST", EXCHANGE_PATH, body, Some("node-a"), None)
            .is_err()
    );
}

#[test]
fn response_signatures_bind_to_one_request() {
    let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[9; 32]);
    let body = b"response";
    let headers = auth
        .response_headers_for(EXCHANGE_PATH, "node-b", body, "request-a")
        .unwrap();
    assert!(
        auth.verify(
            &headers,
            "RESPONSE",
            EXCHANGE_PATH,
            body,
            Some("node-b"),
            Some("request-a")
        )
        .is_ok()
    );
    assert!(
        auth.verify(
            &headers,
            "RESPONSE",
            EXCHANGE_PATH,
            body,
            Some("node-b"),
            Some("request-b")
        )
        .is_err()
    );
}

#[test]
fn fleet_secret_loading_accepts_private_raw_or_hex_files_only() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fleet-secret");
    fs::write(&path, [3_u8; 32]).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    FleetAuth::load("1f91ca65-7793-48cc-866e-ac15690130e1", &path).unwrap();

    fs::write(&path, format!("{}\n", hex::encode([4_u8; 32]))).unwrap();
    FleetAuth::load("1f91ca65-7793-48cc-866e-ac15690130e1", &path).unwrap();

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(FleetAuth::load("1f91ca65-7793-48cc-866e-ac15690130e1", &path).is_err());
}

#[tokio::test]
async fn an_authenticated_failure_has_a_request_bound_signature() {
    let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[5; 32]);
    let body = Bytes::from_static(b"not json");
    let request_digest = FleetAuth::body_digest(&body);
    let response = receive_exchange(
        None,
        State(PeerState {
            backend: Local(Arc::new(plain_memory("target").unwrap())),
            node: "target".into(),
            auth: auth.clone(),
            fleet: FleetContext::legacy(BTreeSet::from(["source".into()])),
            outbound_notify: watch::channel(0_u64).0,
            shards: Default::default(),
        }),
        auth.request_headers("source", &body).unwrap(),
        body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    auth.verify(
        &headers,
        "RESPONSE",
        EXCHANGE_PATH,
        &bytes,
        Some("target"),
        Some(&request_digest),
    )
    .unwrap();
}

#[tokio::test]
async fn the_peer_route_accepts_an_exchange_above_axums_default_body_limit() {
    let fleet = "1f91ca65-7793-48cc-866e-ac15690130e1";
    let auth = FleetAuth::test(fleet, &[5; 32]);
    let state = PeerState {
        backend: Local(Arc::new(plain_memory("target").unwrap())),
        node: "target".into(),
        auth: auth.clone(),
        fleet: FleetContext::legacy(BTreeSet::from(["source".into()])),
        outbound_notify: watch::channel(0_u64).0,
        shards: Default::default(),
    };
    let exchange = ReplicationExchange {
        build: None,
        claim_ranges: None,
        projection_digests: Default::default(),
        peer: "source".into(),
        fleet_id: fleet.into(),
        shard_id: None,
        schema_digest: Plain.schema_digest(),
        projection_registry_digest: None,
        projection_versions: Default::default(),
        authority_digest: String::new(),
        graph_digest: String::new(),
        inventory: ReplicationInventory::default(),
        envelopes: Vec::new(),
        signature_requests: Vec::new(),
        signatures: Vec::new(),
    };
    let mut body = serde_json::to_vec(&exchange).unwrap();
    body.resize(2 * 1024 * 1024 + 1, b' ');
    let request_digest = FleetAuth::body_digest(&body);
    let mut request = Request::builder()
        .method("POST")
        .uri(EXCHANGE_PATH)
        .body(Body::from(body.clone()))
        .unwrap();
    request
        .headers_mut()
        .extend(auth.request_headers("source", &body).unwrap());
    let response = peer_router(state, Router::new())
        .oneshot(request)
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let headers = response.headers().clone();
    let response_body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    auth.verify(
        &headers,
        "RESPONSE",
        EXCHANGE_PATH,
        &response_body,
        Some("target"),
        Some(&request_digest),
    )
    .unwrap();
}

#[test]
fn exchange_bodies_deflate_and_refuse_a_body_that_inflates_too_far() {
    let body = serde_json::to_vec(&serde_json::json!({"envelopes": vec!["same"; 1_000]})).unwrap();
    let compressed = deflate(&body).unwrap();
    assert!(compressed.len() * 10 < body.len());
    assert_eq!(inflate(&compressed, "test").unwrap(), body);

    let bomb = deflate(&vec![0_u8; MAX_EXCHANGE_BYTES + 1]).unwrap();
    assert!(bomb.len() < 1024 * 1024);
    let refused = inflate(
        &bomb,
        "the answer to `/v1/peer/exchange` from peer invented-peer",
    )
    .unwrap_err()
    .to_string();
    assert_eq!(
        refused,
        format!(
            "the exchange body inflates past {MAX_EXCHANGE_BYTES} bytes (the answer to \
             `/v1/peer/exchange` from peer invented-peer, {} compressed bytes)",
            bomb.len()
        )
    );
    assert!(inflate(b"not deflate", "test").is_err());

    let mut headers = HeaderMap::new();
    assert!(!accepts_deflate(&headers));
    headers.insert(
        "accept-encoding",
        HeaderValue::from_static("gzip, Deflate;q=0.5"),
    );
    assert!(accepts_deflate(&headers));
    headers.insert(
        "accept-encoding",
        HeaderValue::from_static("gzip, deflated"),
    );
    assert!(!accepts_deflate(&headers));
}

#[tokio::test]
async fn the_peer_route_deflates_only_for_a_requester_that_asks() {
    let fleet = "1f91ca65-7793-48cc-866e-ac15690130e1";
    let auth = FleetAuth::test(fleet, &[5; 32]);
    // Enough envelopes that the answer to an empty inventory is worth compressing.
    let target = Arc::new(plain_memory("target").unwrap());
    target.bind_fleet(fleet).unwrap();
    for index in 0..300 {
        target
            .append_claim(&ClaimInput {
                subject: format!("host/peer-{index}"),
                kind: "transport.observed".into(),
                actor: None,
                fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let state = PeerState {
        backend: Local(target),
        node: "target".into(),
        auth: auth.clone(),
        fleet: FleetContext::legacy(BTreeSet::from(["source".into()])),
        outbound_notify: watch::channel(0_u64).0,
        shards: Default::default(),
    };
    let exchange = ReplicationExchange {
        build: None,
        claim_ranges: None,
        projection_digests: Default::default(),
        peer: "source".into(),
        fleet_id: fleet.into(),
        shard_id: None,
        schema_digest: Plain.schema_digest(),
        projection_registry_digest: None,
        projection_versions: Default::default(),
        authority_digest: String::new(),
        graph_digest: String::new(),
        inventory: ReplicationInventory::default(),
        envelopes: Vec::new(),
        signature_requests: Vec::new(),
        signatures: Vec::new(),
    };
    let body = serde_json::to_vec(&exchange).unwrap();
    let request_digest = FleetAuth::body_digest(&body);
    // An older build sends plain JSON and does not ask for compression; a new one sends a
    // compressed request once the peer has answered compressed, and always asks.
    for (compress, ask) in [(false, false), (false, true), (true, true)] {
        let mut request = Request::builder()
            .method("POST")
            .uri(EXCHANGE_PATH)
            .body(Body::from(if compress {
                deflate(&body).unwrap()
            } else {
                body.clone()
            }))
            .unwrap();
        request
            .headers_mut()
            .extend(auth.request_headers("source", &body).unwrap());
        if compress {
            request.headers_mut().insert(
                "content-encoding",
                HeaderValue::from_static(EXCHANGE_ENCODING),
            );
        }
        if ask {
            request.headers_mut().insert(
                "accept-encoding",
                HeaderValue::from_static(EXCHANGE_ENCODING),
            );
        }
        let response = peer_router(state.clone(), Router::new())
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers().clone();
        assert!(
            accepts_deflate(&headers),
            "a new build takes compressed requests"
        );
        assert_eq!(deflated(&headers), ask);
        let response_body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let response_body = if ask {
            inflate(&response_body, "test").unwrap()
        } else {
            response_body.to_vec()
        };
        assert!(response_body.len() >= DEFLATE_MIN_BYTES);
        auth.verify(
            &headers,
            "RESPONSE",
            EXCHANGE_PATH,
            &response_body,
            Some("target"),
            Some(&request_digest),
        )
        .unwrap();
    }
}

#[tokio::test]
async fn signed_peer_exchange_moves_new_authority_in_both_directions() {
    let fleet = "1f91ca65-7793-48cc-866e-ac15690130e1";
    let auth = FleetAuth::test(fleet, &[6; 32]);
    let source = Arc::new(plain_memory("source").unwrap());
    let target = Arc::new(plain_memory("target").unwrap());
    source.bind_fleet(fleet).unwrap();
    target.bind_fleet(fleet).unwrap();
    source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("source-up".into()),
        })
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = PeerState {
        backend: Local(target.clone()),
        node: "target".into(),
        auth: auth.clone(),
        fleet: FleetContext::legacy(BTreeSet::from(["source".into()])),
        outbound_notify: watch::channel(0_u64).0,
        shards: Default::default(),
    };
    let connection_ports = Arc::new(Mutex::new(BTreeSet::new()));
    let observed_ports = connection_ports.clone();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route(EXCHANGE_PATH, post(receive_exchange))
                .layer(axum::middleware::from_fn(
                    move |request: Request<Body>, next: axum::middleware::Next| {
                        let observed_ports = observed_ports.clone();
                        async move {
                            if let Some(axum::extract::ConnectInfo(address)) =
                                request
                                    .extensions()
                                    .get::<axum::extract::ConnectInfo<SocketAddr>>()
                            {
                                observed_ports.lock().unwrap().insert(address.port());
                            }
                            next.run(request).await
                        }
                    },
                ))
                .with_state(state)
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .into_future(),
    );
    let peer = PeerConfig {
        name: "target".into(),
        url: format!("http://{address}"),
    };
    let http = replication_http_client();
    let pushed = exchange(
        &http,
        &Local(source.clone()),
        "source",
        &peer,
        &auth,
        &FleetContext::legacy(BTreeSet::from(["target".into()])),
    )
    .await
    .unwrap();
    assert!(pushed, "the peer stored what this node pushed");
    assert!(
        target
            .latest_claim("host/source", Some("transport.observed"))
            .unwrap()
            .is_some()
    );

    target
        .append_claim(&ClaimInput {
            subject: "host/target".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("target-up".into()),
        })
        .unwrap();
    let pulled = exchange(
        &http,
        &Local(source.clone()),
        "source",
        &peer,
        &auth,
        &FleetContext::legacy(BTreeSet::from(["target".into()])),
    )
    .await
    .unwrap();
    assert!(pulled, "this node stored what the peer sent");
    assert!(
        source
            .latest_claim("host/target", Some("transport.observed"))
            .unwrap()
            .is_some()
    );
    let source_status = source.replication_status(true, Some(fleet), &[]).unwrap();
    let target_status = target.replication_status(true, Some(fleet), &[]).unwrap();
    assert_eq!(
        source_status.authority_digest,
        target_status.authority_digest
    );
    assert_eq!(
        connection_ports.lock().unwrap().len(),
        1,
        "the two-phase exchanges and later wakeup should reuse one TCP connection"
    );
    let moved = exchange(
        &replication_http_client(),
        &Local(source.clone()),
        "source",
        &peer,
        &auth,
        &FleetContext::legacy(BTreeSet::from(["target".into()])),
    )
    .await
    .unwrap();
    assert!(
        !moved,
        "converged nodes store nothing, so the worker may rest"
    );
    assert_eq!(
        connection_ports.lock().unwrap().len(),
        2,
        "a fresh client should demonstrate the former extra dial"
    );
    let summary = source.export_replication_summary(fleet).unwrap();
    assert!(summary.inventory.envelopes.is_empty());
    assert!(!summary.inventory.digest.is_empty());
    let converged = target
        .export_replication_exchange(fleet, &summary.inventory)
        .unwrap();
    assert!(converged.inventory.envelopes.is_empty());
    assert!(converged.envelopes.is_empty());
    server.abort();
}

#[tokio::test]
async fn a_duplicate_inbound_exchange_does_not_wake_outbound_replication() {
    let fleet = "1f91ca65-7793-48cc-866e-ac15690130e1";
    let auth = FleetAuth::test(fleet, &[7; 32]);
    let source = plain_memory("source").unwrap();
    let target = Arc::new(plain_memory("target").unwrap());
    source.bind_fleet(fleet).unwrap();
    target.bind_fleet(fleet).unwrap();
    source
        .append_claim(&ClaimInput {
            subject: "host/source".into(),
            kind: "transport.observed".into(),
            actor: None,
            fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("source-up".into()),
        })
        .unwrap();
    let exchange = source
        .export_replication_exchange(fleet, &ReplicationInventory::default())
        .unwrap();
    let body = Bytes::from(serde_json::to_vec(&exchange).unwrap());
    let (outbound_notify, mut outbound_wake) = watch::channel(0_u64);
    let state = PeerState {
        backend: Local(target),
        node: "target".into(),
        auth: auth.clone(),
        fleet: FleetContext::legacy(BTreeSet::from(["source".into()])),
        outbound_notify,
        shards: Default::default(),
    };

    let first = receive_exchange(
        None,
        State(state.clone()),
        auth.request_headers("source", &body).unwrap(),
        body.clone(),
    )
    .await;
    assert!(first.status().is_success());
    outbound_wake.changed().await.unwrap();
    let _ = outbound_wake.borrow_and_update();

    let duplicate = receive_exchange(
        None,
        State(state.clone()),
        auth.request_headers("source", &body).unwrap(),
        body,
    )
    .await;
    assert!(duplicate.status().is_success());
    assert!(
        !outbound_wake.has_changed().unwrap(),
        "a duplicate receipt must not start a replication echo loop"
    );
}

#[tokio::test]
async fn fresh_retry_after_accepted_but_unanswered_exchange_applies_envelope_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let fleet_id = "1f91ca65-7793-48cc-866e-ac15690130e1";
    let auth = FleetAuth::test(fleet_id, &[7; 32]);
    let source = plain_memory("source").unwrap();
    let target = Arc::new(plain_memory("target").unwrap());
    source.bind_fleet(fleet_id).unwrap();
    target.bind_fleet(fleet_id).unwrap();
    source.append_claim(&ClaimInput {
        subject: "note/copper".into(),
        kind: "example.note".into(),
        actor: None,
        fields: BTreeMap::from([("status".into(), Value::String("up".into()))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some("source-up".into()),
    }).unwrap();
    let request = source.export_replication_exchange(fleet_id, &ReplicationInventory::default()).unwrap();
    let state = PeerState::new(
        Local(Arc::clone(&target)), "target".into(), auth.clone(),
        FleetContext::legacy(BTreeSet::from(["source".into()])),
    );
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&attempts);
    let release = Arc::new(tokio::sync::Notify::new());
    let unblock = Arc::clone(&release);
    let router = peer_router(state, Router::new()).layer(axum::middleware::from_fn(
        move |request: Request<Body>, next: axum::middleware::Next| {
            let seen = Arc::clone(&seen);
            let unblock = Arc::clone(&unblock);
            async move {
                let first = seen.fetch_add(1, Ordering::Relaxed) == 0;
                // Apply and authenticate before losing the first response.
                let response = next.run(request).await;
                if first {
                    unblock.notified().await;
                }
                response
            }
        },
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig { name: "target".into(), url: format!("http://{}", listener.local_addr().unwrap()) };
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    let http = reqwest::Client::builder().timeout(Duration::from_millis(250)).build().unwrap();
    let result = post_signed_to::<_, ReplicationExchange>(
        &http, &peer, "source", &auth,
        &FleetContext::legacy(BTreeSet::from(["target".into()])),
        EXCHANGE_PATH, &request, false,
    ).await;
    release.notify_one();
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
    assert_eq!(target.claims_for("note/copper", Some("example.note")).unwrap().len(), 1);
    server.abort();
    let _ = server.await;
}

#[test]
fn the_dial_set_comes_from_membership_and_skips_dial_out_members() {
    let member =
        |name: &str, state: &str, mode: &str, endpoints: Vec<Value>| crate::fleet::MemberView {
            name: name.into(),
            member_key: format!("{name}-key"),
            node_id: None,
            state: state.into(),
            mode: mode.into(),
            endpoints,
            endpoints_published: true,
            start: 1,
            end: None,
            ended: None,
            removed_by: None,
            removal_reason: None,
        };
    let loopback = |port: u16| {
        vec![serde_json::json!({"transport": "loopback", "address": format!("127.0.0.1:{port}")})]
    };
    let view = FleetView {
        anchor: Some("a-key".into()),
        members: vec![
            member("a", "current", "listening", loopback(1)),
            member("server", "current", "listening", loopback(2)),
            member("laptop", "current", "dial-out", Vec::new()),
            member("gone", "ended", "listening", loopback(3)),
            member("quiet", "current", "listening", Vec::new()),
        ],
        legacy_removed: vec!["old".into()],
    };
    let config = |name: &str, port: u16| PeerConfig {
        name: name.into(),
        url: format!("http://127.0.0.1:{port}"),
    };
    let targets = dial_targets(
        &view,
        "a",
        &[
            config("server", 9002),
            config("laptop", 9003),
            config("gone", 9004),
            config("old", 9005),
            config("legacy", 9006),
            PeerConfig {
                name: "inbound".into(),
                url: String::new(),
            },
        ],
        LocalTransports::default(),
    );
    assert_eq!(
        targets,
        BTreeMap::from([
            (
                "server".to_owned(),
                vec![
                    Route::Http("http://127.0.0.1:9002".into()),
                    Route::Http("http://127.0.0.1:2".into())
                ]
            ),
            (
                "legacy".to_owned(),
                vec![Route::Http("http://127.0.0.1:9006".into())]
            ),
        ])
    );
}

fn transport_order_view() -> FleetView {
    let member = |name: &str, endpoints| crate::fleet::MemberView {
        name: name.into(),
        member_key: format!("{name}-key"),
        state: "current".into(),
        mode: "listening".into(),
        node_id: None,
        endpoints_published: true,
        endpoints,
        start: 1,
        end: None,
        ended: None,
        removed_by: None,
        removal_reason: None,
    };
    FleetView {
        members: vec![
            member(
                "modern",
                vec![
                    serde_json::json!({"transport": "loopback", "address": "127.0.0.1:4000"}),
                    serde_json::json!({"transport": "tailscale", "address": "100.101.102.103:31313"}),
                    serde_json::json!({"transport": "fabric", "node": "modern-node", "protocol": "st3/fleet/x"}),
                ],
            ),
            // An older member need not advertise Fabric to remain reachable.
            member(
                "legacy",
                vec![
                    serde_json::json!({"transport": "tailscale", "address": "100.101.102.104:31313"}),
                ],
            ),
        ],
        ..FleetView::default()
    }
}

#[test]
fn dial_targets_follow_local_transport_order_with_mixed_legacy_endpoints() {
    let view = transport_order_view();
    let tailscale = Route::Http("http://100.101.102.103:31313".into());
    let fabric = Route::Fabric {
        node: "modern-node".into(),
        protocol: "st3/fleet/x".into(),
    };
    let loopback = Route::Http("http://127.0.0.1:4000".into());
    for (names, expected, legacy_enabled) in [
        (
            None,
            vec![tailscale.clone(), fabric.clone(), loopback.clone()],
            true,
        ),
        (
            Some(vec!["fabric", "tailscale"]),
            vec![fabric.clone(), tailscale.clone(), loopback.clone()],
            true,
        ),
        (
            Some(vec!["tailscale", "fabric"]),
            vec![tailscale.clone(), fabric.clone(), loopback.clone()],
            true,
        ),
        (
            Some(vec!["fabric"]),
            vec![fabric, loopback.clone()],
            false,
        ),
        (
            Some(vec!["tailscale"]),
            vec![tailscale, loopback.clone()],
            true,
        ),
        (Some(vec![]), vec![loopback], false),
    ] {
        let names = names.map(|names| names.into_iter().map(String::from).collect::<Vec<_>>());
        let targets = dial_targets(
            &view,
            "own",
            &[],
            LocalTransports::configured(names.as_deref(), true, true),
        );
        let mut expected = BTreeMap::from([("modern".to_owned(), expected)]);
        if legacy_enabled {
            expected.insert(
                "legacy".into(),
                vec![Route::Http("http://100.101.102.104:31313".into())],
            );
        }
        assert_eq!(targets, expected, "{names:?}");
    }
}

#[test]
fn explicit_peer_routes_precede_local_order_and_bypass_disabled_transports() {
    let view = transport_order_view();
    let peers = [
        PeerConfig {
            name: "modern".into(),
            url: "http://100.101.102.105:31313".into(),
        },
        PeerConfig {
            name: "legacy".into(),
            url: "fabric://legacy-node/st3/fleet/x".into(),
        },
        PeerConfig {
            name: "config-only".into(),
            url: "fabric://config-node/st3/fleet/x".into(),
        },
    ];
    for names in [vec!["fabric", "tailscale"], vec![]] {
        let names = names.into_iter().map(String::from).collect::<Vec<_>>();
        let local = LocalTransports::configured(Some(&names), true, true);
        let targets = dial_targets(&view, "own", &peers, local);
        for member in &view.members {
            let peer = peers.iter().find(|peer| peer.name == member.name).unwrap();
            let mut expected = vec![parse_route(&peer.url).unwrap()];
            expected.extend(routes_from_endpoints(&member.endpoints, local));
            assert_eq!(targets[&member.name], expected, "{names:?}");
        }
        assert_eq!(
            targets["config-only"],
            vec![parse_route(&peers[2].url).unwrap()],
            "{names:?}"
        );
    }
}

fn fleet_claim(store: &Store, kind: &str, subject: &str, fields: Value) {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: None,
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn member_context(
    store: &Store,
    own: &MemberKey,
    bootstrap: &[&str],
    config_peers: &[&str],
    legacy: bool,
) -> FleetContext {
    FleetContext::member(store, own, bootstrap, config_peers, legacy)
}

#[tokio::test]
async fn a_stale_worker_refusal_does_not_remove_a_new_local_member_key() {
    let root = tempfile::tempdir().unwrap();
    let state_dir = root.path();
    let file = FleetFile {
        fleet_id: "3b241101-e2bb-4255-8caf-4136c566a962".into(),
        node: Some("birch".into()),
        ..FleetFile::default()
    };
    file.save(state_dir).unwrap();
    let current = MemberKey::load_or_create(&file.node_key_path(state_dir)).unwrap();
    let old = MemberKey::generate().unwrap().0;
    let store = plain_memory("birch").unwrap();
    let mut worker = FleetContext::member(&store, &old, &[], &[], false);
    worker.state_dir = Some(state_dir.to_path_buf());
    worker.mark_removed(
        "alder",
        &RemovedFromFleet {
            code: "member-left".into(),
            message: "the previous birch incarnation left".into(),
        },
    )
    .await;
    assert!(worker.is_removed(), "the old worker still stops syncing");
    assert_eq!(FleetFile::load(state_dir).unwrap().unwrap(), file);
    assert!(!crate::fleet::RemovalNotice::path(state_dir).exists());
    assert_eq!(
        MemberKey::load(&file.node_key_path(state_dir))
            .unwrap()
            .public(),
        current.public()
    );
}

#[tokio::test]
async fn a_current_worker_refusal_keeps_its_notice_and_settings_durable() {
    let root = tempfile::tempdir().unwrap();
    let state_dir = root.path();
    let file = FleetFile {
        fleet_id: "3b241101-e2bb-4255-8caf-4136c566a962".into(),
        ..FleetFile::default()
    };
    file.save(state_dir).unwrap();
    let own = MemberKey::load_or_create(&file.node_key_path(state_dir)).unwrap();
    let store = plain_memory("birch").unwrap();
    let mut worker = FleetContext::member(&store, &own, &[], &[], false);
    worker.state_dir = Some(state_dir.to_path_buf());
    worker.mark_removed(
        "alder",
        &RemovedFromFleet {
            code: "member-left".into(),
            message: "birch left the fleet".into(),
        },
    )
    .await;
    let notice = crate::fleet::RemovalNotice::load(state_dir).unwrap();
    assert_eq!(notice.reported_by, "alder");
    assert_eq!(notice.code, "member-left");
    assert_eq!(notice.message, "birch left the fleet");
    assert!(notice.learned_at_unix_ms > 0);
    assert!(
        FleetFile::load(state_dir)
            .unwrap()
            .unwrap()
            .removed
            .is_some()
    );
}

/// Endpoint publication holds the settings lock across a request to the daemon. A refusal that
/// arrives meanwhile waits for the lock without stopping the runtime that request needs, so the
/// lock is released and the removal is written.
#[test]
fn a_refusal_waits_for_the_settings_lock_without_blocking_the_runtime() {
    let root = tempfile::tempdir().unwrap();
    let state_dir = root.path().to_path_buf();
    let file = FleetFile {
        fleet_id: "3b241101-e2bb-4255-8caf-4136c566a962".into(),
        ..FleetFile::default()
    };
    file.save(&state_dir).unwrap();
    let (finished, done) = std::sync::mpsc::channel();
    let directory = state_dir.clone();
    // A runtime with one thread: the refusal and the request holding the lock share it.
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let own = MemberKey::load_or_create(&file.node_key_path(&directory)).unwrap();
            let store = plain_memory("birch").unwrap();
            let mut worker = FleetContext::member(&store, &own, &[], &[], false);
            worker.state_dir = Some(directory.clone());
            let lock = FleetFile::lock_settings(&directory).unwrap();
            let publication = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                drop(lock);
            });
            worker
                .mark_removed(
                    "alder",
                    &RemovedFromFleet {
                        code: "member-removed".into(),
                        message: "birch was removed".into(),
                    },
                )
                .await;
            publication.await.unwrap();
        });
        finished.send(()).unwrap();
    });
    done.recv_timeout(Duration::from_secs(30))
        .expect("the refusal blocked the runtime that holds the settings lock");
    assert!(
        FleetFile::load(&state_dir)
            .unwrap()
            .unwrap()
            .removed
            .is_some()
    );
}

#[tokio::test]
async fn pairing_admission_binds_signed_membership_before_sealing_the_answer() {
    use crate::fleet::code::{JoinCode, fingerprint};
    use crate::fleet::handshake::{JoinResponse, Joiner};
    let fleet = "3b241101-e2bb-4255-8caf-4136c566a962";
    let anchor = Arc::new(MemberKey::generate().unwrap().0);
    let member = MemberKey::generate().unwrap().0;
    let store = Arc::new(plain_memory("a").unwrap());
    store.bind_fleet(fleet).unwrap();
    store.pin_fleet_anchor(anchor.public()).unwrap();
    store.set_member_key(Some(anchor.clone())).unwrap();
    fleet_claim(&store, "fleet.member-admitted", "host/a", serde_json::json!({
        "fleet_id": fleet, "member_key": anchor.public(), "via": "anchor", "mode": "listening"
    }));
    let invitation = store.create_fleet_invite(
        Some("b"), Duration::from_secs(600), &["fabric".into()], "person/test", false,
    ).unwrap();
    let code = JoinCode {
        fleet_id: uuid::Uuid::parse_str(fleet).unwrap(),
        invite: hex::decode(invitation.invite).unwrap().try_into().unwrap(),
        token: invitation.token,
        fingerprint: fingerprint(anchor.public()),
        expires_at: invitation.expires_at_unix_ms / 1000,
        name: Some("b".into()),
        migrate: false,
        endpoints: Vec::new(),
        pairing: None,
    };
    let node_id = "a".repeat(64);
    let session = Joiner::start_bound(&code, &member, "b", "dial-out", None, "test", Some(node_id.clone())).unwrap();
    let state = PeerState {
        backend: Local(store.clone()),
        node: "a".into(),
        auth: FleetAuth::test(fleet, &[9; 32]).with_member_key(Some(anchor.clone())),
        fleet: member_context(&store, &anchor, &[], &[], false),
        outbound_notify: watch::channel(0_u64).0,
        shards: Default::default(),
    };
    let body = serde_json::json!({
        "request": session.request(), "node_id": node_id,
        "expected_fleet_id": fleet, "expected_sponsor_key": anchor.public()
    });
    let response = receive_pairing_admission(State(state.clone()), Bytes::from(serde_json::to_vec(&body).unwrap())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), MAX_JOIN_BYTES).await.unwrap();
    let answer: JoinResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(session.open(&answer).unwrap().fleet_id, fleet);
    store.verify_fleet_pairing_admission("b", member.public(), &node_id).unwrap();
    let view = store.fleet_view().unwrap();
    let admitted = view.current("b");
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].member_key, member.public());
    assert_eq!(admitted[0].node_id.as_deref(), Some(node_id.as_str()));
    assert!(admitted[0].endpoints.is_empty(), "admission must not wait for joiner endpoint publication");
    assert!(!admitted[0].endpoints_published);
    let encoded = serde_json::to_value(&view).unwrap();
    assert_eq!(serde_json::from_value::<FleetView>(encoded).unwrap(), view);
    assert!(store.verify_fleet_pairing_admission("b", member.public(), &"b".repeat(64)).is_err());
    assert!(store.verify_fleet_pairing_admission("b", anchor.public(), &node_id).is_err());
    let mut wrong_fleet = body.clone();
    wrong_fleet["expected_fleet_id"] = serde_json::json!("3b241101-e2bb-4255-8caf-4136c566a963");
    let refused = receive_pairing_admission(
        State(state.clone()), Bytes::from(serde_json::to_vec(&wrong_fleet).unwrap()),
    ).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let mut wrong_node = body;
    wrong_node["node_id"] = serde_json::json!("b".repeat(64));
    let refused = receive_pairing_admission(State(state), Bytes::from(serde_json::to_vec(&wrong_node).unwrap())).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn members_exchange_with_signatures_and_a_removed_member_is_refused() {
    let fleet = "3b241101-e2bb-4255-8caf-4136c566a962";
    let secret = [9_u8; 32];
    let anchor = Arc::new(MemberKey::generate().unwrap().0);
    let member = Arc::new(MemberKey::generate().unwrap().0);
    let a = Arc::new(plain_memory("a").unwrap());
    let b = Arc::new(plain_memory("b").unwrap());
    for (store, key) in [(&a, &anchor), (&b, &member)] {
        store.bind_fleet(fleet).unwrap();
        store.pin_fleet_anchor(anchor.public()).unwrap();
        store.set_member_key(Some(key.clone())).unwrap();
    }
    let admitted = |name: &str, key: &MemberKey, via: &str| {
        serde_json::json!({
            "fleet_id": fleet, "member_key": key.public(), "via": via, "mode": "listening"
        })
        .as_object()
        .map(|fields| {
            fleet_claim(
                &a,
                "fleet.member-admitted",
                &format!("host/{name}"),
                Value::Object(fields.clone()),
            )
        })
    };
    admitted("a", &anchor, "anchor");
    admitted("b", &member, "invite");

    // a listens as a member, without legacy acceptance.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let a_fleet = member_context(&a, &anchor, &[], &[], false);
    let state = PeerState {
        backend: Local(a.clone()),
        node: "a".into(),
        auth: FleetAuth::test(fleet, &secret).with_member_key(Some(anchor.clone())),
        fleet: a_fleet.clone(),
        outbound_notify: watch::channel(0_u64).0,
        shards: Default::default(),
    };
    let server = tokio::spawn(async move {
        axum::serve(listener, peer_router(state, Router::new()))
            .await
            .unwrap();
    });
    let peer = PeerConfig {
        name: "a".into(),
        url: format!("http://{address}"),
    };
    let http = replication_http_client();
    let b_auth = FleetAuth::test(fleet, &secret).with_member_key(Some(member.clone()));
    // b knows nothing yet but its anchor, which is enough to trust a's answer.
    let b_fleet = member_context(&b, &member, &[anchor.public()], &[], false);
    exchange(&http, &Local(b.clone()), "b", &peer, &b_auth, &b_fleet)
        .await
        .unwrap();
    assert!(matches!(
        b.fleet_membership().unwrap().state("b"),
        crate::fleet::MemberState::Current(_)
    ));

    // Without its member key, b is refused.
    let unsigned = exchange(
        &http,
        &Local(b.clone()),
        "b",
        &peer,
        &FleetAuth::test(fleet, &secret),
        &b_fleet,
    )
    .await
    .unwrap_err();
    assert!(
        unsigned.to_string().contains("member-signature-required"),
        "{unsigned:#}"
    );
    let failure = unsigned.downcast_ref::<ReplicationExchangeFailure>().unwrap();
    assert_eq!(failure.code, "member-signature-required");
    assert_eq!(failure.phase, ReplicationFailurePhase::Verify);
    assert_eq!(failure.http_status, Some(401));

    // a removes b; b's next exchange gets a signed refusal that names its key.
    fleet_claim(
        &a,
        "fleet.member-removed",
        "host/b",
        serde_json::json!({"member_key": member.public(), "high_water": 100, "reason": "test"}),
    );
    *a_fleet.view.write().unwrap() = a.fleet_view().unwrap();
    *b_fleet.view.write().unwrap() = b.fleet_view().unwrap();
    let refused = exchange(&http, &Local(b.clone()), "b", &peer, &b_auth, &b_fleet)
        .await
        .unwrap_err();
    let removed = refused
        .downcast_ref::<RemovedFromFleet>()
        .expect("a signed refusal naming b's key");
    assert_eq!(removed.code, "member-removed");

    // Another machine without a key, posing as b or as an unknown name, is refused too.
    // It accepts a's answers as a legacy config peer, so it sees a's refusal itself.
    let stranger = FleetContext::legacy(BTreeSet::from(["a".into()]));
    for name in ["b", "stranger"] {
        let error = exchange(
            &http,
            &Local(b.clone()),
            name,
            &peer,
            &FleetAuth::test(fleet, &secret),
            &stranger,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("403"), "{error:#}");
    }
    server.abort();
}

// Fixture imports must run the same admission and projection phases as Local::receive;
// storing wire envelopes alone leaves membership claims pending.
fn admit_fixture_exchange(
    store: &Store,
    peer: &str,
    fleet_id: &str,
    exchange: &ReplicationExchange,
) {
    store
        .receive_replication_exchange(peer, fleet_id, exchange)
        .unwrap();
    store.validate_replication_backlog().unwrap();
    store.apply_replication_repairs().unwrap();
    store.project_replication_backlog().unwrap();
}

/// The HTTP tunnel below is the scoped Fabric boundary: there is no static peer
/// fallback, and its availability is derived solely from runtime desired policy.
struct LeaveFixture {
    receiver: PeerState<Local<Store>>,
    leaving: Arc<Store>,
    leaving_fleet: FleetContext,
    leaving_auth: FleetAuth,
    key: Arc<MemberKey>,
    key_bytes: Vec<u8>,
    anchor_bytes: Vec<u8>,
    other_key: Arc<MemberKey>,
}

fn leave_fixture() -> LeaveFixture {
    let fleet = "3b241101-e2bb-4255-8caf-4136c566a962";
    let (anchor, anchor_bytes) = MemberKey::generate().unwrap();
    let anchor = Arc::new(anchor);
    let (member, key_bytes) = MemberKey::generate().unwrap();
    let key = Arc::new(member);
    let other_key = Arc::new(MemberKey::generate().unwrap().0);
    let a = Arc::new(plain_memory("a").unwrap());
    let b = Arc::new(plain_memory("b").unwrap());
    for (store, key) in [(&a, &anchor), (&b, &key)] {
        store.bind_fleet(fleet).unwrap();
        store.pin_fleet_anchor(anchor.public()).unwrap();
        store.set_member_key(Some(key.clone())).unwrap();
    }
    for (name, member, via) in [
        ("a", &anchor, "anchor"),
        ("b", &key, "invite"),
        ("c", &other_key, "invite"),
    ] {
        fleet_claim(
            &a,
            "fleet.member-admitted",
            &format!("host/{name}"),
            serde_json::json!({"fleet_id": fleet, "member_key": member.public(),
                "via": via, "mode": "listening"}),
        );
    }
    fleet_claim(
        &a,
        "fleet.member-endpoints",
        "host/a",
        serde_json::json!({"member_key": anchor.public(), "mode": "listening",
            "endpoints": [{"transport": "fabric", "node": "fabric-a",
                "protocol": default_fabric_protocol(fleet)}]}),
    );
    let shared = a
        .export_replication_exchange(fleet, &ReplicationInventory::default())
        .unwrap();
    assert!(
        matches!(
            a.fleet_membership().unwrap().state("a"),
            crate::fleet::MemberState::Current(_)
        ),
        "the fixture anchor must be admitted before transport: {:?}",
        a.fleet_view().unwrap()
    );
    assert!(
        shared.envelopes.iter().all(|envelope| {
            envelope.member_key.as_deref() == Some(anchor.public())
                && envelope.signature.as_deref().is_some_and(|signature| {
                    crate::fleet::verify_signature(
                        anchor.public(),
                        &crate::fleet::envelope_signature_message(
                            fleet,
                            &envelope.writer,
                            envelope.sequence,
                            &envelope.hash,
                        ),
                        signature,
                    )
                })
        }),
        "fixture envelopes must carry valid anchor signatures"
    );
    admit_fixture_exchange(&b, "a", fleet, &shared);
    assert!(
        matches!(
            b.fleet_membership().unwrap().state("a"),
            crate::fleet::MemberState::Current(_)
        ),
        "initial fixture replication must import the anchor: {:?}; unresolved: {:?}",
        b.fleet_view().unwrap(),
        b.replica_records(true).unwrap()
    );
    fleet_claim(
        &b,
        "fleet.member-endpoints",
        "host/b",
        serde_json::json!({"member_key": key.public(), "mode": "listening",
            "endpoints": [{"transport": "fabric", "node": "fabric-b",
                "protocol": default_fabric_protocol(fleet)}]}),
    );
    let advertised = b
        .export_replication_exchange(fleet, &ReplicationInventory::default())
        .unwrap();
    admit_fixture_exchange(&a, "b", fleet, &advertised);
    // The product runtime stamps a zero leave high-water with its envelope sequence.
    // Plain deliberately has no product hooks, so supply the same signed sequence here.
    let high_water = b
        .replication_inventory()
        .unwrap()
        .envelopes
        .iter()
        .filter(|envelope| envelope.writer == "b")
        .map(|envelope| envelope.sequence)
        .max()
        .unwrap_or(0)
        + 1;
    fleet_claim(
        &b,
        "fleet.member-left",
        "host/b",
        serde_json::json!({"member_key": key.public(), "high_water": high_water}),
    );
    assert!(
        matches!(
            a.fleet_membership().unwrap().state("b"),
            crate::fleet::MemberState::Current(_)
        ),
        "the receiver must admit the leaving member before transport: {:?}",
        a.fleet_view().unwrap()
    );
    assert!(
        matches!(
            b.fleet_membership().unwrap().state("a"),
            crate::fleet::MemberState::Current(_)
        ),
        "the anchor remains admitted after another member leaves: {:?}",
        b.fleet_view().unwrap()
    );
    assert!(
        matches!(
            b.fleet_membership().unwrap().state("b"),
            crate::fleet::MemberState::Ended(member)
                if member.ended.as_deref() == Some("left")
        ),
        "the source must contain its signed leave before transport: {:?}",
        b.fleet_view().unwrap()
    );
    LeaveFixture {
        receiver: PeerState::new(
            Local(a.clone()),
            "a".into(),
            FleetAuth::test(fleet, &[9; 32]).with_member_key(Some(anchor.clone())),
            member_context(&a, &anchor, &[], &[], false),
        ),
        leaving_fleet: member_context(&b, &key, &[], &[], false),
        leaving_auth: FleetAuth::test(fleet, &[9; 32]).with_member_key(Some(key.clone())),
        leaving: b,
        key,
        key_bytes,
        anchor_bytes,
        other_key,
    }
}

async fn retained_for_test(fleet: &FleetContext) -> BTreeSet<String> {
    let leave = fleet.leave.lock().await;
    retained_leave_members(
        &fleet.view.read().unwrap(),
        fleet.own_key.as_deref().unwrap(),
        &leave,
    )
}

#[tokio::test]
async fn runtime_only_leave_keeps_both_carriers_until_signed_proof_receipt() {
    let fixture = leave_fixture();
    let receiver = fixture.receiver.clone();
    let receiver_fleet = receiver.fleet.clone();
    let leaving_fleet = fixture.leaving_fleet.clone();
    let key = fixture.key.public().to_owned();
    let observed_key = key.clone();
    let router = peer_router(receiver, Router::new()).layer(axum::middleware::from_fn(
        move |request: Request<Body>, next: axum::middleware::Next| {
            let receiver = receiver_fleet.clone();
            let leaving = leaving_fleet.clone();
            let key = observed_key.clone();
            async move {
                let view = receiver.view.read().unwrap().clone();
                let retained = retained_leave_members(
                    &view,
                    receiver.own_key.as_deref().unwrap(),
                    &*receiver.leave.lock().await,
                );
                let current = view
                    .members
                    .iter()
                    .any(|member| member.member_key == key && member.state == "current");
                assert!(
                    current || retained.contains(&key),
                    "runtime-only tunnel was retired before request"
                );
                if request.uri().path() == LEAVE_RECEIPT_PATH {
                    assert!(
                        leaving.leave.lock().await.pending.contains_key("a"),
                        "receipt must follow delivery and authentication of the signed-left proof"
                    );
                    assert!(!receiver.leave.lock().await.completed.contains(&key));
                }
                next.run(request).await
            }
        },
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "a".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    let http = replication_http_client();
    exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap();
    assert!(
        retained_for_test(&fixture.receiver.fleet)
            .await
            .contains(&key),
        "receiving the signed leave must not revoke before exporting the response"
    );
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<RemovedFromFleet>().unwrap().code,
        "member-left"
    );
    assert!(
        fixture
            .receiver
            .fleet
            .leave
            .lock()
            .await
            .completed
            .contains(&key)
    );
    assert!(
        !retained_for_test(&fixture.receiver.fleet)
            .await
            .contains(&key)
    );
    assert!(
        retained_for_test(&fixture.leaving_fleet)
            .await
            .contains(&key),
        "only CLI cleanup retires the leaving node's carrier"
    );
    assert!(
        fixture
            .receiver
            .fleet
            .accept(&Sender {
                name: "b".into(),
                member_key: Some(key),
                member_signature_valid: true
            })
            .is_err(),
        "receipts do not reopen ordinary replication"
    );
    server.abort();
}

#[tokio::test]
async fn lost_leave_receipt_keeps_proof_and_carrier_for_retry() {
    let mut fixture = leave_fixture();
    let root = tempfile::tempdir().unwrap();
    let settings = FleetFile {
        fleet_id: fixture.leaving_auth.fleet_id().into(),
        node: Some("b".into()),
        ..FleetFile::default()
    };
    settings.save(root.path()).unwrap();
    crate::fleet::join::write_private(&settings.node_key_path(root.path()), &fixture.key_bytes)
        .unwrap();
    fixture.leaving_fleet.state_dir = Some(root.path().to_path_buf());
    let receiver_fleet = fixture.receiver.fleet.clone();
    let lose_receipt = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let lose = lose_receipt.clone();
    let router =
        peer_router(fixture.receiver.clone(), Router::new()).layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: axum::middleware::Next| {
                let lose = lose.clone();
                async move {
                    if request.uri().path() == LEAVE_RECEIPT_PATH
                        && lose.load(std::sync::atomic::Ordering::Relaxed)
                    {
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                    next.run(request).await
                }
            },
        ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "a".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    let http = replication_http_client();
    exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap();
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert!(
        error.downcast_ref::<RemovedFromFleet>().is_none(),
        "generic receipt loss is not completion"
    );
    assert!(!fixture.leaving_fleet.is_removed());
    assert!(
        fixture
            .leaving_fleet
            .leave
            .lock()
            .await
            .pending
            .contains_key("a")
    );
    assert!(receiver_fleet.leave.lock().await.completed.is_empty());
    let persisted = fs::read(root.path().join("fleet/leave-carriers.json")).unwrap();
    *fixture.leaving_fleet.leave.lock().await = serde_json::from_slice(&persisted).unwrap();
    assert_eq!(
        fixture
            .leaving_fleet
            .leave
            .lock()
            .await
            .confirmer
            .as_deref(),
        fixture.receiver.fleet.own_key.as_deref(),
        "restart must keep the exact confirmer incarnation"
    );
    lose_receipt.store(false, std::sync::atomic::Ordering::Relaxed);
    let mut forged = fixture.leaving_fleet.leave.lock().await.pending["a"].clone();
    forged.body.push(' ');
    let refused = post_signed_to::<_, Value>(
        &http,
        &peer,
        "b",
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
        LEAVE_RECEIPT_PATH,
        &forged,
        false,
    )
    .await
    .unwrap_err();
    assert!(refused.to_string().contains("403"), "{refused:#}");
    assert!(
        receiver_fleet.leave.lock().await.completed.is_empty(),
        "an ended member cannot invent or tamper with a receipt proof"
    );
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<RemovedFromFleet>().unwrap().code,
        "member-left"
    );
    assert!(
        receiver_fleet
            .leave
            .lock()
            .await
            .completed
            .contains(fixture.key.public())
    );
    server.abort();
}

#[tokio::test]
async fn cut_receipt_response_remains_unconfirmed_without_a_signed_ack() {
    let fixture = leave_fixture();
    let receiver_fleet = fixture.receiver.fleet.clone();
    let router =
        peer_router(fixture.receiver.clone(), Router::new()).layer(axum::middleware::from_fn(
            |request: Request<Body>, next: axum::middleware::Next| async move {
                let receipt = request.uri().path() == LEAVE_RECEIPT_PATH;
                let response = next.run(request).await;
                if receipt {
                    StatusCode::BAD_GATEWAY.into_response()
                } else {
                    response
                }
            },
        ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "a".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    let http = replication_http_client();
    exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap();
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert!(error.downcast_ref::<RemovedFromFleet>().is_none());
    assert!(
        receiver_fleet
            .leave
            .lock()
            .await
            .completed
            .contains(fixture.key.public())
    );
    assert!(
        fixture
            .leaving_fleet
            .leave
            .lock()
            .await
            .pending
            .contains_key("a")
    );
    assert!(
        !fixture.leaving_fleet.is_removed(),
        "carrier withdrawal is not a receipt ACK: it can also mean deadline expiry"
    );
    server.abort();
}

#[tokio::test]
async fn a_durable_confirmer_fences_competing_push_and_pull_exports_even_with_a_stale_view() {
    let fixture = leave_fixture();
    let backend = Local(fixture.leaving.clone());
    // The payload was exported before checking the fresh membership generation.
    let payload = fixture
        .leaving
        .export_replication_exchange(
            fixture.leaving_auth.fleet_id(),
            &ReplicationInventory::default(),
        )
        .unwrap();
    select_leave_confirmer(&backend, &fixture.leaving_fleet, "a")
        .await
        .unwrap();
    let persisted = serde_json::to_vec(&*fixture.leaving_fleet.leave.lock().await).unwrap();
    *fixture.leaving_fleet.leave.lock().await = serde_json::from_slice(&persisted).unwrap();
    // Deliberately stale worker view: selector authority is the backend's signed view.
    *fixture.leaving_fleet.view.write().unwrap() =
        fixture.receiver.fleet.view.read().unwrap().clone();
    let other = PeerConfig {
        name: "c".into(),
        url: "http://127.0.0.1:1".into(),
    };
    let error = post_signed(
        &replication_http_client(),
        &backend,
        &other,
        "b",
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
        &payload,
        false,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("selected member"), "{error:#}");

    let c = Arc::new(plain_memory("c").unwrap());
    c.bind_fleet(fixture.leaving_auth.fleet_id()).unwrap();
    c.pin_fleet_anchor(fixture.receiver.fleet.own_key.as_deref().unwrap())
        .unwrap();
    c.set_member_key(Some(fixture.other_key.clone())).unwrap();
    let shared = fixture
        .receiver
        .backend
        .0
        .export_replication_exchange(
            fixture.leaving_auth.fleet_id(),
            &ReplicationInventory::default(),
        )
        .unwrap();
    admit_fixture_exchange(&c, "a", fixture.leaving_auth.fleet_id(), &shared);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "b".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let router = peer_router(
        PeerState::new(
            backend,
            "b".into(),
            fixture.leaving_auth.clone(),
            fixture.leaving_fleet.clone(),
        ),
        Router::new(),
    );
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    let error = exchange(
        &replication_http_client(),
        &Local(c.clone()),
        "c",
        &peer,
        &FleetAuth::test(fixture.leaving_auth.fleet_id(), &[9; 32])
            .with_member_key(Some(fixture.other_key.clone())),
        &member_context(&c, &fixture.other_key, &[], &[], false),
    )
    .await
    .unwrap_err();
    // The listener answers a typed 500: its free-form refusal prose is withheld from the dialer
    // (retained exchange failures), so the state checks below carry what the refusal was.
    assert!(error.to_string().contains("replication-peer-rejected"), "{error:#}");
    assert!(
        c.fleet_view()
            .unwrap()
            .members
            .iter()
            .any(|member| member.name == "b" && member.state == "current"),
        "the competing pull must never receive the self-left payload"
    );
    server.abort();
}

#[tokio::test]
async fn pulling_the_selected_members_leave_retains_only_its_direct_carrier() {
    let fixture = leave_fixture();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "b".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let router = peer_router(
        PeerState::new(
            Local(fixture.leaving.clone()),
            "b".into(),
            fixture.leaving_auth.clone(),
            fixture.leaving_fleet.clone(),
        ),
        Router::new(),
    );
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    // A normal follow-up may refuse the now-ended responder, but the direct receive
    // itself must retain its runtime-only carrier for the subsequent leave proof.
    let _ = exchange(
        &replication_http_client(),
        &fixture.receiver.backend,
        "a",
        &peer,
        &fixture.receiver.auth,
        &fixture.receiver.fleet,
    )
    .await;
    assert!(
        fixture
            .receiver
            .fleet
            .leave
            .lock()
            .await
            .awaiting
            .contains(fixture.key.public())
    );
    assert!(
        retained_for_test(&fixture.receiver.fleet)
            .await
            .contains(fixture.key.public())
    );
    assert_eq!(
        fixture
            .leaving_fleet
            .leave
            .lock()
            .await
            .confirmer
            .as_deref(),
        fixture.receiver.fleet.own_key.as_deref()
    );
    // A third-party relay does not acquire a direct leave-confirmation carrier.
    let relay = Arc::new(plain_memory("c").unwrap());
    relay.bind_fleet(fixture.leaving_auth.fleet_id()).unwrap();
    relay
        .pin_fleet_anchor(fixture.receiver.fleet.own_key.as_deref().unwrap())
        .unwrap();
    relay
        .set_member_key(Some(fixture.other_key.clone()))
        .unwrap();
    let before = fixture
        .receiver
        .backend
        .0
        .export_replication_exchange(
            fixture.leaving_auth.fleet_id(),
            &ReplicationInventory::default(),
        )
        .unwrap();
    admit_fixture_exchange(&relay, "a", fixture.leaving_auth.fleet_id(), &before);
    let relayed_context = member_context(&relay, &fixture.other_key, &[], &[], false);
    assert!(
        !retained_for_test(&relayed_context)
            .await
            .contains(fixture.key.public())
    );
    server.abort();
}

#[tokio::test]
async fn lost_leave_exchange_response_keeps_the_selected_receiver_retryable() {
    let fixture = leave_fixture();
    let receiver_fleet = fixture.receiver.fleet.clone();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let drop_once = dropped.clone();
    let router =
        peer_router(fixture.receiver.clone(), Router::new()).layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: axum::middleware::Next| {
                let receiver = receiver_fleet.clone();
                let dropped = drop_once.clone();
                async move {
                    let exchange = request.uri().path() == EXCHANGE_PATH;
                    let response = next.run(request).await;
                    let admitted_leave =
                        receiver.view.read().unwrap().members.iter().any(|member| {
                            member.name == "b" && member.ended.as_deref() == Some("left")
                        });
                    if exchange
                        && admitted_leave
                        && !dropped.swap(true, std::sync::atomic::Ordering::Relaxed)
                    {
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                    response
                }
            },
        ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "a".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    let http = replication_http_client();
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert!(dropped.load(std::sync::atomic::Ordering::Relaxed));
    assert!(error.downcast_ref::<RemovedFromFleet>().is_none());
    assert!(
        fixture
            .receiver
            .fleet
            .leave
            .lock()
            .await
            .awaiting
            .contains(fixture.key.public())
    );
    assert!(
        fixture.leaving_fleet.leave.lock().await.pending.is_empty(),
        "a lost initial response is not yet an authenticated leave proof"
    );
    assert!(
        select_leave_confirmer(&Local(fixture.leaving.clone()), &fixture.leaving_fleet, "c")
            .await
            .is_err(),
        "an ambiguous send cannot switch confirmers and strand the original receiver"
    );
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<RemovedFromFleet>().unwrap().code,
        "member-left"
    );
    assert!(
        fixture
            .receiver
            .fleet
            .leave
            .lock()
            .await
            .completed
            .contains(fixture.key.public())
    );
    server.abort();
}

#[tokio::test]
async fn dial_out_pull_cannot_pin_leave_and_a_listening_confirmer_can_finish() {
    let fixture = leave_fixture();
    let fleet_id = fixture.leaving_auth.fleet_id();
    let c = Arc::new(plain_memory("c").unwrap());
    c.bind_fleet(fleet_id).unwrap();
    c.pin_fleet_anchor(fixture.receiver.fleet.own_key.as_deref().unwrap())
        .unwrap();
    c.set_member_key(Some(fixture.other_key.clone())).unwrap();
    let shared = fixture
        .receiver
        .backend
        .0
        .export_replication_exchange(fleet_id, &ReplicationInventory::default())
        .unwrap();
    admit_fixture_exchange(&c, "a", fleet_id, &shared);
    fleet_claim(
        &c,
        "fleet.member-endpoints",
        "host/c",
        serde_json::json!({"member_key": fixture.other_key.public(),
            "mode": "dial-out", "endpoints": []}),
    );
    let dial_out = c
        .export_replication_exchange(fleet_id, &ReplicationInventory::default())
        .unwrap();
    admit_fixture_exchange(&fixture.leaving, "c", fleet_id, &dial_out);
    refresh_fleet_view(&Local(fixture.leaving.clone()), &fixture.leaving_fleet).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let leaver = PeerConfig {
        name: "b".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let router = peer_router(
        PeerState::new(
            Local(fixture.leaving.clone()),
            "b".into(),
            fixture.leaving_auth.clone(),
            fixture.leaving_fleet.clone(),
        ),
        Router::new(),
    );
    let leaver_server = tokio::spawn(axum::serve(listener, router).into_future());
    let error = exchange(
        &replication_http_client(),
        &Local(c.clone()),
        "c",
        &leaver,
        &FleetAuth::test(fleet_id, &[9; 32]).with_member_key(Some(fixture.other_key.clone())),
        &member_context(&c, &fixture.other_key, &[], &[], false),
    )
    .await
    .unwrap_err();
    // The listener answers a typed 500: its free-form refusal prose is withheld from the dialer
    // (retained exchange failures), so the state checks below carry what the refusal was.
    assert!(error.to_string().contains("replication-peer-rejected"), "{error:#}");
    assert!(
        fixture.leaving_fleet.leave.lock().await.confirmer.is_none(),
        "a dial-out pull must not strand the durable selector"
    );
    assert!(
        c.fleet_view()
            .unwrap()
            .members
            .iter()
            .any(|member| member.name == "b" && member.state == "current"),
        "reject the requester before delivering the self-left payload"
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let confirmer = PeerConfig {
        name: "a".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let confirmer_server = tokio::spawn(
        axum::serve(
            listener,
            peer_router(fixture.receiver.clone(), Router::new()),
        )
        .into_future(),
    );
    let http = replication_http_client();
    exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &confirmer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap();
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &confirmer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<RemovedFromFleet>().unwrap().code,
        "member-left"
    );
    assert_eq!(
        fixture
            .leaving_fleet
            .leave
            .lock()
            .await
            .confirmer
            .as_deref(),
        fixture.receiver.fleet.own_key.as_deref()
    );
    assert!(
        fixture
            .receiver
            .fleet
            .leave
            .lock()
            .await
            .completed
            .contains(fixture.key.public())
    );
    leaver_server.abort();
    confirmer_server.abort();
}

#[tokio::test]
async fn legacy_hmac_only_receipt_ack_cannot_complete_leave() {
    let mut fixture = leave_fixture();
    fixture.leaving_fleet.legacy = true;
    fixture.leaving_fleet.config_peers.insert("a".into());
    assert_eq!(
        fixture.leaving_fleet.accept(&Sender {
            name: "a".into(),
            member_key: None,
            member_signature_valid: false
        }),
        Ok(Acceptance::Legacy)
    );
    let hmac_state = PeerState::new(
        fixture.receiver.backend.clone(),
        "a".into(),
        FleetAuth::test(fixture.leaving_auth.fleet_id(), &[9; 32]),
        fixture.receiver.fleet.clone(),
    );
    let key = fixture.key.public().to_owned();
    let router =
        peer_router(fixture.receiver.clone(), Router::new()).layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: axum::middleware::Next| {
                let hmac_state = hmac_state.clone();
                let key = key.clone();
                async move {
                    let path = request.uri().path().to_owned();
                    if path == LEAVE_RECEIPT_PATH || path == "/test/legacy" {
                        let body = to_bytes(request.into_body(), MAX_EXCHANGE_BYTES)
                            .await
                            .unwrap();
                        return signed_response_for(
                            &hmac_state,
                            &path,
                            &FleetAuth::body_digest(&body),
                            0,
                            serde_json::json!({"acknowledged": key}),
                        )
                        .unwrap();
                    }
                    next.run(request).await
                }
            },
        ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "a".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    let http = replication_http_client();
    exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap();
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("not signed by the selected confirmer"),
        "{error:#}"
    );
    assert!(error.downcast_ref::<RemovedFromFleet>().is_none());
    assert!(
        fixture
            .leaving_fleet
            .leave
            .lock()
            .await
            .pending
            .contains_key("a")
    );
    assert!(!fixture.leaving_fleet.is_removed());
    assert!(
        fixture
            .receiver
            .fleet
            .leave
            .lock()
            .await
            .completed
            .is_empty()
    );
    let (legacy, _): (Value, bool) = post_signed_to(
        &http,
        &peer,
        "b",
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
        "/test/legacy",
        &serde_json::json!({}),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        legacy["acknowledged"],
        fixture.key.public(),
        "receipt authentication must not disable ordinary configured legacy responses"
    );
    server.abort();
}

#[tokio::test]
async fn withheld_receipt_expires_after_restart_without_confirming_the_sender() {
    let mut fixture = leave_fixture();
    let root = tempfile::tempdir().unwrap();
    let settings = FleetFile {
        fleet_id: fixture.leaving_auth.fleet_id().into(),
        node: Some("a".into()),
        ..FleetFile::default()
    };
    settings.save(root.path()).unwrap();
    crate::fleet::join::write_private(&settings.node_key_path(root.path()), &fixture.anchor_bytes)
        .unwrap();
    fixture.receiver.fleet.state_dir = Some(root.path().to_path_buf());
    let router =
        peer_router(fixture.receiver.clone(), Router::new()).layer(axum::middleware::from_fn(
            |request: Request<Body>, next: axum::middleware::Next| async move {
                if request.uri().path() == LEAVE_RECEIPT_PATH {
                    return StatusCode::BAD_GATEWAY.into_response();
                }
                next.run(request).await
            },
        ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "a".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(axum::serve(listener, router).into_future());
    let http = replication_http_client();
    exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap();
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert!(error.downcast_ref::<RemovedFromFleet>().is_none());
    let deadline = fixture
        .receiver
        .fleet
        .leave
        .lock()
        .await
        .receiver_deadlines_unix_ms[fixture.key.public()];
    assert!(
        crate::store::now_ms() < deadline,
        "restart occurs before the first grace deadline"
    );
    let persisted = fs::read(root.path().join("fleet/leave-carriers.json")).unwrap();
    let anchor = MemberKey::load(&settings.node_key_path(root.path())).unwrap();
    let mut restarted = member_context(&fixture.receiver.backend.0, &anchor, &[], &[], false);
    restarted.state_dir = Some(root.path().to_path_buf());
    restarted.leave = Arc::new(tokio::sync::Mutex::new(
        serde_json::from_slice(&persisted).unwrap(),
    ));
    let view = restarted.view.read().unwrap().clone();
    {
        let leave = restarted.leave.lock().await;
        assert_eq!(
            leave.receiver_deadlines_unix_ms[fixture.key.public()],
            deadline
        );
        assert!(
            retained_leave_members_at(&view, anchor.public(), &leave, deadline - 1)
                .contains(fixture.key.public()),
            "restart must preserve the original remaining grace"
        );
        assert!(
            !retained_leave_members_at(&view, anchor.public(), &leave, deadline)
                .contains(fixture.key.public()),
            "expiry removes the runtime peer and replication grant from desired policy"
        );
        assert!(
            !leave.completed.contains(fixture.key.public()),
            "expiry is not a receipt"
        );
    }
    let replay = fixture
        .leaving
        .export_replication_exchange(
            fixture.leaving_auth.fleet_id(),
            &ReplicationInventory::default(),
        )
        .unwrap();
    receive_from_peer(
        &fixture.receiver.backend,
        &restarted,
        "b",
        fixture.leaving_auth.fleet_id(),
        &replay,
        None,
        Default::default(),
    )
    .await
    .unwrap();
    let leave = restarted.leave.lock().await;
    assert_eq!(
        leave.receiver_deadlines_unix_ms[fixture.key.public()],
        deadline,
        "replay must never renew a deadline"
    );
    assert!(
        leave.awaiting.contains(fixture.key.public()),
        "keep expired unconfirmed evidence"
    );
    assert!(
        !retained_leave_members_at(&view, anchor.public(), &leave, deadline + 1)
            .contains(fixture.key.public()),
        "expired replay cannot re-add a carrier"
    );
    assert!(!fixture.leaving_fleet.is_removed());
    assert!(
        fixture
            .leaving_fleet
            .leave
            .lock()
            .await
            .pending
            .contains_key("a"),
        "receiver expiry must not publish sender/CLI confirmation"
    );
    server.abort();
}

#[tokio::test]
async fn ordinary_signatures_discard_expired_provisional_leave_deadlines() {
    let mut fixture = leave_fixture();
    let root = tempfile::tempdir().unwrap();
    let settings = FleetFile {
        fleet_id: fixture.leaving_auth.fleet_id().into(),
        node: Some("a".into()),
        ..FleetFile::default()
    };
    settings.save(root.path()).unwrap();
    crate::fleet::join::write_private(&settings.node_key_path(root.path()), &fixture.anchor_bytes)
        .unwrap();
    fixture.receiver.fleet.state_dir = Some(root.path().to_path_buf());
    let key = fixture.key.public().to_owned();
    // A conservative signature-only admission can provisionally protect a carrier.
    // Model that exchange's deadline having elapsed before its authoritative fold.
    {
        let mut leave = fixture.receiver.fleet.leave.lock().await;
        leave.awaiting.insert(key.clone());
        leave
            .receiver_deadlines_unix_ms
            .insert(key.clone(), crate::store::now_ms().saturating_sub(1));
        leave.receiver_first_receive_unix_ms.insert(
            key.clone(),
            crate::store::now_ms().saturating_sub(LEAVE_RECEIPT_GRACE.as_millis() + 1),
        );
        save_leave_state(&fixture.receiver.fleet, &leave)
            .await
            .unwrap();
    }
    let mut ordinary = fixture
        .leaving
        .export_replication_exchange(
            fixture.leaving_auth.fleet_id(),
            &ReplicationInventory::default(),
        )
        .unwrap();
    let signed = ordinary
        .envelopes
        .iter()
        .find(|envelope| {
            envelope.member_key.as_deref() == Some(key.as_str())
                && !envelope
                    .payload
                    .bytes()
                    .unwrap()
                    .windows(b"fleet.member-left".len())
                    .any(|part| part == b"fleet.member-left")
        })
        .unwrap()
        .clone();
    ordinary.envelopes.clear();
    ordinary.signature_requests.clear();
    ordinary.signatures = vec![crate::claim::ReplicaEnvelopeSignature {
        writer: signed.writer,
        sequence: signed.sequence,
        hash: signed.hash,
        member_key: key.clone(),
        signature: signed.signature.unwrap(),
    }];
    assert!(may_carry_direct_leave(&ordinary, &key));
    receive_from_peer(
        &fixture.receiver.backend,
        &fixture.receiver.fleet,
        "b",
        fixture.leaving_auth.fleet_id(),
        &ordinary,
        None,
        Default::default(),
    )
    .await
    .unwrap();
    let persisted: LeaveState =
        serde_json::from_slice(&fs::read(root.path().join("fleet/leave-carriers.json")).unwrap())
            .unwrap();
    assert!(!persisted.awaiting.contains(&key));
    assert!(
        !persisted.receiver_deadlines_unix_ms.contains_key(&key),
        "a current member's ordinary signature must not consume its future leave grace"
    );
    assert!(!persisted.receiver_first_receive_unix_ms.contains_key(&key));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "a".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(
        axum::serve(
            listener,
            peer_router(fixture.receiver.clone(), Router::new()),
        )
        .into_future(),
    );
    let http = replication_http_client();
    let before_departure = crate::store::now_ms();
    exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap();
    let deadline = fixture
        .receiver
        .fleet
        .leave
        .lock()
        .await
        .receiver_deadlines_unix_ms[&key];
    assert!(
        deadline > crate::store::now_ms(),
        "the genuine leave gets a fresh grace"
    );
    assert!(
        deadline >= before_departure.saturating_add(LEAVE_RECEIPT_GRACE.as_millis()),
        "only the confirmed real departure starts its full grace"
    );
    assert!(
        retained_for_test(&fixture.receiver.fleet)
            .await
            .contains(&key)
    );
    let error = exchange(
        &http,
        &Local(fixture.leaving.clone()),
        "b",
        &peer,
        &fixture.leaving_auth,
        &fixture.leaving_fleet,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<RemovedFromFleet>().unwrap().code,
        "member-left"
    );
    assert!(
        fixture
            .receiver
            .fleet
            .leave
            .lock()
            .await
            .completed
            .contains(&key)
    );
    server.abort();
}

#[tokio::test]
async fn interrupted_leave_admission_keeps_first_receive_bound_and_denies_failed_journaling() {
    let fixture = leave_fixture();
    let root = tempfile::tempdir().unwrap();
    let settings = FleetFile {
        fleet_id: fixture.leaving_auth.fleet_id().into(),
        node: Some("a".into()),
        ..FleetFile::default()
    };
    settings.save(root.path()).unwrap();
    crate::fleet::join::write_private(&settings.node_key_path(root.path()), &fixture.anchor_bytes)
        .unwrap();
    let key = fixture.key.public().to_owned();
    let first_receive = crate::store::now_ms().saturating_sub(LEAVE_RECEIPT_GRACE.as_millis() + 1);
    let mut before_admission = LeaveState::default();
    before_admission.awaiting.insert(key.clone());
    before_admission
        .receiver_first_receive_unix_ms
        .insert(key.clone(), first_receive);
    let anchor = MemberKey::load(&settings.node_key_path(root.path())).unwrap();
    let mut journal_owner = member_context(&fixture.receiver.backend.0, &anchor, &[], &[], false);
    journal_owner.state_dir = Some(root.path().to_path_buf());
    save_leave_state(&journal_owner, &before_admission)
        .await
        .unwrap();
    let exchange = fixture
        .leaving
        .export_replication_exchange(
            fixture.leaving_auth.fleet_id(),
            &ReplicationInventory::default(),
        )
        .unwrap();
    // Commit the signed departure, then model interruption before deadline publication.
    fixture
        .receiver
        .backend
        .receive(
            "b",
            fixture.leaving_auth.fleet_id(),
            &exchange,
            None,
            Default::default(),
        )
        .await
        .unwrap();
    let persisted = fs::read(root.path().join("fleet/leave-carriers.json")).unwrap();
    let mut restarted = member_context(&fixture.receiver.backend.0, &anchor, &[], &[], false);
    restarted.leave = Arc::new(tokio::sync::Mutex::new(
        serde_json::from_slice(&persisted).unwrap(),
    ));
    restarted.state_dir = Some(root.path().join("missing-fleet"));
    refresh_fleet_view(&fixture.receiver.backend, &restarted).await;
    {
        let leave = restarted.leave.lock().await;
        assert!(
            leave.persistence_error.is_some(),
            "failed deadline publication is visible"
        );
        assert!(
            !leave.receiver_deadlines_unix_ms.contains_key(&key),
            "never accept the non-durable deadline"
        );
        let view = restarted.view.read().expect("fleet view lock poisoned");
        assert!(
            !retained_leave_members(&view, anchor.public(), &leave).contains(&key),
            "persistence failure denies carrier retention"
        );
    }
    // Restart from the unchanged pre-import journal once persistence is available.
    restarted.state_dir = Some(root.path().to_path_buf());
    restarted.leave = Arc::new(tokio::sync::Mutex::new(
        serde_json::from_slice(&persisted).unwrap(),
    ));
    refresh_fleet_view(&fixture.receiver.backend, &restarted).await;
    let leave = restarted.leave.lock().await;
    assert_eq!(
        leave.receiver_deadlines_unix_ms[&key],
        first_receive.saturating_add(LEAVE_RECEIPT_GRACE.as_millis())
    );
    assert!(leave.persistence_error.is_none());
    let view = restarted.view.read().expect("fleet view lock poisoned");
    assert!(
        !retained_leave_members(&view, anchor.public(), &leave).contains(&key),
        "restart must not renew grace consumed during interruption"
    );
    assert!(
        !leave.completed.contains(&key),
        "neither restart nor expiry confirms receipt"
    );
}

mod retry_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn fallback_returns_to_preferred_after_cooldown_and_survives_repeat_failure() {
        use parking_lot::Mutex;
        use std::sync::atomic::{AtomicBool, Ordering};

        // Use the existing signed peer harness with two fake HTTP routes. No external
        // transport or Fabric process participates; only in-process loopback listeners.
        let fleet_id = "d706eab5-433c-474b-ae43-fd2a7891d85d";
        let auth = FleetAuth::test(fleet_id, &[42; 32]);
        // Concurrent status reads need the file-backed WAL model, not SQLite's shared-cache
        // in-memory table locks while the worker commits an exchange.
        let root = tempfile::tempdir().unwrap();
        let local = Arc::new(plain_open(&root.path().join("harbor.sqlite3"), "harbor").unwrap());
        let remote = Arc::new(plain_memory("beacon").unwrap());
        for store in [&local, &remote] {
            store.bind_fleet(fleet_id).unwrap();
        }
        let preferred_up = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let mut urls = Vec::new();
        let mut servers = Vec::new();
        for preferred in [true, false] {
            let up = preferred_up.clone();
            let observed = attempts.clone();
            let app = peer_router(
                PeerState {
                    backend: Local(remote.clone()),
                    node: "beacon".into(),
                    auth: auth.clone(),
                    fleet: FleetContext::legacy(BTreeSet::from(["harbor".into()])),
                    outbound_notify: watch::channel(0).0,
                    shards: Default::default(),
                },
                Router::new(),
            )
            .layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    let up = up.clone();
                    let observed = observed.clone();
                    async move {
                        if request.method() == axum::http::Method::POST {
                            observed.lock().push(preferred);
                            if preferred && !up.load(Ordering::SeqCst) {
                                // Fail the HTTP stream, not an application response (503
                                // must retain the route under the transport-only policy).
                                return Response::new(Body::from_stream(
                                    futures_util::stream::once(async {
                                        Err::<Bytes, _>(std::io::Error::from(
                                            std::io::ErrorKind::ConnectionReset,
                                        ))
                                    }),
                                ));
                            }
                        }
                        next.run(request).await
                    }
                },
            ));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            urls.push(Route::Http(format!(
                "http://{}",
                listener.local_addr().unwrap()
            )));
            servers.push(tokio::spawn(axum::serve(listener, app).into_future()));
        }
        // Socket readiness must not race Tokio's automatic clock advances.
        let awake = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let (_route_tx, routes) = watch::channel(urls);
        let (notify, _) = watch::channel(0);
        let fleet = FleetContext::legacy(BTreeSet::from(["beacon".into()]));
        let dialer = tokio::spawn(dial_peer(
            Local(local.clone()),
            "harbor".into(),
            "beacon".into(),
            routes,
            auth,
            fleet.clone(),
            notify.subscribe(),
        ));
        async fn settled(
            local: &Store,
            ready: impl Fn(&crate::replication::ReplicationWorkerStatus) -> bool,
        ) {
            // Paused time must not outrun real socket/blocking-store work. Wait for the
            // worker's completed transition before advancing its retry/cooldown clock.
            let started = std::time::Instant::now();
            loop {
                let worker = local
                    .replication_sync
                    .lock()
                    .unwrap()
                    .get("beacon")
                    .and_then(|progress| progress.worker.clone());
                if worker.as_ref().is_some_and(&ready) {
                    return;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(30),
                    "worker did not settle: {worker:?}"
                );
                tokio::task::yield_now().await;
            }
        }
        settled(&local, |worker| worker.phase == "backoff").await;
        tokio::time::advance(Duration::from_secs(2)).await;
        settled(&local, |worker| {
            worker.phase == "idle" && worker.route_is_preferred == Some(false)
        })
        .await;
        assert!(fleet.activity.read().unwrap().contains_key("beacon"));
        assert!(attempts.lock()[0], "preferred fails first");
        assert_eq!(attempts.lock().last(), Some(&false), "fallback succeeds");
        let worker = local
            .replication_status(false, None, &["beacon".into()])
            .unwrap()
            .peers
            .remove(0)
            .worker
            .unwrap();
        assert_eq!(worker.route_is_preferred, Some(false));
        assert!(worker.preferred_retry_at_unix_ms.is_some());
        preferred_up.store(true, Ordering::SeqCst);
        // Local writes wake the worker throughout cooldown, but neither retry early nor
        // slide the deadline on each successful fallback exchange.
        for _ in 0..3 {
            let before = attempts.lock().len();
            notify.send_modify(|generation| *generation += 1);
            tokio::time::advance(Duration::from_secs(10)).await;
            settled(&local, |worker| {
                worker.phase == "idle" && attempts.lock().len() > before
            })
            .await;
        }
        assert_eq!(
            attempts.lock().iter().filter(|&&route| route).count(), 2,
            "one failed send plus one fresh retry; cooldown must add no preferred attempts"
        );
        notify.send_modify(|generation| *generation += 1);
        tokio::time::advance(Duration::from_secs(31)).await;
        settled(&local, |worker| {
            worker.phase == "idle" && worker.route_is_preferred == Some(true)
        })
        .await;
        assert_eq!(attempts.lock().last(), Some(&true), "preferred recovers");
        let worker = local
            .replication_status(false, None, &["beacon".into()])
            .unwrap()
            .peers
            .remove(0)
            .worker
            .unwrap();
        assert_eq!(worker.route_is_preferred, Some(true));
        assert_eq!(worker.preferred_retry_at_unix_ms, None);
        preferred_up.store(false, Ordering::SeqCst);
        notify.send_modify(|generation| *generation += 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        settled(&local, |worker| worker.phase == "backoff").await;
        tokio::time::advance(Duration::from_secs(2)).await;
        settled(&local, |worker| {
            worker.phase == "idle" && worker.route_is_preferred == Some(false)
        })
        .await;
        assert_eq!(attempts.lock().last(), Some(&false));
        let preferred_before = attempts.lock().iter().filter(|&&route| route).count();
        notify.send_modify(|generation| *generation += 1);
        tokio::time::advance(Duration::from_secs(61)).await;
        settled(&local, |worker| worker.phase == "backoff").await;
        tokio::time::advance(Duration::from_secs(2)).await;
        settled(&local, |worker| {
            worker.phase == "idle" && worker.route_is_preferred == Some(false)
        })
        .await;
        let observed = attempts.lock();
        assert!(observed.iter().filter(|&&route| route).count() > preferred_before);
        assert_eq!(
            observed.last(),
            Some(&false),
            "repeat failure keeps fallback usable"
        );
        drop(observed);
        dialer.abort();
        awake.abort();
        for server in servers {
            server.abort();
        }
    }

    #[tokio::test]
    async fn authenticated_activity_does_not_start_redundant_healthy_exchanges() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let fleet_id = "d706eab5-433c-474b-ae43-fd2a7891d85d";
        let auth = FleetAuth::test(fleet_id, &[42; 32]);
        let local = Arc::new(plain_memory("harbor").unwrap());
        let remote = Arc::new(plain_memory("beacon").unwrap());
        for store in [&local, &remote] {
            store.bind_fleet(fleet_id).unwrap();
        }
        let fleet = FleetContext::legacy(BTreeSet::from(["beacon".into()]));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let app = peer_router(
            PeerState {
                backend: Local(remote),
                node: "beacon".into(),
                auth: auth.clone(),
                fleet: FleetContext::legacy(BTreeSet::from(["harbor".into()])),
                outbound_notify: watch::channel(0).0,
                shards: Default::default(),
            },
            Router::new(),
        )
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                counter.fetch_add(1, Ordering::Relaxed);
                async move { next.run(request).await }
            },
        ));
        let server = tokio::spawn(axum::serve(listener, app).into_future());
        let (_route_tx, routes) = watch::channel(vec![Route::Http(url)]);
        let (notify, _) = watch::channel(0);
        let dialer = tokio::spawn(dial_peer(
            Local(local),
            "harbor".into(),
            "beacon".into(),
            routes,
            auth,
            fleet.clone(),
            notify.subscribe(),
        ));
        // The first exchange moves both nodes' fleet bindings, so the dialer starts a follow-up
        // exchange at once; on a loaded machine that can come long after the first answer.
        // Measure from the healthy wait: a completed exchange, then no request for longer than
        // the coalescing window.
        tokio::time::timeout(Duration::from_secs(30), async {
            let mut seen = requests.load(Ordering::Relaxed);
            loop {
                tokio::time::sleep(3 * REPLICATION_WAKE_COALESCE).await;
                let now = requests.load(Ordering::Relaxed);
                if now == seen && fleet.activity.read().unwrap().contains_key("beacon") {
                    break;
                }
                seen = now;
            }
        })
        .await
        .unwrap();
        let before = requests.load(Ordering::Relaxed);
        // A successful client read/probe proves peer life, but it does not change the graph.
        fleet.note_activity("beacon");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            requests.load(Ordering::Relaxed),
            before,
            "authenticated activity interrupted healthy anti-entropy coalescing"
        );
        dialer.abort();
        server.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn signs_of_life_during_a_failed_attempt_are_not_lost_before_the_retry_wait() {
        let fleet = FleetContext::legacy(BTreeSet::new());
        let (_route_tx, mut routes) = watch::channel(Vec::new());
        let mut inbound = fleet.inbound_changed.subscribe();
        let mut activity = fleet.activity_changed.subscribe();
        let mut connectivity = fleet.connectivity_changed.subscribe();
        let http = replication_http_client();
        for completed_exchange in [false, true] {
            let attempt_started = tokio::time::Instant::now();
            tokio::time::advance(Duration::from_secs(1)).await;
            if completed_exchange {
                fleet
                    .inbound
                    .write()
                    .unwrap()
                    .insert("traveller".into(), tokio::time::Instant::now());
                fleet
                    .inbound_changed
                    .send_modify(|generation| *generation += 1);
            } else {
                fleet.note_activity("traveller");
            }
            // The outbound request fails later, after a coalesced notification was read.
            inbound.borrow_and_update();
            activity.borrow_and_update();
            tokio::time::advance(REPLICATION_EXCHANGE_TIMEOUT).await;
            assert!(
                wait_peer_retry(
                    &Local(Arc::new(plain_memory("retry-fixture").unwrap())),
                    crate::store::now_ms(),
                    Duration::from_secs(3600),
                    &mut routes,
                    &mut inbound,
                    &mut activity,
                    &mut connectivity,
                    &fleet,
                    "traveller",
                    attempt_started,
                    None,
                    &http,
                    None,
                    &mut BTreeSet::new(),
                )
                .await
            );
            tokio::time::advance(Duration::from_millis(1)).await;
        }
        // Old activity must not reset the next failed attempt's backoff.
        assert!(
            !wait_peer_retry(
                &Local(Arc::new(plain_memory("retry-fixture").unwrap())),
                crate::store::now_ms(),
                Duration::from_secs(3600),
                &mut routes,
                &mut inbound,
                &mut activity,
                &mut connectivity,
                &fleet,
                "traveller",
                tokio::time::Instant::now(),
                None,
                &http,
                None,
                &mut BTreeSet::new(),
            )
            .await
        );
    }

    #[tokio::test]
    async fn a_silent_http_return_caps_backoff_without_an_inventory_export() {
        let store = Arc::new(plain_memory("beacon").unwrap());
        let fleet = FleetContext::legacy(BTreeSet::new());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(
            axum::serve(
                listener,
                peer_router(
                    PeerState {
                        backend: Local(store),
                        node: "beacon".into(),
                        auth: FleetAuth::test("invented-fleet", &[42; 32]),
                        fleet: fleet.clone(),
                        outbound_notify: watch::channel(0).0,
                        shards: Default::default(),
                    },
                    Router::new(),
                ),
            )
            .into_future(),
        );
        let (_route_tx, mut routes) = watch::channel(vec![Route::Http(url.clone())]);
        let mut inbound = fleet.inbound_changed.subscribe();
        let mut activity = fleet.activity_changed.subscribe();
        let mut connectivity = fleet.connectivity_changed.subscribe();
        let http = replication_http_client();
        let previous = (
            url.clone(),
            tokio::time::Instant::now() - Duration::from_secs(120),
        );
        let started = tokio::time::Instant::now();
        assert!(
            !tokio::time::timeout(
                Duration::from_secs(35),
                wait_peer_retry(
                    &Local(Arc::new(plain_memory("retry-fixture").unwrap())),
                    crate::store::now_ms(),
                    Duration::from_secs(3600),
                    &mut routes,
                    &mut inbound,
                    &mut activity,
                    &mut connectivity,
                    &fleet,
                    "traveller",
                    started,
                    Some(&previous),
                    &http,
                    None,
                    &mut BTreeSet::new(),
                )
            )
            .await
            .unwrap()
        );
        assert!(
            fleet.activity.read().unwrap().is_empty(),
            "HEAD must not refresh authenticated activity"
        );
        assert!(
            fleet.inbound.read().unwrap().is_empty(),
            "HEAD must not invoke receive_exchange"
        );

        // Without a successful signed exchange for five minutes, absence stays quiet.
        inbound.borrow_and_update();
        let old = (url, tokio::time::Instant::now() - PEER_PROBE_WINDOW);
        tokio::time::pause();
        assert!(
            !wait_peer_retry(
                &Local(Arc::new(plain_memory("retry-fixture").unwrap())),
                crate::store::now_ms(),
                Duration::from_secs(3600),
                &mut routes,
                &mut inbound,
                &mut activity,
                &mut connectivity,
                &fleet,
                "traveller",
                tokio::time::Instant::now(),
                Some(&old),
                &http,
                None,
                &mut BTreeSet::new(),
            )
            .await
        );
        server.abort();
    }

    #[tokio::test]
    async fn isolated_leaves_refuse_direct_fabric_routes_and_converge_through_the_hub() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let names = ["indigo", "amber", "cobalt"];
        let fleet_id = "94cd11ba-c582-4558-9c84-c3bda922eb6d";
        let auth = FleetAuth::test(fleet_id, &[37; 32]);
        let stores = names.map(|name| {
            let store =
                Arc::new(plain_open(&root.path().join(format!("{name}.db")), name).unwrap());
            store.bind_fleet(fleet_id).unwrap();
            store
        });
        let log = root.path().join("refusals");
        let grant = root.path().join("grant");
        let program = root.path().join("fabric");
        std::fs::write(&program, format!(
            "#!/bin/sh\nif test -f '{}'; then printf '{{\"address\":\"%s\"}}\\n' \"$(cat '{}')\"; exit 0; fi\necho \"$2\" >> '{}'\necho '{{\"error\":{{\"code\":\"peer_not_permitted_for_service\",\"class\":\"refusal\",\"message\":\"peer not permitted for service st3-peer-v1\",\"peer\":null,\"service\":\"st3-peer-v1\"}}}}'\nexit 77\n",
            grant.display(), grant.display(), log.display()
        )).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let contexts = names.map(|name| {
            let mut context = FleetContext::legacy(
                names
                    .iter()
                    .filter(|peer| **peer != name)
                    .map(|peer| (*peer).into())
                    .collect(),
            );
            context.fabric = Some(Fabric::new(program.clone()));
            context
        });
        let wakes = names.map(|_| watch::channel(0).0);
        let mut tasks = Vec::new();
        let mut addresses = Vec::new();
        for index in 0..3 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            addresses.push(listener.local_addr().unwrap());
            let state = PeerState {
                backend: Local(stores[index].clone()),
                node: names[index].into(),
                auth: auth.clone(),
                fleet: contexts[index].clone(),
                outbound_notify: wakes[index].clone(),
                shards: Default::default(),
            };
            tasks.push(tokio::spawn(async move {
                axum::serve(listener, peer_router(state, Router::new()))
                    .await
                    .unwrap();
            }));
        }
        let mut routes = Vec::new();
        for (from, to) in [(1, 0), (2, 0), (1, 2), (2, 1)] {
            let route = if to == 0 {
                Route::Http(format!("http://{}", addresses[to]))
            } else {
                Route::Fabric {
                    node: names[to].into(),
                    protocol: "st3-peer-v1".into(),
                }
            };
            let (sender, receiver) = watch::channel(vec![route]);
            routes.push(sender);
            tasks.push(tokio::spawn(dial_peer(
                Local(stores[from].clone()),
                names[from].into(),
                names[to].into(),
                receiver,
                auth.clone(),
                contexts[from].clone(),
                wakes[from].subscribe(),
            )));
        }
        let write = |index: usize, note: &str| {
            stores[index]
                .append_claim(&crate::claim::ClaimInput {
                    subject: format!("daemon/{note}"),
                    kind: "daemon.diagnostic".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("severity".into(), Value::String("warning".into())),
                        (
                            "code".into(),
                            Value::String("isolated-refusal-proof".into()),
                        ),
                        ("reason".into(), Value::String(note.into())),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
            wakes[index].send_modify(|generation| *generation += 1);
        };
        // The hub cannot dial an isolated leaf, so a leaf that finished its exchanges before the
        // other leaf pushed learns of it at its next contact. Stand in for that contact while
        // waiting: an inbound change wakes the hub dialers, never a refused route.
        async fn converge(stores: &[Arc<Store>], contexts: &[FleetContext]) {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    for context in contexts {
                        context
                            .inbound_changed
                            .send_modify(|generation| *generation += 1);
                    }
                    let snapshots = stores
                        .iter()
                        .map(|store| store.replication_status(true, None, &[]).unwrap())
                        .collect::<Vec<_>>();
                    if snapshots.iter().all(|snapshot| {
                        snapshot.authority_digest == snapshots[0].authority_digest
                            && snapshot.graph_digest == snapshots[0].graph_digest
                    }) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await
            .expect("the leaves must converge through the hub");
        }
        for index in 0..3 {
            write(index, &format!("initial-{}", names[index]));
        }
        converge(&stores, &contexts).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while std::fs::read_to_string(&log)
                .unwrap_or_default()
                .lines()
                .count()
                < 2
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        for (from, to) in [(1, 2), (2, 1)] {
            let status = stores[from]
                .replication_status(true, Some(fleet_id), &[names[to].into()])
                .unwrap();
            assert_eq!(status.peers[0].status, "refused");
            assert!(status.peers[0].last_error.is_none());
            assert!(
                status.peers[0]
                    .refusal_reason
                    .as_ref()
                    .unwrap()
                    .contains("that member's Fabric grants")
            );
        }
        // Busy graph, network and online events do not change the refusal classification.
        for round in 0..8 {
            for index in 0..3 {
                write(index, &format!("busy-{round}-{}", names[index]));
                contexts[index]
                    .connectivity_changed
                    .send_modify(|generation| *generation += 1);
                contexts[index]
                    .online
                    .write()
                    .unwrap()
                    .insert("cobalt".into(), tokio::time::Instant::now());
                contexts[index]
                    .inbound_changed
                    .send_modify(|generation| *generation += 1);
            }
            converge(&stores, &contexts).await;
        }
        // A grant refusal must retain the preferred route, not bypass access policy by
        // selecting an otherwise healthy HTTP fallback.
        routes[2].send_replace(vec![
            Route::Fabric {
                node: "cobalt".into(),
                protocol: "st3-peer-v1".into(),
            },
            Route::Http(format!("http://{}", addresses[2])),
        ]);
        tokio::time::sleep(Duration::from_millis(100)).await;
        use crate::replication::ReplicationCarrier;
        let (success, failure) = stores[1].replication_peer_transport("cobalt");
        assert!(success.is_none(), "grant refusal must not select HTTP fallback");
        let failed_fabric = failure.unwrap();
        assert_eq!(failed_fabric.carrier, ReplicationCarrier::Fabric);
        assert_eq!(failed_fabric.fabric_route.as_deref(), Some("unknown"));
        // An explicit membership route change remains usable immediately.
        let before_route_change = 0;
        std::fs::write(&grant, addresses[2].to_string()).unwrap();
        routes[2].send_replace(vec![Route::Fabric {
            node: "cobalt-new-route".into(),
            protocol: "st3-peer-v1".into(),
        }]);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                // An incoming exchange can advance last-seen before the changed route
                // completes. Wait for this dial's own carrier evidence, not reachability.
                if stores[1]
                    .replication_peer_transport("cobalt")
                    .0
                    .is_some_and(|record| {
                        record.carrier == ReplicationCarrier::Fabric
                            && record.at_unix_ms > before_route_change
                    })
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("a changed route must interrupt normal refusal backoff");
        let (success, failure) = stores[1].replication_peer_transport("cobalt");
        let fabric_success = success.unwrap();
        assert_eq!(
            fabric_success.carrier,
            ReplicationCarrier::Fabric,
            "the tunnel URL is loopback, the carrier is Fabric"
        );
        assert_eq!(fabric_success.fabric_route.as_deref(), Some("unknown"));
        assert_eq!(
            failure,
            Some(failed_fabric),
            "later success retains the earlier failed route"
        );

        for task in tasks {
            task.abort();
            let _ = task.await;
        }
    }

    /// `fabric dial` never connects, so a member's refusal reaches the worker as a tunnel that
    /// closes. The worker then asks `fabric probe --json`, and only a refusal class reports the
    /// peer `refused`. A refused route keeps normal peer backoff (docs/st3/replication.md), so the
    /// probe is asked once per attempt and never faster than that backoff allows.
    #[tokio::test]
    async fn a_tunnel_that_closes_is_a_refusal_only_when_the_probe_says_so() {
        use std::os::unix::fs::PermissionsExt as _;
        let fleet_id = "94cd11ba-c582-4558-9c84-c3bda922eb6d";
        let auth = FleetAuth::test(fleet_id, &[37; 32]);
        let answer = |outcome: &str, code: &str, class: &str| {
            format!(
                "{{\"peer\":\"amber\",\"outcome\":\"{outcome}\",\"error\":\"fabric:{code} words\",\"code\":\"{code}\",\"class\":\"{class}\"}}"
            )
        };
        for (probe, refused) in [
            (
                answer("unreachable", "peer_not_permitted_for_service", "refusal"),
                true,
            ),
            (
                answer("unreachable", "peer_not_in_allow_list", "refusal"),
                true,
            ),
            (answer("unreachable", "unreachable", "fault"), false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = Arc::new(plain_open(&root.path().join("indigo.db"), "indigo").unwrap());
            store.bind_fleet(fleet_id).unwrap();
            // A tunnel that accepts and closes at once, as a refused one does.
            let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = dead.local_addr().unwrap();
            let closer = tokio::spawn(async move {
                loop {
                    let _ = dead.accept().await;
                }
            });
            let probes = root.path().join("probes");
            let program = root.path().join("fabric");
            std::fs::write(
                &program,
                format!(
                    "#!/bin/sh\ncase \"$1\" in\n  dial) echo '{{\"address\":\"{address}\"}}';;\n  probe) echo \"$2\" >> '{}'; echo '{probe}'; exit 2;;\nesac\n",
                    probes.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut context = FleetContext::legacy(["amber".into()].into());
            context.fabric = Some(Fabric::new(program));
            let (_routes, receiver) = watch::channel(vec![Route::Fabric {
                node: "amber".into(),
                protocol: "st3-peer-v1".into(),
            }]);
            let wake = watch::channel(0).0;
            let started = std::time::Instant::now();
            let worker = tokio::spawn(dial_peer(
                Local(store.clone()),
                "indigo".into(),
                "amber".into(),
                receiver,
                auth.clone(),
                context,
                wake.subscribe(),
            ));
            // Wait until the worker has asked Fabric what the closed tunnel meant.
            tokio::time::timeout(Duration::from_secs(10), async {
                while std::fs::read_to_string(&probes)
                    .unwrap_or_default()
                    .lines()
                    .count()
                    == 0
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("{probe}: the probe was never asked"));
            // Then until it has recorded what the probe said, rather than for a fixed time.
            let status = || {
                store
                    .replication_status(true, Some(fleet_id), &["amber".into()])
                    .unwrap()
                    .peers
                    .remove(0)
            };
            let peer = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let peer = status();
                    if peer.last_failed_exchange.is_some() {
                        return peer;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("{probe}: the failed exchange was never recorded"));
            let failed = peer
                .last_failed_exchange
                .as_ref()
                .expect("the closed Fabric tunnel records its attempted carrier");
            assert_eq!(
                failed.carrier,
                crate::replication::ReplicationCarrier::Fabric,
                "{probe}"
            );
            assert_eq!(failed.fabric_route.as_deref(), Some("unknown"), "{probe}");
            assert!(peer.last_successful_exchange.is_none(), "{probe}");
            if refused {
                assert_eq!(peer.status, "refused", "{probe}");
                assert!(peer.last_error.is_none());
                assert!(
                    peer.refusal_reason
                        .as_ref()
                        .unwrap()
                        .contains("that member's Fabric grants")
                );
                let detail = failed.failure.as_ref().expect("retained Fabric refusal detail");
                assert_eq!(detail.phase, ReplicationFailurePhase::Dial);
                assert!(!detail.cause.contains("that member's Fabric grants"));
                // No hot loop: one probe per attempt, and attempts no closer together than the
                // backoff's shortest jitter allows. A slow runner only makes fewer attempts.
                tokio::time::sleep(Duration::from_millis(600)).await;
                let elapsed = started.elapsed();
                let asked = std::fs::read_to_string(&probes).unwrap().lines().count();
                let mut allowed = 1;
                let mut earliest = Duration::ZERO;
                loop {
                    earliest += PeerBackoff::delay(allowed as u32 - 1, 0);
                    if earliest > elapsed {
                        break;
                    }
                    allowed += 1;
                }
                assert!((1..=allowed).contains(&asked), "{probe}: {asked} probes in {elapsed:?}");
                assert_eq!(status().status, "refused", "{probe}");
            } else {
                assert_ne!(peer.status, "refused", "{probe}");
                assert!(peer.refusal_reason.is_none(), "{probe}");
            }
            worker.abort();
            closer.abort();
        }
    }

    /// A peer with a preferred HTTP route that breaks its next `broken` POSTs before it works,
    /// and a Fabric fallback whose grants refuse this node. Returns the local store, the HTTP
    /// route's remaining broken POSTs and POST count, the Fabric dial log, the wake, and the
    /// tasks to abort. One failed exchange is two broken POSTs: the send and its one retry on a
    /// fresh connection.
    async fn refused_fallback_peer(
        root: &Path,
        broken: usize,
    ) -> (
        Arc<Store>,
        Arc<std::sync::atomic::AtomicUsize>,
        Arc<std::sync::atomic::AtomicUsize>,
        PathBuf,
        watch::Sender<u64>,
        Vec<tokio::task::JoinHandle<()>>,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let fleet_id = "5b0d7a52-7f43-4c3e-9a51-2f2d6c1e8b70";
        let auth = FleetAuth::test(fleet_id, &[23; 32]);
        let local = Arc::new(plain_open(&root.join("harbor.db"), "harbor").unwrap());
        let remote = Arc::new(plain_open(&root.join("beacon.db"), "beacon").unwrap());
        for store in [&local, &remote] {
            store.bind_fleet(fleet_id).unwrap();
        }
        let failing = Arc::new(AtomicUsize::new(broken));
        let posts = Arc::new(AtomicUsize::new(0));
        let (left, counted) = (failing.clone(), posts.clone());
        let app = peer_router(
            PeerState {
                backend: Local(remote.clone()),
                node: "beacon".into(),
                auth: auth.clone(),
                fleet: FleetContext::legacy(BTreeSet::from(["harbor".into()])),
                outbound_notify: watch::channel(0).0,
                shards: Default::default(),
            },
            Router::new(),
        )
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let (left, counted) = (left.clone(), counted.clone());
                async move {
                    if request.method() == axum::http::Method::POST {
                        counted.fetch_add(1, Ordering::SeqCst);
                        if left
                            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                            .is_ok()
                        {
                            // A broken stream, as a member restarting under a deploy leaves.
                            return Response::new(Body::from_stream(futures_util::stream::once(
                                async {
                                    Err::<Bytes, _>(std::io::Error::from(
                                        std::io::ErrorKind::ConnectionReset,
                                    ))
                                },
                            )));
                        }
                    }
                    next.run(request).await
                }
            },
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dials = root.join("dials");
        let program = root.join("fabric");
        std::fs::write(&program, format!(
            "#!/bin/sh\necho \"$1 $2\" >> '{}'\necho '{{\"error\":{{\"code\":\"peer_not_permitted_for_service\",\"class\":\"refusal\",\"message\":\"peer not permitted for service st3-peer-v1\",\"peer\":null,\"service\":\"st3-peer-v1\"}}}}'\nexit 77\n",
            dials.display()
        )).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut fleet = FleetContext::legacy(BTreeSet::from(["beacon".into()]));
        fleet.fabric = Some(Fabric::new(program));
        let (routes, receiver) = watch::channel(vec![
            Route::Http(url),
            Route::Fabric {
                node: "beacon".into(),
                protocol: "st3-peer-v1".into(),
            },
        ]);
        let wake = watch::channel(0).0;
        let worker = tokio::spawn(dial_peer(
            Local(local.clone()),
            "harbor".into(),
            "beacon".into(),
            receiver,
            auth,
            fleet,
            wake.subscribe(),
        ));
        let keep_routes = tokio::spawn(async move {
            routes.closed().await;
        });
        (local, failing, posts, dials, wake, vec![server, worker, keep_routes])
    }

    fn fabric_dials(log: &Path) -> usize {
        std::fs::read_to_string(log).unwrap_or_default().lines().count()
    }

    /// #4152: the preferred route fails once (its member restarts) and the worker fails over to
    /// a Fabric route the member's grants refuse. The refusal is that route's answer, not the
    /// peer's: the worker reports it as `refused`, returns to the preferred route on that route's
    /// own short back-off, and holds the refused route instead of redialing it as if it were a
    /// network fault.
    #[tokio::test]
    async fn a_refused_fallback_route_never_stalls_the_preferred_one() {
        use std::sync::atomic::Ordering;
        let root = tempfile::tempdir().unwrap();
        let (local, failing, posts, dials, wake, tasks) =
            refused_fallback_peer(root.path(), 2).await;
        // Waits for an exchange that failed after `since` and then for a success after that
        // failure. A success alone is not the recovery: an exchange already in flight, or the
        // follow-up of one that moved envelopes, can complete after `since` and before the
        // preferred route breaks. The route's first retry comes about a second after its
        // failure; the bound only catches a hang, as without the fix the route is never dialed
        // again.
        let recovered_after = |since: u128| {
            let local = local.clone();
            async move {
                tokio::time::timeout(Duration::from_secs(30), async {
                    loop {
                        if let (Some(success), Some(failure)) =
                            local.replication_peer_transport("beacon")
                            && failure.at_unix_ms > since
                            && success.at_unix_ms > failure.at_unix_ms
                        {
                            return success;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
            }
        };
        let success = recovered_after(0)
            .await
            .expect("the preferred route must exchange again within its short back-off");
        assert_ne!(success.carrier, crate::replication::ReplicationCarrier::Fabric);
        assert_eq!(fabric_dials(&dials), 1, "the refused route was dialed once");
        let peer = local
            .replication_status(true, None, &["beacon".into()])
            .unwrap()
            .peers
            .remove(0);
        assert_eq!(peer.status, "up", "a working route makes the peer up");
        assert!(peer.refusal_reason.is_none());
        let failed = peer.last_failed_exchange.expect("the refusal stays as the last failure");
        assert_eq!(failed.carrier, crate::replication::ReplicationCarrier::Fabric);

        // Another brief failure of the preferred route does not redial the refused one: it is
        // held, and the preferred route is retried on its own.
        let before = crate::store::now_ms();
        let posted = posts.load(Ordering::SeqCst);
        failing.store(2, Ordering::SeqCst);
        wake.send_modify(|generation| *generation += 1);
        recovered_after(before)
            .await
            .expect("the preferred route recovers again without the refused one");
        // The failed exchange's two broken POSTs, then the one that recovered.
        assert!(posts.load(Ordering::SeqCst) >= posted + 3);
        assert_eq!(fabric_dials(&dials), 1, "a refused route is held, not retried as a fault");
        for task in tasks {
            task.abort();
        }
    }

    /// A peer whose every route fails keeps the long, accumulating back-off: the refused route
    /// is held and reported as `refused`, and the failing route is not retried faster for it.
    #[tokio::test]
    async fn a_peer_whose_every_route_fails_keeps_the_long_back_off() {
        use std::sync::atomic::Ordering;
        let root = tempfile::tempdir().unwrap();
        let (local, _failing, posts, dials, _wake, tasks) =
            refused_fallback_peer(root.path(), usize::MAX).await;
        let started = std::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(15), async {
            while fabric_dials(&dials) == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the worker fails over to the refused route");
        tokio::time::sleep(Duration::from_secs(4)).await;
        let elapsed = started.elapsed();
        // Attempts no closer together than the back-off's shortest jitter allows, plus the one
        // return to the preferred route right after the refusal.
        let mut allowed = 1;
        let mut earliest = Duration::ZERO;
        loop {
            earliest += PeerBackoff::delay(allowed as u32 - 1, 0);
            if earliest > elapsed {
                break;
            }
            allowed += 1;
        }
        let attempts = posts.load(Ordering::SeqCst).div_ceil(2);
        assert!(
            (1..=allowed + 1).contains(&attempts),
            "{attempts} attempts in {elapsed:?}"
        );
        assert_eq!(fabric_dials(&dials), 1, "a refused route is held, not retried as a fault");
        let peer = local
            .replication_status(true, None, &["beacon".into()])
            .unwrap()
            .peers
            .remove(0);
        assert_eq!(peer.status, "refused", "{peer:?}");
        assert!(
            peer.refusal_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("that member's Fabric grants"))
        );
        assert!(peer.last_successful_exchange.is_none());
        for task in tasks {
            task.abort();
        }
    }

    #[test]
    fn absent_members_cost_a_few_attempts_per_hour_after_exponential_backoff() {
        // Use the shortest jitter so this bounds the most aggressive retry schedule.
        for away in [Duration::from_secs(120), Duration::from_secs(4 * 3600)] {
            let mut elapsed = Duration::ZERO;
            let mut attempts = 0;
            while elapsed < away {
                elapsed += PeerBackoff::delay(attempts, 0);
                attempts += 1;
            }
            assert!(
                attempts <= if away.as_secs() == 120 { 9 } else { 20 },
                "{attempts} retries in {away:?}"
            );
        }
        assert_ne!(PeerBackoff::delay(10, 0), PeerBackoff::delay(10, 400));
    }

    #[tokio::test(start_paused = true)]
    async fn an_inbound_return_or_changed_route_interrupts_even_hours_of_absence() {
        let http = replication_http_client();
        let fleet = FleetContext::legacy(BTreeSet::new());
        let (route_tx, mut routes) = watch::channel(vec![Route::Http("http://127.0.0.1:1".into())]);
        let mut inbound = fleet.inbound_changed.subscribe();
        let mut activity = fleet.activity_changed.subscribe();
        let mut connectivity = fleet.connectivity_changed.subscribe();
        for away in [Duration::from_secs(120), Duration::from_secs(4 * 3600)] {
            tokio::time::advance(away).await;
            let backend = Local(Arc::new(plain_memory("retry-fixture").unwrap()));
            let mut early_spent = BTreeSet::new();
            let wait = wait_peer_retry(
                &backend,
                crate::store::now_ms(),
                Duration::from_secs(300),
                &mut routes,
                &mut inbound,
                &mut activity,
                &mut connectivity,
                &fleet,
                "traveller",
                tokio::time::Instant::now(),
                None,
                &http,
                None,
                &mut early_spent,
            );
            tokio::pin!(wait);
            assert!(futures_util::poll!(&mut wait).is_pending());
            // Another peer returning must not reset this member's backoff.
            fleet
                .inbound
                .write()
                .unwrap()
                .insert("other".into(), tokio::time::Instant::now());
            fleet
                .inbound_changed
                .send_modify(|generation| *generation += 1);
            assert!(futures_util::poll!(&mut wait).is_pending());
            fleet
                .inbound
                .write()
                .unwrap()
                .insert("traveller".into(), tokio::time::Instant::now());
            fleet
                .inbound_changed
                .send_modify(|generation| *generation += 1);
            assert!(wait.await);
        }
        tokio::time::advance(Duration::from_millis(1)).await;
        let backend = Local(Arc::new(plain_memory("retry-fixture").unwrap()));
        let mut early_spent = BTreeSet::new();
        let wait = wait_peer_retry(
            &backend,
            crate::store::now_ms(),
            Duration::from_secs(300),
            &mut routes,
            &mut inbound,
            &mut activity,
            &mut connectivity,
            &fleet,
            "traveller",
            tokio::time::Instant::now(),
            None,
            &http,
            None,
            &mut early_spent,
        );
        tokio::pin!(wait);
        assert!(futures_util::poll!(&mut wait).is_pending());
        route_tx.send_replace(vec![Route::Http("http://127.0.0.1:2".into())]);
        assert!(wait.await);
    }

    #[tokio::test]
    async fn isolated_outbound_only_node_converges_after_two_minutes_and_four_hours() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let fleet_id = "d706eab5-433c-474b-ae43-fd2a7891d85d";
        let auth = FleetAuth::test(fleet_id, &[41; 32]);
        let root = tempfile::tempdir().unwrap();
        let names = ["harbor", "beacon", "traveller"];
        let stores = names.map(|name| {
            let store =
                Arc::new(plain_open(&root.path().join(format!("{name}.db")), name).unwrap());
            store.bind_fleet(fleet_id).unwrap();
            store
        });
        let contexts = names.map(|name| {
            FleetContext::legacy(
                names
                    .iter()
                    .filter(|peer| **peer != name)
                    .map(|peer| (*peer).into())
                    .collect(),
            )
        });
        let wakes = names.map(|_| watch::channel(0).0);
        let mut tasks = Vec::new();
        let mut addresses = Vec::new();
        let mut shutdowns = Vec::new();
        for index in 0..2 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            addresses.push(format!("http://{}", listener.local_addr().unwrap()));
            let state = PeerState {
                backend: Local(stores[index].clone()),
                node: names[index].into(),
                auth: auth.clone(),
                fleet: contexts[index].clone(),
                outbound_notify: wakes[index].clone(),
                shards: Default::default(),
            };
            let (shutdown, stopped) = tokio::sync::oneshot::channel();
            shutdowns.push(shutdown);
            tasks.push(tokio::spawn(async move {
                axum::serve(listener, peer_router(state, Router::new()))
                    .with_graceful_shutdown(async {
                        let _ = stopped.await;
                    })
                    .await
                    .unwrap();
            }));
        }
        // The traveller accepts no inbound requests. The rejector counts costly redundant
        // attempts, while its real node can initiate authenticated exchanges to both servers.
        let rejector = TcpListener::bind("127.0.0.1:0").await.unwrap();
        addresses.push(format!("http://{}", rejector.local_addr().unwrap()));
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = attempts.clone();
        tasks.push(tokio::spawn(async move {
            while let Ok((stream, _)) = rejector.accept().await {
                count.fetch_add(1, Ordering::Relaxed);
                drop(stream);
            }
        }));
        let start = |from: usize, to: usize| {
            let (routes, receiver) = watch::channel(vec![Route::Http(addresses[to].clone())]);
            let task = tokio::spawn(dial_peer(
                Local(stores[from].clone()),
                names[from].into(),
                names[to].into(),
                receiver,
                auth.clone(),
                contexts[from].clone(),
                wakes[from].subscribe(),
            ));
            (routes, task)
        };
        let mut dialers = vec![start(0, 1), start(1, 0), start(0, 2), start(1, 2)];
        let mut outbound = vec![start(2, 0), start(2, 1)];
        let write = |index: usize, note: &str| {
            stores[index]
                .append_claim(&crate::claim::ClaimInput {
                    subject: format!("daemon/{note}"),
                    kind: "daemon.diagnostic".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("severity".into(), Value::String("warning".into())),
                        ("code".into(), Value::String("isolated-proof".into())),
                        ("reason".into(), Value::String(note.into())),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
            wakes[index].send_modify(|generation| *generation += 1);
        };
        async fn converge(stores: &[Arc<Store>]) {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let digests = stores
                    .iter()
                    .map(|store| {
                        store
                            .replication_status(true, None, &[])
                            .unwrap()
                            .authority_digest
                    })
                    .collect::<BTreeSet<_>>();
                if digests.len() == 1 {
                    return;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "isolated nodes did not converge within ten seconds"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        write(0, "initial-harbor");
        write(1, "initial-beacon");
        write(2, "initial-traveller");
        converge(&stores).await;
        for (label, away) in [("minutes", 120), ("hours", 4 * 3600)] {
            for (_, task) in outbound.drain(..) {
                task.abort();
                let _ = task.await;
            }
            let before = attempts.load(Ordering::Relaxed);
            write(0, &format!("{label}-harbor"));
            write(1, &format!("{label}-beacon"));
            write(2, &format!("{label}-traveller"));
            // Run actual worker timers over minutes and hours on a virtual clock. Local
            // wakes throughout the absence must not undo the failed peers' backoff.
            tokio::time::pause();
            for _ in 0..away / 10 {
                tokio::time::advance(Duration::from_secs(10)).await;
                for wake in &wakes[..2] {
                    wake.send_modify(|generation| *generation += 1);
                }
                for _ in 0..8 {
                    tokio::task::yield_now().await;
                }
            }
            tokio::time::resume();
            let retries = attempts.load(Ordering::Relaxed) - before;
            // Each unanswered logical attempt can use one fresh-connection retry;
            // keep the same backoff budget while counting both wire connections.
            let allowed_logical_attempts = if away == 120 { 20 } else { 40 };
            assert!(
                retries <= 2 * allowed_logical_attempts,
                "{retries} wire connections in {away}s, budget {allowed_logical_attempts} logical attempts"
            );
            for server in &stores[..2] {
                server.age_replication_peer_for_test("traveller");
                let status = server
                    .replication_status(true, Some(fleet_id), &["traveller".into()])
                    .unwrap();
                assert_eq!(status.peers[0].status, "last-seen");
                // The servers' failed dials to the traveller's old address are evidence, kept
                // with their time until the traveller exchanges again.
                assert_eq!(
                    status.peers[0].last_error.is_some(),
                    status.peers[0].last_failure_at_unix_ms.is_some()
                );
            }
            // A changed source address is irrelevant: only the returning side needs to
            // reach the stable servers. Both queues drain despite server hour-long backoff.
            outbound = vec![start(2, 0), start(2, 1)];
            converge(&stores).await;
            // The stores agreeing does not mean each server's last answer to the traveller
            // carried its final inventory: a server that relays the traveller's own claims from
            // the other server after answering has a changed inventory, which asks its worker to
            // push to the traveller one coalescing interval later. That push is the policy for
            // new local envelopes, not a redundant connection. Wake the traveller so both servers
            // answer once more with the settled inventory, and measure only after they have.
            wakes[2].send_modify(|generation| *generation += 1);
            let settled_by = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let mut settled = true;
                for index in 0..2 {
                    let current = Local(stores[index].clone())
                        .export(fleet_id, &ReplicationInventory::default(), true, &[])
                        .await
                        .unwrap()
                        .exchange
                        .authority_digest;
                    let answered = contexts[index]
                        .inbound_authority
                        .read()
                        .unwrap()
                        .get("traveller")
                        .cloned();
                    settled &= answered.as_deref() == Some(current.as_str());
                }
                if settled {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < settled_by,
                    "the servers did not answer the traveller with their settled inventory within ten seconds"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let before = attempts.load(Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert_eq!(
                attempts.load(Ordering::Relaxed),
                before,
                "recent inbound exchange opened a redundant connection"
            );
        }
        for (_, task) in outbound.drain(..) {
            task.abort();
            let _ = task.await;
        }
        // An always-on server uses exactly the same absence policy. Stop its listener and
        // dialers for four virtual hours; return on a different port, while its neighbor
        // retains the old address and an hour-long retry. Its own announcement converges.
        let (_, harbor_dialer) = dialers.remove(0);
        harbor_dialer.abort();
        let _ = harbor_dialer.await;
        let (_, harbor_dialer) = dialers.remove(1);
        harbor_dialer.abort();
        let _ = harbor_dialer.await;
        shutdowns.remove(0).send(()).unwrap();
        tasks.remove(0).await.unwrap();
        write(0, "server-offline-queued");
        write(1, "server-peer-queued");
        tokio::time::pause();
        for _ in 0..4 * 3600 / 10 {
            tokio::time::advance(Duration::from_secs(10)).await;
            wakes[1].send_modify(|generation| *generation += 1);
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
        }
        tokio::time::resume();
        stores[1].age_replication_peer_for_test("harbor");
        assert_eq!(
            stores[1]
                .replication_status(true, Some(fleet_id), &["harbor".into()])
                .unwrap()
                .peers[0]
                .status,
            "last-seen"
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert_ne!(
            format!("http://{}", listener.local_addr().unwrap()),
            addresses[0]
        );
        let state = PeerState {
            backend: Local(stores[0].clone()),
            node: names[0].into(),
            auth: auth.clone(),
            fleet: contexts[0].clone(),
            outbound_notify: wakes[0].clone(),
            shards: Default::default(),
        };
        tasks.push(tokio::spawn(async move {
            axum::serve(listener, peer_router(state, Router::new()))
                .await
                .unwrap();
        }));
        dialers.push(start(0, 1));
        converge(&stores[..2]).await;
        for (_, task) in dialers.drain(..) {
            task.abort();
        }
        for task in tasks {
            task.abort();
        }
    }

    #[tokio::test]
    async fn a_network_change_refreshes_tailnet_without_waiting_for_the_minute_timer() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let polls = root.path().join("polls");
        let program = root.path().join("tailnet");
        std::fs::write(&program, format!(
            "#!/bin/sh\nprintf '%s\\n' poll >> '{}'\nprintf '%s\\n' '{{\"TailscaleIPs\":[]}}'\n",
            polls.display(),
        )).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (changed, connectivity) = watch::channel(0);
        let task = tokio::spawn(keep_tailnet_current(
            program,
            None,
            None,
            Endpoints::default(),
            Arc::default(),
            watch::channel(0).0,
            connectivity,
        ));
        async fn observed(path: &Path, count: usize) {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if std::fs::read_to_string(path)
                        .unwrap_or_default()
                        .lines()
                        .count()
                        >= count
                    {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("tailnet did not refresh within two seconds");
        }
        observed(&polls, 1).await;
        changed.send_modify(|generation| *generation += 1);
        observed(&polls, 2).await;
        task.abort();
        let _ = task.await;
    }

    #[tokio::test(start_paused = true)]
    async fn fabric_online_events_reset_only_the_member_they_name() {
        let http = replication_http_client();
        let mut fleet = FleetContext::legacy(BTreeSet::new());
        let member = crate::fleet::MemberView {
            name: "traveller".into(),
            state: "current".into(),
            mode: "listening".into(),
            member_key: "member-key".into(),
            node_id: None,
            endpoints_published: true,
            start: 0,
            end: None,
            ended: None,
            removed_by: None,
            removal_reason: None,
            endpoints: vec![
                serde_json::json!({"transport":"fabric", "node":"invented-node-id", "protocol":"sync"}),
            ],
        };
        fleet.view.write().unwrap().members.push(member);
        fleet
            .configured_fabric_peers
            .insert("configured-id".into(), "beacon".into());
        let (_route_tx, mut routes) =
            watch::channel(vec![Route::Http("http://127.0.0.1:1".into())]);
        let mut inbound = fleet.inbound_changed.subscribe();
        let mut activity = fleet.activity_changed.subscribe();
        let mut connectivity = fleet.connectivity_changed.subscribe();
        let backend = Local(Arc::new(plain_memory("retry-fixture").unwrap()));
        let mut early_spent = BTreeSet::new();
        let wait = wait_peer_retry(
            &backend,
            crate::store::now_ms(),
            Duration::from_secs(3600),
            &mut routes,
            &mut inbound,
            &mut activity,
            &mut connectivity,
            &fleet,
            "traveller",
            tokio::time::Instant::now(),
            None,
            &http,
            None,
            &mut early_spent,
        );
        tokio::pin!(wait);
        // Retain the routes sender while waiting.
        assert!(futures_util::poll!(&mut wait).is_pending());
        apply_fabric_presence(
            &fleet,
            &serde_json::json!({"reset":false,"events":[{"peer_id":"invented-node-id","online":false}]}),
        );
        assert!(futures_util::poll!(&mut wait).is_pending());
        apply_fabric_presence(
            &fleet,
            &serde_json::json!({"reset":false,"events":[{"peer_id":"invented-node-id","online":true}]}),
        );
        assert!(wait.await);
        apply_fabric_presence(
            &fleet,
            &serde_json::json!({"reset":false,"events":[{"peer_id":"configured-id","online":true}]}),
        );
        assert!(fleet.online.read().unwrap().contains_key("beacon"));
    }
}

#[derive(Clone)]
struct SlowExport {
    local: Local<Store>,
    slow: Arc<std::sync::atomic::AtomicBool>,
    fail_next: Arc<std::sync::atomic::AtomicBool>,
    receives: Arc<std::sync::atomic::AtomicUsize>,
}

impl Backend for SlowExport {
    async fn export(
        &self,
        fleet_id: &str,
        inventory: &ReplicationInventory,
        summary_only: bool,
        signature_requests: &[ReplicaEnvelopeId],
    ) -> Result<ReplicationExportResponse> {
        if !summary_only
            && self
                .fail_next
                .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            anyhow::bail!("injected export failure");
        }
        if !summary_only && self.slow.swap(false, std::sync::atomic::Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_secs(25)).await;
        }
        self.local
            .export(fleet_id, inventory, summary_only, signature_requests)
            .await
    }
    async fn receive(
        &self,
        peer: &str,
        fleet_id: &str,
        exchange: &ReplicationExchange,
        round_trip: Option<Duration>,
        carrier: crate::replication::ReplicationCarrier,
    ) -> Result<ReplicationReceiveResponse> {
        let result = self
            .local
            .receive(peer, fleet_id, exchange, round_trip, carrier)
            .await?;
        self.receives
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(result)
    }
    async fn checkpoint_manifest(
        &self,
        request: &CheckpointManifestRequest,
    ) -> Result<CheckpointManifestPage> {
        self.local.checkpoint_manifest(request).await
    }
    async fn checkpoint_need(&self) -> Result<Option<CheckpointManifestNeed>> {
        self.local.checkpoint_need().await
    }
    async fn adopt_checkpoint(
        &self,
        manifest: &CheckpointManifest,
    ) -> Result<Vec<CheckpointAction>> {
        self.local.adopt_checkpoint(manifest).await
    }
    async fn publish_endpoints(&self, mode: &str, endpoints: &[Value]) -> Result<()> {
        self.local.publish_endpoints(mode, endpoints).await
    }
    async fn redeem(&self, request: &JoinRequest) -> Result<Value> {
        self.local.redeem(request).await
    }
    async fn fleet_view(&self) -> Result<FleetView> {
        self.local.fleet_view().await
    }
    async fn record_failure(
        &self,
        peer: &str,
        status: &str,
        error: &str,
        carrier: Option<crate::replication::ReplicationCarrier>,
        failure: Option<&crate::replication::ReplicationExchangeFailure>,
    ) -> Result<()> {
        self.local.record_failure(peer, status, error, carrier, failure)
            .await
    }
}

/// Isolated two-node proof: a cancelled response leaves admitted envelopes intact;
/// identical signed retries share backend work while a 25-second export exceeds the
/// old caller deadline. No live graph, transport or fleet configuration is used.
#[tokio::test]
async fn slow_export_returns_signed_overload_and_preserves_interrupted_receipt() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    let fleet_id = "1f91ca65-7793-48cc-866e-ac15690130e1";
    let auth = FleetAuth::test(fleet_id, &[7; 32]);
    let left = Arc::new(plain_open(&root.path().join("birch.sqlite3"), "birch").unwrap());
    let right = Arc::new(plain_open(&root.path().join("cedar.sqlite3"), "cedar").unwrap());
    for store in [&left, &right] {
        store.bind_fleet(fleet_id).unwrap();
        store
            .append_claim(&ClaimInput {
                subject: format!("note/{}", store.origin),
                kind: "example.note".into(),
                actor: None,
                fields: BTreeMap::from([("text".into(), Value::String(store.origin.clone()))]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let backend = SlowExport {
        local: Local(right.clone()),
        slow: Arc::new(AtomicBool::new(true)),
        fail_next: Arc::new(AtomicBool::new(false)),
        receives: Arc::new(AtomicUsize::new(0)),
    };
    let state = PeerState::new(
        backend.clone(),
        "cedar".into(),
        auth.clone(),
        FleetContext::legacy(BTreeSet::from(["birch".into()])),
    );
    let query = left
        .export_replication_exchange(fleet_id, &ReplicationInventory::default())
        .unwrap();
    let bytes = Bytes::from(serde_json::to_vec(&query).unwrap());
    let digest = FleetAuth::body_digest(&bytes);
    let headers = auth.request_headers("birch", &bytes).unwrap();
    let interrupted = tokio::spawn(receive_exchange(
        None,
        State(state.clone()),
        headers.clone(),
        bytes.clone(),
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        while backend.receives.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    interrupted.abort();
    assert!(
        right
            .latest_claim("note/birch", Some("example.note"))
            .unwrap()
            .is_some()
    );

    let started = tokio::time::Instant::now();
    let overload = receive_exchange(None, State(state.clone()), headers.clone(), bytes.clone()).await;
    assert_eq!(overload.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(started.elapsed() < REPLICATION_EXCHANGE_TIMEOUT);
    let overload_headers = overload.headers().clone();
    let body = to_bytes(overload.into_body(), MAX_EXCHANGE_BYTES)
        .await
        .unwrap();
    auth.verify_sender(
        &overload_headers,
        "RESPONSE",
        EXCHANGE_PATH,
        &body,
        Some("cedar"),
        Some(&digest),
    )
    .unwrap();
    assert!(
        auth.verify_sender(
            &overload_headers,
            "RESPONSE",
            EXCHANGE_PATH,
            &body,
            Some("cedar"),
            Some("different-request")
        )
        .is_err()
    );
    let answer: PeerResponse<Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(answer.value["code"], "replication-overloaded");
    assert_eq!(answer.value["retry_after_ms"], 5000);
    assert_eq!(
        backend.receives.load(Ordering::Relaxed),
        1,
        "retry must reuse admitted work"
    );

    // A different request from the same peer is refused immediately without adding work.
    let changed = Bytes::from(
        serde_json::to_vec(&left.export_replication_summary(fleet_id).unwrap()).unwrap(),
    );
    let busy = receive_exchange(
        None,
        State(state.clone()),
        auth.request_headers("birch", &changed).unwrap(),
        changed,
    )
    .await;
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(backend.receives.load(Ordering::Relaxed), 1);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "cedar".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let app = peer_router(state, Router::new()).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            count.fetch_add(1, Ordering::Relaxed);
            async move { next.run(request).await }
        },
    ));
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let fleet = FleetContext::legacy(BTreeSet::from(["cedar".into()]));
    let (answer, _) = post_signed(
        &replication_http_client(),
        &Local(left.clone()),
        &peer,
        "birch",
        &auth,
        &fleet,
        &query,
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        requests.load(Ordering::Relaxed),
        1,
        "completed in-flight export is fetched, not restarted"
    );
    assert_eq!(backend.receives.load(Ordering::Relaxed), 1);
    Local(left.clone())
        .receive("cedar", fleet_id, &answer, None, Default::default())
        .await
        .unwrap();
    for store in [&left, &right] {
        for writer in ["birch", "cedar"] {
            assert_eq!(
                store
                    .claims_for(&format!("note/{writer}"), Some("example.note"))
                    .unwrap()
                    .len(),
                1
            );
        }
    }
    // Once an answer is delivered, a later identical query must export fresh data.
    post_signed(
        &replication_http_client(),
        &Local(left.clone()),
        &peer,
        "birch",
        &auth,
        &fleet,
        &query,
        false,
    )
    .await
    .unwrap();
    assert_eq!(backend.receives.load(Ordering::Relaxed), 2);
    assert_eq!(
        right
            .claims_for("note/birch", Some("example.note"))
            .unwrap()
            .len(),
        1
    );
    exchange(
        &replication_http_client(),
        &Local(left.clone()),
        "birch",
        &peer,
        &auth,
        &fleet,
    )
    .await
    .unwrap();
    assert_eq!(
        left.replication_inventory().unwrap().digest,
        right.replication_inventory().unwrap().digest
    );
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn authenticated_overload_polls_have_separate_time_and_request_bounds() {
    // Direct client/server HTTP with injected clock: every response is an authenticated
    // overload; the five-second minimum permits at most 60 requests in five minutes.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[7; 32]);
    let fleet = FleetContext::legacy(BTreeSet::from(["cedar".into()]));
    let state = PeerState::new(
        Local(Arc::new(plain_memory("cedar").unwrap())),
        "cedar".into(),
        auth.clone(),
        FleetContext::legacy(BTreeSet::from(["birch".into()])),
    );
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let app = Router::new().route(
        EXCHANGE_PATH,
        post(move |body: Bytes| {
            let state = state.clone();
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::Relaxed);
                signed_overload(&state, &FleetAuth::body_digest(&body))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "cedar".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    // Prevent automatic clock advances from racing socket readiness.
    let keep_awake = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    let started = tokio::time::Instant::now();
    let task = tokio::spawn(async move {
        post_signed(
            &replication_http_client(),
            &Local(Arc::new(plain_memory("birch").unwrap())),
            &peer,
            "birch",
            &auth,
            &fleet,
            &plain_memory("birch")
                .unwrap()
                .export_replication_summary(auth.fleet_id())
                .unwrap(),
            false,
        )
        .await
    });
    for _ in 0..65 {
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        if task.is_finished() {
            break;
        }
        tokio::time::advance(OVERLOAD_RETRY).await;
    }
    let error = task.await.unwrap().unwrap_err();
    assert!(error.is::<PeerOverloaded>(), "{error:#}");
    assert!(started.elapsed() <= EXCHANGE_POLL_BUDGET + OVERLOAD_RETRY);
    assert!(requests.load(Ordering::Relaxed) <= 60);
    assert!(requests.load(Ordering::Relaxed) > 1);
    keep_awake.abort();
    server.abort();
}

#[tokio::test]
async fn worker_phase_status_is_local_and_reports_retry_deadline() {
    let store = Arc::new(plain_memory("birch").unwrap());
    let index = store.index().unwrap();
    let attempt = crate::store::now_ms();
    for phase in ["exchange", "backoff", "idle"] {
        let retry = (phase == "backoff").then_some(Duration::from_secs(30));
        record_worker(&Local(store.clone()), "cedar", phase, attempt, retry, None).await;
        let status = store
            .replication_status(false, None, &["cedar".into()])
            .unwrap();
        let worker = status.peers[0].worker.as_ref().unwrap();
        assert_eq!(worker.phase, phase);
        assert_eq!(worker.last_attempt_at_unix_ms, attempt);
        assert_eq!(worker.next_retry_at_unix_ms.is_some(), retry.is_some());
    }
    assert_eq!(
        store.index().unwrap(),
        index,
        "worker state must never create replicated claims"
    );
    let fleet = FleetContext::legacy(BTreeSet::new());
    fleet.note_activity("cedar");
    assert_eq!(
        retry_delay(Duration::from_secs(3600), &fleet, "cedar"),
        Duration::from_secs(30)
    );
    assert_eq!(
        retry_delay(Duration::from_secs(3600), &fleet, "absent"),
        Duration::from_secs(3600)
    );
}

#[tokio::test]
async fn worker_route_status_survives_phase_updates_and_clears_preferred_retry() {
    let store = Arc::new(plain_memory("birch").unwrap());
    let backend = Local(store.clone());
    let attempt = crate::store::now_ms();
    let index = store.index().unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    record_worker(
        &backend,
        "cedar",
        "idle",
        attempt,
        None,
        Some((false, Some(deadline))),
    )
    .await;
    let status = || {
        serde_json::to_value(
            store
                .replication_status(false, None, &["cedar".into()])
                .unwrap(),
        )
        .unwrap()["peers"][0]["worker"]
            .clone()
    };
    let fallback = status();
    assert_eq!(fallback["route_is_preferred"], false);
    let retry = fallback["preferred_retry_at_unix_ms"].as_u64().unwrap();
    assert!(u128::from(retry) > attempt);
    assert!(u128::from(retry) <= crate::store::now_ms() + 60_000);
    // A lower-level exchange or probe updates phase without forgetting the chosen route.
    record_worker(&backend, "cedar", "exchange", attempt, None, None).await;
    assert_eq!(status()["route_is_preferred"], false);
    assert_eq!(status()["preferred_retry_at_unix_ms"], retry);
    // Failover can wrap to the first route before the existing cooldown expires.
    // Its pending offset-reset timer must not be presented as another preferred-route retry.
    record_worker(
        &backend,
        "cedar",
        "exchange",
        attempt,
        None,
        Some((true, Some(deadline))),
    )
    .await;
    assert_eq!(status()["route_is_preferred"], true);
    assert!(status().get("preferred_retry_at_unix_ms").is_none());
    assert_eq!(store.index().unwrap(), index, "route status is host-local");
    let legacy: crate::replication::ReplicationWorkerStatus = serde_json::from_value(
        serde_json::json!({"phase": "idle", "last_attempt_at_unix_ms": attempt,
            "next_retry_at_unix_ms": null}),
    )
    .unwrap();
    assert_eq!(legacy.route_is_preferred, None);
    assert_eq!(legacy.preferred_retry_at_unix_ms, None);
}

#[tokio::test]
async fn overload_requires_authentication_and_clamps_signed_retry_delays() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let auth = FleetAuth::test("invented-fleet", &[7; 32]);
    let query = plain_memory("birch")
        .unwrap()
        .export_replication_summary(auth.fleet_id())
        .unwrap();
    // Each request gets one controlled wire answer: unsigned, forged, replayed, then
    // authenticated delays below, within, and above the permitted range.
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let signing = auth.clone();
    let app = Router::new().route(
        EXCHANGE_PATH,
        post(move |body: Bytes| {
            let index = count.fetch_add(1, Ordering::Relaxed);
            let signing = signing.clone();
            async move {
                let millis = [0, 0, 0, 0, 4999, 10_000, 30_001, u64::MAX][index];
                let envelope = PeerResponse::new(
                    "cedar",
                    0,
                    serde_json::json!({
                        "code": "replication-overloaded", "retry_after_ms": millis
                    }),
                );
                let bytes = serde_json::to_vec(&envelope).unwrap();
                let mut response = (StatusCode::SERVICE_UNAVAILABLE, bytes.clone()).into_response();
                if index != 0 {
                    let auth = if index == 1 {
                        FleetAuth::test("invented-fleet", &[8; 32])
                    } else {
                        signing
                    };
                    let digest = if index == 2 {
                        "another-request".into()
                    } else {
                        FleetAuth::body_digest(&body)
                    };
                    response.headers_mut().extend(
                        auth.response_headers_for(EXCHANGE_PATH, "cedar", &bytes, &digest)
                            .unwrap(),
                    );
                }
                response
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = PeerConfig {
        name: "cedar".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let http = replication_http_client();
    let fleet = FleetContext::legacy(BTreeSet::from(["cedar".into()]));
    let local = Local(Arc::new(plain_memory("birch").unwrap()));
    for expected_count in 1..=3 {
        let error = post_signed(&http, &local, &peer, "birch", &auth, &fleet, &query, false)
            .await
            .unwrap_err();
        assert!(
            !error.is::<PeerOverloaded>(),
            "unverified overload must not enter polling: {error:#}"
        );
        assert_eq!(
            requests.load(Ordering::Relaxed),
            expected_count,
            "unverified overload must not be retried"
        );
        assert!(
            fleet.activity.read().unwrap().is_empty(),
            "unverified overload cannot mark the peer alive"
        );
    }
    for seconds in [5, 5, 10, 30, 30] {
        let error = post_signed_to::<_, ReplicationExchange>(
            &http,
            &peer,
            "birch",
            &auth,
            &fleet,
            EXCHANGE_PATH,
            &query,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<PeerOverloaded>().unwrap().retry_after,
            Duration::from_secs(seconds)
        );
        assert!(fleet.activity.read().unwrap().contains_key("cedar"));
    }
    server.abort();
}

#[tokio::test]
async fn failed_export_job_is_removed_and_identical_retry_preserves_receipt() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    let auth = FleetAuth::test("invented-fleet", &[7; 32]);
    let left = plain_open(&root.path().join("birch.sqlite3"), "birch").unwrap();
    let right = Arc::new(plain_open(&root.path().join("cedar.sqlite3"), "cedar").unwrap());
    for store in [&left, right.as_ref()] {
        store.bind_fleet(auth.fleet_id()).unwrap();
    }
    left.append_claim(&ClaimInput {
        subject: "note/copper".into(),
        kind: "example.note".into(),
        actor: None,
        fields: BTreeMap::from([("text".into(), Value::String("copper".into()))]),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: None,
    })
    .unwrap();
    let query = left
        .export_replication_exchange(auth.fleet_id(), &ReplicationInventory::default())
        .unwrap();
    let bytes = Bytes::from(serde_json::to_vec(&query).unwrap());
    let backend = SlowExport {
        local: Local(right.clone()),
        slow: Arc::new(AtomicBool::new(false)),
        fail_next: Arc::new(AtomicBool::new(true)),
        receives: Arc::new(AtomicUsize::new(0)),
    };
    let state = PeerState::new(
        backend.clone(),
        "cedar".into(),
        auth.clone(),
        FleetContext::legacy(BTreeSet::from(["birch".into()])),
    );
    for expected_status in [StatusCode::INTERNAL_SERVER_ERROR, StatusCode::OK] {
        let response = receive_exchange(
            None,
            State(state.clone()),
            auth.request_headers("birch", &bytes).unwrap(),
            bytes.clone(),
        )
        .await;
        assert_eq!(response.status(), expected_status);
        let headers = response.headers().clone();
        let answer = to_bytes(response.into_body(), MAX_EXCHANGE_BYTES)
            .await
            .unwrap();
        auth.verify_sender(
            &headers,
            "RESPONSE",
            EXCHANGE_PATH,
            &answer,
            Some("cedar"),
            Some(&FleetAuth::body_digest(&bytes)),
        )
        .unwrap();
        assert!(
            state.fleet.exchange_jobs.peers.lock().unwrap().is_empty(),
            "failed and delivered jobs must not pin the peer slot"
        );
        assert_eq!(
            right
                .claims_for("note/copper", Some("example.note"))
                .unwrap()
                .len(),
            1,
            "admitted envelope survives export failure and remains idempotent"
        );
    }
    assert_eq!(
        backend.receives.load(Ordering::Relaxed),
        2,
        "identical retry must recompute the failed job"
    );
}

#[tokio::test(start_paused = true)]
async fn an_always_failing_http_peer_cannot_refresh_its_own_probe_window() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let probes = Arc::new(AtomicUsize::new(0));
    let count = probes.clone();
    let app = Router::new().route(
        EXCHANGE_PATH,
        axum::routing::head(move || {
            count.fetch_add(1, Ordering::Relaxed);
            async { StatusCode::OK }
        })
        .post(|| async { StatusCode::UNAUTHORIZED }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let peer = PeerConfig {
        name: "cedar".into(),
        url: url.clone(),
    };
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let keep_awake = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    let fleet = FleetContext::legacy(BTreeSet::from(["cedar".into()]));
    let (_route_tx, mut routes) = watch::channel(vec![Route::Http(url.clone())]);
    let mut inbound = fleet.inbound_changed.subscribe();
    let mut activity = fleet.activity_changed.subscribe();
    let mut connectivity = fleet.connectivity_changed.subscribe();
    let http = replication_http_client();
    let auth = FleetAuth::test("invented-fleet", &[7; 32]);
    let store = Arc::new(plain_memory("birch").unwrap());
    let local = Local(store.clone());
    let query = store.export_replication_summary(auth.fleet_id()).unwrap();
    let previous = (url, tokio::time::Instant::now());
    // A recent successful exchange permits a cheap probe to shorten a retry once.
    let error = post_signed(&http, &local, &peer, "birch", &auth, &fleet, &query, false)
        .await
        .unwrap_err();
    assert!(!error.is::<PeerOverloaded>());
    {
        let started = tokio::time::Instant::now();
        let mut early_spent = BTreeSet::new();
        let wait = wait_peer_retry(
            &local,
            crate::store::now_ms(),
            Duration::from_secs(3600),
            &mut routes,
            &mut inbound,
            &mut activity,
            &mut connectivity,
            &fleet,
            "cedar",
            started,
            Some(&previous),
            &http,
            None,
            &mut early_spent,
        );
        tokio::pin!(wait);
        let mut completed = false;
        // Drive sockets without Tokio advancing time ahead of them.
        for _ in 0..35 {
            tokio::select! {
                result = &mut wait => { assert!(!result); completed = true; break; }
                _ = async { for _ in 0..100 { tokio::task::yield_now().await; } } => {}
            }
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        assert!(
            completed,
            "a recent HTTP route should recover within 30 seconds"
        );
    }
    assert!(probes.load(Ordering::Relaxed) > 0);
    assert!(fleet.activity.read().unwrap().is_empty());
    // The endpoint still answers, but repeated failing signed exchanges cannot extend
    // eligibility. Once the authenticated window expires, a full quiet wait resumes.
    tokio::time::advance(PEER_PROBE_WINDOW).await;
    let before = probes.load(Ordering::Relaxed);
    let error = post_signed(&http, &local, &peer, "birch", &auth, &fleet, &query, false)
        .await
        .unwrap_err();
    assert!(!error.is::<PeerOverloaded>());
    let started = tokio::time::Instant::now();
    let delay = retry_delay(Duration::from_secs(3600), &fleet, "cedar");
    assert_eq!(delay, Duration::from_secs(3600));
    let mut early_spent = BTreeSet::new();
    let wait = wait_peer_retry(
        &local,
        crate::store::now_ms(),
        delay,
        &mut routes,
        &mut inbound,
        &mut activity,
        &mut connectivity,
        &fleet,
        "cedar",
        started,
        Some(&previous),
        &http,
        None,
        &mut early_spent,
    );
    tokio::pin!(wait);
    for _ in 0..2 {
        tokio::select! {
            _ = &mut wait => panic!("an expired route must retain quiet backoff"),
            _ = async { for _ in 0..100 { tokio::task::yield_now().await; } } => {}
        }
        tokio::time::advance(Duration::from_secs(15)).await;
    }
    tokio::time::advance(Duration::from_secs(3570)).await;
    assert!(!wait.await);
    assert_eq!(
        probes.load(Ordering::Relaxed),
        before,
        "an expired route must not keep probing itself alive"
    );
    keep_awake.abort();
    server.abort();
}

#[tokio::test]
async fn concurrent_slow_exports_refuse_more_work_with_authenticated_overload() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(plain_open(&root.path().join("cedar.sqlite3"), "cedar").unwrap());
    let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[7; 32]);
    store.bind_fleet(auth.fleet_id()).unwrap();
    let fleet = FleetContext::legacy(
        ["birch", "alder", "ash", "fir", "elm"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    let query = store.export_replication_summary(auth.fleet_id()).unwrap();
    let received = Arc::new(AtomicUsize::new(0));
    let mut requests = Vec::new();
    for name in ["birch", "alder", "ash", "fir"] {
        let mut input = query.clone();
        input.peer = name.into();
        let bytes = Bytes::from(serde_json::to_vec(&input).unwrap());
        let backend = SlowExport {
            local: Local(store.clone()),
            slow: Arc::new(AtomicBool::new(true)),
            fail_next: Arc::new(AtomicBool::new(false)),
            receives: received.clone(),
        };
        let state = PeerState::new(backend, "cedar".into(), auth.clone(), fleet.clone());
        requests.push(tokio::spawn(receive_exchange(
            None,
            State(state),
            auth.request_headers(name, &bytes).unwrap(),
            bytes,
        )));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while received.load(Ordering::Relaxed) < MAX_EXCHANGE_JOBS {
            for request in &mut requests {
                if request.is_finished() {
                    let response = request.await.unwrap();
                    let status = response.status();
                    let body = to_bytes(response.into_body(), MAX_EXCHANGE_BYTES)
                        .await
                        .unwrap();
                    panic!(
                        "unexpected early admission response {status}: {}",
                        String::from_utf8_lossy(&body)
                    );
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut input = query;
    input.peer = "elm".into();
    let query = Bytes::from(serde_json::to_vec(&input).unwrap());
    let state = PeerState::new(Local(store), "cedar".into(), auth.clone(), fleet.clone());
    let started = tokio::time::Instant::now();
    let response = receive_exchange(
        None,
        State(state),
        auth.request_headers("elm", &query).unwrap(),
        query.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(started.elapsed() < Duration::from_secs(1));
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), MAX_EXCHANGE_BYTES)
        .await
        .unwrap();
    auth.verify_sender(
        &headers,
        "RESPONSE",
        EXCHANGE_PATH,
        &body,
        Some("cedar"),
        Some(&FleetAuth::body_digest(&query)),
    )
    .unwrap();
    assert_eq!(received.load(Ordering::Relaxed), MAX_EXCHANGE_JOBS);
    assert_eq!(
        fleet.exchange_jobs.peers.lock().unwrap().len(),
        MAX_EXCHANGE_JOBS
    );
    for request in requests {
        request.abort();
    }
}

/// Early recovery after an outage (st2#3079). A link on one held listener that the test switches
/// between the ways an outage and a recovery look to the worker.
mod early_recovery {
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    /// Down: accept each connection and close it without a byte, as the messaging-faults link
    /// does while disabled.
    const DOWN: u8 = 0;
    /// Up, refusing: answer HEAD with 405 and refuse each exchange with 401 after reading it, as a
    /// reachable older peer does.
    const REFUSING: u8 = 1;
    /// Up for HEAD only: answer HEAD with 405, but close each exchange without an answer.
    const HEAD_ONLY: u8 = 2;

    /// Read one request exactly, head and content-length body; `None` at the end of the stream.
    /// Answering with request bytes unread would let the close become a reset.
    async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<bool> {
        let mut received = Vec::new();
        let mut buffer = [0_u8; 8192];
        let head_end = loop {
            let count = stream.read(&mut buffer).await.ok()?;
            if count == 0 {
                return None;
            }
            received.extend_from_slice(&buffer[..count]);
            if let Some(at) = received.windows(4).position(|window| window == b"\r\n\r\n") {
                break at + 4;
            }
        };
        let head = String::from_utf8_lossy(&received[..head_end]).to_ascii_lowercase();
        let length: usize = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map_or(0, |value| value.trim().parse().unwrap());
        let mut body = received.len() - head_end;
        while body < length {
            let count = stream.read(&mut buffer).await.ok()?;
            if count == 0 {
                return None;
            }
            body += count;
        }
        Some(head.starts_with("post "))
    }

    struct Link {
        url: String,
        mode: Arc<AtomicU8>,
        /// Exchanges the peer behind the link read, in any mode but `DOWN`.
        exchanges: Arc<AtomicUsize>,
        server: tokio::task::JoinHandle<()>,
    }

    impl Link {
        async fn start(mode: u8) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let mode = Arc::new(AtomicU8::new(mode));
            let exchanges = Arc::new(AtomicUsize::new(0));
            let (serving, counted) = (mode.clone(), exchanges.clone());
            let server = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let (mode, exchanges) = (serving.clone(), counted.clone());
                    tokio::spawn(async move {
                        loop {
                            if mode.load(Ordering::SeqCst) == DOWN {
                                return;
                            }
                            let Some(post) = read_request(&mut stream).await else {
                                return;
                            };
                            if post {
                                exchanges.fetch_add(1, Ordering::SeqCst);
                            }
                            let answer: &[u8] = match (mode.load(Ordering::SeqCst), post) {
                                (DOWN, _) | (HEAD_ONLY, true) => return,
                                (_, true) => {
                                    b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n"
                                }
                                (_, false) => {
                                    b"HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\n\r\n"
                                }
                            };
                            if stream.write_all(answer).await.is_err() {
                                return;
                            }
                        }
                    });
                }
            });
            Self {
                url,
                mode,
                exchanges,
                server,
            }
        }

        fn set(&self, mode: u8) {
            self.mode.store(mode, Ordering::SeqCst);
        }
    }

    /// One worker's view of the peer behind a link: what `dial_peer` holds between attempts.
    struct Worker {
        fleet: FleetContext,
        routes: watch::Receiver<Vec<Route>>,
        _route_tx: watch::Sender<Vec<Route>>,
        inbound: watch::Receiver<u64>,
        activity: watch::Receiver<u64>,
        connectivity: watch::Receiver<u64>,
        http: reqwest::Client,
        auth: FleetAuth,
        local: Local<Store>,
        query: ReplicationExchange,
        peer: PeerConfig,
        /// The route's last successful exchange, just before the outage.
        previous: (String, tokio::time::Instant),
        early_spent: BTreeSet<String>,
        attempts: usize,
    }

    impl Worker {
        fn new(link: &Link) -> Self {
            let fleet = FleetContext::legacy(BTreeSet::from(["cedar".into()]));
            let (route_tx, routes) = watch::channel(vec![Route::Http(link.url.clone())]);
            let inbound = fleet.inbound_changed.subscribe();
            let activity = fleet.activity_changed.subscribe();
            let connectivity = fleet.connectivity_changed.subscribe();
            let auth = FleetAuth::test("invented-fleet", &[7; 32]);
            let store = Arc::new(plain_memory("birch").unwrap());
            let query = store.export_replication_summary(auth.fleet_id()).unwrap();
            Self {
                fleet,
                routes,
                _route_tx: route_tx,
                inbound,
                activity,
                connectivity,
                http: replication_http_client(),
                auth,
                local: Local(store),
                query,
                peer: PeerConfig {
                    name: "cedar".into(),
                    url: link.url.clone(),
                },
                previous: (link.url.clone(), tokio::time::Instant::now()),
                early_spent: BTreeSet::new(),
                attempts: 0,
            }
        }

        /// One failed signed exchange, and whether it went unanswered on its route.
        async fn attempt(&mut self) -> Option<String> {
            self.attempts += 1;
            let error = post_signed(
                &self.http,
                &self.local,
                &self.peer,
                "birch",
                &self.auth,
                &self.fleet,
                &self.query,
                false,
            )
            .await
            .unwrap_err();
            exchange_unanswered(&error).then(|| self.peer.url.clone())
        }

        /// Wait out a failure, advancing paused time a second at a time while sockets run, and
        /// return the seconds the wait took, or `None` when it outlasted `limit` seconds.
        async fn wait(&mut self, unanswered: Option<&str>, limit: u64) -> Option<u64> {
            let started = tokio::time::Instant::now();
            let wait = wait_peer_retry(
                &self.local,
                crate::store::now_ms(),
                Duration::from_secs(3600),
                &mut self.routes,
                &mut self.inbound,
                &mut self.activity,
                &mut self.connectivity,
                &self.fleet,
                "cedar",
                started,
                Some(&self.previous),
                &self.http,
                unanswered,
                &mut self.early_spent,
            );
            tokio::pin!(wait);
            for _ in 0..limit {
                tokio::select! {
                    woken = &mut wait => {
                        assert!(!woken, "only a timer or a probe ends these waits");
                        return Some(started.elapsed().as_secs());
                    }
                    _ = async { for _ in 0..100 { tokio::task::yield_now().await; } } => {}
                }
                tokio::time::advance(Duration::from_secs(1)).await;
            }
            None
        }
    }

    fn keep_awake() -> tokio::task::JoinHandle<()> {
        tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        })
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_restored_right_after_an_unanswered_exchange_is_retried_at_its_first_probe() {
        let awake = keep_awake();
        let link = Link::start(DOWN).await;
        let mut worker = Worker::new(&link);
        let unanswered = worker.attempt().await;
        assert_eq!(unanswered.as_deref(), Some(link.url.as_str()));
        // The link returns at once, to a reachable peer that answers HEAD with 405.
        link.set(REFUSING);
        let waited = worker.wait(unanswered.as_deref(), 10).await;
        let waited = waited.expect("the retry goes out within the 10-second recovery bound");
        assert!(
            waited <= PEER_PROBE_INTERVAL.as_secs() + 1,
            "retried at the first probe after the link returned: {waited} s"
        );
        assert!(
            worker.fleet.activity.read().unwrap().is_empty(),
            "a probe is not activity"
        );
        // The early attempt reaches the peer, which refuses it: a reachable failure, which keeps
        // the 30-second cap although the same route answers HEAD with 405.
        let reached = worker.attempt().await;
        assert_eq!(reached, None);
        assert_eq!(link.exchanges.load(Ordering::SeqCst), 1);
        assert_eq!(worker.wait(reached.as_deref(), 25).await, None);
        awake.abort();
        link.server.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn an_early_attempt_is_spent_until_a_probe_sees_the_route_down_again() {
        let awake = keep_awake();
        // The link answers HEAD but drops every exchange: up for the probe, down for the POST.
        let link = Link::start(HEAD_ONLY).await;
        let mut worker = Worker::new(&link);
        let first = worker.attempt().await;
        assert!(first.is_some());
        let early = worker.wait(first.as_deref(), 10).await;
        assert!(early.is_some_and(|waited| waited <= PEER_PROBE_INTERVAL.as_secs() + 1));
        // The early attempt fails the same way. Its route still answers HEAD, so no second early
        // attempt: the 30-second cap holds.
        let second = worker.attempt().await;
        assert!(second.is_some());
        assert_eq!(worker.wait(second.as_deref(), 25).await, None);
        // A probe that finds the route down re-arms it, and the next answer releases one more.
        let third = worker.attempt().await;
        link.set(DOWN);
        assert_eq!(worker.wait(third.as_deref(), 4).await, None);
        link.set(HEAD_ONLY);
        let rearmed = worker.wait(third.as_deref(), 10).await;
        assert!(rearmed.is_some_and(|waited| waited <= PEER_PROBE_INTERVAL.as_secs() + 1));
        // Three logical attempts, each an export and a signature; two were released
        // early, one per observed outage/recovery. Each unanswered attempt has one
        // additional wire send through a fresh connection, not another recovery credit.
        assert_eq!(worker.attempts, 3);
        assert_eq!(link.exchanges.load(Ordering::SeqCst), 2 * worker.attempts);
        awake.abort();
        link.server.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_probe_of_another_route_or_past_its_window_releases_nothing_early() {
        let awake = keep_awake();
        let link = Link::start(DOWN).await;
        let mut worker = Worker::new(&link);
        let unanswered = worker.attempt().await;
        link.set(REFUSING);
        // The failure was on another route: this route answering says nothing about it.
        assert_eq!(worker.wait(Some("http://127.0.0.1:1"), 25).await, None);
        // Past the probe window there are no probes, so nothing is released or re-armed.
        tokio::time::advance(PEER_PROBE_WINDOW).await;
        assert_eq!(worker.wait(unanswered.as_deref(), 45).await, None);
        assert!(worker.early_spent.is_empty());
        awake.abort();
        link.server.abort();
    }

    /// A link in front of a real peer that, for a span of paused time, drops every open
    /// connection and closes each new one without an answer, as the messaging-faults link does
    /// between `link.drop()` and `link.enabled = True`. Otherwise it forwards bytes both ways.
    struct Outage {
        url: String,
        down_until: Arc<Mutex<tokio::time::Instant>>,
        dropped: Arc<watch::Sender<u64>>,
        /// Exchanges that reached the link while it was down.
        refused: Arc<AtomicUsize>,
        server: tokio::task::JoinHandle<()>,
    }

    impl Outage {
        async fn start(peer: SocketAddr) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let down_until = Arc::new(Mutex::new(tokio::time::Instant::now()));
            let dropped = Arc::new(watch::channel(0).0);
            let refused = Arc::new(AtomicUsize::new(0));
            let (until, drops, counted) = (down_until.clone(), dropped.clone(), refused.clone());
            let server = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let down = tokio::time::Instant::now() < *until.lock().unwrap();
                    let (mut dropped, counted) = (drops.subscribe(), counted.clone());
                    tokio::spawn(async move {
                        if down {
                            if read_request(&mut stream).await == Some(true) {
                                counted.fetch_add(1, Ordering::SeqCst);
                            }
                            return;
                        }
                        let Ok(mut upstream) = tokio::net::TcpStream::connect(peer).await else {
                            return;
                        };
                        tokio::select! {
                            _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream) => {}
                            _ = dropped.changed() => {}
                        }
                    });
                }
            });
            Self {
                url,
                down_until,
                dropped,
                refused,
                server,
            }
        }

        /// Drop every open connection and refuse new ones for `outage` of paused time.
        fn drop_for(&self, outage: Duration) {
            *self.down_until.lock().unwrap() = tokio::time::Instant::now() + outage;
            self.dropped.send_modify(|generation| *generation += 1);
        }
    }

    /// The per-PR contract for the messaging-faults `link_seconds` case: a link outage of a few
    /// seconds, after a healthy exchange, resumes replication within that case's 10-second read
    /// deadline, without spinning on the dead link while it is down.
    #[tokio::test(start_paused = true)]
    async fn a_seconds_long_link_outage_resumes_exchange_within_the_read_deadline() {
        const STEP: Duration = Duration::from_millis(100);
        let awake = keep_awake();
        let fleet_id = "d706eab5-433c-474b-ae43-fd2a7891d85d";
        let auth = FleetAuth::test(fleet_id, &[42; 32]);
        let root = tempfile::tempdir().unwrap();
        let local = Arc::new(plain_open(&root.path().join("harbor.sqlite3"), "harbor").unwrap());
        let remote = Arc::new(plain_memory("beacon").unwrap());
        for store in [&local, &remote] {
            store.bind_fleet(fleet_id).unwrap();
        }
        // When each exchange reached the peer, in paused time.
        let exchanges = Arc::new(Mutex::new(Vec::new()));
        let observed = exchanges.clone();
        let app = peer_router(
            PeerState {
                backend: Local(remote.clone()),
                node: "beacon".into(),
                auth: auth.clone(),
                fleet: FleetContext::legacy(BTreeSet::from(["harbor".into()])),
                outbound_notify: watch::channel(0).0,
                shards: Default::default(),
            },
            Router::new(),
        )
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                if request.method() == axum::http::Method::POST {
                    observed.lock().unwrap().push(tokio::time::Instant::now());
                }
                next.run(request)
            },
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let link = Outage::start(listener.local_addr().unwrap()).await;
        let peer = tokio::spawn(axum::serve(listener, app).into_future());
        let (_route_tx, routes) = watch::channel(vec![Route::Http(link.url.clone())]);
        let (notify, _) = watch::channel(0);
        let dialer = tokio::spawn(dial_peer(
            Local(local.clone()),
            "harbor".into(),
            "beacon".into(),
            routes,
            auth,
            FleetContext::legacy(BTreeSet::from(["beacon".into()])),
            notify.subscribe(),
        ));
        fn phase(local: &Store) -> Option<String> {
            local
                .replication_sync
                .lock()
                .unwrap()
                .get("beacon")
                .and_then(|progress| progress.worker.clone())
                .map(|worker| worker.phase)
        }
        // Paused time must not outrun loopback sockets or the store's blocking work: let an
        // exchange in flight finish before each step of the clock.
        async fn step(local: &Store) {
            let started = std::time::Instant::now();
            loop {
                for _ in 0..100 {
                    tokio::task::yield_now().await;
                }
                if phase(local).as_deref() != Some("exchange")
                    || started.elapsed() > Duration::from_secs(10)
                {
                    break;
                }
            }
            tokio::time::advance(STEP).await;
        }
        // A warm link, as the case's cross-node warmup leaves it: the first exchanges have
        // settled and the worker has been idle for two seconds.
        let warm = tokio::time::Instant::now() + Duration::from_secs(20);
        while !(phase(&local).as_deref() == Some("idle")
            && exchanges
                .lock()
                .unwrap()
                .last()
                .is_some_and(|at| at.elapsed() >= Duration::from_secs(2)))
        {
            assert!(tokio::time::Instant::now() < warm, "no healthy exchange");
            step(&local).await;
        }
        // The link drops, and a message is written while it is down.
        let outage = Duration::from_secs(3);
        let started = tokio::time::Instant::now();
        let ended = started + outage;
        link.drop_for(outage);
        fleet_claim(
            &local,
            "transport.observed",
            "host/harbor",
            serde_json::json!({"status": "up"}),
        );
        notify.send_modify(|generation| *generation += 1);
        let resumed = loop {
            if let Some(at) = exchanges
                .lock()
                .unwrap()
                .iter()
                .copied()
                .find(|at| *at > started)
            {
                break at;
            }
            assert!(
                tokio::time::Instant::now() < ended + Duration::from_secs(30),
                "no exchange after the outage"
            );
            step(&local).await;
        };
        // No exchange reaches the peer through a dropped link, so the first one that does was
        // sent after the link returned.
        assert!(resumed >= ended);
        assert!(
            resumed - ended < Duration::from_secs(10),
            "the first exchange after the outage started {:?} after it ended",
            resumed - ended
        );
        // That exchange succeeds and carries the message written during the outage.
        let delivered = tokio::time::Instant::now() + Duration::from_secs(10);
        while !(phase(&local).as_deref() == Some("idle")
            && remote
                .latest_claim("host/harbor", Some("transport.observed"))
                .unwrap()
                .is_some())
        {
            assert!(
                tokio::time::Instant::now() < delivered,
                "the message did not replicate"
            );
            step(&local).await;
        }
        // Not a spin: the logical attempt budget remains the shortest-jitter
        // backoff schedule, with at most two wire sends per logical attempt.
        let (mut schedule, mut allowed) = (Duration::ZERO, 0);
        while schedule < outage {
            schedule += PeerBackoff::delay(allowed, 0);
            allowed += 1;
        }
        let refused = link.refused.load(Ordering::SeqCst);
        let allowed_sends = 2 * allowed as usize;
        assert!(
            (1..=allowed_sends).contains(&refused),
            "{refused} wire sends in a {outage:?} outage; backoff allows {allowed} logical attempts ({allowed_sends} sends)"
        );
        dialer.abort();
        peer.abort();
        awake.abort();
        link.server.abort();
    }

    /// The classification the early attempt rests on, for the pinned reqwest 0.12.28 and hyper
    /// 1.11.0: if a version bump changes how a close before the response head surfaces, this
    /// fails rather than the recovery silently reverting to the 30-second cap.
    #[tokio::test]
    async fn only_an_exchange_that_got_no_response_head_is_unanswered() {
        let refused = tokio::net::TcpSocket::new_v4().unwrap();
        refused.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let send = |url: String, timeout: Duration| async move {
            replication_http_client()
                .post(format!("{url}{EXCHANGE_PATH}"))
                .body(vec![b'x'; 64 * 1024])
                .timeout(timeout)
                .send()
                .await
        };
        let refused_error = send(
            format!("http://{}", refused.local_addr().unwrap()),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(exchange_unanswered(&anyhow::Error::new(refused_error)));
        let closing = Link::start(DOWN).await;
        for _ in 0..4 {
            let closed = send(closing.url.clone(), Duration::from_secs(5))
                .await
                .unwrap_err();
            assert!(
                exchange_unanswered(&anyhow::Error::new(closed).context("before headers")),
                "a close before any answer is unanswered"
            );
        }
        closing.server.abort();
        // A peer that reads the exchange and never answers was reached: it stalled.
        let stalling = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", stalling.local_addr().unwrap());
        let held = tokio::spawn(async move {
            let (mut stream, _) = stalling.accept().await.unwrap();
            read_request(&mut stream).await;
            std::future::pending::<()>().await;
        });
        let stalled = send(url, Duration::from_millis(500)).await.unwrap_err();
        assert!(stalled.is_timeout());
        assert!(!exchange_unanswered(&anyhow::Error::new(stalled)));
        held.abort();
        // A peer that answers every exchange with a redirect to itself did answer: reqwest gives
        // up after its redirect limit, and that is no outage.
        let redirecting = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", redirecting.local_addr().unwrap());
        let looping = tokio::spawn(async move {
            loop {
                let (mut stream, _) = redirecting.accept().await.unwrap();
                tokio::spawn(async move {
                    while read_request(&mut stream).await.is_some() {
                        let answer = format!(
                            "HTTP/1.1 302 Found\r\nlocation: {EXCHANGE_PATH}\r\ncontent-length: 0\r\n\r\n"
                        );
                        if stream.write_all(answer.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        let redirected = send(url, Duration::from_secs(5)).await.unwrap_err();
        assert!(redirected.is_redirect(), "{redirected:?}");
        assert!(!exchange_unanswered(&anyhow::Error::new(redirected)));
        looping.abort();
        // A request this node could not build never left it.
        let unbuilt = replication_http_client()
            .post("not a url")
            .send()
            .await
            .unwrap_err();
        assert!(unbuilt.is_builder(), "{unbuilt:?}");
        assert!(!exchange_unanswered(&anyhow::Error::new(unbuilt)));
    }
}

#[tokio::test]
async fn signed_backend_metadata_auth_and_protocol_errors_keep_the_current_route() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn backoff_after(
        store: &Store,
        preferred: &AtomicUsize,
        fallback: &AtomicUsize,
        attempts: usize,
    ) -> crate::replication::ReplicationWorkerStatus {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                assert_eq!(fallback.load(Ordering::SeqCst), 0, "fallback was selected");
                let worker = store
                    .replication_sync
                    .lock()
                    .unwrap()
                    .get("peer")
                    .and_then(|progress| progress.worker.clone());
                if let Some(worker) = worker
                    && worker.phase == "backoff"
                    && preferred.load(Ordering::SeqCst) >= attempts
                {
                    return worker;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the real dialer must retry the application failure with backoff")
    }

    let fleet_id = "fdf00ee7-a1f7-4ed7-a5ec-a6558b61cd92";
    let auth = FleetAuth::test(fleet_id, &[53; 32]);
    let state = PeerState::new(
        Local(Arc::new(plain_memory("peer").unwrap())),
        "peer".into(),
        auth.clone(),
        FleetContext::legacy(BTreeSet::from(["local".into()])),
    );
    for (status, code) in [
        (StatusCode::INTERNAL_SERVER_ERROR, "replication-request-failed"),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "replication-exchange-metadata-too-large",
        ),
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            "replication-exchange-metadata-too-large",
        ),
        (StatusCode::UNAUTHORIZED, "auth-failed"),
        (StatusCode::UNPROCESSABLE_ENTITY, "invalid-exchange"),
        (StatusCode::OK, "invalid-protocol"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let local = Arc::new(plain_open(&root.path().join("local.sqlite3"), "local").unwrap());
        local.bind_fleet(fleet_id).unwrap();
        let preferred_attempts = Arc::new(AtomicUsize::new(0));
        let fallback_attempts = Arc::new(AtomicUsize::new(0));
        let observed = preferred_attempts.clone();
        let state = state.clone();
        let app = Router::new().route(
            EXCHANGE_PATH,
            post(move |body: Bytes| {
                let state = state.clone();
                let observed = observed.clone();
                async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    // A signed success carrying the wrong type exercises protocol decoding;
                    // all other cases are signed application refusals.
                    let mut response = signed_response(
                        &state,
                        &FleetAuth::body_digest(&body),
                        0,
                        serde_json::json!({"code": code}),
                    )
                    .unwrap();
                    *response.status_mut() = status;
                    response
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let preferred = Route::Http(format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(axum::serve(listener, app).into_future());
        let observed = fallback_attempts.clone();
        let fallback_app = Router::new().route(
            EXCHANGE_PATH,
            post(move |_: Bytes| {
                let observed = observed.clone();
                async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fallback = Route::Http(format!("http://{}", listener.local_addr().unwrap()));
        let fallback_server = tokio::spawn(axum::serve(listener, fallback_app).into_future());
        let (_routes, routes) = watch::channel(vec![preferred, fallback]);
        let (notify, _) = watch::channel(0);
        let dialer = tokio::spawn(dial_peer(
            Local(local.clone()),
            "local".into(),
            "peer".into(),
            routes,
            auth.clone(),
            FleetContext::legacy(BTreeSet::from(["peer".into()])),
            notify.subscribe(),
        ));
        let first = backoff_after(&local, &preferred_attempts, &fallback_attempts, 1).await;
        let second = backoff_after(&local, &preferred_attempts, &fallback_attempts, 2).await;
        dialer.abort();
        server.abort();
        fallback_server.abort();
        assert_eq!(first.route_is_preferred, Some(true), "{code}");
        assert_eq!(second.route_is_preferred, Some(true), "{code}");
        assert!(first.next_retry_at_unix_ms.is_some(), "{code}");
        assert!(second.next_retry_at_unix_ms.is_some(), "{code}");
        assert!(
            second.last_attempt_at_unix_ms > first.last_attempt_at_unix_ms,
            "{code}: the normal backoff must lead to another preferred-route attempt"
        );
        assert_eq!(fallback_attempts.load(Ordering::SeqCst), 0, "{code}");
    }
}

fn claim_walk_note(store: &Store, subject: &str) -> String {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
        .id
}

/// Walk `target`'s view of `source`'s claim ranges to the end, the way the worker does, with
/// each query carried on an inventory that matches the source's own: envelope transport has
/// nothing to move, so only the walk can bring a claim over. Returns the queries sent.
fn walk_to_the_end(source: &Store, target: &Store, fleet: &str) -> Vec<crate::replication::ClaimRangeQuery> {
    let first = target
        .receive_replication_exchange(&source.origin, fleet, &source.export_replication_summary(fleet).unwrap())
        .unwrap();
    let Some(comparison) = first.claim_ranges.filter(|comparison| !all_same(comparison)) else {
        return Vec::new();
    };
    let mut walk = if comparison.same {
        ClaimWalk::finished(&comparison.pair)
    } else {
        ClaimWalk::new(&comparison.pair)
    };
    walk.follow_retired(&comparison);
    let mut queries = Vec::new();
    while let Some(query) = walk.next_query() {
        assert!(queries.len() < 64, "a walk ends");
        queries.push(query.clone());
        let mut inventory = source.export_replication_summary(fleet).unwrap().inventory;
        inventory.claim_range_query = Some(query.clone());
        let answer = source.export_replication_exchange(fleet, &inventory).unwrap();
        let receipt = target.receive_replication_exchange(&source.origin, fleet, &answer).unwrap();
        target.validate_replication_backlog().unwrap();
        if let Some(comparison) = &receipt.claim_ranges {
            walk.absorb(query, comparison);
        }
    }
    queries
}

fn claim_walk_pair(fleet: &str) -> (Store, Store) {
    let source = plain_memory("source").unwrap();
    let target = plain_memory("target").unwrap();
    source.bind_fleet(fleet).unwrap();
    target.bind_fleet(fleet).unwrap();
    for n in 0..20 {
        claim_walk_note(&source, &format!("note/shared-{n}"));
    }
    let push = source
        .export_replication_exchange(fleet, &ReplicationInventory::default())
        .unwrap();
    target.receive_replication_exchange("source", fleet, &push).unwrap();
    target.validate_replication_backlog().unwrap();
    (source, target)
}

fn fold_claim_ranges(stores: &[&Store]) {
    for store in stores {
        store.fold_claim_ranges_paced(|_| {}).unwrap();
    }
}

#[test]
fn a_claim_walk_fetches_exactly_the_claims_a_node_lacks() {
    let fleet = "7c0a1d2e-3f40-4a5b-8c6d-7e8f90a1b2c3";
    let (source, target) = claim_walk_pair(fleet);
    let lacking: BTreeSet<String> =
        (0..3).map(|n| claim_walk_note(&source, &format!("note/only-source-{n}"))).collect();
    source.export_replication_summary(fleet).unwrap();
    fold_claim_ranges(&[&source, &target]);

    let queries = walk_to_the_end(&source, &target, fleet);
    assert!(queries[0].tops, "a walk starts at the tops");
    let asked: BTreeSet<String> = queries.iter().flat_map(|query| query.claims.clone()).collect();
    assert_eq!(asked, lacking, "it asks for exactly the claims it lacks");
    assert!(queries.iter().all(|query| query.leaves_of.len() <= crate::replication::CLAIM_RANGE_QUERY_TOPS
        && query.ids_of.len() <= crate::replication::CLAIM_RANGE_QUERY_LEAVES));
    for id in &lacking {
        assert!(target.claim_by_id(id).unwrap().is_some(), "fetched through ordinary admission");
    }
    fold_claim_ranges(&[&target]);
    let summary = source.claim_range_summary().unwrap();
    assert!(target.compare_claim_ranges(&summary).unwrap().unwrap().same, "the same claims");
}

#[test]
fn a_claim_walk_asks_for_nothing_this_node_holds_or_retired() {
    let fleet = "7c0a1d2e-3f40-4a5b-8c6d-7e8f90a1b2c4";
    let (source, target) = claim_walk_pair(fleet);
    let retired = claim_walk_note(&source, "note/retired-at-target");
    source.export_replication_summary(fleet).unwrap();
    claim_walk_note(&target, "note/only-target");
    target
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-h5c50c3e143a8 owner=smallclaims::sync::worker_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO retired_claim_ids(id) VALUES (unhex(?1))", [&retired])
        .unwrap();
    fold_claim_ranges(&[&source, &target]);

    let queries = walk_to_the_end(&source, &target, fleet);
    assert!(!queries.is_empty(), "the claim sets differ, so the walk runs");
    assert!(queries.iter().all(|query| query.claims.is_empty()), "{queries:?}");
    assert!(target.claim_by_id(&retired).unwrap().is_none(), "a retired claim is never brought back");
}

#[test]
fn a_finished_claim_walk_is_not_repeated_for_the_same_claims() {
    let mut walk = ClaimWalk::new("ours/theirs");
    let tops = walk.next_query().unwrap();
    assert!(tops.tops);
    walk.absorb(tops, &crate::replication::ClaimRangeComparison {
        pair: "ours/theirs".into(),
        tops: vec![3],
        ..Default::default()
    });
    assert_eq!(walk.next_query().unwrap().leaves_of, vec![3]);
    assert_eq!(walk.next_query(), None);
    assert_eq!(walk.next_query(), None, "nothing is asked again until the claims change");

    let mut walk = ClaimWalk::new("ours/theirs");
    let tops = walk.next_query().unwrap();
    walk.retry(tops.clone());
    assert_eq!(walk.next_query(), Some(tops), "a peer still folding is asked again");
}

#[tokio::test]
async fn a_claim_walk_runs_over_the_signed_peer_route() {
    let fleet = "7c0a1d2e-3f40-4a5b-8c6d-7e8f90a1b2c5";
    let auth = FleetAuth::test(fleet, &[7; 32]);
    let (source, target) = claim_walk_pair(fleet);
    let lacking = claim_walk_note(&source, "note/only-source");
    source.export_replication_summary(fleet).unwrap();
    // The route records each sender as up; record it first, so the source's index is folded
    // when it answers.
    source.record_transport_observation("target", "up", None, None).unwrap();
    target.record_transport_observation("source", "up", None, None).unwrap();
    fold_claim_ranges(&[&source, &target]);
    let (source, target) = (Arc::new(source), Arc::new(target));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = PeerState {
        backend: Local(source.clone()),
        node: "source".into(),
        auth: auth.clone(),
        fleet: FleetContext::legacy(BTreeSet::from(["target".into()])),
        outbound_notify: watch::channel(0_u64).0,
        shards: Default::default(),
    };
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route(EXCHANGE_PATH, post(receive_exchange))
                .with_state(state)
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .into_future(),
    );
    let peer = PeerConfig { name: "source".into(), url: format!("http://{address}") };
    // As if envelope transport had nothing left to move, the target presents the source's own
    // inventory, so only the walk can bring the claim over.
    let mut summary = target.export_replication_summary(fleet).unwrap();
    summary.inventory = source.export_replication_summary(fleet).unwrap().inventory;
    let comparison = target
        .compare_claim_ranges(&source.claim_range_summary().unwrap())
        .unwrap()
        .filter(|comparison| !comparison.same)
        .expect("the claim sets differ");
    let fetched = walk_claim_ranges(
        &replication_http_client(),
        &Local(target.clone()),
        "target",
        &peer,
        &auth,
        &FleetContext::legacy(BTreeSet::from(["source".into()])),
        &mut crate::sync::telemetry::Round::new("source"),
        Default::default(),
        std::time::Instant::now(),
        &summary,
        &comparison,
    )
    .await
    .unwrap();
    assert!(fetched, "tops, leaves, IDs and the claim fit in one exchange's round trips");
    assert!(target.claim_by_id(&lacking).unwrap().is_some());
    server.abort();
}

fn retire_claim_ids(store: &Store, ids: &[String]) {
    let connection = store.connection.write();
    for id in ids {
        connection
            .execute(
                &format!(
                    /* sqlite-large-value: allow-for-now lv-hbdb013909404 owner=smallclaims::sync::worker_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO retired_claim_ids(id) VALUES ({})",
                    crate::store::retired::identity_sql("?1")
                ),
                [id],
            )
            .unwrap();
    }
}

fn local_trim_member(origin: &str, fleet: &str) -> Store {
    let store = plain_memory(origin).unwrap();
    store.bind_fleet(fleet).unwrap();
    store.import_retired_identities_paced(|_| {}).unwrap();
    store.trim_locally(true).unwrap();
    store
}

/// Push everything `from` holds to `to`, as an older build or a first sync does, whatever
/// `to`'s inventory lists.
fn push_all(from: &Store, from_name: &str, to: &Store, fleet: &str) {
    let push = from.export_replication_exchange(fleet, &ReplicationInventory::default()).unwrap();
    to.receive_replication_exchange(from_name, fleet, &push).unwrap();
    to.validate_replication_backlog().unwrap();
}

fn retired_summary(store: &Store) -> Option<crate::replication::ClaimRangeDigest> {
    store.claim_range_summary().unwrap().retired.and_then(|retired| retired.retained)
}

#[test]
fn members_that_trim_locally_learn_each_others_retired_claims_by_walking_ranges() {
    let fleet = "7c0a1d2e-3f40-4a5b-8c6d-7e8f90a1b2d1";
    let source = local_trim_member("source", fleet);
    let target = local_trim_member("target", fleet);
    // Retired IDs spread over many leaves, and one claim the target still holds.
    let held = claim_walk_note(&target, "note/held-at-target");
    let mut retired: Vec<String> = (0..100u32)
        .map(|n| {
            use sha2::Digest as _;
            hex::encode(sha2::Sha256::digest(n.to_be_bytes()))
        })
        .collect();
    retired.push(held.clone());
    retire_claim_ids(&source, &retired);
    fold_claim_ranges(&[&source, &target]);
    assert!(retired_summary(&source).is_some(), "a member that trims locally summarizes its retired IDs");

    let queries = walk_to_the_end(&source, &target, fleet);
    assert!(queries.iter().any(|query| query.retired.is_some()), "{queries:?}");
    assert!(queries.iter().all(|query| query.claims.is_empty()), "a retired ID is recorded, never fetched");
    let connection = target.readers.get();
    for id in &retired {
        assert!(crate::store::retired::claim_retired(&connection, id).unwrap(), "{id}");
    }
    drop(connection);
    fold_claim_ranges(&[&target]);
    assert_eq!(retired_summary(&target), retired_summary(&source));
    // What it learned it neither serves nor takes back.
    assert!(target.envelopes_carrying_claims(std::slice::from_ref(&held)).unwrap().is_empty());

    // An ordinary member neither sends its retired IDs nor learns a peer's.
    let ordinary = plain_memory("ordinary").unwrap();
    ordinary.bind_fleet(fleet).unwrap();
    ordinary.import_retired_identities_paced(|_| {}).unwrap();
    fold_claim_ranges(&[&ordinary]);
    assert!(retired_summary(&ordinary).is_none());
    walk_to_the_end(&source, &ordinary, fleet);
    assert!(!crate::store::retired::claim_retired(&ordinary.readers.get(), &retired[0]).unwrap());
}

#[test]
fn a_fresh_member_joining_from_trimmed_members_ends_with_the_same_answers() {
    let fleet = "7c0a1d2e-3f40-4a5b-8c6d-7e8f90a1b2d2";
    // A member on an older build, or one that has not trimmed: it still holds what the others
    // deleted, and pushes everything it holds.
    let holder = plain_memory("holder").unwrap();
    holder.bind_fleet(fleet).unwrap();
    let kept: Vec<String> = (0..12).map(|n| claim_walk_note(&holder, &format!("note/kept-{n}"))).collect();
    let deleted: Vec<String> = (0..4).map(|n| claim_walk_note(&holder, &format!("note/superseded-{n}"))).collect();

    // The first trimmed member deleted them, and with them their envelopes, which carried
    // nothing else; the second learns that only from the first.
    let oak = local_trim_member("oak", fleet);
    retire_claim_ids(&oak, &deleted);
    holder.seal_local_batches().unwrap();
    for id in &deleted {
        let (writer, sequence, hash): (String, u64, String) = holder
            .readers
            .get()
            .query_row(
                "SELECT envelopes.writer, envelopes.sequence, envelopes.envelope_hash
                 FROM claims JOIN replica_envelopes AS envelopes ON envelopes.batch_id=claims.batch_id
                 WHERE claims.id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        crate::store::retired::retire_envelope_tx(&oak.connection.write(), &writer, sequence, &hash).unwrap();
    }
    push_all(&holder, "holder", &oak, fleet);
    let elm = local_trim_member("elm", fleet);
    push_all(&oak, "oak", &elm, fleet);
    fold_claim_ranges(&[&oak, &elm]);
    walk_to_the_end(&oak, &elm, fleet);
    for id in &deleted {
        assert!(crate::store::retired::claim_retired(&elm.readers.get(), id).unwrap(), "elm learned {id}");
    }
    push_all(&holder, "holder", &elm, fleet);

    // A fresh member joins from the trimmed members, then meets the holder.
    let fresh = local_trim_member("fresh", fleet);
    push_all(&elm, "elm", &fresh, fleet);
    fold_claim_ranges(&[&elm, &fresh]);
    walk_to_the_end(&elm, &fresh, fleet);
    push_all(&holder, "holder", &fresh, fleet);
    fold_claim_ranges(&[&holder, &oak, &elm, &fresh]);
    walk_to_the_end(&holder, &fresh, fleet);

    for member in [&oak, &elm, &fresh] {
        for id in &kept {
            assert!(member.claim_by_id(id).unwrap().is_some(), "{id}");
        }
        for id in &deleted {
            assert!(member.claim_by_id(id).unwrap().is_none(), "{} re-admitted {id}", member.origin);
        }
    }
    let answers = |store: &Store| store.claim_range_summary().unwrap();
    assert_eq!(answers(&fresh).retained, answers(&oak).retained, "the same claims");
    assert_eq!(answers(&elm).retained, answers(&oak).retained);
    assert_eq!(retired_summary(&fresh), retired_summary(&oak), "the same deletions");
    assert_ne!(answers(&holder).retained, answers(&oak).retained, "control: the holder differs");
}

/// The tombstones a cut on `store` records for these claims and the envelopes that carry them.
fn tombstones_of(
    store: &Store,
    ids: &[String],
) -> (Vec<crate::store::checkpoint::EnvelopeTombstone>, Vec<crate::store::checkpoint::ClaimTombstone>) {
    let connection = store.readers.get();
    let mut envelopes = Vec::new();
    let mut claims = Vec::new();
    for id in ids {
        let (writer, sequence, hash, subject, kind): (String, u64, String, String, String) = connection
            .query_row(
                "SELECT envelopes.writer, envelopes.sequence, envelopes.envelope_hash, claims.subject, claims.kind
                 FROM claims JOIN replica_envelopes AS envelopes ON envelopes.batch_id=claims.batch_id
                 WHERE claims.id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        let envelope = crate::store::checkpoint::EnvelopeTombstone {
            writer: writer.clone(),
            sequence,
            envelope_hash: hash.clone(),
            accepted_at_unix_ms: 1,
        };
        if !envelopes.contains(&envelope) {
            envelopes.push(envelope);
        }
        claims.push(crate::store::checkpoint::ClaimTombstone {
            id: id.clone(),
            writer,
            sequence,
            envelope_hash: hash,
            subject,
            kind,
            actor: None,
            predecessors: Vec::new(),
            operation_id: None,
            request_digest: None,
            accepted_at_unix_ms: 1,
        });
    }
    (envelopes, claims)
}

/// A cut that records its tombstones and then stops before trimming, as a graph-changed stop,
/// the drop-newest abort or a crash leaves it, deleted nothing, so it retires nothing a peer could
/// learn, and a fresh member still gets its claims. What a cut deletes enters the ledger in the
/// deleting transaction, so a chunk that rolls back retires nothing either.
#[test]
fn a_cut_that_records_and_stops_teaches_no_peer_to_refuse_its_claims() {
    let fleet = "7c0a1d2e-3f40-4a5b-8c6d-7e8f90a1b2d3";
    let holder = plain_memory("holder").unwrap();
    holder.bind_fleet(fleet).unwrap();
    let recorded: Vec<String> = (0..3).map(|n| claim_walk_note(&holder, &format!("note/recorded-{n}"))).collect();
    holder.seal_local_batches().unwrap();
    let oak = local_trim_member("oak", fleet);
    push_all(&holder, "holder", &oak, fleet);
    let (envelopes, claims) = tombstones_of(&oak, &recorded);
    {
        let mut connection = oak.connection.write();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h87aeaa1b47d9 owner=smallclaims::sync::worker_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoints(id, cut_unix_ms, state, updated_at_unix_ms)
                 VALUES('checkpoint/stopped', 1, 'recording', 1)",
                [],
            )
            .unwrap();
        crate::store::checkpoint::record_checkpoint_tombstones_tx(&transaction, "checkpoint/stopped", &envelopes, &claims)
            .unwrap();
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h302a4fb16aef owner=smallclaims::sync::worker_tests TODO=stage-generation-and-keyed-reclamation */ "UPDATE checkpoints SET state='graph-changed' WHERE id='checkpoint/stopped'", []).unwrap();
        transaction.commit().unwrap();
        let transaction = connection.transaction().unwrap();
        crate::store::checkpoint::delete_dropped_rows_tx(&transaction, &envelopes, &claims).unwrap();
        transaction.rollback().unwrap();
    }
    for id in &recorded {
        assert!(!crate::store::retired::claim_retired(&oak.readers.get(), id).unwrap(), "oak retired {id}");
        assert!(oak.claim_by_id(id).unwrap().is_some(), "oak still holds {id}");
    }

    // A peer that walks oak's retired ranges learns nothing, and a fresh member that joins from
    // both and then meets the holder takes the claims.
    let elm = local_trim_member("elm", fleet);
    fold_claim_ranges(&[&oak, &elm]);
    walk_to_the_end(&oak, &elm, fleet);
    let fresh = local_trim_member("fresh", fleet);
    fold_claim_ranges(&[&fresh]);
    walk_to_the_end(&oak, &fresh, fleet);
    walk_to_the_end(&elm, &fresh, fleet);
    push_all(&holder, "holder", &fresh, fleet);
    push_all(&holder, "holder", &elm, fleet);
    for id in &recorded {
        assert!(!crate::store::retired::claim_retired(&elm.readers.get(), id).unwrap(), "elm learned {id}");
        assert!(fresh.claim_by_id(id).unwrap().is_some(), "fresh refused {id}");
        assert!(elm.claim_by_id(id).unwrap().is_some(), "elm refused {id}");
    }

    // Deleting commits the ledger rows with it.
    {
        let mut connection = oak.connection.write();
        let transaction = connection.transaction().unwrap();
        crate::store::checkpoint::delete_dropped_rows_tx(&transaction, &envelopes, &claims).unwrap();
        transaction.commit().unwrap();
    }
    let connection = oak.readers.get();
    for id in &recorded {
        assert!(crate::store::retired::claim_retired(&connection, id).unwrap(), "{id}");
    }
    for envelope in &envelopes {
        assert!(
            crate::store::retired::envelope_retired(&connection, &envelope.writer, envelope.sequence, &envelope.envelope_hash)
                .unwrap()
        );
    }
}

/// An older build's claim-range summary, query and answer carry no `retired` part. Each decodes,
/// and the summary compares exactly as before; what an ordinary member sends has no such field.
#[test]
fn an_older_peers_claim_range_messages_decode_and_compare_as_before() {
    use crate::replication::{ClaimRangeComparison, ClaimRangeQuery, ClaimRangeSummary};
    fn older<T: serde::Serialize + serde::de::DeserializeOwned>(message: &T) -> T {
        let mut value = serde_json::to_value(message).unwrap();
        value.as_object_mut().unwrap().remove("retired").expect("the newer message has a retired part");
        serde_json::from_value(value).unwrap()
    }
    let fleet = "7c0a1d2e-3f40-4a5b-8c6d-7e8f90a1b2d4";
    let source = local_trim_member("source", fleet);
    let target = local_trim_member("target", fleet);
    claim_walk_note(&source, "note/only-source");
    retire_claim_ids(&source, &[format!("{:064x}", 7)]);
    fold_claim_ranges(&[&source, &target]);

    let summary = source.claim_range_summary().unwrap();
    let from_older = older(&summary);
    assert_eq!(from_older, ClaimRangeSummary { retired: None, ..summary.clone() });
    let compared = target.compare_claim_ranges(&from_older).unwrap().unwrap();
    assert!(compared.retired.is_none(), "nothing is compared that the peer did not send");
    let mut newer = target.compare_claim_ranges(&summary).unwrap().unwrap();
    assert!(newer.retired.is_some());
    newer.retired = None;
    assert_eq!(compared, newer, "the retained comparison is the same");

    let query = ClaimRangeQuery {
        tops: true,
        retired: Some(Box::new(ClaimRangeQuery { tops: true, ..Default::default() })),
        ..Default::default()
    };
    assert_eq!(older(&query), ClaimRangeQuery { retired: None, ..query.clone() });
    let answer = ClaimRangeComparison { retired: Some(Box::default()), ..compared.clone() };
    assert_eq!(older(&answer), compared);
    for sent in [
        serde_json::to_string(&from_older).unwrap(),
        serde_json::to_string(&older(&query)).unwrap(),
        serde_json::to_string(&compared).unwrap(),
    ] {
        assert!(!sent.contains("retired"), "{sent}");
    }
}

#[test]
fn a_retired_step_the_answer_left_out_is_asked_again() {
    use crate::replication::ClaimRangeComparison;
    let mut walk = ClaimWalk::finished("ours/theirs");
    walk.follow_retired(&ClaimRangeComparison {
        pair: "ours/theirs".into(),
        same: true,
        retired: Some(Box::new(ClaimRangeComparison { pair: "retired".into(), ..Default::default() })),
        ..Default::default()
    });
    let query = walk.next_query().unwrap();
    let asked = query.retired.clone().expect("the retired walk asks its tops");
    assert!(asked.tops);
    // The peer's retired ranges were folding, so its answer has no retired part.
    walk.absorb(query, &ClaimRangeComparison { pair: "ours/theirs".into(), same: true, ..Default::default() });
    assert_eq!(walk.next_query().and_then(|query| query.retired), Some(asked));
}
