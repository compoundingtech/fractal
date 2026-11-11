use super::*;
use crate::store::tests_shared_missions::publish_carry;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture(parts: usize) -> (Store, Vec<Aggregate>) {
    let store = Store::open_memory("node").unwrap();
    publish_carry(
        &store,
        "version 2\nmission \"carry\" state=\"ready\" {\n concurrent-runs\n goal \"Exercise fenced decisions.\"\n step \"work\" { agentless }\n}\n",
        "publish-fence",
    );
    let aggregates = (0..parts)
        .map(|index| {
            let run = store.create_mission_run_inner(
                &crate::model::MissionRunRequest {
                    mission: "carry".into(),
                    revision: None,
                    workspace: "/tmp".into(),
                    requester: Some("person/operator".into()),
                    mode: None,
                    inputs: Default::default(),
                    idempotency_key: format!("fence-run-{index}"),
                },
                None,
                None,
                None,
            ).unwrap();
            let aggregate = run_part_of_tx(&store.connection.write(), &run.subject)
                .unwrap().unwrap();
            assert!(matches!(aggregate, Aggregate::RunPart(_)));
            aggregate
        })
        .collect();
    (store, aggregates)
}

fn mark_pending(connection: &Connection, aggregate: &Aggregate) {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hf35e185e6103 owner=fractal_store::store::run_fence::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2)",
        params![pending_run_rebuild_key(aggregate).unwrap(), serde_json::to_string(aggregate).unwrap()],
    ).unwrap();
}

fn mark_all_pending(store: &Store, aggregates: &[Aggregate]) {
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    for aggregate in aggregates {
        mark_pending(&transaction, aggregate);
    }
    transaction.commit().unwrap();
}

fn pending_count(connection: &Connection) -> usize {
    pending_run_rebuilds_tx(connection).unwrap().len()
}

