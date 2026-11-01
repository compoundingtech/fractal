//! The daily answers check. Once a day each member compares what readers answer from its claims
//! with what each peer it dials answers, kind by kind (see `crate::answers`), at the start of the
//! UTC day. It waits an hour into the day, so replication has brought every member what was
//! written before the cut, and checks a peer only after an exchange with it moved nothing.
//!
//! A kind that differs is compared again two hours later. Each member keeps its summary at a cut
//! until a claim older than the cut arrives, so a real difference stays, and comparing more often
//! only costs the members another read. A difference that lasts two
//! hours is narrowed to buckets and keys and handed to the store to record, which raises a fault.
//! A kind whose differing keys all moved past the cut on one member is no difference: that member
//! only computed after the newer claim arrived. An agreeing check is handed over too, so the store
//! can end an earlier fault. A peer on a build
//! without the check answers 404 and is not compared.
use super::*;
use crate::answers::{
    AnswersCheck, AnswersReply, AnswersRequest, KindReport, claims_compare, differing_buckets,
    differing_keys, differing_kinds,
};
use crate::sync::ANSWERS_PATH;

const DAY_MS: u128 = 86_400_000;
const GRACE_MS: u128 = 3_600_000;
/// How soon to ask again when either member is still computing.
const PENDING: Duration = Duration::from_secs(60);
/// How soon to compare again after a difference, longer than `ANSWERS_FRESH`.
const RECHECK: Duration = Duration::from_secs(2 * 60 * 60);
/// How long a difference lasts before it is reported.
const SETTLE: Duration = Duration::from_secs(2 * 60 * 60);
/// The claim kinds narrowed to keys (fold readers are always narrowed), the buckets read per
/// request, and the keys named per kind.
const MAX_KINDS: usize = 8;
const MAX_BUCKETS: usize = 8;
const MAX_KEYS: usize = 20;

/// The cut a check compares at: the start of the UTC day an hour ago.
pub fn daily_cut(now_unix_ms: u128) -> u128 {
    now_unix_ms.saturating_sub(GRACE_MS) / DAY_MS * DAY_MS
}

/// Where one member's check of one peer stands for the current cut.
struct PeerCheck {
    cut: u128,
    next: tokio::time::Instant,
    differs_since: Option<tokio::time::Instant>,
    done: bool,
}

impl PeerCheck {
    fn new(cut: u128) -> Self {
        Self { cut, next: tokio::time::Instant::now(), differs_since: None, done: false }
    }
}

/// Each check by node and peer, so tests of several nodes in one process keep theirs apart.
static CHECKS: std::sync::LazyLock<std::sync::Mutex<BTreeMap<(String, String), PeerCheck>>> =
    std::sync::LazyLock::new(Default::default);

#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Either member is still computing its answers.
    Pending,
    /// The peer has no answers route.
    Unsupported,
    /// The answers differ and the difference has not lasted long enough to report.
    Differ,
    /// What to record: agreement, or every kind that differs with the keys found.
    Found(AnswersCheck),
}

