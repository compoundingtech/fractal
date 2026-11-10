//! After it starts, a fleet member acts on its seats' placement only once its replication has
//! caught up with a peer (#4743). A member that comes back holding an old declaration would
//! otherwise relaunch a seat that has since moved to another host, record that seat's old
//! runtime as ended, or retire it, all from desired state its peers have already replaced.
//! Until then it still adopts and records runtimes that are alive, which is true wherever a seat
//! is placed. An operator, a person or an agent with a reason, can let the member go on without
//! peers it cannot reach, for its current daemon start only.
use super::*;
use crate::model::{ClaimRecord, FractalError};
use serde_json::json;
use tokio::time::Instant;

pub use crate::model::MEMBER_ADMISSION_OVERRIDE_KIND as OVERRIDE_KIND;

/// How long a member waits for its peers before an operator is told.
const WAITING_ALERT_AFTER: Duration = Duration::from_secs(5 * 60);

/// The incremental key each gated action reads, so the items it held are evaluated again once
/// the member is admitted.
const ADMISSION_KEY: &str = "member-admission";

#[derive(Default)]
pub(super) struct MemberAdmission {
    admitted: bool,
    /// Whether this process has cleared an alert left from a start that had the setting on.
    cleared_while_off: bool,
    /// When this member began waiting, monotonic and wall.
    waiting_since: Option<(Instant, u128)>,
}

fn attention_subject(host: &str) -> String {
    let digest = hex::encode(sha2::Sha256::digest(format!("member-admission:{host}").as_bytes()));
    format!("attention/member-admission-{}", &digest[..32])
}

/// This member's current daemon start, which an override names.
pub fn current_start(store: &Store, host: &str) -> Result<Option<ClaimRecord>> {
    Ok(store
        .latest_claim(&format!("daemon/{host}"), Some("daemon.started"))?
        .filter(|claim| claim.origin == host))
}