fn input(key: &str) -> ClaimInput {
    ClaimInput {
        subject: "custom/fence/action".into(),
        kind: "custom.fence.decided".into(),
        actor: Some("person/operator".into()),
        fields: BTreeMap::new(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: Some(key.into()),
    }
}

#[test]
fn each_pending_aggregate_commits_and_returns_its_writer_before_the_next_turn() {
    let (store, aggregates) = fixture(2);
    mark_all_pending(&store, &aggregates);
    let mut counts = Vec::new();
    let actions = AtomicUsize::new(0);
    execute_with_turn_hook(
        &store.graph,
        Some(RunMutationFence::All),
        None,
        |transaction| -> Result<(), FractalError> {
            assert_eq!(pending_count(transaction), 0);
            assert_eq!(actions.fetch_add(1, Ordering::SeqCst), 0);
            Ok(())
        },
        || {
            let writer = store.connection.write();
            assert!(writer.is_autocommit(), "the rebuild transaction must already be committed");
            let remaining = pending_count(&writer);
            assert_eq!(pending_count(&store.readers.get()), remaining);
            counts.push(remaining);
        },
    ).unwrap();
    assert_eq!(counts, [1, 0], "each committed writer turn drains exactly one independent part");
    assert_eq!(actions.load(Ordering::SeqCst), 1);
}

#[test]
fn final_recheck_drains_a_marker_committed_between_turns() {
    let (store, aggregates) = fixture(2);
    mark_all_pending(&store, &aggregates[..1]);
    let mut turns = 0;
    let actions = AtomicUsize::new(0);
    execute_with_turn_hook(
        &store.graph,
        Some(RunMutationFence::All),
        None,
        |transaction| -> Result<(), FractalError> {
            assert_eq!(pending_count(transaction), 0);
            actions.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        || {
            turns += 1;
            if turns == 1 {
                let mut writer = store.connection.write();
                let transaction = writer.transaction().unwrap();
                mark_pending(&transaction, &aggregates[1]);
                transaction.commit().unwrap();
            }
        },
    ).unwrap();
    assert_eq!(turns, 2);
    assert_eq!(actions.load(Ordering::SeqCst), 1);
}

#[test]
fn zero_budget_refuses_before_a_turn_and_unbounded_retry_runs_the_action_once() {
    let (store, aggregates) = fixture(3);
    mark_all_pending(&store, &aggregates);
    let actions = AtomicUsize::new(0);
    let mut turns = 0;
    let error = execute_with_turn_hook(
        &store.graph,
        Some(RunMutationFence::All),
        Some(Duration::ZERO),
        |_| -> Result<(), FractalError> {
            actions.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        || turns += 1,
    ).unwrap_err();
    assert_eq!(error.code, "database-busy");
    assert_eq!(error.details["reason"], "run-projection-drain");
    assert_eq!(error.details["pending_parts"], 3);
    assert_eq!(turns, 0);
    assert_eq!(actions.load(Ordering::SeqCst), 0);
    assert_eq!(pending_count(&store.readers.get()), 3);

    execute(
        &store.graph,
        Some(RunMutationFence::All),
        None,
        |transaction| -> Result<(), FractalError> {
            assert_eq!(pending_count(transaction), 0);
            actions.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    ).unwrap();
    assert_eq!(actions.load(Ordering::SeqCst), 1);
    assert_eq!(pending_count(&store.readers.get()), 0);
}

#[test]
fn append_retry_with_the_same_idempotency_key_returns_the_original_claim() {
    let (store, aggregates) = fixture(3);
    mark_all_pending(&store, &aggregates);
    let request = input("fenced-append-retry");
    let refused_actions = AtomicUsize::new(0);
    let error = execute(
        &store.graph,
        Some(RunMutationFence::All),
        Some(Duration::ZERO),
        |transaction| -> Result<ClaimRecord, FractalError> {
            refused_actions.fetch_add(1, Ordering::SeqCst);
            append_claim_tx(
                transaction,
                &store.origin,
                &request.subject,
                &request.kind,
                request.actor.as_deref(),
                &json!({"fields": request.fields, "idempotency_key": request.idempotency_key}),
                &[],
                None,
            ).map_err(internal)
        },
    ).unwrap_err();
    assert_eq!(error.code, "database-busy");
    assert_eq!(error.details["reason"], "run-projection-drain");
    assert_eq!(refused_actions.load(Ordering::SeqCst), 0);
    assert!(store.claims_for(&request.subject, Some(&request.kind)).unwrap().is_empty());
    assert_eq!(pending_count(&store.readers.get()), 3);
    let admission = |connection: &Connection| -> Result<(), FractalError> {
        assert_eq!(pending_count(connection), 0);
        Ok(())
    };
    let first = store.append_claim_with_run_projection(&request, &admission).unwrap();
    // A later replay must still fence any newly pending work without appending again.
    mark_all_pending(&store, &aggregates);
    let retry = store.append_claim_with_run_projection(&request, &admission).unwrap();
    assert_eq!(retry.id, first.id);
    assert_eq!(retry.store_index, first.store_index);
    assert_eq!(retry.body, first.body);
    assert_eq!(store.claims_for(&request.subject, Some(&request.kind)).unwrap().len(), 1);
    assert_eq!(pending_count(&store.readers.get()), 0);
}

#[test]
fn append_preparation_and_admission_read_the_rebuilt_projection() {
    let (store, aggregates) = fixture(1);
    let Aggregate::RunPart(run) = &aggregates[0] else {
        panic!("fixture must contain a run part");
    };
    let phase = |connection: &Connection| -> String {
        connection.query_row("SELECT phase FROM mission_runs WHERE id=?1", [run], |row| row.get(0)).unwrap()
    };
    let expected = phase(&store.readers.get());
    assert_ne!(expected, "revision-draining");
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h2cebc723b81a owner=fractal_store::store::run_fence::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET phase='revision-draining' WHERE id=?1", [run],
    ).unwrap();
    mark_all_pending(&store, &aggregates);
    assert_eq!(phase(&store.readers.get()), "revision-draining");
    let preparations = AtomicUsize::new(0);
    let admissions = AtomicUsize::new(0);
    let prepare = |connection: &Connection, request: &ClaimInput| -> Result<BTreeMap<String, Value>, FractalError> {
        assert_eq!(pending_count(connection), 0);
        assert_eq!(phase(connection), expected);
        preparations.fetch_add(1, Ordering::SeqCst);
        let mut fields = request.fields.clone();
        fields.insert("observed_phase".into(), json!(phase(connection)));
        Ok(fields)
    };
    let admission = |connection: &Connection| -> Result<(), FractalError> {
        assert_eq!(pending_count(connection), 0);
        assert_eq!(phase(connection), expected);
        admissions.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    let (claim, inserted) = append_claim_with_commit_context(
        &store.graph, &input("fresh-append"), None, None, None, None,
        ClaimCommitContext {
            all_run_projections: true,
            admission: Some(&admission),
            prepare_request_fields: Some(&prepare),
            ..Default::default()
        },
    ).unwrap();
    assert!(inserted);
    assert_eq!(preparations.load(Ordering::SeqCst), 1);
    assert_eq!(admissions.load(Ordering::SeqCst), 1);
    assert_eq!(claim.body["fields"]["observed_phase"], expected);
}

#[test]
fn run_fence_drains_its_part_without_consuming_an_unrelated_part() {
    let (store, aggregates) = fixture(2);
    mark_all_pending(&store, &aggregates);
    let Aggregate::RunPart(run) = &aggregates[0] else {
        panic!("fixture must contain a run part");
    };
    let subject = format!("mission-run/{run}");
    execute(
        &store.graph,
        Some(RunMutationFence::Run(&subject)),
        None,
        |transaction| -> Result<(), FractalError> {
            assert!(pending_run_for_subject_tx(transaction, &subject)?.is_none());
            assert_eq!(pending_run_rebuilds_tx(transaction)?, BTreeSet::from([aggregates[1].clone()]));
            Ok(())
        },
    ).unwrap();
    assert_eq!(pending_count(&store.readers.get()), 1);
}

#[test]
fn committed_drains_survive_a_later_action_refusal() {
    let (store, aggregates) = fixture(3);
    mark_all_pending(&store, &aggregates);
    let actions = AtomicUsize::new(0);
    let error = execute(
        &store.graph,
        Some(RunMutationFence::All),
        None,
        |transaction| -> Result<(), FractalError> {
            assert_eq!(pending_count(transaction), 0);
            actions.fetch_add(1, Ordering::SeqCst);
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-hfcad3b11e6e1 owner=fractal_store::store::run_fence::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('fence-test-action','rolled-back')",
                [],
            ).map_err(internal)?;
            Err(FractalError::new("stale-subject", "the fenced action's expected subject changed"))
        },
    ).unwrap_err();
    assert_eq!(error.code, "stale-subject");
    assert_eq!(actions.load(Ordering::SeqCst), 1);
    let reader = store.readers.get();
    assert_eq!(pending_count(&reader), 0, "earlier rebuild commits survive the action rollback");
    let action_persisted: bool = reader.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='fence-test-action')",
        [],
        |row| row.get(0),
    ).unwrap();
    assert!(!action_persisted);
}
