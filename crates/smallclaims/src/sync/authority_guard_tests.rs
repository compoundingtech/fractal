//! Count actual worker requests while store-scoped subscriptions follow committed changes.
use super::*;
use crate::fleet::handshake::JoinRequest;
use crate::replication::{ReceiveOptions, ReplicationExportResponse, ReplicationReceiveResponse};
use crate::store::Runtime as _;
use crate::sync::Local;
use std::sync::atomic::{AtomicUsize, Ordering};

const FLEET: &str = "fixture-fleet";

#[derive(Clone)]
struct GuardBackend {
    authority_waits: Arc<AtomicUsize>,
    exports: Arc<AtomicUsize>,
    views: Arc<AtomicUsize>,
    receives: Arc<AtomicUsize>,
    changes: Arc<AtomicUsize>,
    receive_changed: bool,
    store: Arc<crate::store::Store>,
}

fn response(digest: String) -> ReplicationExportResponse {
    ReplicationExportResponse {
        store_index: 0,
        exchange: ReplicationExchange {
            peer: "local".into(),
            fleet_id: FLEET.into(),
            shard_id: None,
            schema_digest: String::new(),
            authority_digest: digest.clone(),
            graph_digest: String::new(),
            projection_digests: Default::default(),
            projection_registry_digest: Some(
                crate::store::runtime::Plain.projection_registry_digest(),
            ),
            projection_versions: crate::store::runtime::Plain.projection_versions(),
            inventory: ReplicationInventory {
                digest,
                ..Default::default()
            },
            envelopes: Vec::new(),
            signature_requests: Vec::new(),
            signatures: Vec::new(),
            build: None,
            claim_ranges: None,
            claim_member_signatures: Vec::new(),
            signed_envelopes: Vec::new(),
            unservable_released: Default::default(),
        },
    }
}

impl GuardBackend {
    fn new() -> Self {
        Self {
            authority_waits: Arc::default(),
            exports: Arc::default(),
            views: Arc::default(),
            receives: Arc::default(),
            changes: Arc::default(),
            receive_changed: false,
            store: Arc::new(crate::store::Store::open_memory(
                "local", Arc::new(crate::store::runtime::Plain),
            ).unwrap()),
        }
    }
}

impl Backend for GuardBackend {
    async fn wait_revision(
        &self,
        request: crate::sync::revision::RevisionRequest,
    ) -> Result<crate::sync::revision::StoreRevision> {
        if matches!(request.kind, crate::sync::revision::RevisionKind::Authority) {
            self.authority_waits.fetch_add(1, Ordering::Relaxed);
        }
        self.store.wait_replication_revision(request).await
    }
    async fn export(
        &self,
        _fleet: &str,
        _inventory: &ReplicationInventory,
        _summary: bool,
        _signatures: &[crate::claim::ReplicaEnvelopeId],
    ) -> Result<ReplicationExportResponse> {
        self.exports.fetch_add(1, Ordering::Relaxed);
        Ok(response("ordinary-full-export".into()))
    }
    async fn receive(
        &self,
        _peer: &str,
        _fleet: &str,
        _exchange: &ReplicationExchange,
        _options: ReceiveOptions<'_>,
    ) -> Result<ReplicationReceiveResponse> {
        self.receives.fetch_add(1, Ordering::Relaxed);
        Ok(ReplicationReceiveResponse {
            receipt: crate::replication::ReplicationReceipt {
                received: 0,
                duplicate: 0,
                signatures: 0,
                inventory: ReplicationInventory::default(),
                claim_ranges: None,
                signed_envelopes: 0,
                refused_signed_envelopes: 0,
            },
            changed: self.receive_changed,
            store_index: 0,
        })
    }
    async fn publish_endpoints(&self, _mode: &str, _endpoints: &[Value]) -> Result<()> {
        unreachable!()
    }
    async fn redeem(&self, _request: &JoinRequest) -> Result<Value> {
        unreachable!()
    }
    async fn fleet_view(&self) -> Result<FleetView> {
        self.views.fetch_add(1, Ordering::Relaxed);
        Ok(FleetView::default())
    }
    async fn record_failure(
        &self,
        _peer: &str,
        _status: &str,
        _error: &str,
        _carrier: Option<crate::replication::ReplicationCarrier>,
        _failure: Option<&crate::replication::ReplicationExchangeFailure>,
    ) -> Result<()> {
        Ok(())
    }
    async fn changed(&self) {
        self.changes.fetch_add(1, Ordering::Relaxed);
    }
}

