//! Local timing of replication-gated due ticks; operator attention uses existing failure claims.
use super::*;
use tokio::time::Instant;

/// A due tick must remain locally observed as replication-blocked for fifteen minutes.
/// Restart resets this observation window; it never changes admission or catch-up policy.
const BLOCKED_TICK_AFTER: Duration = Duration::from_secs(15 * 60);

pub(super) struct BlockedTick {
    revision: String,
    occurrence: u64,
    scheduled_at: i64,
    observed: Instant,
    observed_wall: u128,
}

fn attention_subject(schedule: &str, host: &str) -> String {
    let digest = hex::encode(sha2::Sha256::digest(format!("{schedule}:{host}").as_bytes()));
    format!("attention/schedule-replication-{}", &digest[..32])
}

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn clear_schedule_replication_block(&self, schedule: &DesiredSubject) -> Result<()> {
        self.schedule_replication_blocks.lock().unwrap_or_else(PoisonError::into_inner)
            .remove(&schedule.subject);
        let attention = attention_subject(&schedule.subject, &self.host);
        if let Some(failure) = self.store.operational_failure(&attention)?
            && failure.status == "pending" {
            self.store.recover_operational_failure(&attention,
                "The gate cleared, the blocked tick progressed, or this member no longer owns the active schedule.",
                &format!("schedule-replication-cleared:{}", failure.request))?;
        }
        Ok(())
    }

    pub(super) fn observe_schedule_replication_gate(
        &self, schedule: &DesiredSubject, ready: bool, wall: u128,
    ) -> Result<()> {
        if ready { return self.clear_schedule_replication_block(schedule); }
        let Some(spec) = owned_schedule_spec(&self.store, schedule)? else { return Ok(()) };
        if spec.stopped { return self.clear_schedule_replication_block(schedule); }
        let Some(revision) = self.store.selected_desired_revision(&schedule.subject)? else { return Ok(()) };
        let reached = self.store.claims_for(&schedule.subject, Some("schedule.occurrence-reached"))?;
        let last = reached.iter().filter_map(|claim| claim.body["fields"]["occurrence"].as_u64()).max();
        // A tick whose work was requested can be held at the second admission boundary too.
        let requests = self.store.pending_schedule_work_requests(&schedule.subject)?;
        let queued = requests.iter().filter_map(|request| request.body["fields"]["occurrence"].as_u64()).min();
        let previous = {
            let blocks = self.schedule_replication_blocks.lock().unwrap_or_else(PoisonError::into_inner);
            blocks.get(&schedule.subject).map(|block| (
                block.revision == revision && (queued == Some(block.occurrence)
                    || last.is_none_or(|last| last < block.occurrence)),
                block.occurrence, block.scheduled_at,
            ))
        };
        if previous.is_some_and(|(current, _, _)| !current) {
            self.clear_schedule_replication_block(schedule)?;
        }
        let candidate = if let Some((true, occurrence, at)) = previous {
            Some((occurrence, at))
        } else if let Some(occurrence) = queued {
            reached.iter().find(|claim| claim.body["fields"]["occurrence"].as_u64() == Some(occurrence))
                .and_then(|claim| claim.body["fields"]["scheduled_at_unix_ms"].as_str()?.parse().ok())
                .map(|at| (occurrence, at))
        } else {
            due_tick(&spec, last, wall.min(i64::MAX as u128) as i64)?
        };
        let Some((occurrence, scheduled_at)) = candidate else {
            return self.clear_schedule_replication_block(schedule);
        };
        let (deadline, report) = {
            let mut blocks = self.schedule_replication_blocks.lock().unwrap_or_else(PoisonError::into_inner);
            let block = blocks.entry(schedule.subject.clone()).or_insert_with(|| BlockedTick {
                revision: revision.clone(), occurrence, scheduled_at,
                observed: Instant::now(), observed_wall: wall,
            });
            if block.revision != revision {
                *block = BlockedTick { revision, occurrence, scheduled_at,
                    observed: Instant::now(), observed_wall: wall };
            }
            let deadline = block.observed_wall.saturating_add(BLOCKED_TICK_AFTER.as_millis());
            let report = (block.observed.elapsed() >= BLOCKED_TICK_AFTER)
                .then_some((block.occurrence, block.scheduled_at, block.observed_wall));
            (deadline, report)
        };
        if let Some((occurrence, scheduled_at, observed_wall)) = report {
            let attention = attention_subject(&schedule.subject, &self.host);
            if self.store.operational_failure(&attention)?.is_some_and(|item| item.status == "pending") {
                return Ok(());
            }
            let peers = schedule_admission_peers(&self.store, &self.host, &self.schedule_peers)?;
            let sync = self.store.replication_peer_sync(&peers);
            let peers = peers.iter().map(|peer| match sync.get(peer) {
                Some(progress) => format!("{peer}: peer_only_envelopes={}, local_only_envelopes={}, stale={}, measured_at_unix_ms={}",
                    progress.peer_only_envelopes, progress.local_only_envelopes, progress.stale, progress.measured_at_unix_ms),
                None => format!("{peer}: peer_only_envelopes=unknown, local_only_envelopes=unknown (no comparison)"),
            }).collect::<Vec<_>>().join("; ");
            self.store.record_operational_failure(&attention, &AttentionRequest {
                reviewer: "person/operator".into(),
                title: "A due schedule tick is blocked by replication".into(),
                reason: format!("{} on {}: occurrence {occurrence}, due at {scheduled_at}, has remained replication-blocked for at least 15 minutes. Peer comparisons: [{peers}]. projection_deferred={}, catching_up={}, first_sync_syncing={}. Inspect replication status and restore the blocking peer's exchange; this warning does not grant admission or takeover.",
                    schedule.subject, self.host, self.store.replication_projection_deferred(), self.store.replication_catching_up(),
                    self.store.first_sync()?.is_some_and(|sync| sync.state == "syncing")),
                severity: "warning".into(), targets: vec![schedule.subject.clone()],
                actor: RECONCILER_ACTOR.into(),
                idempotency_key: format!("schedule-replication-blocked:{}:{}:{observed_wall}", schedule.subject, self.host),
            })?;
        } else {
            self.arm_restart(&format!("schedule-replication-attention:{}", schedule.subject), deadline);
        }
        Ok(())
    }
}

