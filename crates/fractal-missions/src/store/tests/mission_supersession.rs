use super::*;
use super::run_retry::ORIGINAL;

fn apply_source(store: &Store, source: &str, key: &str) {
    let intent = parse_intent(source, &store.origin).unwrap();
    let planned = store.mission(&intent, IntentInput {
        kdl: source.into(), source_name: None,
    }).unwrap();
    store.apply(&intent, &planned.subject_tokens, key).unwrap();
}

fn start_source(store: &Store, after: Option<&str>) -> MissionRunView {
    let revision = store.mission_spec("retry-work", None).unwrap().unwrap().revision;
    let after = after.map_or_else(String::new, |run| format!("after {run:?}\n"));
    apply_source(store, &format!(
        "version 2\nmission-run \"supersession-source\" {{\n mission \"mission/retry-work@{revision}\"\n workspace \"/work/retry\"\n requester \"person/requester\"\n input \"value\" \"unchanged\"\n {after}}}\n"
    ), "start-supersession-source");
    store.mission_run("supersession-source").unwrap().unwrap()
}

fn current(store: &Store, run: &MissionRunView) -> MissionRunView {
    store.mission_run(&run.subject).unwrap().unwrap()
}

fn replacement(store: &Store, source: &MissionRunView) -> MissionRunView {
    let subject = current(store, source).superseded_by.unwrap();
    store.mission_run(&subject).unwrap().unwrap()
}

fn assert_link(source: &MissionRunView, target: &MissionRunView) {
    assert_eq!(source.superseded_by.as_deref(), Some(target.subject.as_str()));
    assert_eq!(target.supersedes.as_deref(), Some(source.subject.as_str()));
    assert_eq!(target.inputs, source.inputs);
    assert_eq!(target.workspace, source.workspace);
    assert_eq!(target.requester, source.requester);
    assert_eq!(target.mode, source.mode);
    assert!(target.retry_of.is_none());
    assert!(source.retried_by.is_empty());
    assert!(target.steps.iter().all(|step| step.status == "pending" && step.attempt == 1));
    assert!(target.steps.iter().all(|step| source.steps.iter().all(|old| old.subject != step.subject)));
}

fn rebuild(store: &Store, runs: &[&MissionRunView]) {
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    for run in runs {
        rebuild_run_part_tx(&transaction, &run.id).unwrap();
    }
    transaction.commit().unwrap();
}

fn finish_cancellation(store: &Store, run: &MissionRunView) {
    // No runtime has been launched: ordinary supersession cancellation already cancelled
    // every non-final step. Finalization acknowledges that there is nothing left to reap.
    let cancelling = current(store, run);
    assert_eq!(cancelling.phase, "cleanup-cancelled");
    assert!(cancelling.steps.iter().all(|step| step.status == "cancelled"));
    assert!(store.set_mission_run_state(&run.subject, "cancelled", "terminal", Some("cancelled work has finished cleanup")).unwrap());
}

fn requeue_heads(store: &Store) {
    // Replay disposable work without changing authoritative publications or refusal claims.
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hb4f3c30874b6 owner=fractal_missions::store::tests::mission_supersession TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_mission_supersession_heads_v1(mission_id) SELECT mission_id FROM mission_definitions",
        [],
    ).unwrap();
}

