//! The fault snapshot's failure episodes, kept between snapshots. A host holds thousands of
//! operational failures, nearly all of them on runs that ended long ago, and a snapshot every 30
//! seconds read them all again. Each failure now remembers what it read and is read again only
//! when the change feed names one of those keys, when its time comes, or when it rests on a read
//! the feed cannot name (`UNKEYED_READ`). A full rebuild on a timer compares every answer with
//! the cache; a difference is a correction, counted in `doctor --performance`, and means a read
//! is missing its key.
use super::attention_snapshot::UNKEYED_READ;
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
type Faults = (Option<AttentionItemView>, Option<AttentionItemView>);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Failures read again (not answered from the cache) on this thread.
    pub static FAULT_CACHE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Off for a test that counts what the cache alone costs.
    pub static FAULT_CACHE_PARITY: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

impl Store {
    /// The operational and broken-gate faults of the current failure episodes, from the cache
    /// where nothing they read has changed. Runs inside the snapshot's cut.
    pub(super) fn cached_failure_faults(
        &self,
        as_of: u128,
    ) -> Result<(Vec<AttentionItemView>, Vec<AttentionItemView>)> {
        let failures = self.operational_failures()?;
        let kept = self.extension::<FaultCache>();
        let mut kept = kept.0.lock().unwrap_or_else(PoisonError::into_inner);
        // An earlier time than the cache's answers gets its own reading and leaves the cache.
        if kept.as_ref().is_some_and(|cache| as_of < cache.as_of) {
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
                    "st3: fault cache correction: {} changed with no change among {} reads",
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
                serde_json::to_value(&(&operational, &gates))?,
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
    ) -> Result<(Vec<AttentionItemView>, Vec<AttentionItemView>)> {
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
