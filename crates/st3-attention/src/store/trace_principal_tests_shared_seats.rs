#![allow(unused_imports)]
pub use st3_store::store::trace_principal_tests_shared_seats::*;
use crate::store::*;
use crate::store::trace_principal::*;


pub fn declare(store: &Store, name: &str, child: &str, author: Option<&str>, key: &str) {
        let source = format!("version 2\nagent \"{name}\" {{ {child} command \"true\"; }}");
        let intent = crate::graph::parse_internal_intent(&source, "alder").unwrap();
        let expected = intent
            .subjects
            .keys()
            .map(|subject| {
                (
                    subject.clone(),
                    store
                        .selected_desired_token(subject)
                        .unwrap()
                        .into_iter()
                        .collect(),
                )
            })
            .collect();
        if let Some(person) = author.filter(|actor| actor.starts_with("person/")) {
            store.ensure_principal_key(person).unwrap();
        }
        store.apply_as(&intent, &expected, key, author).unwrap();
        store.seal_local_batches().unwrap();
        store.judge_claims(true).unwrap();
    }
