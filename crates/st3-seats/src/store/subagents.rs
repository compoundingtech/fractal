//! The part of `store::subagents` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_attention::store::subagents::*;
pub use st3_attention::store::subagents::{SUBAGENT_KINDS, Seat, SubagentView, check_subagent_claim_tx};

/// How long a subagent's lease lasts from its appearance or renewal.
pub const SUBAGENT_LEASE_MS: u64 = 10 * 60 * 1000;

/// A subagent this node ended because what started it went away or its lease ran out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EndedSubagent {
    pub agent: String,
    pub subagent_id: String,
    pub outcome: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SubagentSweep {
    pub ended: Vec<EndedSubagent>,
    /// The earliest lease that still runs on this node, when the sweep should look again.
    pub next_expiry_unix_ms: Option<u64>,
    /// Every subagent this node recorded before this claim has ended, so the next sweep starts
    /// here. An end is never undone and new appearances sort later.
    pub low_water: u64,
}

/// Why an open subagent must end now, as its outcome and reason, or `None` while it may run.
fn ending(seat: &Seat, subagent: &SubagentView, now: u64) -> Option<(&'static str, String)> {
    if seat.declared.as_deref() != Some("agent") {
        return Some(("seat-stopped", "its seat was stopped or removed".into()));
    }
    match &seat.runtime {
        Some((status, _)) if status == "stopped" => {
            return Some(("seat-stopped", "its seat was stopped".into()));
        }
        Some((status, _))
            if matches!(status.as_str(), "exited" | "vanished" | "absent" | "failed") =>
        {
            return Some(("harness-exited", format!("its harness {status}")));
        }
        Some((_, Some(incarnation))) if *incarnation != subagent.incarnation_id => {
            return Some((
                "harness-exited",
                format!("its harness restarted as incarnation {incarnation}"),
            ));
        }
        _ => {}
    }
    subagent
        .expired_at(now)
        .then(|| ("expired", "its lease ran out without a renewal".into()))
}

pub trait SubagentsStore {
    /// Every open subagent, oldest first.
    fn all_open_subagents(&self) -> Result<Vec<SubagentView>>;
    /// Every subagent running at `now`: open, with a lease that runs past it. Oldest first.
    fn running_subagents(&self, now: u64) -> Result<Vec<SubagentView>>;
    /// End each subagent recorded on this node from `low_water` on whose seat was stopped or
    /// removed, whose harness exited or restarted, or whose lease ran out.
    fn end_stale_subagents(&self, now: u64, low_water: u64) -> Result<SubagentSweep, St3Error>;
}

impl SubagentsStore for Store {

    fn all_open_subagents(&self) -> Result<Vec<SubagentView>> {
        open_subagents_at(&self.readers.get(), Scope::All)
    }

    fn running_subagents(&self, now: u64) -> Result<Vec<SubagentView>> {
        let mut running = open_subagents_at(&self.readers.get(), Scope::Running(now))?;
        // A renewal that ran out leaves only the appearance's lease to check.
        running.retain(|subagent| !subagent.expired_at(now));
        Ok(running)
    }