/// The peers an override releases, when it is well formed and names `start`.
fn released_peers(claim: &ClaimRecord, start: &str) -> Option<Vec<String>> {
    let fields = &claim.body["fields"];
    let actor = claim.actor.as_deref()?;
    let reason = fields["reason"].as_str()?;
    if fields["start"].as_str() != Some(start)
        || !(actor.starts_with("person/") || actor.starts_with("agent/"))
        || reason.trim().is_empty()
    {
        return None;
    }
    let peers = fields["peers"]
        .as_array()?
        .iter()
        .map(|peer| peer.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    (!peers.is_empty()).then_some(peers)
}

/// An override that lets `host` go on without `peers` for its current start.
pub fn override_input(
    store: &Store,
    host: &str,
    actor: &str,
    peers: Vec<String>,
    reason: &str,
    configured: &[String],
    idempotency_key: String,
) -> Result<ClaimInput, FractalError> {
    let invalid = |message: String| FractalError::new("invalid-admission-override", message);
    if !(actor.starts_with("person/") || actor.starts_with("agent/")) {
        return Err(invalid("an override is applied by a person or an agent".into()));
    }
    if reason.trim().is_empty() {
        return Err(invalid("an override needs a reason".into()));
    }
    let known = schedule_admission_peers(store, host, configured)
        .map_err(|error| FractalError::new("internal", format!("{error:#}")))?;
    let peers = peers.into_iter().collect::<BTreeSet<_>>();
    if peers.is_empty() {
        return Err(invalid("an override names the peers it releases".into()));
    }
    if let Some(unknown) = peers.iter().find(|peer| !known.contains(peer)) {
        return Err(invalid(format!("`{unknown}` is not a peer of host/{host}")));
    }
    let start = current_start(store, host)
        .map_err(|error| FractalError::new("internal", format!("{error:#}")))?
        .ok_or_else(|| invalid(format!("host/{host} has recorded no daemon start")))?;
    Ok(ClaimInput {
        subject: format!("host/{host}"),
        kind: OVERRIDE_KIND.into(),
        actor: Some(actor.into()),
        fields: BTreeMap::from([
            ("start".into(), Value::String(start.id.clone())),
            ("peers".into(), json!(peers)),
            ("reason".into(), Value::String(reason.trim().into())),
        ]),
        evidence: vec![start.id],
        expected_subject: None,
        idempotency_key: Some(idempotency_key),
    })
}

impl<R: RuntimeControl> Reconciler<R> {
    /// Decide, once per pass, whether this member may act on its seats' placement, and tell an
    /// operator when it has waited too long.
    pub(super) fn observe_member_admission(&self) -> Result<()> {
        // Once admitted, a member stays admitted for this start: a released peer that comes back
        // does not pause seats the member already acts on.
        if !self.waits_for_peers_after_start {
            // Off: admitted. An alert raised while the setting was on is cleared once.
            let clear = {
                let mut state = self.member_admission.lock().unwrap_or_else(PoisonError::into_inner);
                state.admitted = true;
                !std::mem::replace(&mut state.cleared_while_off, true)
            };
            if clear {
                self.clear_member_admission_alert()?;
            }
            return Ok(());
        }
        let latched = self.member_admission.lock().unwrap_or_else(PoisonError::into_inner).admitted;
        let waiting = if latched { None } else { self.member_admission_waiting()? };
        let admitted = waiting.is_none();
        self.incremental.observe_value(ADMISSION_KEY, admitted.to_string());
        let wall = now_ms();
        let report = {
            let mut state = self.member_admission.lock().unwrap_or_else(PoisonError::into_inner);
            state.admitted = admitted;
            if admitted {
                state.waiting_since = None;
                None
            } else {
                let (since, since_wall) = *state.waiting_since.get_or_insert((Instant::now(), wall));
                if since.elapsed() >= WAITING_ALERT_AFTER {
                    Some(since_wall)
                } else {
                    self.arm_restart(
                        "member-admission-alert",
                        since_wall.saturating_add(WAITING_ALERT_AFTER.as_millis()),
                    );
                    None
                }
            }
        };
        let attention = attention_subject(&self.host);
        let Some(peers) = waiting else {
            return self.clear_member_admission_alert();
        };
        let peers = self.describe_admission_peers(&peers);
        self.record_once(
            &format!("daemon/{}", self.host),
            "daemon.diagnostic",
            BTreeMap::from([
                ("severity".into(), Value::String("warning".into())),
                ("code".into(), Value::String("member-admission-waiting".into())),
                ("status".into(), Value::String("waiting".into())),
                (
                    "reason".into(),
                    Value::String(format!(
                        "this member waits for replication from its peers after starting before it starts, restarts, retires or records the end of any seat: [{peers}]"
                    )),
                ),
            ]),
        )?;
        let Some(since_wall) = report else { return Ok(()) };
        if self.store.operational_failure(&attention)?.is_some_and(|item| item.status == "pending") {
            return Ok(());
        }
        self.store.record_operational_failure(&attention, &AttentionRequest {
            reviewer: "person/operator".into(),
            title: "A restarted member is waiting for its peers before acting on its seats".into(),
            reason: format!(
                "host/{} started at {since_wall} and has not caught up with a peer for at least 5 minutes, so it starts, restarts, retires and records the end of none of its seats. Peers: [{peers}]. projection_deferred={}, catching_up={}. Restore a peer's exchange; to go on without peers that cannot be reached, a person or an agent records `st fleet admit-without --peer NAME --reason TEXT` on this member, for this start only.",
                self.host,
                self.store.replication_projection_deferred(),
                self.store.replication_catching_up(),
            ),
            severity: "warning".into(),
            targets: vec![format!("daemon/{}", self.host)],
            actor: RECONCILER_ACTOR.into(),
            idempotency_key: format!("member-admission-waiting:{}:{since_wall}", self.host),
        })?;
        Ok(())
    }

    fn clear_member_admission_alert(&self) -> Result<()> {
        let attention = attention_subject(&self.host);
        if let Some(failure) = self.store.operational_failure(&attention)?
            && failure.status == "pending"
        {
            self.store.recover_operational_failure(
                &attention,
                "The member caught up with a peer, an override released the peers it waited for, or waiting after a start was turned off.",
                &format!("member-admission-cleared:{}", failure.request),
            )?;
        }
        Ok(())
    }

    /// Whether this member may act on `subject`'s placement: it is admitted, or the seat's
    /// current declaration is one this member made since it started (a run it created, for
    /// instance), which is its own current decision rather than one it may have held from before.
    /// Each action that assumes a seat is placed here asks this first.
    pub(super) fn member_admitted_for(&self, subject: &str) -> Result<bool> {
        if !self.waits_for_peers_after_start {
            return Ok(true);
        }
        smallclaims::touched::note_read(|| ADMISSION_KEY.to_owned());
        if self.member_admission.lock().unwrap_or_else(PoisonError::into_inner).admitted {
            return Ok(true);
        }
        let Some(token) = self.store.selected_desired_token(subject)? else {
            return Ok(false);
        };
        Ok(self.store.claim_by_id(&token)?.is_some_and(|claim| {
            claim.origin == self.host && claim.accepted_at_unix_ms >= self.started_at_unix_ms
        }))
    }

    /// The peers this member still waits for, or `None` once it may act.
    fn member_admission_waiting(&self) -> Result<Option<Vec<String>>> {
        if self.store.replication_projection_deferred()
            || self.store.replication_catching_up()
            || self.store.first_sync()?.is_some_and(|sync| sync.state == "syncing")
        {
            return Ok(Some(Vec::new()));
        }
        let peers = schedule_admission_peers(&self.store, &self.host, &self.schedule_peers)?;
        if peers.is_empty() {
            return Ok(None);
        }
        let sync = self.store.replication_peer_sync(&peers);
        let released = self.released_admission_peers()?;
        // A released peer that is back, measured since this start, counts again.
        let counted = peers
            .into_iter()
            .filter(|peer| {
                !released.contains(peer)
                    || sync.get(peer).is_some_and(|progress| progress.measured_at_unix_ms >= self.started_at_unix_ms)
            })
            .collect::<Vec<_>>();
        if counted.is_empty()
            || schedule_progress_ready(
                counted.iter().filter_map(|peer| sync.get(peer)),
                self.started_at_unix_ms,
                false,
            )
        {
            return Ok(None);
        }
        Ok(Some(counted))
    }

    /// The peers that overrides of this member's current start release. Only this member reads
    /// its own subject, and an override of an earlier start releases nothing.
    fn released_admission_peers(&self) -> Result<BTreeSet<String>> {
        let Some(start) = current_start(&self.store, &self.host)? else {
            return Ok(BTreeSet::new());
        };
        Ok(self
            .store
            .claims_for(&format!("host/{}", self.host), Some(OVERRIDE_KIND))?
            .iter()
            .filter_map(|claim| released_peers(claim, &start.id))
            .flatten()
            .collect())
    }

    fn describe_admission_peers(&self, peers: &[String]) -> String {
        let sync = self.store.replication_peer_sync(peers);
        peers
            .iter()
            .map(|peer| match sync.get(peer) {
                Some(progress) => format!(
                    "{peer}: peer_only_envelopes={}, stale={}, measured_at_unix_ms={}",
                    progress.peer_only_envelopes, progress.stale, progress.measured_at_unix_ms
                ),
                None => format!("{peer}: not measured since this start"),
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{FakeRuntime, apply_source};

    fn started(store: &Store, key: &str) -> ClaimRecord {
        store
            .append_claim(&ClaimInput {
                subject: format!("daemon/{}", store.origin),
                kind: "daemon.started".into(),
                actor: None,
                fields: BTreeMap::from([("status".into(), Value::String("running".into()))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap()
    }

    fn admit_without(store: &Store, host: &str, key: &str) -> ClaimRecord {
        let input = override_input(store, host, "agent/example/operations", vec!["peer".into()],
            "the peer is being rebuilt", &["peer".into()], key.into()).unwrap();
        store.append_claim(&input).unwrap()
    }

    fn seat(store: &Store, root: &std::path::Path) {
        apply_source(store, &format!(
            "version 2\nagent \"worker\" {{ workspace {:?}; command \"sleep 1000\"; restart \"always\" }}\n",
            root.display().to_string()), "worker");
    }

    fn reconciler(store: &Arc<Store>, runtime: &Arc<FakeRuntime>) -> Reconciler<FakeRuntime> {
        Reconciler::new(store.clone(), runtime.clone(), store.origin.clone(), Arc::new(Notify::new()))
            .with_schedule_peers(vec!["peer".into()])
            .with_wait_for_peers_after_start(true)
    }

    fn alert(store: &Store) -> Option<String> {
        store.operational_failure(&attention_subject(&store.origin)).unwrap().map(|failure| failure.status)
    }

    #[tokio::test(start_paused = true)]
    async fn a_member_with_no_reachable_peer_waits_alerts_and_goes_on_only_by_override() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        started(&store, "start");
        seat(&store, root.path());
        let runtime = Arc::new(FakeRuntime::default());
        // The seat's declaration is held from before this start.
        let reconciler = reconciler(&store, &runtime).with_started_at_for_test(now_ms() + 1);
        reconciler.reconcile_once().unwrap();
        reconciler.reconcile_once().unwrap();
        assert!(runtime.started_members.lock().unwrap().is_empty(), "nothing starts before a peer is caught up with");
        assert_eq!(alert(&store), None);

        tokio::time::advance(WAITING_ALERT_AFTER).await;
        reconciler.reconcile_once().unwrap();
        assert_eq!(alert(&store).as_deref(), Some("pending"), "an operator is told after five minutes");
        assert!(runtime.started_members.lock().unwrap().is_empty(), "the alert admits nothing");

        // An override needs a reason and a person or agent, and names peers of this member.
        let refused = |actor: &str, peers: Vec<String>, reason: &str| {
            override_input(&store, "node", actor, peers, reason, &["peer".into()], "refused".into()).unwrap_err().code
        };
        assert_eq!(refused("agent/example/operations", vec!["peer".into()], " "), "invalid-admission-override");
        assert_eq!(refused("host/node", vec!["peer".into()], "why"), "invalid-admission-override");
        assert_eq!(refused("agent/example/operations", vec!["stranger".into()], "why"), "invalid-admission-override");
        admit_without(&store, "node", "override");
        reconciler.reconcile_once().unwrap();
        assert_eq!(runtime.started_members.lock().unwrap().len(), 1, "the override lets the member go on");
        assert_ne!(alert(&store).as_deref(), Some("pending"), "and ends the alert");
    }

    #[tokio::test]
    async fn an_override_is_ignored_after_a_later_start_and_by_any_other_member() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let runtime = Arc::new(FakeRuntime::default());
        started(&store, "first-start");
        admit_without(&store, "node", "override");
        let first = reconciler(&store, &runtime);
        assert_eq!(first.member_admission_waiting().unwrap(), None);

        // The daemon starts again: the override named the earlier start.
        started(&store, "second-start");
        let second = reconciler(&store, &runtime);
        assert_eq!(second.member_admission_waiting().unwrap(), Some(vec!["peer".to_owned()]));

        // Another member never reads an override of this one, even one naming its own start.
        let other = Arc::new(Store::open_memory("other").unwrap());
        let other_start = started(&other, "other-start");
        let mut foreign = override_input(&store, "node", "agent/example/operations", vec!["peer".into()],
            "the peer is being rebuilt", &["peer".into()], "foreign".into()).unwrap();
        foreign.fields.insert("start".into(), Value::String(other_start.id.clone()));
        foreign.evidence = vec![other_start.id.clone()];
        other.append_claim(&foreign).unwrap();
        let other = reconciler(&other, &runtime);
        assert_eq!(other.member_admission_waiting().unwrap(), Some(vec!["peer".to_owned()]));
    }

    #[tokio::test]
    async fn a_waiting_member_starts_a_seat_it_declared_since_it_started() {
        // A run this member creates, for instance a schedule's standby taking over an occurrence,
        // declares its seats here after the start: that is this member's current decision.
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        started(&store, "start");
        let runtime = Arc::new(FakeRuntime::default());
        let reconciler = reconciler(&store, &runtime).with_started_at_for_test(now_ms().saturating_sub(1));
        seat(&store, root.path());
        reconciler.reconcile_once().unwrap();
        assert!(reconciler.member_admission_waiting().unwrap().is_some());
        assert_eq!(runtime.started_members.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn without_the_setting_a_member_acts_at_once_as_before() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        started(&store, "start");
        seat(&store, root.path());
        let runtime = Arc::new(FakeRuntime::default());
        let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()))
            .with_schedule_peers(vec!["peer".into()])
            .with_started_at_for_test(now_ms() + 1);
        reconciler.reconcile_once().unwrap();
        assert_eq!(runtime.started_members.lock().unwrap().len(), 1, "an unmeasured peer holds nothing back");
        assert!(store.claims_for("daemon/node", Some("daemon.diagnostic")).unwrap().iter()
            .all(|claim| claim.body["fields"]["code"] != "member-admission-waiting"));
    }

    #[tokio::test]
    async fn a_member_with_no_peers_starts_its_seats_at_once() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        seat(&store, root.path());
        let runtime = Arc::new(FakeRuntime::default());
        let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
        reconciler.reconcile_once().unwrap();
        assert_eq!(runtime.started_members.lock().unwrap().len(), 1);
    }
}