/// Observe a missed tick even for `skip`, without changing the scheduler's actual selection.
/// Pinning the first observed candidate keeps short intervals from resetting the warning clock.
fn due_tick(spec: &crate::model::ScheduleSpec, last: Option<u64>, now: i64) -> Result<Option<(u64, i64)>> {
    let candidate = if let Some(at) = spec.at_unix_ms {
        if last.is_some() { return Ok(None); }
        (0, at)
    } else if let Some(calendar) = &spec.calendar {
        calendar_occurrence(calendar, last, now, "latest", None)?
    } else if let (Some(interval), Some(anchor)) = (spec.every_ms.filter(|ms| *ms > 0), spec.anchor_unix_ms) {
        if now < anchor { return Ok(None); }
        let current = (now - anchor) as u64 / interval;
        if last.is_some_and(|last| last >= current) { return Ok(None); }
        (current, anchor.checked_add(i64::try_from(interval.checked_mul(current).context("schedule occurrence overflow")?)?)
            .context("schedule timestamp overflow")?)
    } else { return Ok(None) };
    Ok((candidate.1 <= now).then_some(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::tests::{FakeRuntime, apply_source};

    fn setup(clock: &str) -> (Arc<Store>, Reconciler<FakeRuntime>, DesiredSubject) {
        let store = Arc::new(Store::open_memory("node").unwrap());
        apply_source(&store, &format!(r#"version 2
schedule "orchard" {{ {clock}
 work {{ mission "orchard"; workspace "/workspace/orchard" }}
}}"#), "initial");
        let schedule = store.desired_subjects().unwrap().into_iter().find(|s| s.kind == "schedule").unwrap();
        let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()),
            "node".into(), Arc::new(Notify::new())).with_schedule_peers(vec!["peer".into()]);
        store.replication_sync.lock().unwrap().insert("peer".into(), smallclaims::store::PeerSyncProgress {
            measured: Some(smallclaims::replication::ReplicationPeerSync {
                peer_only_envelopes: 7, local_only_envelopes: 3, measured_at_unix_ms: now_ms(),
                ..Default::default()
            }), ..Default::default()
        });
        (store, reconciler, schedule)
    }

    #[tokio::test(start_paused = true)]
    async fn blocked_tick_warns_at_bound_once_with_peer_counts_and_recovers() {
        let (store, reconciler, schedule) = setup(r#"at "2020-01-01T00:00:00Z""#);
        let wall = now_ms();
        assert!(!reconciler.schedules_caught_up(&schedule).unwrap());
        let attention = attention_subject(&schedule.subject, "node");
        assert!(store.operational_failure(&attention).unwrap().is_none());
        tokio::time::advance(BLOCKED_TICK_AFTER - Duration::from_millis(1)).await;
        reconciler.observe_schedule_replication_gate(&schedule, false, wall + 899_999).unwrap();
        assert!(store.operational_failure(&attention).unwrap().is_none());
        tokio::time::advance(Duration::from_millis(1)).await;
        reconciler.observe_schedule_replication_gate(&schedule, false, wall + 900_000).unwrap();
        let warning = store.operational_failure(&attention).unwrap().unwrap();
        assert_eq!(warning.status, "pending");
        assert!(warning.reason.contains("peer: peer_only_envelopes=7, local_only_envelopes=3"));
        assert!(warning.reason.contains("occurrence 0"));
        reconciler.observe_schedule_replication_gate(&schedule, false, wall + 900_001).unwrap();
        assert_eq!(store.operational_failure(&attention).unwrap().unwrap().request, warning.request);
        reconciler.observe_schedule_replication_gate(&schedule, true, wall + 900_002).unwrap();
        assert_ne!(store.operational_failure(&attention).unwrap().unwrap().status, "pending");
        assert!(reconciler.schedule_replication_blocks.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn future_tick_and_restart_do_not_inherit_a_warning_window() {
        let (store, reconciler, schedule) = setup(r#"at "2030-01-01T00:00:00Z""#);
        let wall = now_ms();
        reconciler.observe_schedule_replication_gate(&schedule, false, wall).unwrap();
        tokio::time::advance(BLOCKED_TICK_AFTER).await;
        reconciler.observe_schedule_replication_gate(&schedule, false, wall + 900_000).unwrap();
        assert!(store.operational_failure(&attention_subject(&schedule.subject, "node")).unwrap().is_none());
        assert!(reconciler.schedule_replication_blocks.lock().unwrap().is_empty());

        let (store, reconciler, schedule) = setup(r#"at "2020-01-01T00:00:00Z""#);
        reconciler.observe_schedule_replication_gate(&schedule, false, wall).unwrap();
        tokio::time::advance(BLOCKED_TICK_AFTER).await;
        let restarted = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()),
            "node".into(), Arc::new(Notify::new())).with_schedule_peers(vec!["peer".into()]);
        restarted.observe_schedule_replication_gate(&schedule, false, wall + 900_000).unwrap();
        assert!(store.operational_failure(&attention_subject(&schedule.subject, "node")).unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn short_skip_intervals_preserve_first_blocked_tick_and_revision_resets_it() {
        let (store, reconciler, schedule) = setup(r#"every "1m"; anchor "2020-01-01T00:00:00Z"; catch-up "skip""#);
        let wall = now_ms();
        reconciler.observe_schedule_replication_gate(&schedule, false, wall).unwrap();
        let original = reconciler.schedule_replication_blocks.lock().unwrap()[&schedule.subject].occurrence;
        for minute in 1..=15 {
            tokio::time::advance(Duration::from_secs(60)).await;
            reconciler.observe_schedule_replication_gate(&schedule, false, wall + minute * 60_000).unwrap();
        }
        let attention = attention_subject(&schedule.subject, "node");
        let warning = store.operational_failure(&attention).unwrap().unwrap();
        assert!(warning.reason.contains(&format!("occurrence {original},")));
        apply_source(&store, r#"version 2
schedule "orchard" {
 every "2m"; anchor "2020-01-01T00:00:00Z"; catch-up "skip"
 work { mission "orchard"; workspace "/workspace/orchard" }
}"#, "changed");
        let changed = store.desired_subjects().unwrap().into_iter().find(|s| s.kind == "schedule").unwrap();
        reconciler.observe_schedule_replication_gate(&changed, false, wall + 900_001).unwrap();
        assert_ne!(store.operational_failure(&attention).unwrap().unwrap().status, "pending");
    }

    #[tokio::test(start_paused = true)]
    async fn replicated_active_run_retires_warning_before_open_work_return() {
        let (store, reconciler, schedule) = setup(r#"at "2020-01-01T00:00:00Z""#);
        let wall = now_ms();
        reconciler.observe_schedule_replication_gate(&schedule, false, wall).unwrap();
        tokio::time::advance(BLOCKED_TICK_AFTER).await;
        reconciler.observe_schedule_replication_gate(&schedule, false, wall + 900_000).unwrap();
        let attention = attention_subject(&schedule.subject, "node");
        assert_eq!(store.operational_failure(&attention).unwrap().unwrap().status, "pending");
        apply_source(&store, r#"version 2
mission "orchard" state="ready" {
 goal "Exercise replicated schedule progress."
 step "wait" { agentless }
}"#, "mission");
        let root = tempfile::tempdir().unwrap();
        let run = store.create_mission_run(&MissionRunRequest {
            mission: "orchard".into(), revision: None, workspace: root.path().display().to_string(),
            requester: None, mode: Some("run".into()), inputs: BTreeMap::new(),
            idempotency_key: "active-orchard".into(),
        }).unwrap();
        let request = store.append_claim(&ClaimInput {
            subject: schedule.subject.clone(), kind: "schedule.work-requested".into(), actor: None,
            fields: BTreeMap::from([
                ("revision".into(), Value::String(store.selected_desired_revision(&schedule.subject).unwrap().unwrap())),
                ("occurrence".into(), Value::from(0)),
                ("mission".into(), Value::String("mission/orchard".into())),
                ("mission_revision".into(), Value::String(run.revision)),
                ("workspace".into(), Value::String(root.path().display().to_string())),
                ("inputs".into(), serde_json::json!({})),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: Some("requested-orchard".into()),
        }).unwrap();
        store.append_claim(&ClaimInput {
            subject: schedule.subject.clone(), kind: "schedule.work-started".into(), actor: None,
            fields: BTreeMap::from([
                ("request".into(), Value::String(request.id)),
                ("mission_run".into(), Value::String(run.subject)),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: Some("started-orchard".into()),
        }).unwrap();
        assert!(store.schedule_has_active_started_run(&schedule.subject).unwrap());
        reconciler.reconcile_schedule(&schedule).unwrap();
        assert_ne!(store.operational_failure(&attention).unwrap().unwrap().status, "pending");
        assert!(reconciler.schedule_replication_blocks.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn persisted_warning_retires_after_restart_and_replicated_completion() {
        let (store, reconciler, schedule) = setup(r#"at "2020-01-01T00:00:00Z""#);
        let wall = now_ms();
        reconciler.observe_schedule_replication_gate(&schedule, false, wall).unwrap();
        tokio::time::advance(BLOCKED_TICK_AFTER).await;
        reconciler.observe_schedule_replication_gate(&schedule, false, wall + 900_000).unwrap();
        let attention = attention_subject(&schedule.subject, "node");
        assert_eq!(store.operational_failure(&attention).unwrap().unwrap().status, "pending");
        drop(reconciler);
        let restarted = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()),
            "node".into(), Arc::new(Notify::new())).with_schedule_peers(vec!["peer".into()]);
        assert!(restarted.schedule_replication_blocks.lock().unwrap().is_empty());
        assert_eq!(store.operational_failure(&attention).unwrap().unwrap().status, "pending");
        let peer = Store::open_memory("peer").unwrap();
        peer.append_claim(&ClaimInput {
            subject: schedule.subject.clone(), kind: "schedule.occurrence-reached".into(), actor: None,
            fields: BTreeMap::from([
                ("revision".into(), Value::String(store.selected_desired_revision(&schedule.subject).unwrap().unwrap())),
                ("occurrence".into(), Value::from(0)),
                ("scheduled_at_unix_ms".into(), Value::String("1577836800000".into())),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: Some("peer-completed".into()),
        }).unwrap();
        store.import_replication("peer", &peer.export_replication(0).unwrap()).unwrap();
        restarted.observe_schedule_replication_gate(&schedule, false, wall + 900_001).unwrap();
        assert_ne!(store.operational_failure(&attention).unwrap().unwrap().status, "pending");
        assert!(restarted.schedule_replication_blocks.lock().unwrap().is_empty());
        tokio::time::advance(BLOCKED_TICK_AFTER).await;
        restarted.observe_schedule_replication_gate(&schedule, false, wall + 1_800_001).unwrap();
        assert_ne!(store.operational_failure(&attention).unwrap().unwrap().status, "pending");
    }
}
