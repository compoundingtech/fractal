//! The fault snapshot's failure episodes, kept between snapshots. A host holds thousands of
//! operational failures, nearly all of them on runs that ended long ago, and a snapshot every 30
//! seconds read them all again. Each failure now remembers what it read and is read again only
//! when the change feed names one of those keys, when its time comes, or when it rests on a read
//! the feed cannot name (`UNKEYED_READ`). A full rebuild on a timer compares every answer with
//! the cache; a difference is a correction, counted in `doctor --performance`, and means a read
//! is missing its key.
use super::attention_snapshot::{OperationalFault, UNKEYED_READ};
use super::*;
use crate::incremental::{FULL_PASS_INTERVAL_MS, Incremental};

/// A host whose rebuilds found no correction for this long rebuilds every [`QUIET_REBUILD_MS`];
/// any correction brings it back to [`FULL_PASS_INTERVAL_MS`].
pub(super) const QUIET_AFTER_MS: u128 = 24 * 60 * 60 * 1000;
pub(super) const QUIET_REBUILD_MS: u128 = 30 * 60 * 1000;

#[derive(Default)]
pub(super) struct FaultCache(Mutex<Option<Cache>>);

struct Cache {
    incremental: Incremental,
    entries: HashMap<String, Entry>,
    /// The `as_of` of the last snapshot, the last full rebuild, and since when every rebuild
    /// matched.
    as_of: u128,
    rebuilt: u128,
    clean_since: u128,
}

#[derive(Clone)]
struct Entry {
    /// The episode as folded, so a recovery or a later failure of the same episode reads again.
    failure: Value,
    unkeyed: bool,
    faults: Faults,
}

/// What one failure gives: its operational fault and its broken-gate fault for an agent.
type Faults = (Option<OperationalFault>, Option<AttentionItemView>);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Failures read again (not answered from the cache) on this thread.
    pub static FAULT_CACHE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Off for a test that counts what the cache alone costs.
    pub static FAULT_CACHE_PARITY: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Non-episode fault families, indexed by their source rather than rebuilt for each delivery.
#[derive(Default)]
pub(super) struct OperationalFamilyCache(Mutex<FamilyCache>);

#[derive(Default)]
struct FamilyCache {
    cursor: Option<(u64, i64)>,
    desired_cursor: Option<u64>,
    as_of: Option<u128>,
    incremental: Incremental,
    entries: BTreeMap<String, Vec<AttentionItemView>>,
    bootstraps: u64,
    recomputed: u64,
    deadline_recomputed: u64,
}