/// Compare answers with `peer` if this cut's check is due.
pub(super) async fn check_answers<B: Backend>(
    http: &reqwest::Client,
    backend: &B,
    node: &str,
    peer: &PeerConfig,
    auth: &FleetAuth,
    fleet: &FleetContext,
) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let cut = daily_cut(now_ms);
    let key = (node.to_owned(), peer.name.clone());
    let settled = {
        let mut checks = CHECKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let check = checks.entry(key.clone()).or_insert_with(|| PeerCheck::new(cut));
        if check.cut != cut {
            *check = PeerCheck::new(cut);
        }
        if check.done || tokio::time::Instant::now() < check.next {
            return;
        }
        check.differs_since.is_some_and(|since| since.elapsed() >= worker_interval(SETTLE))
    };
    let mut step = compare_answers(http, backend, node, peer, auth, fleet, cut, settled).await;
    // One line per peer and cut for each outcome, so an operator can confirm the check ran.
    match &step {
        Ok(Step::Unsupported) => eprintln!("st3: {} keeps no answers; not compared at {cut}", peer.name),
        Ok(Step::Differ) if !settled => eprintln!(
            "st3: answers at {cut} differ from {}'s; comparing again until the difference settles",
            peer.name
        ),
        _ => {}
    }
    if let Ok(Step::Found(check)) = &step {
        if check.differences.is_empty() {
            eprintln!("st3: answers at {cut} agree with {}'s", peer.name);
        } else {
            eprintln!(
                "st3: answers at {cut} differ from {}'s in {}",
                peer.name,
                check.differences.iter().map(|report| report.kind.as_str()).collect::<Vec<_>>().join(", ")
            );
        }
        if let Err(error) = backend.record_answers_check(check).await {
            step = Err(error);
        }
    }
    let mut checks = CHECKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(check) = checks.get_mut(&key).filter(|check| check.cut == cut) else {
        return;
    };
    let now = tokio::time::Instant::now();
    match step {
        Ok(Step::Pending) => check.next = now + worker_interval(PENDING),
        Ok(Step::Unsupported | Step::Found(_)) => check.done = true,
        Ok(Step::Differ) => {
            check.differs_since.get_or_insert(now);
            check.next = now + worker_interval(RECHECK);
        }
        Err(error) => {
            eprintln!("st3: comparing answers with {} failed: {error:#}", peer.name);
            check.next = now + worker_interval(RECHECK);
        }
    }
}

/// One comparison at `cut`. It finds what to record when the members agree, or when they still
/// differ once `settled`.
#[allow(clippy::too_many_arguments)]
async fn compare_answers<B: Backend>(
    http: &reqwest::Client,
    backend: &B,
    node: &str,
    peer: &PeerConfig,
    auth: &FleetAuth,
    fleet: &FleetContext,
    cut: u128,
    settled: bool,
) -> Result<Step> {
    let ask = |kind: Option<&str>, buckets: BTreeSet<usize>| AnswersRequest {
        cut_unix_ms: cut,
        kind: kind.map(str::to_owned),
        buckets,
    };
    let both = async |request: AnswersRequest| -> Result<Option<(AnswersReply, AnswersReply)>> {
        let Some(ours) = backend.answers(&request).await? else { return Ok(None) };
        let Some(theirs) = fetch_answers(http, peer, node, auth, fleet, &request).await? else {
            return Ok(None);
        };
        Ok(Some((ours, theirs)))
    };
    let Some((ours, theirs)) = both(ask(None, BTreeSet::new())).await? else {
        return Ok(Step::Unsupported);
    };
    let (Some(ours), Some(theirs)) = (ours.summary, theirs.summary) else {
        return Ok(Step::Pending);
    };
    let compare_claims = claims_compare(&ours, &theirs);
    let kinds = differing_kinds(&ours, &theirs, compare_claims);
    if !kinds.is_empty() && !settled {
        return Ok(Step::Differ);
    }
    let mut differences = Vec::new();
    let mut narrowed = 0;
    for kind in kinds {
        let mut report = KindReport {
            kind: kind.kind,
            ours: kind.ours,
            theirs: kind.theirs,
            keys: Vec::new(),
            complete: false,
        };
        // A fold reader's keys come with the summary, so narrowing one computes nothing.
        let reader = report.kind.starts_with(crate::answers::READER_PREFIX);
        if reader || narrowed < MAX_KINDS {
            narrowed += usize::from(!reader);
            let Some((ours, theirs)) = both(ask(Some(&report.kind), BTreeSet::new())).await? else {
                return Ok(Step::Pending);
            };
            let (Some(ours), Some(theirs)) = (ours.buckets, theirs.buckets) else {
                return Ok(Step::Pending);
            };
            let buckets = differing_buckets(&ours.buckets, &theirs.buckets).into_iter().collect::<Vec<_>>();
            // A bucket whose differing keys all move on one member names no key, so read on
            // until one does or every differing bucket is read.
            let mut read = 0;
            for wanted in buckets.chunks(MAX_BUCKETS) {
                let Some((ours, theirs)) = both(ask(Some(&report.kind), wanted.iter().copied().collect())).await? else {
                    return Ok(Step::Pending);
                };
                let (Some(ours), Some(theirs)) = (ours.buckets, theirs.buckets) else {
                    return Ok(Step::Pending);
                };
                report.keys.extend(differing_keys(&ours.keys, &theirs.keys));
                read += wanted.len();
                if !report.keys.is_empty() {
                    break;
                }
            }
            report.complete = read == buckets.len() && report.keys.len() <= MAX_KEYS;
            report.keys.truncate(MAX_KEYS);
            // Every key that differs is moving, and the members count the same claims: they
            // differ only in when they computed.
            let claims = |side: &Option<crate::answers::KindAnswers>| side.as_ref().map_or(0, |side| side.claims);
            let claims_differ = compare_claims && claims(&report.ours) != claims(&report.theirs);
            if report.complete && report.keys.is_empty() && !claims_differ {
                continue;
            }
        }
        differences.push(report);
    }
    Ok(Step::Found(AnswersCheck { peer: peer.name.clone(), cut_unix_ms: cut, differences }))
}

