#![allow(unused_imports)]
use super::*;
use smallclaims::store::checkpoint_agreement::*;
use st3_store::store::checkpoint_agreement_tests_shared_attention_tests::{FLEET, claim_ids, context, kinds, observe, stable_cuts, stable_pair, step, sync};


#[test]
fn a_silent_participant_holds_everything_up_until_a_person_excuses_it() {
    let scratch = tempfile::tempdir().unwrap();
    let first = context(scratch.path(), 0);
    let [alder, birch, cedar] =
        ["alder", "birch", "cedar"].map(|name| Store::open_memory(name).unwrap());
    observe(&alder, 2);
    observe(&birch, 2);
    // Cedar is an old build, or away: it wrote once and never seals.
    observe(&cedar, 1);
    sync(&[&alder, &birch, &cedar]);
    step(&alder, &first);
    step(&birch, &first);
    sync(&[&alder, &birch]);
    for _ in 0..3 {
        assert!(!kinds(&step(&alder, &first)).contains(&"verified"));
        assert!(!kinds(&step(&birch, &first)).contains(&"verified"));
    }

    // Three days after it became due, the first participant to have sealed asks a person.
    let late = CheckpointContext {
        now_unix_ms: first.now_unix_ms + CHECKPOINT_ATTENTION_AFTER_MS,
        ..first.clone()
    };
    // The newest due checkpoint moved on; seal it first.
    step(&alder, &late);
    step(&birch, &late);
    sync(&[&alder, &birch]);
    step(&alder, &late);
    let items = alder
        .checkpoint_attention_items(Some("person/operator"), late.now_unix_ms)
        .unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].title.contains("cedar"));
    assert_eq!(
        serde_json::to_value(&items).unwrap(),
        serde_json::to_value(
            birch
                .checkpoint_attention_items(Some("person/operator"), late.now_unix_ms)
                .unwrap()
        )
        .unwrap()
    );
    assert!(alder.attention_requests(None, true).unwrap().is_empty());

    // An agent cannot excuse it.
    assert!(
        alder
            .excuse_checkpoint_writer(&CheckpointExcuseRequest {
                writer: "cedar".into(),
                reason: "away".into(),
                actor: "agent/alder.worker".into(),
            })
            .is_err()
    );
    alder
        .excuse_checkpoint_writer(&CheckpointExcuseRequest {
            writer: "cedar".into(),
            reason: "the laptop is in a drawer".into(),
            actor: "person/operator".into(),
        })
        .unwrap();
    sync(&[&alder, &birch]);
    for node in [&alder, &birch] {
        assert_eq!(kinds(&step(node, &late)), ["sealed"]);
    }
    sync(&[&alder, &birch]);
    for node in [&alder, &birch] {
        assert_eq!(kinds(&step(node, &late)), ["verified"]);
    }
    sync(&[&alder, &birch]);
    let cut = newest_due_cut(late.now_unix_ms);
    assert_eq!(stable_cuts(&alder), [cut]);
    assert_eq!(kinds(&step(&alder, &late)), ["trimmed"]);
    assert_eq!(kinds(&step(&birch, &late)), ["trimmed"]);

    // Cedar comes back. What it wrote while away replicates. It adopts the checkpoint it
    // missed, and its next seal ends its excusal, so the next checkpoint waits for it.
    observe(&cedar, 1);
    sync(&[&alder, &birch, &cedar]);
    let next = CheckpointContext {
        now_unix_ms: late.now_unix_ms + DAY_MS,
        ..late.clone()
    };
    assert_eq!(kinds(&step(&cedar, &next)), ["manifest-needed"]);
    let need = cedar.checkpoint_manifest_need().unwrap().unwrap();
    let manifest = alder
        .checkpoint_manifest(&need.checkpoint, need.cut_unix_ms)
        .unwrap();
    assert_eq!(
        kinds(&cedar.adopt_checkpoint(&manifest).unwrap()),
        ["trimmed"]
    );
    assert_eq!(claim_ids(&cedar), claim_ids(&alder));
    assert_eq!(kinds(&step(&cedar, &next)), ["sealed"]);
    sync(&[&alder, &birch, &cedar]);
    assert!(
        alder
            .checkpoint_status(next.now_unix_ms, &[])
            .unwrap()
            .excused
            .is_empty()
    );
    let actions = step(&alder, &next);
    let CheckpointAction::Sealed { participants, .. } = &actions[0] else {
        panic!("{actions:?}");
    };
    assert!(participants.contains("cedar"));
}