impl Store {
    pub(super) fn cached_operational_fault_families(&self, as_of: u128, desired_generation: u64) -> Result<Vec<AttentionItemView>> {
        use crate::incremental::change_keys;
        debug_assert!(self.readers.has_pinned_reader());
        let kept = self.extension::<OperationalFamilyCache>();
        let mut cache = kept.0.lock().unwrap_or_else(PoisonError::into_inner);
        // Historical readers do not move the live delivery cursor backwards.
        if as_of == u128::MAX || cache.as_of.is_some_and(|previous| as_of < previous) {
            return self.operational_fault_families(None, as_of);
        }
        let mut selected = BTreeSet::new();
        let (desired_cursor, desired_changes) = self.desired_changes_since(cache.desired_cursor);
        match desired_changes {
            Some(subjects) => {
                for subject in subjects {
                    cache.incremental.touch(&subject);
                    selected.insert(subject);
                }
            }
            None => {
                // Native projection coverage changed: rebuild once from the new cut, not on a timer.
                cache.cursor = None;
                cache.entries.clear();
                cache.incremental = Incremental::default();
            }
        }
        let cursor = cache.cursor;
        let feed = match cursor {
            Some((index, local)) => self.changes_since(index, local)?,
            None => self.changes_since(i64::MAX as u64, i64::MAX)?,
        };
        if cursor.is_none() {
            let connection = self.readers.get();
            let mut query = connection.prepare(
                "SELECT subject FROM desired
                 UNION SELECT 'mission-run/' || id FROM mission_runs
                 UNION SELECT subject FROM claims WHERE kind='reconcile.fault'",
            )?;
            selected.extend(query.query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<BTreeSet<_>>>()?);
            cache.bootstraps += 1;
        }
        for change in &feed.changes {
            for key in change_keys(change) {
                cache.incremental.touch(&key);
                // Owned-set changes can introduce declarations without an intent.desired claim.
                if change.kind == "owned-set.revised" && key.starts_with("desired:") {
                    selected.insert(key.trim_start_matches("desired:").to_owned());
                }
            }
            if matches!(change.kind.as_str(), "intent.desired" | "reconcile.fault" | "mission-run.created") {
                selected.insert(change.subject.clone());
            }
            if change.kind == "record.repaired" {
                let body: Value = serde_json::from_str(&change.body)?;
                let fields = body.get("fields").unwrap_or(&body);
                let connection = self.readers.get();
                let mut query = connection.prepare_cached(
                    "SELECT subject FROM claims WHERE id=?1
                     UNION SELECT claims.subject FROM replica_records
                     JOIN claims ON claims.id=replica_records.claim_id WHERE record_ref=?2",
                )?;
                let repaired = query.query_map(params![
                    fields["replacement"].as_str(), fields["record"].as_str(),
                ], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                for subject in repaired {
                    cache.incremental.touch(&subject);
                    selected.insert(subject);
                }
            }
        }
        selected.extend(cache.incremental.needed_items("", as_of));
        for subject in selected {
            let deadline = cache.incremental.due_of(&subject).is_some_and(|due| due <= as_of);
            let ((items, mut reads), due) = smallclaims::touched::record_due(|| {
                smallclaims::touched::record(|| {
                    if subject.starts_with("step-run/") {
                        // A deferred projection may not have materialized the step row yet.
                        // Resolve its owner from generation claims too: projection completion
                        // is published on the run, not on every step it materializes.
                        let connection = self.readers.get();
                        match run_of_subject_tx(&connection, &subject)? {
                            Some(run) => smallclaims::touched::note_read(|| format!("mission-run/{run}")),
                            None => {
                                smallclaims::touched::note_read(|| "kind:mission-run.created".into());
                                smallclaims::touched::note_read(|| "kind:work.person-asked".into());
                            }
                        }
                    }
                    self.operational_fault_families_for(None, as_of, Some(&subject))
                })
            });
            let items = items?;
            reads.insert(subject.clone());
            cache.incremental.evaluated(&subject, reads, due);
            cache.entries.insert(subject, items);
            cache.recomputed += 1;
            cache.deadline_recomputed += u64::from(deadline);
        }
        // Advance only after every selected source succeeded.
        cache.cursor = Some((feed.index, feed.local));
        cache.desired_cursor = Some(desired_cursor.min(desired_generation));
        cache.as_of = Some(as_of);
        Ok(cache.entries.values().flatten().cloned().collect())
    }

    /// The next semantic eligibility boundary; never a cost-scaled polling interval.
    pub fn fault_delivery_next_due(&self) -> Option<u128> {
        let families = self.extension::<OperationalFamilyCache>();
        let families = families.0.lock().unwrap_or_else(PoisonError::into_inner);
        let failures = self.extension::<FaultCache>();
        let failures = failures.0.lock().unwrap_or_else(PoisonError::into_inner);
        families.incremental.next_due("").into_iter()
            .chain(failures.as_ref().and_then(|cache| cache.incremental.next_due("")))
            .min()
    }

    /// Deterministic cumulative source work, exported by the performance endpoint.
    pub fn fault_family_work(&self) -> Value {
        let cache = self.extension::<OperationalFamilyCache>();
        let cache = cache.0.lock().unwrap_or_else(PoisonError::into_inner);
        serde_json::json!({
            "bootstraps": cache.bootstraps,
            "subjects_recomputed": cache.recomputed,
            "deadline_subjects_recomputed": cache.deadline_recomputed,
            "retained_subjects": cache.entries.len(),
        })
    }
}

impl Store {
    /// The operational and broken-gate faults of the current failure episodes, from the cache
    /// where nothing they read has changed. Runs inside the snapshot's cut.
    pub(super) fn cached_failure_faults(
        &self,
        as_of: u128,
    ) -> Result<(Vec<OperationalFault>, Vec<AttentionItemView>)> {
        // Lock order: a caller takes its snapshot's reader before this lock, and every read
        // under it uses that pinned reader, so holding the lock never waits for the pool.
        debug_assert!(
            self.readers.has_pinned_reader(),
            "the fault cache reads only inside its snapshot's pinned cut"
        );
        let failures = self.operational_failures()?;
        let kept = self.extension::<FaultCache>();
        let mut kept = kept.0.lock().unwrap_or_else(PoisonError::into_inner);
        // A reading for every fault whenever raised (`u128::MAX`, as the client API asks), or for
        // an earlier time than the cache's answers, gets its own reading and leaves the cache.
        // Two callers whose times cross while one holds the lock cost the later one a full
        // reading; callers read at `now`, so that needs them within one snapshot of each other.
        if as_of == u128::MAX || kept.as_ref().is_some_and(|cache| as_of < cache.as_of) {
            return self.read_failure_faults(&failures, as_of);
        }
        let fresh = kept.is_none();
        let cache = kept.get_or_insert_with(|| Cache {
            incremental: Incremental::default(),
            entries: HashMap::new(),
            as_of,
            rebuilt: as_of,
            clean_since: as_of,
        });
        let every = if as_of.saturating_sub(cache.clean_since) >= QUIET_AFTER_MS {
            QUIET_REBUILD_MS
        } else {
            FULL_PASS_INTERVAL_MS
        };
        let full = fresh || as_of.saturating_sub(cache.rebuilt) >= every;
        cache.incremental.observe(self)?;
        cache.as_of = as_of;
        let connection = self.readers.get();
        let mut incarnations = BTreeMap::new();
        let mut corrections = 0;
        let (mut operational, mut gates) = (Vec::new(), Vec::new());
        let mut current = BTreeSet::new();
        for failure in failures.iter() {
            let item = format!("fault/{}", failure.request);
            current.insert(item.clone());
            let folded = serde_json::to_value(failure)?;
            let cached = cache.entries.get(&item).filter(|entry| {
                !entry.unkeyed && entry.failure == folded && !cache.incremental.needs(&item, as_of)
            });
            if let Some(entry) = cached.filter(|_| !full) {
                operational.extend(entry.faults.0.clone());
                gates.extend(entry.faults.1.clone());
                continue;
            }
            let skippable = cached.map(|entry| entry.faults.clone());
            #[cfg(any(test, feature = "test-support"))]
            FAULT_CACHE_READS.with(|reads| reads.set(reads.get() + 1));
            let ((faults, reads), due) = smallclaims::touched::record_due(|| {
                smallclaims::touched::record(|| {
                    self.read_failure(&connection, failure, as_of, &mut incarnations)
                })
            });
            let faults = faults?;
            if let Some(skipped) = skippable
                && !same_faults(&skipped, &faults)?
            {
                corrections += 1;
                crate::performance::record_correction("fault-snapshot");
                eprintln!(
                    "fractal: fault cache correction: {} changed with no change among {} reads",
                    failure.request,
                    reads.len()
                );
            }
            operational.extend(faults.0.clone());
            gates.extend(faults.1.clone());
            let unkeyed = reads.contains(UNKEYED_READ);
            cache.incremental.evaluated(&item, reads, due);
            cache.entries.insert(item, Entry { failure: folded, unkeyed, faults });
        }
        cache.entries.retain(|item, _| current.contains(item));
        cache.incremental.retain("fault/", &current);
        if full {
            cache.rebuilt = as_of;
            smallclaims::profile::note("fault cache rebuild");
            if corrections > 0 {
                smallclaims::profile::note("fault cache correction");
                cache.clean_since = as_of;
            }
        }
        drop(kept);
        #[cfg(any(test, feature = "test-support"))]
        if FAULT_CACHE_PARITY.with(std::cell::Cell::get) {
            let expected = self.read_failure_faults(&failures, as_of)?;
            assert_eq!(
                serde_json::to_value((&operational, &gates))?,
                serde_json::to_value(&expected)?,
                "the fault cache answered differently from a full reading at {as_of}",
            );
        }
        Ok((operational, gates))
    }

