//! The part of `store::agent_view` that moved up from fractal-seats.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use serde::Deserialize;
use smallclaims::ivm::{Contribution, Definition, View};
pub use fractal_seats::store::agent_view::*;
pub use fractal_seats::store::agent_view::{AgentView, Event, Field, Machine, Run, STATES, Status, Summary, VIEW, definitions, diagnostic, observed};

/// Called only inside the authorized caller's SQLite snapshot. A row has no authority
/// of its own, and an uncertified source never invokes a canonical read fallback.
/// This returns observed harness state only. The work.claimed/work.progress overlay
/// used by current_harness is a separate dependency that must be covered before hookup.
pub fn read_harness(
    connection: &Connection,
    views: &smallclaims::ivm::Views,
    subject: &str,
) -> Result<Option<crate::model::CurrentHarnessView>> {
    let cut = smallclaims::ivm::source_cut(connection)?.context("agent source unavailable")?;
    views.token(connection, VIEW, cut.epoch)?;
    let row: Option<String> = connection
        .query_row(
            "SELECT observed FROM local_agent_harness_rows WHERE subject=?1",
            [subject],
            |r| r.get(0),
        )
        .optional()?;
    row.map(|row| serde_json::from_str(&row))
        .transpose()
        .map(Option::flatten)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{exchange_from, exchange_of, receive_and_project};
    use smallclaims::ivm::{Readiness, SourceCut, Views};
    use fractal_seats::store::agent_view_tests_shared_attention_tests::{SEAT};

    struct Fixture {
        pub store: Store,
        views: Views,
    }

    impl Fixture {
        fn new() -> Self {
            Self::initialize(Store::open_memory("grove").unwrap())
        }
        /// A fixture on a file store; reopening the same path keeps the persisted outputs.
        fn open(path: &std::path::Path) -> Self {
            let fresh = !path.exists();
            let store = Store::open(path, "grove").unwrap();
            if fresh {
                return Self::initialize(store);
            }
            Self { store, views: Views::new(definitions()).unwrap() }
        }
        fn initialize(store: Store) -> Self {
            let views = Views::new(definitions()).unwrap();
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            views.create_schema(&tx).unwrap();
            views
                .initialize_empty(
                    &tx,
                    SourceCut {
                        epoch: 1,
                        admitted: 0,
                        projected: 0,
                        local_generation: 0,
                    },
                )
                .unwrap();
            tx.commit().unwrap();
            drop(connection);
            Self { store, views }
        }
        fn input(subject: &str, kind: &str, fields: Value) -> ClaimInput {
            ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: None,
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            }
        }
        // Admit raw canonical fixtures in a real Store transaction. The public latest-
        // retention path stamps/collapses observations, so it cannot exercise old sparse
        // and mixed histories. Runtime mutation and view maintenance commit together.
        fn append(&self, subject: &str, kind: &str, fields: Value) -> ClaimRecord {
            let input = Self::input(subject, kind, fields);
            validate_claim_input(&input).unwrap();
            let mut connection = self.store.connection.write();
            let tx = connection.transaction().unwrap();
            let claim = append_claim_tx(
                &tx,
                &self.store.origin,
                subject,
                kind,
                None,
                &json!({"fields":input.fields}),
                &[],
                None,
            )
            .unwrap();
            self.capture_tx(&tx, &claim);
            tx.commit().unwrap();
            drop(connection);
            self.check(subject);
            claim
        }
        pub fn capture(&self, claim: &ClaimRecord) {
            let mut connection = self.store.connection.write();
            let tx = connection.transaction().unwrap();
            self.capture_tx(&tx, claim);
            tx.commit().unwrap();
        }
        fn apply_tx(&self, tx: &Transaction<'_>, claim: &ClaimRecord) {
            let key = canonical::claim_key(tx, &claim.id).unwrap();
            let changes = self.views.change(tx, None, Some((claim, &key)), 1).unwrap();
            assert!(
                changes.deferred.is_empty(),
                "{:?}",
                self.views.availability(tx, VIEW, 1)
            );
        }
        fn publish_tx(&self, tx: &Transaction<'_>) {
            let index = current_index(tx).unwrap();
            self.views
                .publish_cut(
                    tx,
                    SourceCut {
                        epoch: 1,
                        admitted: index,
                        projected: index,
                        local_generation: 0,
                    },
                )
                .unwrap();
        }
        fn capture_tx(&self, tx: &Transaction<'_>, claim: &ClaimRecord) {
            self.apply_tx(tx, claim);
            self.publish_tx(tx);
        }
        fn capture_many(&self, claims: &[ClaimRecord]) {
            if claims.is_empty() {
                return;
            }
            let mut connection = self.store.connection.write();
            let tx = connection.transaction().unwrap();
            for claim in claims {
                self.apply_tx(&tx, claim);
            }
            // Only the complete captured transaction prefix may publish readiness.
            self.publish_tx(&tx);
            tx.commit().unwrap();
        }
        fn check(&self, subject: &str) {
            let connection = self.store.readers.get();
            let index = current_index(&connection).unwrap();
            let mut oracle =
                current_harness_fold_at(&connection, subject, Some(index), false, false).unwrap();
            if let Some(view) = oracle.as_mut() {
                seat_status::enrich_harness(&connection, subject, Some(index), view).unwrap();
            }
            let keyed = read_harness(&connection, &self.views, subject).unwrap();
            assert_eq!(
                serde_json::to_value(&keyed).unwrap(),
                serde_json::to_value(&oracle).unwrap(),
                "oracle mismatch {subject}"
            );
        }
    }

    #[test]
    fn ordered_agent_background_jobs_match_canonical_for_current_incarnation_only() {
        let f = Fixture::new();
        let jobs = || read_harness(&f.store.readers.get(), &f.views, SEAT)
            .unwrap().and_then(|view| view.background_jobs);
        f.append(SEAT, "runtime.observed", json!({"status":"running","incarnation_id":"one"}));
        assert_eq!(jobs(), None);
        for count in [2, 0] {
            f.append(SEAT, "harness.observed", json!({"state":"idle","incarnation_id":"one","background_jobs":count}));
            f.check(SEAT);
            assert_eq!(jobs(), Some(count));
        }
        f.append(SEAT, "harness.observed", json!({"state":"idle","incarnation_id":"one"}));
        f.check(SEAT);
        assert_eq!(jobs(), None, "a sparse report does not inherit the old count");
        f.append(SEAT, "harness.observed", json!({"state":"working","incarnation_id":"one","background_jobs":3}));
        f.append(SEAT, "runtime.observed", json!({"status":"running","incarnation_id":"two"}));
        f.check(SEAT);
        assert_eq!(jobs(), None);
        f.append(SEAT, "harness.observed", json!({"state":"idle","incarnation_id":"two","background_jobs":1}));
        f.check(SEAT);
        assert_eq!(jobs(), Some(1));
        f.append(SEAT, "harness.observed", json!({"state":"ended","incarnation_id":"two","background_jobs":1}));
        f.check(SEAT);
        assert_eq!(jobs(), None);
    }

    /// Deliver `envelopes` one at a time, capturing what each admitted, checking the oracle.
    fn deliver_one_by_one(f: &Fixture, envelopes: &[ReplicaEnvelope]) {
        for envelope in envelopes {
            let before = f.store.index().unwrap();
            receive_and_project(&f.store, "relay", &exchange_of("relay", vec![envelope.clone()]));
            let claims = {
                let connection = f.store.readers.get();
                connection.prepare(&format!("SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id WHERE claims.store_index>?1 ORDER BY claims.store_index")).unwrap()
                    .query_map([before], claim_from_row).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
            };
            f.capture_many(&claims);
            f.check(SEAT);
        }
    }

    #[test]
    fn agent_view_survives_reopen_and_redelivery_after_permuted_arrival() {
        let alder = Store::open_memory("alder").unwrap();
        let birch = Store::open_memory("birch").unwrap();
        for source in [&alder, &birch] {
            source.set_write_clock_at(1_800_000_000_000).unwrap();
        }
        alder.append_claim(&Fixture::input(SEAT, "runtime.observed",
            json!({"status":"running","incarnation_id":"one"}))).unwrap();
        alder.append_claim(&Fixture::input(SEAT, "harness.observed",
            json!({"state":"working","incarnation_id":"one","driver":"codex","reason":"first"}))).unwrap();
        birch.append_claim(&Fixture::input(SEAT, "harness.observed",
            json!({"state":"idle","incarnation_id":"one","transport":"native","reason":null}))).unwrap();
        let mut envelopes = exchange_from(&alder, &ReplicationInventory::default()).envelopes;
        envelopes.extend(exchange_from(&birch, &ReplicationInventory::default()).envelopes);
        envelopes.reverse();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("view.sqlite3");
        let before = {
            let f = Fixture::open(&path);
            deliver_one_by_one(&f, &envelopes);
            read_harness(&f.store.readers.get(), &f.views, SEAT).unwrap()
        };
        assert!(before.is_some(), "the view holds the seat before the restart");
        // Reopened, the persisted output reads as before and matches the oracle; repeated
        // deliveries then admit nothing, so only the persisted row can answer.
        let f = Fixture::open(&path);
        assert_eq!(
            serde_json::to_value(read_harness(&f.store.readers.get(), &f.views, SEAT).unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
        f.check(SEAT);
        deliver_one_by_one(&f, &envelopes);
    }

    #[test]
    fn ordered_agent_sparse_fields_legacy_since_and_incarnation_reset_match_canonical() {
        let f = Fixture::new();
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"one"}),
        );
        f.append(SEAT,"harness.observed",json!({"state":"idle","driver":"codex","incarnation_id":"one","reason":"paused","input_buffer":"draft"}));
        for _ in 0..20 {
            f.append(
                SEAT,
                "harness.observed",
                json!({"state":"idle","incarnation_id":"one"}),
            );
        }
        f.append(SEAT,"harness.observed",json!({"state":"working","incarnation_id":"one","blocked_on":"human","ask":"permission"}));
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"working","incarnation_id":"one"}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"ended","incarnation_id":"one"}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"working","incarnation_id":"one","ask":null,"reason":null}),
        );
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"two"}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"ready","incarnation_id":"one"}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"working","incarnation_id":"two","driver":"claude"}),
        );
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"stopped","incarnation_id":"two"}),
        );
    }

    #[test]
    fn ordered_agent_native_fences_and_mixed_stamps_match_canonical() {
        let f = Fixture::new();
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"one"}),
        );
        f.append(SEAT,"harness.observed",json!({"state":"working","driver":"codex","incarnation_id":"one","status_transition":true,"observed_since_ms":1,"observed_at_ms":2}));
        for auth in [false, true, false, true] {
            f.append(SEAT,"harness.observed",json!({"state":"working","driver":"codex","incarnation_id":"one","provider_auth":auth}));
            f.append(
                SEAT,
                "harness.observed",
                json!({"state":"working","incarnation_id":"one","status_transition":false}),
            );
        }
        for code in [
            "provider-auth-expired",
            "provider-auth-restored",
            "provider-trust-prompt",
            "provider-auth-restored",
            "provider-update-prompt",
            "provider-update-restored",
            "claude-channel-unattached",
            "claude-channel-attached",
        ] {
            f.append(
                SEAT,
                "harness.diagnostic",
                json!({"code":code,"incarnation_id":"one","driver":"claude","reason":"screen"}),
            );
            f.append(
                SEAT,
                "harness.observed",
                json!({"state":"idle","incarnation_id":"one","driver":"claude"}),
            );
            f.append(SEAT,"harness.observed",json!({"state":"idle","incarnation_id":"one","status_transition":false,"observed_since_ms":3}));
        }
        f.append(
            SEAT,
            "harness.diagnostic",
            json!({"code":"harness-admission-failed","incarnation_id":"one","reason":"probe"}),
        );
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"stopped","incarnation_id":"one"}),
        );
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"two"}),
        );
    }

    #[test]
    fn ordered_agent_unnamed_fields_merge_in_canonical_epoch() {
        let f = Fixture::new();
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"idle","reason":"old"}),
        );
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"one"}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"working","driver":"codex","reason":"unnamed"}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"idle","incarnation_id":"one","transport":"native"}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"working","reason":null,"incarnation_id":null}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"ended","incarnation_id":"one"}),
        );
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"two"}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"working","incarnation_id":"one","reason":"old-incarnation"}),
        );
    }

    #[test]
    fn ordered_agent_unrelated_and_duplicate_inputs_change_no_public_row() {
        let f = Fixture::new();
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"one"}),
        );
        let claim = f.append(
            SEAT,
            "harness.observed",
            json!({"state":"idle","incarnation_id":"one"}),
        );
        let connection = f.store.readers.get();
        let generation: u64 = connection
            .query_row(
                "SELECT generation FROM ivm_views WHERE name=?1",
                [VIEW],
                |r| r.get(0),
            )
            .unwrap();
        drop(connection);
        f.capture(&claim);
        f.append(
            "daemon/grove",
            "daemon.diagnostic",
            json!({"code":"test","reason":"unrelated","severity":"warning"}),
        );
        let connection = f.store.readers.get();
        let next: u64 = connection
            .query_row(
                "SELECT generation FROM ivm_views WHERE name=?1",
                [VIEW],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(next, generation);
        assert!(matches!(
            f.views.readiness(&connection, VIEW, 1).unwrap(),
            Readiness::Ready(_)
        ));
        f.check(SEAT);
    }

    #[test]
    fn ordered_agent_ties_permuted_replication_and_duplicate_arrival_match_canonical() {
        let alder = Store::open_memory("alder").unwrap();
        let birch = Store::open_memory("birch").unwrap();
        for source in [&alder, &birch] {
            source.set_write_clock_at(1_800_000_000_000).unwrap();
        }
        alder
            .append_claim(&Fixture::input(
                SEAT,
                "runtime.observed",
                json!({"status":"running","incarnation_id":"one"}),
            ))
            .unwrap();
        alder
            .append_claim(&Fixture::input(
                SEAT,
                "harness.observed",
                json!({"state":"working","incarnation_id":"one","driver":"codex","reason":"first"}),
            ))
            .unwrap();
        birch
            .append_claim(&Fixture::input(
                SEAT,
                "harness.observed",
                json!({"state":"idle","incarnation_id":"one","transport":"native","reason":null}),
            ))
            .unwrap();
        birch
            .append_claim(&Fixture::input(
                SEAT,
                "harness.diagnostic",
                json!({"code":"provider-auth-expired","incarnation_id":"one","driver":"codex"}),
            ))
            .unwrap();
        let mut envelopes = exchange_from(&alder, &ReplicationInventory::default()).envelopes;
        envelopes.extend(exchange_from(&birch, &ReplicationInventory::default()).envelopes);
        let forward = Fixture::new();
        let reverse = Fixture::new();
        for (f, order) in [
            (&forward, envelopes.clone()),
            (&reverse, envelopes.iter().rev().cloned().collect()),
        ] {
            for envelope in order.iter().chain(order.iter()) {
                let before = f.store.index().unwrap();
                receive_and_project(
                    &f.store,
                    "relay",
                    &exchange_of("relay", vec![envelope.clone()]),
                );
                let claims = {
                    let connection = f.store.readers.get();
                    connection.prepare(&format!("SELECT {CLAIM_COLUMNS} FROM claims JOIN batches ON batches.id=claims.batch_id WHERE claims.store_index>?1 ORDER BY claims.store_index")).unwrap()
                        .query_map([before],claim_from_row).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
                };
                f.capture_many(&claims);
                f.check(SEAT);
            }
        }
        let left = read_harness(&forward.store.readers.get(), &forward.views, SEAT).unwrap();
        let right = read_harness(&reverse.store.readers.get(), &reverse.views, SEAT).unwrap();
        assert_eq!(
            serde_json::to_value(left).unwrap(),
            serde_json::to_value(right).unwrap()
        );
    }

    #[test]
    fn ordered_agent_rollback_preserves_rows_and_frontiers() {
        let f = Fixture::new();
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"one"}),
        );
        f.append(
            SEAT,
            "harness.observed",
            json!({"state":"idle","incarnation_id":"one"}),
        );
        let before = read_harness(&f.store.readers.get(), &f.views, SEAT).unwrap();
        let index = f.store.index().unwrap();
        {
            let mut connection = f.store.connection.write();
            let tx = connection.transaction().unwrap();
            let claim = append_claim_tx(
                &tx,
                "grove",
                SEAT,
                "harness.observed",
                None,
                &json!({"fields":{"state":"working","incarnation_id":"one"}}),
                &[],
                None,
            )
            .unwrap();
            let key = canonical::claim_key(&tx, &claim.id).unwrap();
            f.views.change(&tx, None, Some((&claim, &key)), 1).unwrap();
            let next = current_index(&tx).unwrap();
            f.views
                .publish_cut(
                    &tx,
                    SourceCut {
                        epoch: 1,
                        admitted: next,
                        projected: next,
                        local_generation: 0,
                    },
                )
                .unwrap();
            assert_eq!(
                read_harness(&tx, &f.views, SEAT).unwrap().unwrap().state,
                "working"
            );
            tx.rollback().unwrap();
        }
        assert_eq!(f.store.index().unwrap(), index);
        assert_eq!(
            serde_json::to_value(read_harness(&f.store.readers.get(), &f.views, SEAT).unwrap())
                .unwrap(),
            serde_json::to_value(before).unwrap()
        );
        f.check(SEAT);
    }

    #[test]
    fn ordered_agent_source_pending_and_canonical_mutations_fence_keyed_reads() {
        let f = Fixture::new();
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"one"}),
        );
        let claim = f
            .store
            .append_claim(&Fixture::input(
                SEAT,
                "harness.observed",
                json!({"state":"idle","incarnation_id":"one"}),
            ))
            .unwrap();
        assert!(read_harness(&f.store.readers.get(), &f.views, SEAT).is_err());
        f.capture(&claim);
        f.check(SEAT);
        {
            let connection = f.store.connection.write();
            connection
                .execute(
                    "UPDATE claims SET body=json_set(body,'$.fields.reason','edited') WHERE id=?1",
                    [claim.id],
                )
                .unwrap();
        }
        assert!(matches!(
            f.views.readiness(&f.store.readers.get(), VIEW, 1).unwrap(),
            Readiness::Fenced
        ));
        assert!(read_harness(&f.store.readers.get(), &f.views, SEAT).is_err());
    }

    #[test]
    fn ordered_agent_keyed_read_cost_does_not_grow_with_sparse_history() {
        let f = Fixture::new();
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"one"}),
        );
        let mut costs = Vec::new();
        for size in [32, 256] {
            for _ in costs.len() * 32..size {
                let mut connection = f.store.connection.write();
                let tx = connection.transaction().unwrap();
                let claim = append_claim_tx(
                    &tx,
                    "grove",
                    SEAT,
                    "harness.observed",
                    None,
                    &json!({"fields":{"state":"working","incarnation_id":"one"}}),
                    &[],
                    None,
                )
                .unwrap();
                let before = smallclaims::sqlite::work::total();
                f.capture_tx(&tx, &claim);
                let maintenance = smallclaims::sqlite::work::total() - before;
                if claim.store_index == size as u64 {
                    eprintln!(
                        "ordered-agent maintenance observations={size} statements={} vm_steps={}",
                        maintenance.statements, maintenance.vm_steps
                    );
                }
                tx.commit().unwrap();
            }
            f.check(SEAT);
            let connection = f.store.readers.get();
            // Count this connection's VM instructions. Process-wide work totals include
            // other parallel controls and cannot establish a growth bound.
            let instructions = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let counter = instructions.clone();
            connection.progress_handler(
                1,
                Some(move || {
                    counter.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            );
            let before = STATEMENTS_RUN.with(std::cell::Cell::get);
            let row = read_harness(&connection, &f.views, SEAT);
            let statements = STATEMENTS_RUN.with(std::cell::Cell::get) - before;
            connection.progress_handler(0, None::<fn() -> bool>);
            let row = row.unwrap().unwrap();
            let vm_steps = instructions.load(Ordering::Relaxed);
            assert_eq!(row.state, "working");
            costs.push(vm_steps);
            eprintln!(
                "ordered-agent keyed-read observations={size} statements={statements} vm_steps={vm_steps}"
            );
        }
        assert!(
            costs[1] == costs[0],
            "history-dependent keyed read: {costs:?}"
        );
    }

    #[test]
    fn ordered_agent_retained_original_repair_and_deleted_claim_are_explicit() {
        let f = Fixture::new();
        f.append(
            SEAT,
            "runtime.observed",
            json!({"status":"running","incarnation_id":"one"}),
        );
        let original = f.append(
            SEAT,
            "harness.observed",
            json!({"state":"idle","incarnation_id":"one","driver":"codex","reason":"old"}),
        );
        let replacement = f.append(
            SEAT,
            "harness.observed",
            json!({"state":"working","incarnation_id":"one","reason":null}),
        );
        {
            let mut connection = f.store.connection.write();
            let tx = connection.transaction().unwrap();
            let key = canonical::claim_key(&tx, &replacement.id).unwrap();
            let changes = f
                .views
                .repair(&tx, &original, (&replacement, &key), 1)
                .unwrap();
            assert!(changes.deferred.is_empty());
            assert!(changes.changed.is_empty());
            let retained: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM ivm_contributions WHERE view=?1 AND claim_id=?2)",
                    params![VIEW, original.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(
                retained,
                "harness repair must retain the canonical original"
            );
            tx.commit().unwrap();
        }
        f.check(SEAT);
        f.store
            .connection
            .write()
            .execute("DELETE FROM claims WHERE id=?1", [replacement.id])
            .unwrap();
        assert!(
            read_harness(&f.store.readers.get(), &f.views, SEAT).is_err(),
            "delete requires explicit reinstallation; never serve an old row"
        );
    }

    #[test]
    fn ordered_agent_bare_legacy_fields_and_nontext_incarnation_match_canonical() {
        let f = Fixture::new();
        for (kind, body) in [
            (
                "runtime.observed",
                json!({"status":"running","incarnation_id":"one"}),
            ),
            (
                "harness.observed",
                json!({"state":"idle","incarnation_id":"one","driver":"codex","reason":"legacy"}),
            ),
            (
                "harness.observed",
                json!({"state":"working","incarnation_id":17,"transport":"native","reason":null}),
            ),
            (
                "harness.observed",
                json!({"state":"ended","incarnation_id":"one"}),
            ),
            (
                "harness.observed",
                json!({"incarnation_id":"one","blocked_on":"human","ask":"permission"}),
            ),
            (
                "harness.observed",
                json!({"state":"working","incarnation_id":"one","status_transition":false}),
            ),
            (
                "harness.observed",
                json!({"state":"idle","incarnation_id":"one"}),
            ),
            (
                "harness.observed",
                json!({"state":"blocked","incarnation_id":"one","blocked_on":"human"}),
            ),
        ] {
            let mut connection = f.store.connection.write();
            let tx = connection.transaction().unwrap();
            let claim = smallclaims::store::append_claim_record_tx(
                &tx,
                "grove",
                SEAT,
                kind,
                None,
                &body,
                &[],
                None,
            )
            .unwrap();
            f.capture_tx(&tx, &claim);
            tx.commit().unwrap();
            drop(connection);
            f.check(SEAT);
        }
    }

    #[test]
    fn ordered_agent_unsupported_states_and_null_admission_reason_fence_with_evidence() {
        for (kind, fields, expected) in [
            (
                "harness.observed",
                json!({"state":"","incarnation_id":"one"}),
                "unsupported legacy harness state",
            ),
            (
                "harness.observed",
                json!({"state":"legacy-unknown","incarnation_id":"one"}),
                "unsupported legacy harness state",
            ),
            (
                "harness.observed",
                json!({"state":"","incarnation_id":"one","status_transition":false}),
                "unsupported legacy harness state",
            ),
            (
                "harness.diagnostic",
                json!({"code":"harness-admission-failed","incarnation_id":"one","reason":null}),
                "non-text harness admission reason",
            ),
        ] {
            let f = Fixture::new();
            f.append(
                SEAT,
                "runtime.observed",
                json!({"status":"running","incarnation_id":"one"}),
            );
            f.append(
                SEAT,
                "harness.observed",
                json!({"state":"idle","incarnation_id":"one"}),
            );
            let mut connection = f.store.connection.write();
            let tx = connection.transaction().unwrap();
            let claim = smallclaims::store::append_claim_record_tx(
                &tx,
                "grove",
                SEAT,
                kind,
                None,
                &json!({"fields":fields}),
                &[],
                None,
            )
            .unwrap();
            let key = canonical::claim_key(&tx, &claim.id).unwrap();
            let changes = f.views.change(&tx, None, Some((&claim, &key)), 1).unwrap();
            assert!(changes.deferred.contains(VIEW));
            assert!(changes.changed.is_empty());
            f.publish_tx(&tx);
            tx.commit().unwrap();
            drop(connection);
            let connection = f.store.readers.get();
            let status = f.views.availability(&connection, VIEW, 1).unwrap();
            assert!(matches!(status.readiness, Readiness::Fenced));
            assert!(status.error.unwrap().contains(expected));
            assert!(read_harness(&connection, &f.views, SEAT).is_err());
            if kind == "harness.diagnostic" {
                assert!(current_harness_fold_at(&connection, SEAT, None, false, false).is_err());
            }
        }
    }
}