fn assert_queue_converged(store: &Store) {
    let pending: u64 = store.readers.get().query_row(
        "SELECT COUNT(*) FROM local_mission_supersession_heads_v1",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(pending, 0);
    let index = store.index().unwrap();
    assert!(!store.supersede_mission_publications().unwrap());
    assert_eq!(store.index().unwrap(), index);
}

#[test]
fn mission_supersession_reopen_and_rebuild_wait_for_entire_cleanup_ancestry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let store = Store::open(&path, "owner").unwrap();
    let bounded = ORIGINAL.replace("state=\"ready\"", "state=\"ready\" timeout=\"5m\"");
    publish_mission(&store, &bounded, "original");
    let source = start_source(&store, None);
    publish_mission(&store, &bounded.replace("exit 1", "exit 0"), "second");
    let second = replacement(&store, &source);
    publish_mission(&store, &bounded.replace("exit 1", "exit 2"), "third");
    let third = replacement(&store, &second);
    assert_link(&current(&store, &source), &second);
    assert_link(&current(&store, &second), &third);
    assert_eq!(third.phase, "supersession-waiting");
    assert_eq!(third.timeout_ms, Some(300_000));
    assert_eq!(third.deadline_at_unix_ms, None);
    let created = store.claims_for(&third.subject, Some("mission-run.created")).unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].body["fields"]["phase"], "supersession-waiting");
    assert!(created[0].body["fields"]["deadline_at_unix_ms"].is_null());
    drop(store);

    let store = Store::open(&path, "owner").unwrap();
    rebuild(&store, &[&source, &second, &third]);
    assert_eq!(current(&store, &source).generation, source.generation);
    assert_eq!(current(&store, &source).revision, source.revision);
    assert_link(&current(&store, &source), &second);
    assert_link(&current(&store, &second), &current(&store, &third));
    assert_eq!(current(&store, &third).deadline_at_unix_ms, None);
    assert!(!store.resume_superseding_mission_run(&third.subject).unwrap());
    // The immediate predecessor being terminal is insufficient while its ancestor cleans up.
    finish_cancellation(&store, &second);
    assert!(!store.resume_superseding_mission_run(&third.subject).unwrap());
    finish_cancellation(&store, &source);
    assert!(store.resume_superseding_mission_run(&third.subject).unwrap());
    let admitted = current(&store, &third);
    assert_eq!(admitted.phase, "normal");
    assert!(admitted.deadline_at_unix_ms.is_some());
    rebuild(&store, &[&source, &second, &third]);
    assert_eq!(current(&store, &third).deadline_at_unix_ms, admitted.deadline_at_unix_ms);
    assert!(!store.resume_superseding_mission_run(&third.subject).unwrap());
    requeue_heads(&store);
    store.supersede_mission_publications().unwrap();
    assert_queue_converged(&store);
    assert_eq!(current(&store, &source).superseded_by, Some(second.subject.clone()));
    assert_eq!(current(&store, &second).superseded_by, Some(third.subject));
}

#[test]
fn mission_supersession_after_dependency_survives_reopen_rebuild_and_delays_timeout() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let store = Store::open(&path, "owner").unwrap();
    let bounded = ORIGINAL.replace("state=\"ready\"", "state=\"ready\" timeout=\"5m\"");
    publish_mission(&store, &bounded, "original");
    // Reuse the ordinary retry fixture for the still-active prerequisite run.
    let prerequisite = super::run_retry::start(&store);
    let source = start_source(&store, Some(&prerequisite.subject));
    publish_mission(&store, &bounded.replace("exit 1", "exit 0"), "corrected");
    let target = replacement(&store, &source);
    assert_eq!(target.after, source.after);
    assert_eq!(target.after.as_deref(), Some(prerequisite.subject.as_str()));
    assert!(target.steps.iter().any(|step| step.step == "after-run" && step.status == "pending"));
    let created = store.claims_for(&target.subject, Some("mission-run.created")).unwrap();
    assert_eq!(created[0].body["fields"]["after"], prerequisite.subject);
    assert_eq!(created[0].body["fields"]["timeout_ms"], 300_000);
    assert!(created[0].body["fields"]["deadline_at_unix_ms"].is_null());
    drop(store);

    let store = Store::open(&path, "owner").unwrap();
    rebuild(&store, &[&source, &target]);
    assert_link(&current(&store, &source), &current(&store, &target));
    assert_eq!(current(&store, &target).after, source.after);
    assert_eq!(current(&store, &target).deadline_at_unix_ms, None);
    assert!(!store.resume_superseding_mission_run(&target.subject).unwrap());
    finish_cancellation(&store, &source);
    assert!(store.resume_superseding_mission_run(&target.subject).unwrap());
    assert_eq!(current(&store, &target).deadline_at_unix_ms, None);
    // The prerequisite was also superseded by the publication. Acknowledge its ordinary
    // cancellation before accepting the synthetic dependency step's completion.
    finish_cancellation(&store, &prerequisite);
    let after = target.steps.iter().find(|step| step.step == "after-run").unwrap();
    store.set_step_state(&after.subject, "completed", None).unwrap();
    let admitted = current(&store, &target);
    assert!(admitted.deadline_at_unix_ms.is_some());
    rebuild(&store, &[&target]);
    assert_eq!(current(&store, &target).deadline_at_unix_ms, admitted.deadline_at_unix_ms);
    assert_eq!(current(&store, &target).after, source.after);
}

