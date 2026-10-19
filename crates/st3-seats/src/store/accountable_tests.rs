#![allow(unused_imports)]
use super::*;
use st3_missions::store::accountable_tests_shared_seats::*;
use st3_missions::store::accountable_tests_shared_seats::{PERSON, ROOT, SOURCE, intent};


#[test]
fn accountable_root_publication_requires_same_person_and_round_trips() {
    let store = Store::open_memory("alder").unwrap();
    let intent = intent();
    let expected = BTreeMap::from([(ROOT.into(), Vec::new())]);
    for actor in [None, Some("agent/example/parent"), Some("person/robin")] {
        let error = store
            .apply_as(&intent, &expected, "refused", actor)
            .unwrap_err();
        assert_eq!(error.code, "accountable-root-forbidden");
        assert!(store.desired_subject_with_writer(ROOT).unwrap().is_none());
    }
    store
        .apply_as(&intent, &expected, "root", Some(PERSON))
        .unwrap();
    let (desired, author) = store.desired_subject_with_writer(ROOT).unwrap().unwrap();
    assert_eq!(desired, intent.subjects[ROOT]);
    assert_eq!(author.as_deref(), Some(PERSON));
    let claim = store
        .latest_claim(ROOT, Some("intent.desired"))
        .unwrap()
        .unwrap();
    assert_eq!(claim.body["accountable"], PERSON);
    let (_, tree) = store.agent_declaration(ROOT, None).unwrap().unwrap();
    let rendered = crate::graph::render_agent_desired_kdl(&tree).unwrap();
    assert_eq!(
        crate::graph::parse_internal_intent(&rendered, "alder")
            .unwrap()
            .subjects[ROOT]
            .accountable
            .as_deref(),
        Some(PERSON)
    );
    store.rebuild_claim_projections().unwrap();
    assert_eq!(
        store.desired_subject_with_writer(ROOT).unwrap().unwrap().0,
        desired
    );
}
