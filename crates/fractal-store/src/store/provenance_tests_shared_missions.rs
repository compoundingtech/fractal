#![allow(unused_imports)]
use crate::model::{IntentInput, FractalError};
use std::collections::BTreeMap;
use crate::provenance::*;
use crate::parse_intent;
use crate::store::Store;

pub fn publish(store: &Store, source: &str, key: &str) -> Result<(), FractalError> {
    let intent = parse_intent(source, "orchard").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store.apply(&intent, &preview.subject_tokens, key)?;
    Ok(())
}