    /// Every failure's faults, read in full.
    fn read_failure_faults(
        &self,
        failures: &[AttentionRequestView],
        as_of: u128,
    ) -> Result<(Vec<OperationalFault>, Vec<AttentionItemView>)> {
        let connection = self.readers.get();
        let mut incarnations = BTreeMap::new();
        let (mut operational, mut gates) = (Vec::new(), Vec::new());
        for failure in failures {
            let (fault, gate) = self.read_failure(&connection, failure, as_of, &mut incarnations)?;
            operational.extend(fault);
            gates.extend(gate);
        }
        Ok((operational, gates))
    }

    fn read_failure(
        &self,
        connection: &Connection,
        failure: &AttentionRequestView,
        as_of: u128,
        incarnations: &mut BTreeMap<String, Option<String>>,
    ) -> Result<Faults> {
        let fault =
            self.operational_failure_item::<true>(connection, failure, None, as_of, incarnations)?;
        let gate = self
            .broken_gate_item(connection, failure.clone(), None, as_of)?
            .filter(|item| !item.person.starts_with("person/"));
        Ok((fault, gate))
    }

    /// How often this store's fault cache rebuilds in full now, for doctor and tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fault_cache_rebuild_every(&self, as_of: u128) -> Option<u128> {
        let kept = self.extension::<FaultCache>();
        let kept = kept.0.lock().unwrap_or_else(PoisonError::into_inner);
        kept.as_ref().map(|cache| {
            if as_of.saturating_sub(cache.clean_since) >= QUIET_AFTER_MS {
                QUIET_REBUILD_MS
            } else {
                FULL_PASS_INTERVAL_MS
            }
        })
    }
}