/// A trim stops part way through recording, and a repair then changes what the node seals before
/// the cut, so its own plan no longer gives the drop it verified. The tombstones it recorded go
/// before it turns to a peer's manifest.
#[test]
fn a_plan_that_changes_after_an_interrupted_recording_leaves_no_tombstones_behind() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let cut = newest_due_cut(context.now_unix_ms);
    let path = scratch.path().join("alder.sqlite3");
    let alder = Store::open(&path, "alder").unwrap();
    let birch = Store::open_memory("birch").unwrap();
    let checkpoint = stable_pair(&alder, &birch, &context);
    assert_eq!(kinds(&step(&birch, &context)), ["trimmed"]);
    alder.set_trim_chunk_envelopes(2);
    alder.set_trim_row_cost(std::time::Duration::from_millis(12));
    alder.set_trim_fault(Some(TrimFault::AfterTombstoneChunk(1)));
    assert!(alder.checkpoint_step(&context).is_err());
    drop(alder);

    let alder = Store::open(&path, "alder").unwrap();
    let recorded = |store: &Store| -> u64 {
        store.readers.get().query_row(
            "SELECT (SELECT COUNT(*) FROM checkpoint_envelopes) + (SELECT COUNT(*) FROM checkpoint_claims)",
            [], |row| row.get(0),
        ).unwrap()
    };
    assert!(recorded(&alder) > 0, "the crash left tombstones recorded");
    let manifest = birch.checkpoint_manifest(&checkpoint, cut).unwrap();
    alder.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hb6a2de2b0e78 owner=st3_attention::store::checkpoint_agreement_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired' WHERE claim_id=?1",
        [&manifest.claims[0].id],
    ).unwrap();
    assert_eq!(kinds(&step(&alder, &context)), ["manifest-needed"]);
    assert_eq!(recorded(&alder), 0, "no tombstone outlives the plan it came from");
    assert_eq!(
        smallclaims::store::checkpoint_trim::local_checkpoint(&alder.readers.get(), &checkpoint)
            .unwrap()
            .map(|(state, _)| state)
            .as_deref(),
        Some("plan-changed")
    );
    // Adopting the peer's manifest from here is what any node does that cannot trim from its
    // own plan; `a_node_that_did_not_take_part_adopts_the_manifest_and_nothing_else` covers it.
    assert_eq!(alder.checkpoint_manifest_need().unwrap().map(|need| need.checkpoint), Some(checkpoint));
}

#[test]
fn the_agent_card_fold_matches_per_subject_reductions_after_a_trim() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let [alder, birch] = ["alder", "birch"].map(|name| Store::open_memory(name).unwrap());
    for (store, seat) in [(&alder, "agent/alder.keeper"), (&birch, "agent/birch.keeper")] {
        for (kind, fields) in [
            ("runtime.observed", json!({"status": "running", "runtime_id": seat,
                "incarnation_id": "keeper-1"})),
            ("harness.observed", json!({"state": "working", "driver": "omp",
                "incarnation_id": "keeper-1"})),
            ("harness.observed", json!({"state": "idle", "driver": "omp",
                "incarnation_id": "keeper-1"})),
            ("runtime.observed", json!({"status": "stopped", "runtime_id": seat,
                "incarnation_id": "keeper-1"})),
            ("runtime.observed", json!({"status": "running", "runtime_id": seat,
                "incarnation_id": "keeper-2"})),
        ] {
            store
                .append_claim(&ClaimInput {
                    subject: seat.into(),
                    kind: kind.into(),
                    actor: Some(seat.into()),
                    fields: serde_json::from_value(fields).unwrap(),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
    }
    sync(&[&alder, &birch]);
    let mut trimmed = false;
    for _ in 0..4 {
        for node in [&alder, &birch] {
            trimmed |= kinds(&step(node, &context)).contains(&"trimmed");
        }
        sync(&[&alder, &birch]);
    }
    assert!(trimmed, "the fixture reached a trim");
    for node in [&alder, &birch] {
        let indexes = (0..=node.index().unwrap()).collect::<Vec<_>>();
        crate::store::card_fold_tests_shared::assert_card_fold_parity(node, &indexes);
    }
}
