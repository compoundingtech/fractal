//! The part of `store::mailbox_lease` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::mailbox::{Authority, Fence};
use crate::model::{LaunchSpec, MemberKind, MemberSpec};
pub use st3_attention::store::mailbox_lease::*;
pub use st3_attention::store::mailbox_lease::{admit, check_argv_declaration, check_declaration, check_lease_fence, record, record_argv_binding, refuse_argv_native_adoption};

#[cfg(test)]
mod tests {
    use super::*;
    use st3_attention::store::mailbox_lease_shared_seats::*;
    use st3_attention::store::mailbox_lease_shared_seats::{AFTER_FAULT_ACQUISITION, AFTER_PROMOTION_PRECHECK, BEFORE_FAULT_READER, CARD_FAULT_DEADLINES, DURING_FAULT_READ};
    use st3_attention::store::mailbox_lease_tests_shared_seats::*;
    use st3_attention::store::mailbox_lease_tests_shared_seats::{authority, change_after_precheck, finished_without_the_writer, precheck_was_reached, promote_elsewhere, request};

    fn declare(store: &Store) {
        let intent = crate::graph::parse_intent("version 2\nagent \"eval.worker\" { host \"node\"; workspace \"/tmp\"; harness \"omp\" {} }", "node").unwrap();
        store.apply_internal(&intent, "lease-fixture").unwrap();
    }

    fn fixture() -> Store {
        let store = Store::open_memory("node").unwrap();
        declare(&store);
        crate::store::mailbox_tests::ready(&store, "current");
        store
    }