fn same_faults(left: &Faults, right: &Faults) -> Result<bool> {
    Ok(serde_json::to_value(left)? == serde_json::to_value(right)?)
}

#[cfg(test)]
mod family_tests {
    use super::*;

    fn append(store: &Store, subject: &str, kind: &str, fields: Value, at: u128) -> ClaimRecord {
        store.set_write_clock_at(at).unwrap();
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let claim = append_claim_tx(&tx, &store.origin, subject, kind, None,
            &serde_json::json!({"fields": fields}), &[], None).unwrap();
        tx.commit().unwrap();
        claim
    }

    fn fault(store: &Store, subject: &str, status: &str, at: u128) -> ClaimRecord {
        append(store, subject, "reconcile.fault", serde_json::json!({
            "scope": "runtime", "status": status, "reason": "Cannot reconcile",
        }), at)
    }

    fn read(store: &Store, at: u128) -> Vec<AttentionItemView> {
        let generation = store.desired_changes_since(None).0;
        store.read_snapshot_unindexed(|| person_work::with_cut_memo(|| {
            let actual = store.cached_operational_fault_families(at, generation)?;
            let expected = store.operational_fault_families(None, at)?;
            let normalize = |mut items: Vec<AttentionItemView>| {
                items.sort_by(|a, b| (&a.subject, &a.episode).cmp(&(&b.subject, &b.episode)));
                serde_json::to_value(items).unwrap()
            };
            assert_eq!(normalize(actual.clone()), normalize(expected), "family parity at {at}");
            Ok::<_, anyhow::Error>(actual)
        })).unwrap()
    }

    fn recomputed(store: &Store) -> u64 {
        store.fault_family_work()["subjects_recomputed"].as_u64().unwrap()
    }

