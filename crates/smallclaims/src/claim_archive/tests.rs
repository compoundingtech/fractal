use super::*;
use std::sync::Arc;
use crate::store::runtime::Plain;

fn evidence() -> ClaimEvidence {
    ClaimEvidence { id: "claim/cold".into(), store_index: 7, batch_id: "batch/cold".into(),
        subject: "note/history".into(), kind: "example.note".into(), origin: "alder".into(),
        actor: Some("agent/example".into()), body: "{ \"fields\": { \"text\": \"\\u0061\\n\" } }\n".into(),
        predecessors: "[ \"claim/predecessor\" ]".into(), accepted_at_unix_ms: "0001".into() }
}

#[test]
fn archived_claim_raw_text_and_native_record_are_exact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims");
    let original = evidence();
    let seal = write_selected(vec![original.clone()], "rules".into(), 40 * 86_400_000,
        HotWindow::Days30, &path).unwrap();
    let archive = ClaimArchive::open(&path, &seal).unwrap();
    assert_eq!(archive.evidence(&original.id), Some(&original));
    let store = crate::Store::open_memory("birch", Arc::new(Plain)).unwrap();
    assert!(store.claim_by_id(&original.id).unwrap().is_none());
    let actual = store.claim_by_id_with_archive(&original.id, &[archive]).unwrap().unwrap();
    assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(original.record()).unwrap());
    assert!(store.claim_by_id(&original.id).unwrap().is_none(), "default reads stay inactive");
    let mut wrong = seal.clone();
    wrong.sha256 = "0".repeat(64);
    assert!(ClaimArchive::open(&path, &wrong).is_err());
    assert!(write_selected(vec![original], "rules".into(), 40 * 86_400_000,
        HotWindow::Days30, &path).is_err(), "publication never overwrites existing evidence");
}

#[test]
fn duplicate_cold_evidence_must_agree_byte_for_byte() {
    let directory = tempfile::tempdir().unwrap();
    let first = evidence();
    let mut second = first.clone();
    second.body = "{\"fields\":{\"text\":\"a\\n\"}}".into();
    let archives = [first, second].into_iter().enumerate().map(|(index, claim)| {
        let path = directory.path().join(index.to_string());
        let seal = write_selected(vec![claim], "rules".into(), 40 * 86_400_000, HotWindow::Days30, &path).unwrap();
        ClaimArchive::open(&path, &seal).unwrap()
    }).collect::<Vec<_>>();
    let store = crate::Store::open_memory("birch", Arc::new(Plain)).unwrap();
    assert!(store.claim_by_id_with_archive("claim/cold", &archives).is_err());
    assert!(store.claim_by_id_with_archive("claim/absent", &archives).unwrap().is_none());
}

#[test]
fn window_boundary_and_unknown_times_never_establish_old_age() {
    let directory = tempfile::tempdir().unwrap();
    for window in [HotWindow::Days14, HotWindow::Days30] {
        for time in ["invalid".to_owned(), (40 * 86_400_000 - window.milliseconds()).to_string()] {
            let mut claim = evidence();
            claim.accepted_at_unix_ms = time;
            assert!(write_selected(vec![claim], "rules".into(), 40 * 86_400_000,
                window, &directory.path().join("must-not-exist")).is_err());
        }
    }
    assert!(!directory.path().join("must-not-exist").exists());
}

#[test]
fn contradictory_hot_evidence_is_not_hidden_by_the_archive() {
    let store = crate::Store::open_memory("alder", Arc::new(Plain)).unwrap();
    store.set_write_clock_at(100).unwrap();
    let claim = store.append_claim(&crate::ClaimInput {
        subject: "note/history".into(), kind: "example.note".into(), actor: None,
        fields: BTreeMap::new(), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    let mut cold = ClaimEvidence::capture(&store.readers.get(), &claim.id).unwrap().unwrap();
    cold.body.push(' ');
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("different");
    let seal = write_selected(vec![cold], "rules".into(), 40 * 86_400_000, HotWindow::Days30, &path).unwrap();
    let archive = ClaimArchive::open(&path, &seal).unwrap();
    assert!(store.claim_by_id_with_archive(&claim.id, &[archive]).is_err());
    assert!(store.claim_by_id(&claim.id).unwrap().is_some());
}