    fn fault_capture_stall(phase: &str) {
        let store = fixture();
        let subjects = vec![String::from("agent/eval.worker")];
        store.append_claim(&ClaimInput {
            subject: subjects[0].clone(), kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: serde_json::from_value(json!({"condition":"mailbox-channel-lost",
                "incarnation":"current", "reason":"fixture loss", "reviewer":subjects[0],
                "title":"Mailbox lost", "severity":"error", "targets":subjects})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let cut = store.index().unwrap();
        let expected = BTreeMap::from([(subjects[0].clone(), "fixture loss".into())]);
        let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = reached.clone();
        let pause = move || {
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(60));
        };
        match phase {
            "before" => BEFORE_FAULT_READER.with(|hook| *hook.borrow_mut() = Some(Box::new(pause))),
            "during" => DURING_FAULT_READ.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |connection| {
                    let _: i64 = connection.query_row("SELECT 1", [], |row| row.get(0))?;
                    pause();
                    Ok(())
                }));
            }),
            "after" => AFTER_FAULT_ACQUISITION.with(|hook| *hook.borrow_mut() = Some(Box::new(pause))),
            _ => unreachable!(),
        }
        let parent = smallclaims::read_budget::ReadBudget::new("/fault-stall", std::time::Duration::from_secs(15));
        let actual = smallclaims::read_budget::with(Some(parent), ||
            store.read_snapshot(|index| store.mailbox_faults_for(&subjects, index)));
        println!("fault stall phase={phase} injected={}", reached.load(std::sync::atomic::Ordering::SeqCst));
        assert!(reached.load(std::sync::atomic::Ordering::SeqCst), "the control must inject its stall");
        assert_eq!(actual.unwrap(), expected, "a {phase}-work wait is not reader CPU work");
        assert_eq!(store.index().unwrap(), cut, "a stalled read cannot write or repair its cut");
    }

    #[test]
    fn fault_capture_wait_before_reader_does_not_spend_cpu_budget() {
        fault_capture_stall("before");
    }

    #[test]
    fn fault_capture_wait_during_reader_does_not_spend_cpu_budget() {
        fault_capture_stall("during");
    }

    #[test]
    fn fault_capture_wait_after_reader_does_not_spend_cpu_budget() {
        fault_capture_stall("after");
    }

    #[test]
    fn fault_capture_real_sql_overrun_still_refuses_with_live_parent() {
        let store = fixture();
        let subjects = vec!["agent/eval.worker".into()];
        let cut = store.index().unwrap();
        DURING_FAULT_READ.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|connection| {
                let _: i64 = connection.query_row(
                    "WITH RECURSIVE work(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM work WHERE n<1000000)
                     SELECT sum(n) FROM work", [], |row| row.get(0),
                )?;
                Ok(())
            }));
        });
        let parent = smallclaims::read_budget::ReadBudget::new("/fault-work", std::time::Duration::from_secs(15));
        let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        let error = smallclaims::read_budget::with(Some(parent.clone()), ||
            store.read_snapshot(|index| store.mailbox_faults_for(&subjects, index))).unwrap_err();
        let work = scope.finish();
        assert_eq!(error.downcast_ref::<St3Error>().unwrap().code, "read-deadline");
        assert!(parent.check().is_ok(), "the local work limit must refuse before its parent");
        assert!(work.vm_steps > 1000, "the control must execute real SQLite work");
        assert_eq!(store.index().unwrap(), cut, "a refused read cannot return a repaired cut");
    }

    #[test]
    fn fault_capture_stalls_still_observe_parent_cancellation_and_non_request_wall_bound() {
        let store = fixture();
        let subjects = vec!["agent/eval.worker".into()];
        let cut = store.index().unwrap();
        let parent = smallclaims::read_budget::ReadBudget::new("/fault-cancel", std::time::Duration::from_secs(15));
        let cancel = parent.clone();
        AFTER_FAULT_ACQUISITION.with(|hook| *hook.borrow_mut() = Some(Box::new(move || cancel.cancel())));
        let error = smallclaims::read_budget::with(Some(parent), ||
            store.mailbox_faults_for(&subjects, cut)).unwrap_err();
        assert_eq!(error.downcast_ref::<St3Error>().unwrap().code, "read-deadline");
        BEFORE_FAULT_READER.with(|hook| *hook.borrow_mut() = Some(Box::new(||
            std::thread::sleep(std::time::Duration::from_millis(60)))));
        let error = store.mailbox_faults_for(&subjects, cut).unwrap_err();
        assert_eq!(error.downcast_ref::<St3Error>().unwrap().code, "read-deadline");
        assert_eq!(store.index().unwrap(), cut);
    }

    #[test]
    fn fault_capture_keeps_exact_recovery_and_runtime_fences_at_each_cut() {
        let store = fixture();
        let subject = "agent/eval.worker";
        let subjects = vec![subject.into(), "agent/absent".into(), subject.into()];
        let failure = |reason: &str| store.append_claim(&ClaimInput {
            subject: subject.into(), kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: serde_json::from_value(json!({"condition":"mailbox-channel-lost",
                "incarnation":"current", "reason":reason, "reviewer":subject,
                "title":"Mailbox lost", "severity":"error", "targets":[subject]})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let first = failure("first loss");
        let first_cut = store.index().unwrap();
        assert_eq!(store.mailbox_faults_for(&subjects, first_cut).unwrap(),
            BTreeMap::from([(subject.into(), "first loss".into())]));
        store.append_claim(&ClaimInput {
            subject: subject.into(), kind: "operational.recovered".into(),
            actor: Some("daemon/runtime".into()),
            fields: serde_json::from_value(json!({"failure":first.id, "reason":"replay consumed"})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert!(store.mailbox_faults_for(&subjects, store.index().unwrap()).unwrap().is_empty());
        assert_eq!(store.mailbox_faults_for(&subjects, first_cut).unwrap()[subject], "first loss");
        failure("second loss");
        let second_cut = store.index().unwrap();
        assert_eq!(store.mailbox_faults_for(&subjects, second_cut).unwrap()[subject], "second loss",
            "an old recovery must not quiet a new interruption");
        crate::store::mailbox_tests::ready(&store, "replacement");
        assert!(store.mailbox_faults_for(&subjects, store.index().unwrap()).unwrap().is_empty());
        assert_eq!(store.mailbox_faults_for(&subjects, second_cut).unwrap()[subject], "second loss");
        assert!(store.mailbox_faults_for(&[], second_cut).unwrap().is_empty());
        let before = store.index().unwrap();
        let budget = smallclaims::read_budget::ReadBudget::new("expired-card-cut", std::time::Duration::ZERO);
        let error = smallclaims::read_budget::with(Some(budget), ||
            store.mailbox_faults_for(&subjects, second_cut)).unwrap_err();
        assert_eq!(error.downcast_ref::<St3Error>().unwrap().code, "read-deadline");
        assert_eq!(store.index().unwrap(), before, "failure cannot backfill or return a partial fault map");
    }

    #[test]
    fn live_duplicates_and_pid_reuse_cannot_replace_or_receive_the_lease() {
        let store = fixture();
        let authority = authority(1);
        let request = request();
        let bound = store
            .bind_mailbox_with_lease(&request, Some(&authority))
            .unwrap();
        for _ in 0..3 {
            let duplicate = self::request();
            assert_eq!(
                store
                    .bind_mailbox_with_lease(&duplicate, Some(&authority))
                    .unwrap_err()
                    .code,
                "stale-mailbox-session"
            );
            let bindings: u64 = store
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM local_mailbox_bindings", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(bindings, 1);
            store.check_mailbox(&bound).unwrap();
        }
        let mut reused = authority.clone();
        reused.process_token += 1;
        assert!(
            store
                .bind_mailbox_with_lease(&request, Some(&reused))
                .is_err()
        );
        assert!(store.repair_mailbox(&bound, &reused).is_err());
        assert_eq!(
            store
                .bind_mailbox_with_lease(&request, Some(&authority))
                .unwrap()
                .epoch,
            bound.epoch
        );
    }

    #[test]
    fn uncertain_process_probes_preserve_the_canonical_capability() {
        let store = fixture();
        let owner = authority(1);
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        let uncertain = [
            anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::Other)),
            anyhow::anyhow!("malformed process start token"),
            anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::NotFound)),
        ];
        for error in uncertain {
            let result = channel_liveness(&owner, Err(error), || false);
            assert_eq!(result, ChannelLiveness::Indeterminate);
            let connection = store.connection.lock().unwrap();
            assert!(admit_with_probe(&connection, &request(), &owner, |_| result).is_err());
            let (held, _, _) = lease(&connection, &bound).unwrap().unwrap();
            assert_eq!(
                (&held.token, held.epoch, &held.incarnation),
                (&bound.token, bound.epoch, &bound.incarnation)
            );
            let (epoch, count): (u64, u64) = connection.query_row(
                "SELECT epoch,(SELECT COUNT(*) FROM local_mailbox_bindings) FROM local_mailbox_owners",
                [], |row| Ok((row.get(0)?,row.get(1)?)),
            ).unwrap();
            assert_eq!((epoch, count), (bound.epoch, 1));
        }
        let missing = channel_liveness(
            &owner,
            Err(std::io::Error::from(std::io::ErrorKind::NotFound).into()),
            || true,
        );
        let reused = channel_liveness(&owner, Ok(owner.process_token + 1), || false);
        for result in [missing, reused] {
            assert_eq!(result, ChannelLiveness::Dead);
            assert!(
                admit_with_probe(
                    &store.connection.lock().unwrap(),
                    &request(),
                    &owner,
                    |_| result
                )
                .is_ok()
            );
        }
        store.check_mailbox(&bound).unwrap();
    }

    #[test]
    fn malformed_persisted_process_birth_cannot_prove_a_dead_canonical_owner() {
        let store = fixture();
        let owner = authority(1);
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        store.connection.write().execute(
            "UPDATE local_mailbox_leases SET process_token='malformed' WHERE subject=?1 AND component=?2",
            params![bound.subject,bound.component],
        ).unwrap();
        assert!(
            store
                .bind_mailbox_with_lease(&request(), Some(&owner))
                .is_err()
        );
        assert!(store.repair_mailbox(&bound, &owner).is_err());
        let held: (String, u64) = store
            .readers
            .get()
            .query_row(
                "SELECT token,epoch FROM local_mailbox_leases WHERE subject=?1 AND component=?2",
                params![bound.subject, bound.component],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(held, (bound.token, bound.epoch));
    }

    #[test]
    fn host_move_fences_reads_and_receipts_before_runtime_reconciliation() {
        let store = fixture();
        let owner = authority(1);
        let fence = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        let moved = crate::graph::parse_intent(
            "version 2\nagent \"eval.worker\" { host \"other-node\"; workspace \"/tmp\"; harness \"omp\" {} }", "node").unwrap();
        store.apply_internal(&moved, "host-move-fixture").unwrap();
        assert_eq!(
            store.check_mailbox(&fence).unwrap_err().code,
            "stale-mailbox-session"
        );
        let receipt = ClaimInput {
            subject: "message/host-move".into(),
            kind: "message.staged".into(),
            actor: Some(fence.subject.clone()),
            fields: BTreeMap::from([("status".into(), json!("staged"))]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        };
        assert_eq!(
            store
                .append_mailbox_receipt(&receipt, &fence)
                .unwrap_err()
                .code,
            "stale-mailbox-session"
        );
        assert!(store.repair_mailbox(&fence, &owner).is_err());
        let runtime = store.latest_actual_value(&fence.subject).unwrap().unwrap();
        let fields = runtime.get("fields").unwrap_or(&runtime);
        assert_eq!(fields["status"], "running");
        assert_eq!(fields["incarnation_id"], "current");
    }

    #[test]
    fn repair_retains_exact_capability_but_never_rewinds_a_successor_or_revocation() {
        let store = fixture();
        let first = authority(1);
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&first))
            .unwrap();
        store
            .connection
            .lock()
            .unwrap()
            .execute("DELETE FROM local_mailbox_owners", [])
            .unwrap();
        assert!(store.check_mailbox(&bound).is_err());
        assert!(store.repair_mailbox(&bound, &first).unwrap());
        assert!(!store.repair_mailbox(&bound, &first).unwrap());
        store.check_mailbox(&bound).unwrap();
        let successor = store
            .bind_mailbox_with_lease(&request(), Some(&authority(2)))
            .unwrap();
        assert_eq!(successor.epoch, bound.epoch + 1);
        assert!(store.repair_mailbox(&bound, &first).is_err());
        assert!(store.bind_mailbox_with_lease(&bound, Some(&first)).is_err());
        store.check_mailbox(&successor).unwrap();
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM local_mailbox_bindings WHERE token=?1",
                [&successor.token],
            )
            .unwrap();
        assert!(store.repair_mailbox(&successor, &authority(2)).is_err());
        let mut unbound = successor.clone();
        unbound.epoch = 0;
        assert!(
            store
                .bind_mailbox_with_lease(&unbound, Some(&authority(2)))
                .is_err(),
            "a revoked token cannot allocate again"
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn dead_channel_successor_preserves_provider_session_and_retires_old_capability() {
        let store = fixture();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let mut predecessor = authority(1);
        predecessor.pid = child.id();
        predecessor.process_token = st_runtime::process_start_token(child.id()).unwrap();
        let old = store
            .bind_mailbox_with_lease(&request(), Some(&predecessor))
            .unwrap();
        assert!(
            store
                .bind_mailbox_with_lease(&request(), Some(&authority(1)))
                .is_err()
        );
        child.kill().unwrap();
        child.wait().unwrap();
        let new = store
            .bind_mailbox_with_lease(&request(), Some(&authority(1)))
            .unwrap();
        assert_eq!(new.epoch, old.epoch + 1);
        assert!(store.repair_mailbox(&old, &predecessor).is_err());
        assert!(
            store
                .bind_mailbox_with_lease(&old, Some(&predecessor))
                .is_err()
        );
        store.check_mailbox(&new).unwrap();
    }

    #[test]
    fn persisted_canonical_lease_survives_store_reopen_and_rejects_completed_runtime() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.sqlite");
        let bound = {
            let store = Store::open(&path, "node").unwrap();
            declare(&store);
            crate::store::mailbox_tests::ready(&store, "current");
            store
                .bind_mailbox_with_lease(&request(), Some(&authority(1)))
                .unwrap()
        };
        let store = Store::open(&path, "node").unwrap();
        assert_eq!(
            store
                .bind_mailbox_with_lease(&bound, Some(&authority(1)))
                .unwrap()
                .epoch,
            bound.epoch
        );
        assert!(
            store
                .bind_mailbox_with_lease(&request(), Some(&authority(1)))
                .is_err()
        );
        store
            .append_claim(&ClaimInput {
                subject: bound.subject.clone(),
                kind: "runtime.observed".into(),
                actor: Some(bound.subject.clone()),
                fields: BTreeMap::from([
                    ("status".into(), json!("exited")),
                    ("incarnation_id".into(), json!("current")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        assert!(store.repair_mailbox(&bound, &authority(1)).is_err());
        assert!(
            store
                .bind_mailbox_with_lease(&request(), Some(&authority(1)))
                .is_err()
        );
    }

    #[test]
    fn bootstrap_promotion_keeps_capability_and_cannot_undo_provider_takeover() {
        let store = fixture();
        let intent = crate::graph::parse_intent("version 2\nagent \"eval.worker\" { host \"node\"; workspace \"/tmp\"; harness \"codex\" {} }", "node").unwrap();
        store
            .apply_internal(&intent, "codex-bootstrap-fixture")
            .unwrap();
        let mut bootstrap = authority(1);
        bootstrap.provider = "codex".into();
        bootstrap.session = "runtime:current".into();
        bootstrap.sequence = 0;
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&bootstrap))
            .unwrap();
        assert!(
            store
                .bind_mailbox_with_lease(&request(), Some(&bootstrap))
                .is_err()
        );
        let mut provider = authority(1);
        provider.provider = "codex".into();
        let mut wrong_birth = provider.clone();
        wrong_birth.process_token += 1;
        assert!(
            store
                .promote_current_mailbox_ownership(&bound, &wrong_birth)
                .is_err()
        );
        store.promote_current_mailbox_ownership(&bound, &provider).unwrap();
        assert_eq!(
            store.mailbox_lease_authority(&bound).unwrap(),
            Some(provider.clone())
        );
        let reconnect = store
            .bind_mailbox_with_lease(&bound, Some(&provider))
            .unwrap();
        assert_eq!(
            (reconnect.token, reconnect.epoch),
            (bound.token.clone(), bound.epoch)
        );
        assert!(
            store
                .bind_mailbox_with_lease(&bound, Some(&bootstrap))
                .is_err()
        );
        let mut successor = authority(2);
        successor.provider = "codex".into();
        let new = store
            .bind_mailbox_with_lease(&request(), Some(&successor))
            .unwrap();
        assert!(
            store
                .bind_mailbox_with_lease(&bound, Some(&provider))
                .is_err()
        );
        store.promote_current_mailbox_ownership(&bound, &provider).unwrap();
        assert_eq!(
            store.mailbox_lease_authority(&new).unwrap(),
            Some(successor)
        );
        store.check_mailbox(&new).unwrap();
    }

    fn declare_argv(store: &Store, host: &str) -> MemberSpec {
        let intent = crate::graph::parse_intent(
            &format!("version 2\nagent \"eval.worker\" {{ host \"{host}\"; workspace \"/tmp\"; argv \"python3\" \"probe\"; }}"),
            "node",
        ).unwrap();
        store.apply_internal(&intent, "argv-monitor-fixture").unwrap();
        store.desired_subjects_named(&["agent/eval.worker".into()]).unwrap().remove(0).member.unwrap()
    }

    // A typed Store fixture changes the captured member without a runtime
    // reconciliation or process restart. This is not a physical argv witness.
    fn update_argv_member(store: &Store, member: &MemberSpec) {
        let mut intent = crate::graph::parse_intent(
            "version 2\nagent \"eval.worker\" { host \"node\"; workspace \"/tmp\"; argv \"python3\" \"probe\"; }",
            "node",
        ).unwrap();
        intent.subjects.get_mut("agent/eval.worker").unwrap().member = Some(member.clone());
        store
            .apply_internal(&intent, "argv-member-edit-fixture")
            .unwrap();
        assert_eq!(
            store
                .desired_subjects_named(&["agent/eval.worker".into()])
                .unwrap()[0]
                .member
                .as_ref(),
            Some(member)
        );
    }

    #[test]
    fn argv_metadata_edits_preserve_capability_and_receipt_admission() {
        for edit in ["tags", "display", "environment", "restart"] {
            let store = fixture();
            let captured = declare_argv(&store, "node");
            let bound = store
                .bind_argv_mailbox_checked(&request(), &captured, &|| Ok(()))
                .unwrap();
            let runtime = store
                .latest_claim(&bound.subject, Some("runtime.observed"))
                .unwrap()
                .unwrap()
                .id;
            store
                .append_claim(&ClaimInput {
                    subject: "message/argv-metadata".into(),
                    kind: "message.sent".into(),
                    actor: Some("person/fixture".into()),
                    fields: BTreeMap::from([
                        ("status".into(), json!("sent")),
                        ("from".into(), json!("person/fixture")),
                        ("to".into(), json!(bound.subject)),
                        ("content".into(), json!("Original envelope")),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
            let receipt = |phase: &str| ClaimInput {
                subject: "message/argv-metadata".into(),
                kind: format!("message.{phase}"),
                actor: Some(bound.subject.clone()),
                fields: BTreeMap::from([("status".into(), json!(phase))]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(format!("argv-metadata:{phase}")),
            };
            store
                .append_mailbox_receipt_outcome(&receipt("staged"), &bound)
                .unwrap();
            let mut changed = captured.clone();
            match edit {
                "tags" => {
                    changed.tags.insert("purpose".into(), "updated".into());
                }
                "display" => changed.display_name = Some("Updated display".into()),
                "environment" => {
                    changed
                        .environment
                        .insert("ARGV_FIXTURE".into(), "updated".into());
                }
                "restart" => {
                    changed.restart = crate::model::RestartType::Never;
                    changed.restart_intensity.attempts += 1;
                    changed.shutdown_timeout_ms += 1;
                }
                _ => unreachable!(),
            }
            assert_ne!(changed, captured);
            assert!(changed.launch_changes(&captured).is_empty());
            update_argv_member(&store, &changed);
            assert_eq!(
                store
                    .latest_claim(&bound.subject, Some("runtime.observed"))
                    .unwrap()
                    .unwrap()
                    .id,
                runtime
            );
            store.check_mailbox(&bound).unwrap();
            for phase in ["staged", "delivered", "read"] {
                // Cached and fresh receipts must both keep the original current
                // capability; metadata changes do not grant another owner.
                store
                    .append_mailbox_receipt_outcome(&receipt(phase), &bound)
                    .unwrap();
                store
                    .append_mailbox_receipt_outcome(&receipt(phase), &bound)
                    .unwrap();
                assert_eq!(
                    store
                        .claims_for("message/argv-metadata", Some(&format!("message.{phase}")))
                        .unwrap()
                        .len(),
                    1
                );
            }
            for member in [&captured, &changed] {
                let reconnect = store
                    .bind_argv_mailbox_checked(&bound, member, &|| Ok(()))
                    .unwrap();
                assert_eq!(
                    (reconnect.token, reconnect.epoch),
                    (bound.token.clone(), bound.epoch)
                );
            }
            assert!(store.mailbox_lease_authority(&bound).unwrap().is_none());
        }
    }

    #[test]
    fn argv_launch_edits_still_fence_the_unchanged_running_capability() {
        for edit in ["workspace", "cwd", "terminal", "runtime", "raw-argv"] {
            let store = fixture();
            let mut captured = declare_argv(&store, "node");
            if edit == "raw-argv" {
                captured.launch = LaunchSpec::Argv(vec![
                    "wrapper".into(),
                    "--".into(),
                    "program".into(),
                    "--settings".into(),
                    "old.json".into(),
                    "argument".into(),
                ]);
                update_argv_member(&store, &captured);
            }
            let bound = store
                .bind_argv_mailbox_checked(&request(), &captured, &|| Ok(()))
                .unwrap();
            let runtime = store
                .latest_claim(&bound.subject, Some("runtime.observed"))
                .unwrap()
                .unwrap()
                .id;
            let mut changed = captured.clone();
            match edit {
                "workspace" => changed.workspace = "/different".into(),
                "cwd" => changed.cwd = "/different".into(),
                "terminal" => changed.terminal = !captured.terminal,
                "runtime" => changed.runtime_id.push_str("-replacement"),
                "raw-argv" => {
                    let LaunchSpec::Argv(argv) = &mut changed.launch else {
                        unreachable!()
                    };
                    argv[4] = "new.json".into();
                    // Typed wrappers omit generated settings from their launch
                    // comparison. An explicit argv transport must not do that.
                    assert!(changed.launch_changes(&captured).is_empty());
                }
                _ => unreachable!(),
            }
            update_argv_member(&store, &changed);
            assert_eq!(
                store
                    .latest_claim(&bound.subject, Some("runtime.observed"))
                    .unwrap()
                    .unwrap()
                    .id,
                runtime
            );
            assert_eq!(
                store.check_mailbox(&bound).unwrap_err().code,
                "stale-mailbox-session"
            );
            let receipt = ClaimInput {
                subject: "message/argv-launch-edit".into(),
                kind: "message.read".into(),
                actor: Some(bound.subject.clone()),
                fields: BTreeMap::from([("status".into(), json!("read"))]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(format!("argv-launch-edit:{edit}")),
            };
            assert_eq!(
                store
                    .append_mailbox_receipt_outcome(&receipt, &bound)
                    .unwrap_err()
                    .code,
                "stale-mailbox-session"
            );
            assert!(
                store
                    .claims_for(&receipt.subject, Some("message.read"))
                    .unwrap()
                    .is_empty()
            );
            assert!(
                store
                    .bind_argv_mailbox_checked(&bound, &changed, &|| Ok(()))
                    .is_err()
            );
            // Restoring the exact declaration restores this same capability.
            update_argv_member(&store, &captured);
            store.check_mailbox(&bound).unwrap();
        }
    }

    #[test]
    fn argv_channel_binding_retains_runtime_declaration_and_receipt_fences() {
        let store = fixture();
        let member = declare_argv(&store, "node");
        let bound = store.bind_argv_mailbox_checked(&request(), &member, &|| Ok(())).unwrap();
        assert!(store.mailbox_lease_authority(&bound).unwrap().is_none());
        let reconnect = store.bind_argv_mailbox_checked(&bound, &member, &|| Ok(())).unwrap();
        assert_eq!((reconnect.token, reconnect.epoch), (bound.token.clone(), bound.epoch));
        let mut stale = bound.clone();
        stale.incarnation = "previous".into();
        assert!(store.bind_argv_mailbox_checked(&stale, &member, &|| Ok(())).is_err());
        let moved = declare_argv(&store, "other");
        assert!(store.bind_argv_mailbox_checked(&bound, &moved, &|| Ok(())).is_err());
        assert!(store.bind_argv_mailbox_checked(&bound, &member, &|| Ok(())).is_err());
        let mut undeclared = request();
        undeclared.subject = "agent/undeclared".into();
        assert!(store.bind_argv_mailbox_checked(&undeclared, &member, &|| Ok(())).is_err());
        store.append_claim(&ClaimInput {
            subject: bound.subject.clone(), kind: "runtime.observed".into(), actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([("status".into(), json!("running")), ("incarnation_id".into(), json!("replacement"))]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert!(store.bind_argv_mailbox_checked(&bound, &member, &|| Ok(())).is_err());
    }

    #[test]
    fn argv_channel_cannot_discard_qualified_or_revoked_custody() {
        for revoked in [false, true] {
            let store = fixture();
            let owner = authority(1);
            let held = store.bind_mailbox_with_lease(&request(), Some(&owner)).unwrap();
            if revoked {
                store.connection.write().execute("UPDATE local_mailbox_leases SET revoked=1", []).unwrap();
            }
            let member = declare_argv(&store, "node");
            for candidate in [held.clone(), request()] {
                assert!(store.bind_argv_mailbox_checked(&candidate, &member, &|| Ok(())).is_err());
            }
            let retained = lease(&store.readers.get(), &held).unwrap().unwrap();
            assert_eq!((retained.0.token, retained.0.epoch, retained.1, retained.2),
                (held.token, held.epoch, owner, revoked));
        }
    }

    #[test]
    fn argv_capability_refuses_changed_declarations_before_receipt_commit() {
        for replacement in [
            "version 2\nstop \"agent/eval.worker\"",
            "version 2\nagent \"eval.worker\" { host \"other\"; workspace \"/tmp\"; argv \"python3\" \"probe\"; }",
            "version 2\nagent \"eval.worker\" { host \"node\"; workspace \"/tmp\"; harness \"omp\" {} }",
            "version 2\nagent \"eval.worker\" { host \"node\"; workspace \"/tmp\"; argv \"python3\" \"different-program\"; }",
        ] {
            let store = fixture();
            let member = declare_argv(&store, "node");
            let bound = store.bind_argv_mailbox_checked(&request(), &member, &|| Ok(())).unwrap();
            let runtime = store.latest_claim(&bound.subject, Some("runtime.observed")).unwrap().unwrap().id;
            store.append_claim(&ClaimInput {
                subject: "message/argv-fence".into(), kind: "message.sent".into(), actor: Some("person/fixture".into()),
                fields: BTreeMap::from([("status".into(), json!("sent")), ("from".into(), json!("person/fixture")),
                    ("to".into(), json!(bound.subject)), ("content".into(), json!("Original envelope"))]),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
            let receipt = |phase: &str| ClaimInput {
                subject: "message/argv-fence".into(), kind: format!("message.{phase}"), actor: Some(bound.subject.clone()),
                fields: BTreeMap::from([("status".into(), json!(phase))]), evidence: vec![], expected_subject: None,
                idempotency_key: Some(format!("argv-fence:{phase}")),
            };
            // Positive unchanged argv, including a successful receipt and the
            // pre-I/O admission that a caller may already have observed.
            store.append_mailbox_receipt_outcome(&receipt("staged"), &bound).unwrap();
            store.check_mailbox(&bound).unwrap();
            let changed = crate::graph::parse_intent(replacement, "node").unwrap();
            store.apply_internal(&changed, "change-argv-without-runtime-reconciliation").unwrap();
            assert_eq!(store.latest_claim(&bound.subject, Some("runtime.observed")).unwrap().unwrap().id, runtime);
            let held: (String, u64) = store.readers.get().query_row(
                "SELECT incarnation,epoch FROM local_mailbox_owners WHERE subject=?1 AND component=?2",
                params![bound.subject,bound.component], |row| Ok((row.get(0)?,row.get(1)?)),
            ).unwrap();
            assert_eq!(held, (bound.incarnation.clone(),bound.epoch));
            assert_eq!(store.check_mailbox(&bound).unwrap_err().code, "stale-mailbox-session");
            for phase in ["staged","delivered","read"] {
                // Even a cached original receipt must be fenced before its retry;
                // fresh lifecycle claims are checked under the commit writer.
                assert_eq!(store.append_mailbox_receipt_outcome(&receipt(phase), &bound).unwrap_err().code, "stale-mailbox-session");
            }
            assert!(store.claims_for("message/argv-fence",Some("message.delivered")).unwrap().is_empty());
            assert!(store.claims_for("message/argv-fence",Some("message.read")).unwrap().is_empty());
            assert!(store.mailbox_lease_authority(&bound).unwrap().is_none());
        }
    }

    #[test]
    fn argv_designation_survives_reopen_and_cannot_become_native_custody() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("argv.db");
        let store = Store::open(&path,"node").unwrap();
        declare(&store);
        crate::store::mailbox_tests::ready(&store,"current");
        let member = declare_argv(&store,"node");
        let bound = store.bind_argv_mailbox_checked(&request(), &member, &|| Ok(())).unwrap();
        drop(store);
        let store = Store::open(&path,"node").unwrap();
        store.check_mailbox(&bound).unwrap();
        declare(&store);
        assert!(store.check_mailbox(&bound).is_err());
        assert!(store.bind_mailbox_with_lease(&bound,Some(&authority(1))).is_err());
        assert!(store.mailbox_lease_authority(&bound).unwrap().is_none());
        let member = declare_argv(&store,"node");
        store.connection.write().execute("DELETE FROM local_mailbox_bindings WHERE token=?1",[&bound.token]).unwrap();
        let mut retired = bound.clone(); retired.epoch=0;
        assert!(store.bind_argv_mailbox_checked(&retired,&member,&|| Ok(())).is_err());
        assert_eq!(store.readers.get().query_row("SELECT epoch FROM local_mailbox_owners WHERE subject=?1 AND component=?2",
            params![bound.subject,bound.component],|row|row.get::<_,u64>(0)).unwrap(),bound.epoch);
    }

    #[test]
    fn same_process_session_reexec_advances_ownership_without_reissuing_capability() {
        let store = fixture();
        let owner = authority(1);
        let bound = store.bind_mailbox_with_lease(&request(), Some(&owner)).unwrap();
        let mut advanced = owner.clone();
        advanced.sequence += 1;
        for foreign in [
            Authority { pid: owner.pid + 1, ..advanced.clone() },
            Authority { process_token: owner.process_token + 1, ..advanced.clone() },
            Authority { provider: "codex".into(), ..advanced.clone() },
        ] {
            assert!(store.promote_current_mailbox_ownership(&bound, &foreign).is_err());
            assert_eq!(store.mailbox_lease_authority(&bound).unwrap(), Some(owner.clone()));
        }
        let different_session = Authority { session: "different-session".into(), ..advanced.clone() };
        store.promote_current_mailbox_ownership(&bound, &different_session).unwrap();
        assert!(!store.owns_mailbox_lease(&bound, &different_session).unwrap());
        assert_eq!(store.mailbox_lease_authority(&bound).unwrap(), Some(owner.clone()));
        store.promote_current_mailbox_ownership(&bound, &advanced).unwrap();
        assert_eq!(store.mailbox_lease_authority(&bound).unwrap(), Some(advanced.clone()));
        let reconnected = store.bind_mailbox_with_lease(&bound, Some(&advanced)).unwrap();
        assert_eq!(
            (reconnected.token, reconnected.epoch, reconnected.incarnation),
            (bound.token.clone(), bound.epoch, bound.incarnation.clone()),
        );
        assert!(store.bind_mailbox_with_lease(&request(), Some(&advanced)).is_err());
        store.promote_current_mailbox_ownership(&bound, &owner).unwrap();
        assert_eq!(store.mailbox_lease_authority(&bound).unwrap(), Some(advanced));
        let successor = authority(3);
        let replacement = store.bind_mailbox_with_lease(&request(), Some(&successor)).unwrap();
        store.promote_current_mailbox_ownership(&bound, &successor).unwrap();
        assert_eq!(store.mailbox_lease_authority(&replacement).unwrap(), Some(successor));
        assert!(store.check_mailbox(&bound).is_err());
    }

    #[test]
    fn delivery_and_title_have_independent_custody_and_provider_mismatch_is_terminal() {
        let store = fixture();
        let owner = authority(1);
        let delivery = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        let title_request = Fence::new("agent/eval.worker", "current", "title");
        let title = store
            .bind_mailbox_with_lease(&title_request, Some(&owner))
            .unwrap();
        assert_eq!(title.epoch, 1);
        store.check_mailbox(&delivery).unwrap();
        store.check_mailbox(&title).unwrap();
        let mut wrong = authority(2);
        wrong.provider = "claude".into();
        assert!(
            store
                .bind_mailbox_with_lease(&request(), Some(&wrong))
                .is_err()
        );
        let mut wrong_runtime = request();
        wrong_runtime.incarnation = "retired".into();
        assert!(
            store
                .bind_mailbox_with_lease(&wrong_runtime, Some(&owner))
                .is_err()
        );
        store.check_mailbox(&delivery).unwrap();
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE local_mailbox_leases SET revoked=1 WHERE component='delivery'",
                [],
            )
            .unwrap();
        assert!(
            store
                .bind_mailbox_with_lease(&delivery, Some(&owner))
                .is_err()
        );
        assert!(store.repair_mailbox(&delivery, &owner).is_err());
        store.check_mailbox(&title).unwrap();
    }

    #[test]
    fn intentional_stop_cannot_become_automatic_repair_or_new_admission() {
        let store = fixture();
        let owner = authority(1);
        let fence = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        let stopped =
            crate::graph::parse_intent("version 2\nstop \"agent/eval.worker\"", "node").unwrap();
        store
            .apply_internal(&stopped, "intentional-stop-fixture")
            .unwrap();
        assert!(
            store.check_mailbox(&fence).is_err(),
            "a live wrapper must not read after intentional stop"
        );
        assert!(store.mailbox_session_active(&fence, &owner).is_err());
        assert!(store.repair_mailbox(&fence, &owner).is_err());
        assert!(
            store
                .bind_mailbox_with_lease(&request(), Some(&authority(2)))
                .is_err()
        );
    }

    #[test]
    fn empty_registry_adopts_only_the_current_legacy_capability_without_epoch_churn() {
        let store = fixture();
        let old = store.bind_mailbox(&request()).unwrap();
        let current = store.bind_mailbox(&request()).unwrap();
        assert!(
            store
                .bind_mailbox_with_lease(&request(), Some(&authority(2)))
                .is_err()
        );
        assert!(
            store
                .bind_mailbox_with_lease(&old, Some(&authority(2)))
                .is_err()
        );
        let adopted = store
            .bind_mailbox_with_lease(&current, Some(&authority(2)))
            .unwrap();
        assert_eq!(
            (adopted.token, adopted.epoch),
            (current.token.clone(), current.epoch)
        );
        assert_eq!(
            store.mailbox_lease_authority(&current).unwrap(),
            Some(authority(2))
        );
        assert!(store.repair_mailbox(&old, &authority(1)).is_err());
        store.check_mailbox(&current).unwrap();
    }

    // These controls cover `promote_current_mailbox_ownership` itself. They say nothing about
    // other work an attachment request does.
    #[test]
    fn promotions_that_would_change_nothing_do_not_borrow_the_writer() {
        let store = Arc::new(fixture());
        let owner = authority(1);
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        let no_lease = Fence::new("agent/eval.worker", "current", "title");
        let held = store.connection.write();
        for (case, fence, candidate) in [
            ("same sequence, exact owner", bound.clone(), owner.clone()),
            (
                "sequence zero against a promoted lease",
                bound.clone(),
                Authority { sequence: 0, ..owner.clone() },
            ),
            (
                "higher sequence from another session",
                bound.clone(),
                Authority { session: "different-session".into(), sequence: 2, ..owner.clone() },
            ),
            ("no lease row", no_lease, owner.clone()),
        ] {
            let (receiver, worker) = promote_elsewhere(&store, &fence, &candidate);
            assert_eq!(finished_without_the_writer(case, &receiver, worker), Ok(()), "{case}");
        }
        drop(held);
        assert_eq!(store.mailbox_lease_authority(&bound).unwrap(), Some(owner));
    }

    #[test]
    fn a_same_session_sequence_advance_goes_on_to_the_writer_and_records() {
        let store = Arc::new(fixture());
        let owner = authority(1);
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        let advanced = Authority { sequence: 2, ..owner.clone() };
        let reached = change_after_precheck(|| {});
        store
            .promote_current_mailbox_ownership(&bound, &advanced)
            .unwrap();
        assert!(precheck_was_reached(&reached), "an advance is not a no-op");
        assert_eq!(
            store.mailbox_lease_authority(&bound).unwrap(),
            Some(advanced)
        );
    }

    #[test]
    fn a_bootstrap_promotion_from_sequence_zero_goes_on_to_the_writer_and_records() {
        let store = Arc::new(fixture());
        let intent = crate::graph::parse_intent("version 2\nagent \"eval.worker\" { host \"node\"; workspace \"/tmp\"; harness \"codex\" {} }", "node").unwrap();
        store
            .apply_internal(&intent, "codex-bootstrap-fixture")
            .unwrap();
        let mut bootstrap = authority(1);
        bootstrap.provider = "codex".into();
        bootstrap.session = "runtime:current".into();
        bootstrap.sequence = 0;
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&bootstrap))
            .unwrap();
        let mut provider = authority(1);
        provider.provider = "codex".into();
        let reached = change_after_precheck(|| {});
        store
            .promote_current_mailbox_ownership(&bound, &provider)
            .unwrap();
        assert!(precheck_was_reached(&reached), "a bootstrap promotion is not a no-op");
        assert_eq!(
            store.mailbox_lease_authority(&bound).unwrap(),
            Some(provider)
        );
    }

    #[test]
    fn a_lease_revoked_after_the_precheck_is_refused_on_the_writer() {
        let store = Arc::new(fixture());
        let owner = authority(1);
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        let advanced = Authority { sequence: 2, ..owner.clone() };
        let changing = Arc::clone(&store);
        let reached = change_after_precheck(move || {
            changing
                .connection
                .write()
                .execute("UPDATE local_mailbox_leases SET revoked=1", [])
                .unwrap();
        });
        assert!(
            store
                .promote_current_mailbox_ownership(&bound, &advanced)
                .is_err()
        );
        assert!(precheck_was_reached(&reached), "the pre-check saw a lease that could promote");
        // The writer's own read of the revoked row decided; custody is not advanced.
        assert_eq!(store.mailbox_lease_authority(&bound).unwrap(), Some(owner));
    }

    #[test]
    fn a_lease_superseded_after_the_precheck_is_not_overwritten_and_a_no_op_authorizes_nothing() {
        let store = Arc::new(fixture());
        let owner = authority(1);
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        let advanced = Authority { sequence: 2, ..owner.clone() };
        let successor = authority(3);
        let changing = Arc::clone(&store);
        let replaced = Arc::new(std::sync::Mutex::new(None));
        let slot = Arc::clone(&replaced);
        let reached = change_after_precheck(move || {
            let replacement = changing
                .bind_mailbox_with_lease(&request(), Some(&successor))
                .unwrap();
            *slot.lock().unwrap() = Some(replacement);
        });
        // The pre-check read the old lease and the old capability; by the time the writer reads
        // the row it belongs to the successor, so the old fence promotes nothing.
        store
            .promote_current_mailbox_ownership(&bound, &advanced)
            .unwrap();
        assert!(precheck_was_reached(&reached));
        let replacement = replaced.lock().unwrap().clone().unwrap();
        assert_eq!(
            store.mailbox_lease_authority(&replacement).unwrap(),
            Some(authority(3)),
            "the old capability's promotion must not overwrite the successor's custody"
        );
        // A no-op pre-check result is never an authorization: the retired fence is still refused
        // by every later check, whatever the promotion returned.
        store
            .promote_current_mailbox_ownership(&bound, &owner)
            .unwrap();
        assert!(store.check_mailbox(&bound).is_err());
        assert!(!store.owns_mailbox_lease(&bound, &owner).unwrap());
        store.check_mailbox(&replacement).unwrap();
    }

    #[test]
    fn a_failed_lease_read_is_returned_without_borrowing_the_writer() {
        let store = Arc::new(fixture());
        let owner = authority(1);
        let bound = store
            .bind_mailbox_with_lease(&request(), Some(&owner))
            .unwrap();
        store
            .connection
            .write()
            .execute(
                "ALTER TABLE local_mailbox_leases RENAME TO local_mailbox_leases_unreadable",
                [],
            )
            .unwrap();
        let held = store.connection.write();
        let (receiver, worker) = promote_elsewhere(&store, &bound, &owner);
        let result = finished_without_the_writer("failed lease read", &receiver, worker);
        assert!(result.is_err(), "{result:?}");
        drop(held);
    }
}