#[test]
fn mission_supersession_replication_only_source_owner_creates_one_replacement() {
    let owner = Store::open_memory("owner").unwrap();
    let publisher = Store::open_memory("publisher").unwrap();
    publish_mission(&owner, ORIGINAL, "original");
    let source = start_source(&owner, None);
    receive_and_project(&publisher, "owner", &exchange_from(&owner, &Default::default()));
    publish_mission(&publisher, &ORIGINAL.replace("exit 1", "exit 0"), "remote-correction");
    publisher.supersede_mission_publications().unwrap();
    assert!(current(&publisher, &source).superseded_by.is_none());
    assert_eq!(current(&publisher, &source).phase, "normal");
    let exchange = exchange_from(&publisher, &Default::default());
    receive_and_project(&owner, "publisher", &exchange);
    // Receipt/projection alone is not the owner's runtime publication reconciliation.
    assert!(current(&owner, &source).superseded_by.is_none());
    assert!(owner.supersede_mission_publications().unwrap());
    let target = replacement(&owner, &source);
    assert_link(&current(&owner, &source), &target);
    assert_eq!(target.phase, "supersession-waiting");
    let created = owner.claims_for(&target.subject, Some("mission-run.created")).unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].origin, "owner");
    let index = owner.index().unwrap();
    owner.supersede_mission_publications().unwrap();
    assert_eq!(owner.index().unwrap(), index);
    requeue_heads(&owner);
    owner.supersede_mission_publications().unwrap();
    assert_eq!(replacement(&owner, &source).subject, target.subject);
    assert_eq!(owner.claims_for(&source.subject, Some("mission-run.supersede-linked")).unwrap().len(), 1);
    receive_and_project(&publisher, "owner", &exchange_from(&owner, &Default::default()));
    publisher.supersede_mission_publications().unwrap();
    assert_eq!(replacement(&publisher, &source).subject, target.subject);
    assert_link(&current(&publisher, &source), &current(&publisher, &target));
    assert!(!publisher.resume_superseding_mission_run(&target.subject).unwrap());
    assert_queue_converged(&owner);
    assert_queue_converged(&publisher);
}

#[test]
fn mission_supersession_refusal_deduplicates_after_requeue_reopen_and_large_backlog_then_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let store = Store::open(&path, "owner").unwrap();
    // More than one page of unrelated selected heads must not hide the refused head.
    for index in 0..70 {
        publish_mission(&store, &ORIGINAL.replace("\"retry-work\"", &format!("\"historical-{index}\"")), &format!("history-{index}"));
    }
    publish_mission(&store, ORIGINAL, "original");
    let source = start_source(&store, None);
    let refused_revision = publish_mission(&store, &ORIGINAL.replace("input \"value\"", "input \"replacement\""), "incompatible");
    let refused = current(&store, &source);
    assert_eq!(refused.status, "running");
    assert_eq!(refused.phase, "normal");
    assert_eq!(refused.generation, source.generation);
    assert_eq!(refused.inputs, source.inputs);
    assert!(refused.superseded_by.is_none());
    assert!(refused.supersession_fault.is_some());
    let faults = store.claims_for(&source.subject, Some("reconcile.fault")).unwrap();
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].body["fields"]["scope"], "mission/supersession");
    assert_eq!(faults[0].body["fields"]["status"], "faulted");
    let reason = faults[0].body["fields"]["reason"].as_str().unwrap();
    assert!(reason.contains(&source.revision));
    assert!(reason.contains(&refused_revision.revision));
    assert!(reason.contains("missing [replacement]; extra [value]"));
    assert!(faults[0].body["fields"]["publication"].is_string());
    requeue_heads(&store);
    drop(store);

    let store = Store::open(&path, "owner").unwrap();
    rebuild(&store, &[&source]);
    assert!(store.supersede_mission_publications().unwrap());
    assert_queue_converged(&store);
    assert_eq!(store.claims_for(&source.subject, Some("reconcile.fault")).unwrap().len(), 1);
    assert!(current(&store, &source).superseded_by.is_none());
    assert_eq!(current(&store, &source).phase, "normal");
    requeue_heads(&store);
    store.supersede_mission_publications().unwrap();
    assert_queue_converged(&store);
    assert_eq!(store.claims_for(&source.subject, Some("reconcile.fault")).unwrap().len(), 1);
    let compatible = publish_mission(&store, &ORIGINAL.replace("exit 1", "exit 0"), "compatible-recovery");
    let target = replacement(&store, &source);
    assert_eq!(target.revision, compatible.revision);
    assert_link(&current(&store, &source), &target);
    assert!(current(&store, &source).supersession_fault.is_none());
    let faults = store.claims_for(&source.subject, Some("reconcile.fault")).unwrap();
    assert_eq!(faults.len(), 2);
    assert_eq!(faults.iter().filter(|claim| claim.body["fields"]["status"] == "faulted").count(), 1);
    assert_eq!(faults.iter().filter(|claim| claim.body["fields"]["status"] == "recovered").count(), 1);
    requeue_heads(&store);
    store.supersede_mission_publications().unwrap();
    assert_queue_converged(&store);
    assert_eq!(replacement(&store, &source).subject, target.subject);
    assert_eq!(store.claims_for(&source.subject, Some("reconcile.fault")).unwrap().len(), 2);
}