    #[test]
    fn fault_delivery_families_ignore_unrelated_commits_and_recompute_only_changed_source() {
        let store = Store::open_memory("node").unwrap();
        fault(&store, "daemon/first", "faulted", 1_000);
        fault(&store, "daemon/second", "faulted", 2_000);
        assert_eq!(read(&store, 200_000).len(), 2);
        let before = store.fault_family_work();
        append(&store, "custom/fault-tests/unrelated", "custom.recorded",
            serde_json::json!({"value": 1}), 3_000);
        assert_eq!(read(&store, 2_000_000).len(), 2);
        assert_eq!(store.fault_family_work(), before,
            "an unrelated commit and elapsed former rebuild interval do no source work");
        fault(&store, "daemon/first", "healthy", 4_000);
        let remaining = read(&store, 2_000_001);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].subject, "daemon/second");
        assert_eq!(recomputed(&store), before["subjects_recomputed"].as_u64().unwrap() + 1);
        assert_eq!(store.fault_family_work()["bootstraps"], before["bootstraps"]);
    }

    #[test]
    fn fault_delivery_families_select_only_the_subject_whose_eligibility_deadline_arrives() {
        let store = Store::open_memory("node").unwrap();
        let first = fault(&store, "daemon/first", "faulted", 1_000);
        let second = fault(&store, "daemon/second", "faulted", 5_000);
        let first_due = first.accepted_at_unix_ms + 120_000;
        let second_due = second.accepted_at_unix_ms + 120_000;
        assert!(first_due < second_due);
        assert!(read(&store, second.accepted_at_unix_ms).is_empty());
        let before = recomputed(&store);
        assert_eq!(store.fault_delivery_next_due(), Some(first_due));
        assert!(read(&store, first_due - 1).is_empty());
        assert_eq!(recomputed(&store), before);
        let due = read(&store, first_due);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].subject, "daemon/first");
        assert_eq!(recomputed(&store), before + 1);
        assert_eq!(store.fault_delivery_next_due(), Some(second_due));
        assert_eq!(read(&store, second_due).len(), 2);
        assert_eq!(recomputed(&store), before + 2);
        assert_eq!(store.fault_family_work()["deadline_subjects_recomputed"], 2);
    }

    #[test]
    fn fault_delivery_revisits_unmaterialized_step_when_owning_run_projection_finishes() {
        let store = Store::open_memory("node").unwrap();
        crate::store::tests_shared_missions::publish_carry(&store, r#"version 2
mission "fault-projection" state="ready" {
  goal "Keep projection-dependent faults current."
  step "work" { agentless }
}"#, "declare-fault-projection");
        let run = store.create_mission_run_inner(&crate::model::MissionRunRequest {
            mission: "fault-projection".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: Default::default(),
            idempotency_key: "fault-projection-run".into(),
        }, None, None, None).unwrap();
        let step = run.steps.iter().find(|step| step.step == "work").unwrap();
        fault(&store, &step.subject, "faulted", 1_000);
        fault(&store, "daemon/unrelated", "faulted", 2_000);
        {
            let writer = store.connection.write();
            // Hold the exact projected row aside to exercise the admitted-but-not-yet-
            // materialized state without adding any step claim when it later appears.
            writer.execute("CREATE TEMP TABLE deferred_fault_steps AS SELECT * FROM step_runs WHERE subject=?1",
                [&step.subject]).unwrap();
            writer.execute("DELETE FROM step_runs WHERE subject=?1", [&step.subject]).unwrap();
        }
        let at = u128::MAX - 1;
        let initial = read(&store, at);
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].subject, "daemon/unrelated");
        assert_eq!(store.fault_delivery_next_due(), None, "the missing-step answer has no timer");
        let before = store.fault_family_work();
        {
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            tx.execute("INSERT INTO step_runs SELECT * FROM deferred_fault_steps", []).unwrap();
            note_run_projection_finished_tx(&tx,
                &BTreeSet::from([run.subject.trim_start_matches("mission-run/").to_owned()])).unwrap();
            tx.commit().unwrap();
        }
        let completed = read(&store, at);
        assert!(completed.iter().any(|item| item.subject == step.subject));
        assert_eq!(completed.len(), 2);
        assert_eq!(store.fault_family_work()["bootstraps"], before["bootstraps"]);
        assert_eq!(recomputed(&store), before["subjects_recomputed"].as_u64().unwrap() + 2,
            "only the completed run and its dependent fault source are reevaluated");
    }
}
