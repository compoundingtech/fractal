#![allow(unused_imports)]
use crate::store::*;
use crate::store::runtime::*;
use smallclaims::ivm::{Contribution, Definition, Readiness, View, Views, events};
use smallclaims::store::canonical;
use std::sync::Barrier;

pub struct Shadow;

impl View for Shadow {
        fn definition(&self) -> Definition {
            Definition {
                name: "fixture.shadow",
                fingerprint: "fixture.shadow.v1",
                kinds: &[],
                local_kinds: &["fixture.clock"],
                max_contributions: 1,
            }
        }
        fn contributions(
            &self,
            _claim: &ClaimRecord,
            _rank: &canonical::ClaimKey,
        ) -> Result<Vec<Contribution>> {
            Ok(Vec::new())
        }
    }

pub fn registry() -> Arc<Views> {
        Arc::new(Views::new(vec![Box::new(Shadow)]).unwrap())
    }

pub fn boundary(store: &Store, views: &Views) -> events::Boundary {
        store
            .read_snapshot(|_| events::capture(&store.readers.get(), views, "fixture.shadow"))
            .unwrap()
    }