fn publish_owned_revision(store: &Store, source: &str, sequence: u64) {
    use fractal_store::store::{owned_sets, publication};
    let intent = crate::graph::parse_owned_set_intent(source, &store.origin).unwrap();
    let expected_set = store.latest_claim("owned-set/supersession", Some("owned-set.revised"))
        .unwrap().map_or_else(|| "absent".into(), |claim| {
            claim.body["fields"]["revision"].as_str().unwrap().to_owned()
        });
    let mut options = owned_sets::Options {
        set: "supersession".into(), expected_set,
        source: owned_sets::Source {
            repository: "example/missions".into(), r#ref: "refs/heads/main".into(),
            sha: format!("{sequence:040x}"), sequence,
        },
        rollout: None, adopt: Default::default(), allow_empty: false,
        confirm_retire: None, expected_subjects: Default::default(), policy: None,
    };
    let preview = {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let preview = owned_sets::plan_tx(&transaction, &intent, &options).unwrap().preview;
        transaction.rollback().unwrap();
        preview
    };
    assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
    options.confirm_retire = Some(preview.digest);
    options.expected_subjects = preview.expected_subjects;
    store.apply_as_impl(
        &intent, &options.expected_subjects, &format!("owned-revision-{sequence}"),
        Some("person/operator"), None,
        publication::CommitChecks { owned: Some(&options), ..Default::default() },
    ).unwrap();
}

#[test]
fn mission_supersession_owned_head_promotion_after_publication_scan_is_not_lost() {
    let publisher = Store::open_memory("publisher").unwrap();
    let owner = Store::open_memory("owner").unwrap();
    publish_owned_revision(&publisher, ORIGINAL, 1);
    receive_and_project(&owner, "publisher", &exchange_from(&publisher, &Default::default()));
    let source = start_source(&owner, None);
    owner.supersede_mission_publications().unwrap();
    publish_owned_revision(&publisher, &ORIGINAL.replace("exit 1", "exit 0"), 2);
    let through_second = exchange_from(&publisher, &Default::default());
    let second_sequence = through_second.envelopes.iter()
        .filter(|envelope| envelope.writer == "publisher")
        .map(|envelope| envelope.sequence).max().unwrap();
    publish_owned_revision(&publisher, &ORIGINAL.replace("exit 1", "exit 2"), 3);
    let newest = publisher.mission_spec("retry-work", None).unwrap().unwrap();
    let newest_publication = publisher.latest_claim("mission/retry-work", Some("mission.published"))
        .unwrap().unwrap();
    let mut third_only = exchange_from(&publisher, &Default::default());
    // Delay complete, authentic envelopes, never rewrite a claim or split an atomic batch.
    third_only.envelopes.retain(|envelope| {
        envelope.writer != "publisher" || envelope.sequence > second_sequence
    });
    third_only.inventory.envelopes.retain(|envelope| {
        envelope.writer != "publisher" || envelope.sequence > second_sequence
    });
    receive_and_project(&owner, "publisher", &third_only);
    // The regression requires publication authority already admitted before selection.
    // A merely parked transport record would not exercise the lost-head-change case.
    assert!(owner.claims_for("mission/retry-work", Some("mission.published")).unwrap()
        .iter().any(|claim| claim.id == newest_publication.id));
    assert!(owner.mission_spec("retry-work", None).unwrap().is_none(),
        "the incomplete owned-set chain withdraws its selected head until the missing receipt arrives");
    owner.supersede_mission_publications().unwrap();
    assert_queue_converged(&owner);
    assert!(current(&owner, &source).superseded_by.is_none());

    receive_and_project(&owner, "publisher", &through_second);
    assert_eq!(owner.mission_spec("retry-work", None).unwrap().unwrap().revision, newest.revision);
    assert!(owner.supersede_mission_publications().unwrap());
    let target = replacement(&owner, &source);
    assert_eq!(target.revision, newest.revision);
    assert_link(&current(&owner, &source), &target);
    assert_queue_converged(&owner);
    requeue_heads(&owner);
    owner.supersede_mission_publications().unwrap();
    assert_eq!(replacement(&owner, &source).subject, target.subject);
    assert_eq!(owner.claims_for(&source.subject, Some("mission-run.supersede-linked")).unwrap().len(), 1);
}