    fn end_stale_subagents(&self, now: u64, low_water: u64) -> Result<SubagentSweep, St3Error> {
        let mut sweep = SubagentSweep {
            low_water,
            ..SubagentSweep::default()
        };
        let mut stale = Vec::new();
        let mut running = None::<u64>;
        {
            let connection = self.readers.get();
            sweep.low_water = connection
                .query_row(
                    "SELECT COALESCE(MAX(store_index), 0) + 1 FROM claims",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .map_err(internal)?
                .max(low_water);
            let mut seats = BTreeMap::<String, Seat>::new();
            for subagent in
                open_subagents_at(&connection, Scope::RecordedOn(&self.origin, low_water))
                    .map_err(internal)?
            {
                if !seats.contains_key(&subagent.agent) {
                    let state = seat(&connection, &subagent.agent).map_err(internal)?;
                    seats.insert(subagent.agent.clone(), state);
                }
                match ending(&seats[&subagent.agent], &subagent, now) {
                    Some((outcome, reason)) => stale.push((subagent, outcome, reason)),
                    None => {
                        let next = sweep
                            .next_expiry_unix_ms
                            .get_or_insert(subagent.lease_expires_at_unix_ms);
                        *next = (*next).min(subagent.lease_expires_at_unix_ms);
                        let oldest = running.get_or_insert(subagent.store_index);
                        *oldest = (*oldest).min(subagent.store_index);
                    }
                }
            }
        }
        // A subagent that failed to end below stays in the next sweep.
        let mut failed = None::<St3Error>;
        for (subagent, outcome, reason) in stale {
            let mut fields = BTreeMap::from([
                (
                    "subagent_id".into(),
                    Value::String(subagent.subagent_id.clone()),
                ),
                ("outcome".into(), Value::String(outcome.into())),
                ("reason".into(), Value::String(reason.clone())),
                ("ended_at_unix_ms".into(), Value::from(now)),
            ]);
            if subagent.started_at_unix_ms > 0 {
                fields.insert(
                    "duration_ms".into(),
                    Value::from(now.saturating_sub(subagent.started_at_unix_ms)),
                );
            }
            let ended = self.append_claim_outcome(&ClaimInput {
                subject: subagent.agent.clone(),
                kind: "subagent.ended".into(),
                actor: None,
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!(
                    "subagent-sweep:{}:{}",
                    subagent.agent, subagent.subagent_id
                )),
            });
            match ended {
                Ok((_, true)) => sweep.ended.push(EndedSubagent {
                    agent: subagent.agent,
                    subagent_id: subagent.subagent_id,
                    outcome: outcome.into(),
                    reason,
                }),
                Ok((_, false)) => {}
                Err(error) => {
                    let oldest = running.get_or_insert(subagent.store_index);
                    *oldest = (*oldest).min(subagent.store_index);
                    failed.get_or_insert(error);
                }
            }
        }
        if let Some(oldest) = running {
            sweep.low_water = oldest;
        }
        failed.map_or(Ok(sweep), Err)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use st3_attention::store::subagents_tests_shared_seats::*;
    use st3_attention::store::subagents_tests_shared_seats::{SEAT, appear, claim, ids, runtime};

    fn declare(store: &Store, seat: &str) {
        let name = seat.trim_start_matches("agent/");
        let intent = crate::graph::parse_internal_intent(
            &format!("version 2\nagent {name:?} {{ workspace \"/tmp\"; command \"true\"; }}"),
            "alder",
        )
        .unwrap();
        store
            .apply_internal(&intent, &format!("declare-{seat}"))
            .unwrap();
    }

    fn stop(store: &Store, seat: &str) {
        let intent =
            crate::graph::parse_internal_intent(&format!("version 2\nstop {seat:?}\n"), "alder")
                .unwrap();
        store
            .apply_internal(&intent, &format!("stop-{seat}"))
            .unwrap();
    }

    fn running_seat() -> Store {
        let store = Store::open_memory("alder").unwrap();
        declare(&store, SEAT);
        runtime(&store, SEAT, "running", "inc-1");
        store
    }

    #[test]
    fn a_subagent_is_open_from_its_appearance_to_its_end() {
        let store = running_seat();
        assert!(appear(&store, "a1", 50_000).unwrap().1);
        let open = store.open_subagents(SEAT).unwrap();
        assert_eq!(open.len(), 1);
        let subagent = &open[0];
        assert_eq!(subagent.agent, SEAT);
        assert_eq!(subagent.subagent_type.as_deref(), Some("Explore"));
        assert_eq!(subagent.description.as_deref(), Some("map the code"));
        assert_eq!(subagent.step_run.as_deref(), Some("step-run/run-1/claims"));
        assert_eq!(subagent.session_id.as_deref(), Some("session-1"));
        assert_eq!(subagent.lease_expires_at_unix_ms, 50_000);
        assert_eq!(subagent.origin, "alder");
        assert!(subagent.expired_at(50_000) && !subagent.expired_at(49_999));

        // A repeated appearance answers with the first and records nothing.
        let (again, appended) = appear(&store, "a1", 90_000).unwrap();
        assert!(!appended);
        assert_eq!(again.id, subagent.appeared);

        let lease = [("lease_expires_at_unix_ms", json!(80_000))];
        assert!(claim(&store, "subagent.renewed", "a1", &lease).unwrap().1);
        assert_eq!(
            store.open_subagents(SEAT).unwrap()[0].lease_expires_at_unix_ms,
            80_000
        );

        let end = [("outcome", json!("completed")), ("total_tokens", json!(12))];
        let (ended, appended) = claim(&store, "subagent.ended", "a1", &end).unwrap();
        assert!(appended);
        assert!(ids(&store).is_empty());
        // A second end answers with the first; a renewal after the end is refused.
        let (again, appended) = claim(
            &store,
            "subagent.ended",
            "a1",
            &[("outcome", json!("expired"))],
        )
        .unwrap();
        assert!(!appended);
        assert_eq!(again.id, ended.id);
        assert_eq!(
            claim(&store, "subagent.renewed", "a1", &lease)
                .unwrap_err()
                .code,
            "subagent-ended"
        );
        // A subagent that never appeared can be neither renewed nor ended.
        for (kind, field) in [
            ("subagent.renewed", ("lease_expires_at_unix_ms", json!(1))),
            ("subagent.ended", ("outcome", json!("completed"))),
        ] {
            assert_eq!(
                claim(&store, kind, "never", &[field]).unwrap_err().code,
                "unknown-subagent"
            );
        }
        assert_eq!(
            claim(
                &store,
                "subagent.ended",
                "a1",
                &[("outcome", json!("vanished"))]
            )
            .unwrap_err()
            .code,
            "invalid-claim-field"
        );
    }

    #[test]
    fn running_subagents_are_open_with_a_lease_that_runs_on() {
        let store = running_seat();
        appear(&store, "ended", 50_000).unwrap();
        claim(
            &store,
            "subagent.ended",
            "ended",
            &[("outcome", json!("completed"))],
        )
        .unwrap();
        appear(&store, "lapsed", 9_000).unwrap();
        appear(&store, "renewed", 9_000).unwrap();
        claim(
            &store,
            "subagent.renewed",
            "renewed",
            &[("lease_expires_at_unix_ms", json!(30_000))],
        )
        .unwrap();
        appear(&store, "fresh", 20_000).unwrap();
        let running = |now| {
            store
                .running_subagents(now)
                .unwrap()
                .into_iter()
                .map(|subagent| subagent.subagent_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(running(10_000), ["renewed", "fresh"]);
        assert_eq!(running(25_000), ["renewed"]);
        assert!(running(30_000).is_empty());
        // The open ones still include the lapsed subagent until its node ends it.
        assert_eq!(ids(&store), ["lapsed", "renewed", "fresh"]);
    }

    #[test]
    fn a_seat_records_only_its_own_subagents() {
        let store = running_seat();
        let error = store
            .append_client_claim_outcome(&ClaimInput {
                subject: SEAT.into(),
                kind: "subagent.appeared".into(),
                actor: Some("agent/alder.other".into()),
                fields: BTreeMap::from([
                    ("subagent_id".into(), json!("a1")),
                    ("driver".into(), json!("claude")),
                    ("incarnation_id".into(), json!("inc-1")),
                    ("lease_expires_at_unix_ms".into(), json!(1)),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap_err();
        assert_eq!(error.code, "claim-write-forbidden");
    }

    #[test]
    fn the_sweep_ends_an_unrenewed_subagent_and_waits_for_a_running_one() {
        let store = running_seat();
        appear(&store, "short", 10_000).unwrap();
        appear(&store, "long", 30_000).unwrap();
        let sweep = store.end_stale_subagents(9_999, 0).unwrap();
        assert!(sweep.ended.is_empty());
        assert_eq!(sweep.next_expiry_unix_ms, Some(10_000));

        let sweep = store.end_stale_subagents(10_000, sweep.low_water).unwrap();
        assert_eq!(
            sweep.ended,
            [EndedSubagent {
                agent: SEAT.into(),
                subagent_id: "short".into(),
                outcome: "expired".into(),
                reason: "its lease ran out without a renewal".into(),
            }]
        );
        assert_eq!(sweep.next_expiry_unix_ms, Some(30_000));
        assert_eq!(ids(&store), ["long"]);
        let ended = store
            .graph
            .claims_for(SEAT, Some("subagent.ended"))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(ended.actor, None);
        assert_eq!(ended.body["fields"]["duration_ms"], 9_000);

        // A renewal keeps it running past its first lease.
        claim(
            &store,
            "subagent.renewed",
            "long",
            &[("lease_expires_at_unix_ms", json!(60_000))],
        )
        .unwrap();
        let sweep = store.end_stale_subagents(45_000, sweep.low_water).unwrap();
        assert!(sweep.ended.is_empty());
        assert_eq!(sweep.next_expiry_unix_ms, Some(60_000));
    }

    #[test]
    fn the_sweep_ends_subagents_whose_harness_exited_or_restarted() {
        let store = running_seat();
        appear(&store, "a1", u64::MAX / 2).unwrap();
        runtime(&store, SEAT, "running", "inc-2");
        let sweep = store.end_stale_subagents(5_000, 0).unwrap();
        assert_eq!(sweep.ended.len(), 1);
        assert_eq!(sweep.ended[0].outcome, "harness-exited");
        assert_eq!(
            sweep.ended[0].reason,
            "its harness restarted as incarnation inc-2"
        );

        let store = running_seat();
        appear(&store, "a1", u64::MAX / 2).unwrap();
        runtime(&store, SEAT, "exited", "inc-1");
        let sweep = store.end_stale_subagents(5_000, 0).unwrap();
        assert_eq!(sweep.ended[0].outcome, "harness-exited");
        assert_eq!(sweep.ended[0].reason, "its harness exited");
    }

    #[test]
    fn the_sweep_ends_the_subagents_of_a_stopped_seat() {
        let store = running_seat();
        appear(&store, "a1", u64::MAX / 2).unwrap();
        appear(&store, "a2", u64::MAX / 2).unwrap();
        stop(&store, SEAT);
        let sweep = store.end_stale_subagents(5_000, 0).unwrap();
        assert_eq!(
            sweep
                .ended
                .iter()
                .map(|ended| (ended.subagent_id.as_str(), ended.outcome.as_str()))
                .collect::<Vec<_>>(),
            [("a1", "seat-stopped"), ("a2", "seat-stopped")]
        );
        assert!(ids(&store).is_empty());
        assert_eq!(sweep.next_expiry_unix_ms, None);
    }

    #[test]
    fn the_sweep_starts_after_what_has_ended_and_still_finds_new_subagents() {
        let store = running_seat();
        for index in 0..60 {
            appear(&store, &format!("many-{index}"), 1_000).unwrap();
        }
        let sweep = store.end_stale_subagents(1_000, 0).unwrap();
        assert_eq!(sweep.ended.len(), 60);
        assert!(store.all_open_subagents().unwrap().is_empty());
        let low_water = sweep.low_water;
        appear(&store, "later", 2_000).unwrap();
        let sweep = store.end_stale_subagents(1_500, low_water).unwrap();
        assert!(sweep.ended.is_empty());
        assert_eq!(sweep.next_expiry_unix_ms, Some(2_000));
        // The open subagent holds the low water at itself.
        assert!(sweep.low_water > low_water);
        let open = store.open_subagents(SEAT).unwrap();
        assert_eq!(sweep.low_water, open[0].store_index);
        let sweep = store.end_stale_subagents(2_000, sweep.low_water).unwrap();
        assert_eq!(sweep.ended.len(), 1);
    }
}