/// Answer a member's request for this node's answers. It is authenticated exactly like an
/// exchange, and the answer is signed for this path. A backend without answers gets 404, as a
/// build without the route does.
pub(super) async fn receive_answers_request<B: Backend>(
    State(state): State<PeerState<B>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let sender = match state.auth.verify_sender(&headers, "POST", ANSWERS_PATH, &body, None, None) {
        Ok(sender) if state.fleet.accept(&sender).is_ok() => sender,
        _ => return (StatusCode::UNAUTHORIZED, "untrusted answers request").into_response(),
    };
    state.fleet.note_activity(&sender.name);
    let request_digest = FleetAuth::body_digest(&body);
    let result = async {
        let request: AnswersRequest =
            serde_json::from_slice(&body).context("decode the answers request")?;
        let Some(reply) = state.backend.answers(&request).await? else {
            return Ok(StatusCode::NOT_FOUND.into_response());
        };
        let response = signed_response_for(&state, ANSWERS_PATH, &request_digest, 0, reply)?;
        deflate_response(response, accepts_deflate(&headers)).await
    }
    .await;
    result.unwrap_or_else(|error: anyhow::Error| {
        let message = format!("answers request from {} failed: {error:#}", sender.name);
        signed_error_response_for(
            &state,
            ANSWERS_PATH,
            &request_digest,
            0,
            StatusCode::UNPROCESSABLE_ENTITY,
            &message,
        )
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, message).into_response())
    })
}