#[tokio::test(start_paused = true)]
async fn unrelated_wakes_make_zero_guard_and_membership_requests() {
    use crate::sync::revision::{RevisionKind, RevisionRequest};
    let backend = GuardBackend::new();
    let fleet = FleetContext::legacy(BTreeSet::from(["peer".into()]));
    let now = tokio::time::Instant::now();
    fleet.online.write().unwrap().insert("peer".into(), now);
    fleet.inbound.write().unwrap().insert("peer".into(), now);
    let authority = backend.store.replication_revision_snapshot(RevisionKind::Authority).unwrap();
    fleet.inbound_authority.write().unwrap().insert(
        "peer".into(), authority.authority_root.clone().unwrap(),
    );
    *fleet.local_authority.write().unwrap() = Some(authority);
    let initial = backend.wait_revision(RevisionRequest {
        kind: RevisionKind::Membership, after: None,
    }).await.unwrap();
    let (wake, _) = watch::channel(0_u64);
    let (_routes, routes) = watch::channel(vec![Route::Http("http://127.0.0.1:1".into())]);
    let membership = keep_fleet_view_current(backend.clone(), fleet.clone(), Some(initial));
    let dialer = dial_peer(
        backend.clone(), "local".into(), "peer".into(), routes,
        FleetAuth::test(FLEET, &[7; 32]), fleet.clone(), wake.subscribe(),
    );
    tokio::pin!(membership, dialer);
    for _ in 0..100 {
        wake.send_modify(|generation| *generation += 1);
        fleet.inbound_changed.send_modify(|generation| *generation += 1);
        assert!(futures_util::poll!(&mut membership).is_pending());
        assert!(futures_util::poll!(&mut dialer).is_pending());
    }
    tokio::time::advance(REPLICATION_WAKE_COALESCE).await;
    assert!(futures_util::poll!(&mut dialer).is_pending());
    assert_eq!(backend.authority_waits.load(Ordering::Relaxed), 0);
    assert_eq!(backend.views.load(Ordering::Relaxed), 0);
    assert_eq!(backend.exports.load(Ordering::Relaxed), 0);
    backend.store.connection.write().execute(
        "UPDATE fleet_generation SET value=value+1 WHERE id=1", [],
    ).unwrap();
    assert!(futures_util::poll!(&mut membership).is_pending());
    assert_eq!(backend.views.load(Ordering::Relaxed), 1);
    assert_eq!(backend.authority_waits.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn shard_exchanges_and_empty_offers_do_not_fold_graph_membership() {
    for changed in [false, true] {
        let mut backend = GuardBackend::new();
        backend.receive_changed = changed;
        let mut fleet = FleetContext::legacy(BTreeSet::from(["peer".into()]));
        fleet.own_key = Some("local-key".into());
        let mut exchange = response("same".into()).exchange;
        select_leave_confirmer(&backend, &fleet, "peer", &exchange).await.unwrap();
        assert_eq!(backend.views.load(Ordering::Relaxed), 0);
        exchange.shard_id = Some("messages".into());
        for _ in 0..100 {
            receive_from_peer(
                &backend, &fleet, "peer", FLEET, &exchange, ReceiveOptions::default(),
            ).await.unwrap();
            select_leave_confirmer(&backend, &fleet, "peer", &exchange).await.unwrap();
        }
        assert_eq!(backend.receives.load(Ordering::Relaxed), 100);
        assert_eq!(backend.changes.load(Ordering::Relaxed), if changed { 100 } else { 0 });
        assert_eq!(backend.views.load(Ordering::Relaxed), 0);
        assert_eq!(backend.exports.load(Ordering::Relaxed), 0);
        // Empty graph replies and ordinary graph records also do no membership reads.
        exchange.shard_id = None;
        receive_from_peer(
            &backend, &fleet, "peer", FLEET, &exchange, ReceiveOptions::default(),
        ).await.unwrap();
        assert_eq!(backend.views.load(Ordering::Relaxed), 0);
        backend.store.append_claim(&crate::claim::ClaimInput {
            subject: "note/ordinary".into(), kind: "example.note".into(), actor: None,
            fields: BTreeMap::new(), evidence: Vec::new(), expected_subject: None,
            idempotency_key: None,
        }).unwrap();
        let ordinary = backend.store.export_replication_exchange(
            FLEET, &ReplicationInventory::default(),
        ).unwrap();
        receive_from_peer(
            &backend, &fleet, "peer", FLEET, &ordinary, ReceiveOptions::default(),
        ).await.unwrap();
        select_leave_confirmer(&backend, &fleet, "peer", &ordinary).await.unwrap();
        assert_eq!(backend.views.load(Ordering::Relaxed), 0);
    }
}



#[tokio::test]
async fn signed_peer_request_cannot_select_the_local_reduced_mode() {
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt as _;
    let store = Arc::new(
        crate::store::Store::open_memory("target", Arc::new(crate::store::runtime::Plain)).unwrap(),
    );
    store.bind_fleet(FLEET).unwrap();
    store
        .append_claim(&crate::claim::ClaimInput {
            subject: "note/fixture".into(),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let auth = FleetAuth::test(FLEET, &[7; 32]);
    let state = PeerState::new(
        Local(store.clone()),
        "target".into(),
        auth.clone(),
        FleetContext::legacy(BTreeSet::from(["source".into()])),
    );
    let mut query = response(String::new()).exchange;
    query.schema_digest = crate::store::runtime::Plain.schema_digest();
    let mut body = serde_json::to_value(query).unwrap();
    body["peer"] = Value::String("source".into());
    // Even an authenticated peer cannot opt the ordinary peer route into the local mode.
    for key in ["summary_only", "authority_guard", "authority_digest_only"] {
        body[key] = Value::Bool(true);
    }
    let body = serde_json::to_vec(&body).unwrap();
    let headers = auth.request_headers("source", &body).unwrap();
    let mut request = Request::post(EXCHANGE_PATH)
        .body(Body::from(body.clone()))
        .unwrap();
    request.headers_mut().extend(headers);
    let result = peer_router(state, Router::new())
        .oneshot(request)
        .await
        .unwrap();
    assert!(result.status().is_success());
    let headers = result.headers().clone();
    let bytes = to_bytes(result.into_body(), MAX_EXCHANGE_BYTES)
        .await
        .unwrap();
    auth.verify(
        &headers,
        "RESPONSE",
        EXCHANGE_PATH,
        &bytes,
        Some("target"),
        Some(&FleetAuth::body_digest(&body)),
    )
    .unwrap();
    let result: PeerResponse<ReplicationExchange> = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !result.value.inventory.envelopes.is_empty(),
        "signed route retained its ordinary identity proof; the local reduced mode has none"
    );
    assert!(
        !result.value.envelopes.is_empty(),
        "ordinary missing envelopes were still exported"
    );
    let full = store.export_replication_summary(FLEET).unwrap();
    assert_eq!(result.value.authority_digest, full.authority_digest);
}
