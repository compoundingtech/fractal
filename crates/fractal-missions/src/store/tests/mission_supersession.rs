use super::*;
use super::run_retry::{ORIGINAL, fail, retry_options, start};

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
        /* sqlite-large-value: allow-for-now lv-h61cdeaaf5336 owner=fractal_missions::store::tests::mission_supersession TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_mission_supersession_heads_v3(mission_id) SELECT mission_id FROM mission_definitions",
        [],
    ).unwrap();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hb6aa7884a00c owner=fractal_missions::store::tests::mission_supersession TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value=MAX(CAST(value AS INTEGER),(SELECT COALESCE(MAX(seq),0) FROM local_mission_supersession_heads_v3))
         WHERE key='local_mission_supersession_heads_v3_sequence'", [],
    ).unwrap();
}

fn assert_queue_converged(store: &Store) {
    let pending: u64 = store.readers.get().query_row(
        "SELECT COUNT(*) FROM local_mission_supersession_heads_v3",
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

#[test]
fn mission_supersession_equivalent_publication_preserves_pins_and_refusal_episode() {
    for pinned in [true, false] {
        let owner = Store::open_memory("owner").unwrap();
        let peer = Store::open_memory("peer").unwrap();
        let original = ORIGINAL.replace("host \"local\"", "host \"orchid\"");
        publish_mission(&owner, &original, "equivalent-original");
        let source = start(&owner);
        let source = if pinned { fail(&owner, &source) } else { source };
        let fixed = original.replace("exit 1", "exit 0");
        let fixed = if pinned { fixed } else {
            fixed.replace("input \"value\"", "input \"replacement\"")
        };
        let selected = publish_mission(&owner, &fixed, "equivalent-fixed");
        let run = if pinned {
            owner.retry_mission_run(&source.subject, &retry_options("equivalent-pin", true)).unwrap()
        } else { source };
        let head = || owner.readers.get().query_row(
            "SELECT claim_id FROM mission_definitions WHERE mission_id='retry-work'",
            [], |row| row.get::<_, String>(0),
        ).unwrap();
        let previous_head = head();
        let faults = owner.claims_for(&run.subject, Some("reconcile.fault")).unwrap().len();
        assert_eq!(faults, usize::from(!pinned));

        receive_and_project(&peer, "owner", &exchange_from(&owner, &Default::default()));
        // Publish a genuine later receipt for identical code, causally after the owner's
        // current head, without splitting batches or fabricating projection state.
        publish_mission(&peer, &fixed.replace("exit 0", "exit 2"), "equivalent-intermediate");
        publish_mission(&peer, &fixed, "equivalent-republication");
        receive_and_project(&owner, "peer", &exchange_from(&peer, &Default::default()));
        assert_eq!(owner.mission_spec("retry-work", None).unwrap().unwrap().revision, selected.revision);
        assert_ne!(head(), previous_head, "the selected receipt must actually change");
        owner.supersede_mission_publications().unwrap();
        assert!(current(&owner, &run).superseded_by.is_none());
        assert_eq!(owner.claims_for(&run.subject, Some("reconcile.fault")).unwrap().len(), faults);
        assert_queue_converged(&owner);
        if pinned {
            let incompatible = fixed.replace("input \"value\"", "input \"replacement\"");
            publish_mission(&owner, &incompatible, "equivalent-later-incompatible");
            assert!(current(&owner, &run).supersession_fault.is_some());
            let restored = publish_mission(&owner, &fixed, "equivalent-restored-compatible");
            assert_eq!(restored.revision, selected.revision);
            let target = replacement(&owner, &run);
            assert_eq!(target.revision, restored.revision);
            assert!(current(&owner, &run).supersession_fault.is_none());
        }
    }
}

#[test]
fn mission_supersession_when_idle_drains_without_dispatching_a_new_generation() {
    let store = Store::open_memory("owner").unwrap();
    let definition = ORIGINAL.replace("state=\"ready\"", "state=\"ready\" revision-cutover=\"when-idle\"");
    let first = publish_mission(&store, &definition, "idle-original");
    let source = start_source(&store, None);
    let work = source.steps.iter().find(|step| step.step == "prepare").unwrap();
    store.set_step_state(&work.subject, "working", None).unwrap();
    let target = publish_mission(&store, &definition.replace("exit 1", "exit 0"), "idle-corrected");
    assert_eq!(current(&store, &source).phase, "normal");
    assert!(current(&store, &source).superseded_by.is_none());
    assert_eq!(store.supersede_idle_mission_run(&current(&store, &source), &first).unwrap(), Some(true));
    let draining = current(&store, &source);
    assert_eq!(draining.phase, "revision-draining");
    assert_eq!(draining.generation, source.generation);
    assert!(draining.superseded_by.is_none());
    assert_eq!(store.supersede_idle_mission_run(&draining, &first).unwrap(), Some(false));
    rebuild(&store, &[&source]);
    assert_eq!(current(&store, &source).phase, "revision-draining");
    store.set_step_state(&work.subject, "completed", None).unwrap();
    assert_eq!(store.supersede_idle_mission_run(&current(&store, &source), &first).unwrap(), Some(true));
    let fresh = replacement(&store, &source);
    assert_eq!(fresh.revision, target.revision);
    assert_eq!(current(&store, &source).generation, source.generation);
    assert_link(&current(&store, &source), &fresh);
    assert_eq!(fresh.phase, "supersession-waiting");
    assert!(!store.resume_superseding_mission_run(&fresh.id).unwrap());
    store.set_mission_run_state(&source.id, "cancelled", "terminal", Some("idle work and cleanup finished")).unwrap();
    assert!(store.resume_superseding_mission_run(&fresh.id).unwrap());
    assert_eq!(current(&store, &fresh).phase, "normal");
}

#[test]
fn mission_supersession_when_idle_refusal_releases_the_source_and_recovers() {
    let store = Store::open_memory("owner").unwrap();
    let definition = ORIGINAL.replace("state=\"ready\"", "state=\"ready\" revision-cutover=\"when-idle\"");
    let first = publish_mission(&store, &definition, "idle-original");
    let source = start_source(&store, None);
    let work = source.steps.iter().find(|step| step.step == "prepare").unwrap();
    store.set_step_state(&work.subject, "working", None).unwrap();
    let incompatible = definition.replace("input \"value\"", "input \"renamed\"");
    let target = publish_mission(&store, &incompatible, "idle-incompatible");
    assert_eq!(store.supersede_idle_mission_run(&current(&store, &source), &first).unwrap(), Some(true));
    store.set_step_state(&work.subject, "completed", None).unwrap();
    assert_eq!(store.supersede_idle_mission_run(&current(&store, &source), &first).unwrap(), Some(true));
    let refused = current(&store, &source);
    assert_eq!(refused.phase, "normal");
    assert_eq!(refused.generation, source.generation);
    assert!(refused.superseded_by.is_none());
    let reason = refused.supersession_fault.as_deref().unwrap();
    assert!(reason.contains(&source.revision));
    assert!(reason.contains(&target.revision));
    assert_eq!(store.supersede_idle_mission_run(&refused, &first).unwrap(), None);
    let corrected = publish_mission(&store, &definition.replace("exit 1", "exit 0"), "idle-compatible");
    assert_eq!(store.supersede_idle_mission_run(&current(&store, &source), &first).unwrap(), Some(true));
    assert_eq!(replacement(&store, &source).revision, corrected.revision);
    assert!(current(&store, &source).supersession_fault.is_none());
}

#[test]
fn mission_supersession_respects_step_level_human_revision_policy() {
    let store = Store::open_memory("owner").unwrap();
    let protected = ORIGINAL.replace("step \"prepare\"", "step \"prepare\" revisions=\"human-only\"");
    let first = publish_mission(&store, &protected, "protected-original");
    let source = start_source(&store, None);
    publish_mission(&store, &protected.replace("exit 1", "exit 0"), "protected-corrected");
    let retained = current(&store, &source);
    assert_eq!(retained.generation, source.generation);
    assert_eq!(retained.revision, first.revision);
    assert_eq!(retained.phase, "normal");
    assert!(retained.superseded_by.is_none());
    assert_eq!(store.supersede_idle_mission_run(&retained, &first).unwrap(), None);
    assert!(retained.supersession_fault.is_none());
}

fn population(store: &Store, count: usize) -> Vec<MissionRunView> {
    publish_mission(store, ORIGINAL, "population-original");
    (0..count).map(|index| {
        store.create_mission_run(&MissionRunRequest {
            mission: "retry-work".into(), revision: None, workspace: "/work/retry".into(),
            requester: Some("person/requester".into()), mode: None,
            inputs: BTreeMap::from([("value".into(), "unchanged".into())]),
            idempotency_key: format!("population-{index}"),
        }).unwrap()
    }).collect()
}

#[test]
fn mission_supersession_single_head_publication_bounds_source_work_and_drains() {
    let store = Store::open_memory("owner").unwrap();
    let sources = population(&store, 193);
    let target = publish_mission(&store, &ORIGINAL.replace("exit 1", "exit 0"), "population-fixed");
    let linked = sources.iter().filter(|source| current(&store, source).superseded_by.is_some()).count();
    assert!(linked > 0 && linked <= 32,
        "a publication writer must process at most one source batch, not the whole mission: {linked}");
    store.supersede_mission_publications().unwrap();
    let mut replacements = BTreeSet::new();
    for source in &sources {
        let fresh = replacement(&store, source);
        assert_eq!(fresh.revision, target.revision);
        assert_link(&current(&store, source), &fresh);
        assert!(replacements.insert(fresh.subject.clone()));
    }
    assert!(!store.supersede_mission_publications().unwrap());
    for source in &sources {
        assert_eq!(store.claims_for(&replacement(&store, source).subject, Some("mission-run.created")).unwrap().len(), 1);
    }
}

#[test]
fn mission_supersession_single_head_sql_work_scales_linearly_without_warming() {
    fn work(count: usize) -> smallclaims::sqlite::work::SqliteWork {
        let store = Store::open_memory("owner").unwrap();
        let sources = population(&store, count);
        let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        publish_mission(&store, &ORIGINAL.replace("exit 1", "exit 0"), "population-fixed");
        store.supersede_mission_publications().unwrap();
        let work = scope.finish();
        assert!(sources.iter().all(|source| current(&store, source).superseded_by.is_some()));
        work
    }
    let small = work(64);
    let large = work(256);
    assert!(large.statements < small.statements * 6,
        "four times the population must not reload every active pinned definition for each replacement: {small:?} -> {large:?}");
    assert!(large.vm_steps < small.vm_steps * 6,
        "source paging and capacity admission must have bounded per-source work: {small:?} -> {large:?}");
}

#[test]
fn mission_supersession_partial_head_change_restarts_pins_and_refusals_without_starvation() {
    let store = Store::open_memory("owner").unwrap();
    let sources = population(&store, 129);
    publish_mission(&store, &ORIGINAL.replace("input \"value\"", "input \"replacement\""), "population-refused");
    assert!(sources.iter().any(|source| current(&store, source).supersession_fault.is_none()));
    let corrected = publish_mission(&store, &ORIGINAL.replace("exit 1", "exit 0"), "population-corrected");
    store.supersede_mission_publications().unwrap();
    for source in &sources {
        let fresh = replacement(&store, source);
        assert_eq!(fresh.revision, corrected.revision);
        assert!(current(&store, source).supersession_fault.is_none());
    }
    assert!(!store.supersede_mission_publications().unwrap());
}

#[test]
fn mission_supersession_large_population_capacity_refusal_is_atomic_and_bounded() {
    let store = Store::open_memory("owner").unwrap();
    let sources = population(&store, 97);
    publish_mission(&store, &ORIGINAL.replace("concurrent-runs", "concurrent-runs max=1"), "population-limited");
    store.supersede_mission_publications().unwrap();
    for source in &sources {
        let refused = current(&store, source);
        assert_eq!(refused.status, "running");
        assert_eq!(refused.generation, source.generation);
        assert!(refused.superseded_by.is_none());
        assert!(refused.supersession_fault.as_deref().unwrap().contains("mission-run-capacity"));
        assert_eq!(store.claims_for(&source.subject, Some("reconcile.fault")).unwrap().len(), 1);
    }
    assert!(!store.supersede_mission_publications().unwrap());
    assert_eq!(store.active_mission_runs_for_mission("retry-work").unwrap().len(), 97);
}

#[test]
fn mission_run_capacity_keeps_strict_pinned_limits_across_status_rebuild_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("capacity.sqlite");
    let store = Store::open(&path, "owner").unwrap();
    let bounded = ORIGINAL.replace("concurrent-runs", "concurrent-runs max=2")
        .replace("state=\"ready\"", "state=\"ready\" revisions=\"human-only\"");
    let first = publish_mission(&store, &bounded, "bounded-original");
    let request = |key: &str, revision: Option<String>| MissionRunRequest {
        mission: "retry-work".into(), revision, workspace: "/work/retry".into(),
        requester: Some("person/requester".into()), mode: None,
        inputs: BTreeMap::from([("value".into(), "unchanged".into())]),
        idempotency_key: key.into(),
    };
    let one = store.create_mission_run(&request("one", Some(first.revision.clone()))).unwrap();
    let two = store.create_mission_run(&request("two", Some(first.revision))).unwrap();
    publish_mission(&store, &ORIGINAL.replace("state=\"ready\"", "state=\"ready\" revisions=\"human-only\""), "unlimited-head");
    store.set_mission_run_state(&one.id, "blocked", "normal", None).unwrap();
    rebuild(&store, &[&one, &two]);
    assert_eq!(store.create_mission_run(&request("refused", None)).unwrap_err().code, "mission-run-capacity");
    drop(store);
    let store = Store::open(&path, "owner").unwrap();
    assert_eq!(store.create_mission_run(&request("refused", None)).unwrap_err().code, "mission-run-capacity");
    store.request_mission_run_cancellation(&one.id, "release its capacity").unwrap();
    store.create_mission_run(&request("admitted", None)).unwrap();
    assert_eq!(store.create_mission_run(&request("still-refused", None)).unwrap_err().code, "mission-run-capacity");
    store.request_mission_run_cancellation(&two.id, "release the last strict pin").unwrap();
    store.create_mission_run(&request("unlimited-again", None)).unwrap();
}

#[test]
fn mission_run_capacity_admission_sql_is_population_independent() {
    fn work(count: usize) -> smallclaims::sqlite::work::SqliteWork {
        let store = Store::open_memory("owner").unwrap();
        let sources = population(&store, count);
        let mission = store.mission_spec("retry-work", Some(&sources[0].revision)).unwrap().unwrap();
        let reader = store.readers.get();
        let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        for _ in 0..8 {
            fractal_store::store::enforce_mission_run_capacity(&reader, &mission).unwrap();
        }
        scope.finish()
    }
    let small = work(32);
    let large = work(256);
    assert!(large.statements <= small.statements + 8,
        "admission must not load one definition per active run: {small:?} -> {large:?}");
    assert!(large.vm_steps <= small.vm_steps * 2,
        "admission must seek the maintained count and minimum, not fold the active population: {small:?} -> {large:?}");
}

#[test]
fn mission_supersession_targeted_dispatch_check_reaches_unpaged_sources_but_preserves_new_pins() {
    let store = Store::open_memory("owner").unwrap();
    let sources = population(&store, 97);
    let selected = publish_mission(&store, &ORIGINAL.replace("exit 1", "exit 0"), "dispatch-head");
    let unpaged = sources.iter().find(|source| current(&store, source).superseded_by.is_none()).unwrap();
    assert!(store.supersede_current_mission_run(unpaged).unwrap());
    assert_eq!(replacement(&store, unpaged).revision, selected.revision);
    let pin = store.create_mission_run(&MissionRunRequest {
        mission: "retry-work".into(), revision: Some(unpaged.revision.clone()),
        workspace: "/work/retry".into(), requester: Some("person/requester".into()), mode: None,
        inputs: BTreeMap::from([("value".into(), "unchanged".into())]),
        idempotency_key: "post-publication-pin".into(),
    }).unwrap();
    assert!(!store.supersede_current_mission_run(&pin).unwrap());
    store.supersede_mission_publications().unwrap();
    assert!(current(&store, &pin).superseded_by.is_none());
    assert_eq!(current(&store, &pin).revision, unpaged.revision);
}

#[test]
fn mission_supersession_start_fence_is_read_only_and_releases_recorded_refusals() {
    let store = Store::open_memory("owner").unwrap();
    let sources = population(&store, 97);
    publish_mission(&store, &ORIGINAL.replace("input \"value\"", "input \"replacement\""), "start-fence-incompatible");
    let unpaged = sources.iter().find(|source| current(&store, source).supersession_fault.is_none()).unwrap();
    let index = store.index().unwrap();
    assert!(store.mission_run_supersession_pending_tx(&store.readers.get(), unpaged).unwrap());
    assert_eq!(store.index().unwrap(), index, "the start predicate must never write a refusal or link");
    let step = unpaged.steps.iter().find(|step| step.step == "prepare").unwrap();
    assert!(store.set_step_state(&step.subject, "working", None).unwrap_err().to_string().contains("automatic revision cutover"));
    assert!(store.issue_run_credential(&step.subject, step.attempt, 60_000).is_err());
    assert_eq!(store.run_credentials_issued_for_tests(&step.subject), 0);
    assert_eq!(current(&store, unpaged).steps.iter().find(|candidate| candidate.subject == step.subject).unwrap().status, step.status);
    assert!(store.supersede_current_mission_run(unpaged).unwrap());
    let refused = current(&store, unpaged);
    assert!(refused.supersession_fault.is_some());
    assert!(refused.superseded_by.is_none());
    assert!(!store.mission_run_supersession_pending_tx(&store.readers.get(), &refused).unwrap());
    assert!(store.set_step_state(&step.subject, "working", None).unwrap());
    let credential = store.issue_run_credential(&step.subject, step.attempt, 60_000).unwrap();
    assert_eq!(store.run_binding(&credential).unwrap().run, unpaged.subject);
    publish_mission(&store, &ORIGINAL.replace("exit 1", "exit 0"), "start-fence-corrected");
    while store.supersede_mission_publications().unwrap() {}
    assert!(current(&store, unpaged).superseded_by.is_some());
}

#[test]
fn mission_supersession_program_dispatch_credential_is_fenced_before_its_source_page() {
    let store = Store::open_memory("owner").unwrap();
    let sources = population(&store, 97);
    let source = sources.iter().max_by(|left, right| left.id.cmp(&right.id)).unwrap();
    let step = source.steps.iter().find(|step| step.step == "prepare").unwrap();
    store.set_step_state(&step.subject, "working", None).unwrap();
    publish_mission(&store, &ORIGINAL.replace("exit 1", "exit 0"), "credential-head");
    assert!(current(&store, source).superseded_by.is_none());
    let error = store.issue_run_credential(&step.subject, step.attempt, 60_000).unwrap_err();
    assert!(error.to_string().contains("automatic revision cutover"));
    assert_eq!(store.run_credentials_issued_for_tests(&step.subject), 0);
}