/// Ask a peer for its answers. `None` when the peer has no answers route.
pub async fn fetch_answers(
    http: &reqwest::Client,
    peer: &PeerConfig,
    node: &str,
    auth: &FleetAuth,
    fleet: &FleetContext,
    request: &AnswersRequest,
) -> Result<Option<AnswersReply>> {
    let body = serde_json::to_vec(request)?;
    let request_digest = FleetAuth::body_digest(&body);
    let headers = auth.request_headers_for(ANSWERS_PATH, node, &body)?;
    let endpoint = format!("{}{}", peer.url.trim_end_matches('/'), ANSWERS_PATH);
    let response = http
        .post(&endpoint)
        .headers(headers)
        .header("content-type", "application/json")
        .header("accept-encoding", EXCHANGE_ENCODING)
        .body(body)
        .send()
        .await
        .map_err(request_route_failure)
        .with_context(|| format!("answers request to peer {} failed", peer.name))?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.bytes().await.map_err(response_route_failure)?.to_vec();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let bytes = if deflated(&headers) {
        inflate(&bytes, &format!("the answers from peer {}", peer.name))?
    } else {
        bytes
    };
    let responder = auth.verify_sender(
        &headers,
        "RESPONSE",
        ANSWERS_PATH,
        &bytes,
        Some(&peer.name),
        Some(&request_digest),
    )?;
    if let Err(refusal) = fleet.accept(&responder) {
        anyhow::bail!(
            "peer {} failed member authentication ({}): {}",
            peer.name,
            refusal.code,
            refusal.message
        );
    }
    anyhow::ensure!(
        status.is_success(),
        "peer {} returned {status}: {}",
        peer.name,
        String::from_utf8_lossy(&bytes)
    );
    let response: PeerResponse<AnswersReply> =
        serde_json::from_slice(&bytes).context("decode the signed answers")?;
    anyhow::ensure!(
        ACCEPTED_PEER_API_VERSIONS.contains(&response.api_version.as_str()),
        "the peer API version differs"
    );
    fleet.note_activity(&peer.name);
    let answered = |kind: Option<&str>, cut: u128| kind == request.kind.as_deref() && cut == request.cut_unix_ms;
    anyhow::ensure!(
        response.value.summary.as_ref().is_none_or(|summary| answered(None, summary.cut_unix_ms))
            && response
                .value
                .buckets
                .as_ref()
                .is_none_or(|buckets| answered(Some(&buckets.kind), buckets.cut_unix_ms)),
        "peer {} answered another request",
        peer.name
    );
    Ok(Some(response.value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cut_is_the_start_of_the_day_an_hour_ago() {
        let day = 20_390 * DAY_MS;
        assert_eq!(daily_cut(day + GRACE_MS), day);
        assert_eq!(daily_cut(day + GRACE_MS - 1), day - DAY_MS);
        assert_eq!(daily_cut(day + DAY_MS - 1), day);
    }

    use crate::claim::ClaimInput;
    use crate::replication::ReplicationInventory;
    use crate::store::Store;
    use crate::sync::Local;

    const FLEET: &str = "1f91ca65-7793-48cc-866e-ac15690130e1";

    /// A store this process holds that answers on request, or through a daemon's cache.
    #[derive(Clone)]
    struct Answering(Local<Store>, Option<Arc<crate::answers::AnswersCache>>);

    impl Backend for Answering {
        async fn export(
            &self,
            fleet_id: &str,
            inventory: &ReplicationInventory,
            summary_only: bool,
            signature_requests: &[crate::claim::ReplicaEnvelopeId],
        ) -> Result<crate::replication::ReplicationExportResponse> {
            self.0.export(fleet_id, inventory, summary_only, signature_requests).await
        }

        async fn receive(
            &self,
            peer: &str,
            fleet_id: &str,
            exchange: &crate::replication::ReplicationExchange,
            round_trip: Option<Duration>,
            carrier: crate::replication::ReplicationCarrier,
        ) -> Result<crate::replication::ReplicationReceiveResponse> {
            self.0.receive(peer, fleet_id, exchange, round_trip, carrier).await
        }

        async fn checkpoint_manifest(&self, request: &CheckpointManifestRequest) -> Result<CheckpointManifestPage> {
            self.0.checkpoint_manifest(request).await
        }

        async fn checkpoint_need(&self) -> Result<Option<crate::store::CheckpointManifestNeed>> {
            self.0.checkpoint_need().await
        }

        async fn adopt_checkpoint(&self, manifest: &CheckpointManifest) -> Result<Vec<crate::store::CheckpointAction>> {
            self.0.adopt_checkpoint(manifest).await
        }

        async fn publish_endpoints(&self, mode: &str, endpoints: &[Value]) -> Result<()> {
            self.0.publish_endpoints(mode, endpoints).await
        }

        async fn redeem(&self, request: &crate::fleet::handshake::JoinRequest) -> Result<Value> {
            self.0.redeem(request).await
        }

        async fn fleet_view(&self) -> Result<FleetView> {
            self.0.fleet_view().await
        }

        async fn record_failure(
            &self,
            peer: &str,
            status: &str,
            error: &str,
            carrier: Option<crate::replication::ReplicationCarrier>,
            failure: Option<&ReplicationExchangeFailure>,
        ) -> Result<()> {
            self.0.record_failure(peer, status, error, carrier, failure).await
        }

        async fn answers(&self, request: &AnswersRequest) -> Result<Option<AnswersReply>> {
            Ok(Some(match &self.1 {
                Some(cache) => cache.answer(&self.0.0, request),
                None => self.0.0.answer(request)?,
            }))
        }
    }

    fn node(name: &str) -> Arc<Store> {
        Arc::new(Store::open_memory(name, Arc::new(crate::store::runtime::Plain)).unwrap())
    }

    fn state(store: &Store, subject: &str, value: &str) -> String {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "example.state".into(),
                actor: None,
                fields: BTreeMap::from([("value".into(), value.into())]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
            .id
    }

    fn push(from: &Store, to: &Store) {
        let push = from.export_replication_exchange(FLEET, &ReplicationInventory::default()).unwrap();
        to.receive_replication_exchange("alder", FLEET, &push).unwrap();
        to.validate_replication_backlog().unwrap();
        to.project_replication_backlog().unwrap();
    }

    /// A cut every claim a test writes is older than.
    fn tomorrow() -> u128 {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        daily_cut(now + 2 * DAY_MS)
    }

    /// Serve `store` as the peer alder, for birch to ask.
    async fn serve(store: Arc<Store>, auth: &FleetAuth) -> (PeerConfig, tokio::task::JoinHandle<()>) {
        serve_through(store, None, auth).await
    }

    /// Serve `store` as the peer alder, answering through `cache` if there is one.
    async fn serve_through(
        store: Arc<Store>,
        cache: Option<Arc<crate::answers::AnswersCache>>,
        auth: &FleetAuth,
    ) -> (PeerConfig, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = PeerState::new(
            Answering(Local(store), cache),
            "alder".into(),
            auth.clone(),
            FleetContext::legacy(BTreeSet::from(["birch".into()])),
        );
        let router = peer_router(state, Router::new());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (PeerConfig { name: "alder".into(), url: format!("http://{address}") }, server)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_member_names_the_kind_and_key_a_peer_answers_differently() {
        let auth = FleetAuth::test(FLEET, &[6; 32]);
        let alder = node("alder");
        let birch = node("birch");
        state(&alder, "item/a", "one");
        state(&alder, "item/b", "one");
        push(&alder, &birch);
        let (peer, server) = serve(alder.clone(), &auth).await;
        let http = replication_http_client();
        let dialer = FleetContext::legacy(BTreeSet::from(["alder".into()]));
        let backend = Answering(Local(birch.clone()), None);
        let cut = tomorrow();
        let compare = async |settled| {
            compare_answers(&http, &backend, "birch", &peer, &auth, &dialer, cut, settled).await.unwrap()
        };
        let agree = AnswersCheck { peer: "alder".into(), cut_unix_ms: cut, differences: Vec::new() };
        assert_eq!(compare(false).await, Step::Found(agree.clone()));

        let newest = state(&alder, "item/b", "two");
        assert_eq!(compare(false).await, Step::Differ, "a new difference waits to settle");
        let Step::Found(check) = compare(true).await else { panic!("a settled difference is found") };
        assert_eq!(check.differences.len(), 1);
        let report = &check.differences[0];
        assert_eq!(report.kind, "example.state");
        assert!(report.complete);
        assert_eq!(report.keys.len(), 1);
        assert_eq!((report.keys[0].subject.as_str(), report.keys[0].origin.as_str()), ("item/b", "alder"));
        assert_eq!(report.keys[0].theirs.as_deref(), Some(newest.as_str()));

        push(&alder, &birch);
        assert_eq!(compare(true).await, Step::Found(agree));

        // A node outside the fleet gets nothing.
        let stranger = fetch_answers(
            &http,
            &peer,
            "stranger",
            &auth,
            &dialer,
            &AnswersRequest { cut_unix_ms: cut, ..AnswersRequest::default() },
        )
        .await;
        assert!(stranger.is_err());
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_key_that_moves_past_the_cut_on_one_member_is_no_difference() {
        let auth = FleetAuth::test(FLEET, &[6; 32]);
        let alder = node("alder");
        let birch = node("birch");
        state(&alder, "item/a", "one");
        state(&alder, "item/b", "one");
        push(&alder, &birch);
        std::thread::sleep(Duration::from_millis(5));
        let cut = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        std::thread::sleep(Duration::from_millis(5));
        // alder writes again after the cut, so it no longer counts item/b; birch has not seen the
        // newer claim yet and still counts the one before the cut.
        state(&alder, "item/b", "two");
        let (peer, server) = serve(alder.clone(), &auth).await;
        let http = replication_http_client();
        let dialer = FleetContext::legacy(BTreeSet::from(["alder".into()]));
        let backend = Answering(Local(birch.clone()), None);
        let compare = async |settled| {
            compare_answers(&http, &backend, "birch", &peer, &auth, &dialer, cut, settled).await.unwrap()
        };
        let summary = |store: &Store| store.answers_summary(cut).unwrap().kinds["example.state"].clone();
        let (ours, theirs) = (summary(&birch), summary(&alder));
        assert_ne!((ours.keys, &ours.digest), (theirs.keys, &theirs.digest), "item/b counts only on birch");
        assert_eq!(ours.before_cut, theirs.before_cut, "both hold item/b's claim before the cut");
        let agree = AnswersCheck { peer: "alder".into(), cut_unix_ms: cut, differences: Vec::new() };
        assert_eq!(compare(false).await, Step::Found(agree.clone()), "they agree on the claims before the cut");
        assert_eq!(compare(true).await, Step::Found(agree), "a moving key is not reported");
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn seats_that_move_on_one_member_in_more_buckets_than_a_read_are_no_difference() {
        use crate::answers::{AnswerKey, AnswersCache, bucket};
        // Seats in more buckets than one read covers: alder still counts them, birch already
        // saw each of them written after the cut.
        let mut seen = BTreeSet::new();
        let seats = (0..)
            .map(|index| format!("agent/sample.seat-{index}"))
            .filter(|subject| seen.insert(bucket(subject, "")))
            .take(MAX_BUCKETS + 4)
            .collect::<Vec<_>>();
        let member = |moving: bool| {
            let seats = seats.clone();
            let cache = Arc::new(AnswersCache::default());
            cache.set_readers(Arc::new(move |_: &rusqlite::Connection, _: u128| {
                let keys = seats
                    .iter()
                    .map(|subject| AnswerKey {
                        subject: subject.clone(),
                        origin: String::new(),
                        claim: if moving { String::new() } else { "idle".into() },
                        moving,
                    })
                    .collect();
                Ok(BTreeMap::from([("harness".to_owned(), keys)]))
            }));
            cache
        };
        let auth = FleetAuth::test(FLEET, &[6; 32]);
        let (alder, birch) = (node("alder"), node("birch"));
        let (peer, server) = serve_through(alder.clone(), Some(member(false)), &auth).await;
        let http = replication_http_client();
        let dialer = FleetContext::legacy(BTreeSet::from(["alder".into()]));
        let backend = Answering(Local(birch.clone()), Some(member(true)));
        let cut = tomorrow();
        let mut step = Step::Pending;
        for _ in 0..500 {
            step = compare_answers(&http, &backend, "birch", &peer, &auth, &dialer, cut, true).await.unwrap();
            if step != Step::Pending {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let agree = AnswersCheck { peer: "alder".into(), cut_unix_ms: cut, differences: Vec::new() };
        assert_eq!(step, Step::Found(agree), "every differing key moves on birch");
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_member_without_answers_asks_no_peer() {
        let auth = FleetAuth::test(FLEET, &[6; 32]);
        // Nothing listens here: asking would fail.
        let peer = PeerConfig { name: "alder".into(), url: "http://127.0.0.1:9".into() };
        let dialer = FleetContext::legacy(BTreeSet::from(["alder".into()]));
        let step = compare_answers(
            &replication_http_client(), &Local(node("birch")), "birch", &peer, &auth, &dialer, tomorrow(), false,
        )
        .await
        .unwrap();
        assert_eq!(step, Step::Unsupported);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_without_answers_is_not_compared() {
        let auth = FleetAuth::test(FLEET, &[6; 32]);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        // An older build serves the other peer routes and 404 for this one.
        let server = tokio::spawn(async move { axum::serve(listener, Router::new()).await.unwrap() });
        let peer = PeerConfig { name: "alder".into(), url: format!("http://{address}") };
        let dialer = FleetContext::legacy(BTreeSet::from(["alder".into()]));
        let step = compare_answers(
            &replication_http_client(),
            &Answering(Local(node("birch")), None),
            "birch",
            &peer,
            &auth,
            &dialer,
            tomorrow(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(step, Step::Unsupported);
        server.abort();
    }
}
