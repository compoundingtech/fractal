#![allow(unused_imports)]
use crate::AttentionStore;
use super::*;
use smallclaims::store::checkpoint_agreement::*;
use fractal_seats::store::checkpoint_agreement_tests_shared_attention_tests::{FLEET, claim_ids, context, kinds, observe, stable_cuts, step, sync};


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
