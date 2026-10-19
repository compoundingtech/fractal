#![allow(unused_imports)]
pub use fractal_store::store::attention_ivm::tests_shared_seats::*;
use crate::store::attention_ivm::*;
use crate::store::custom::RegistrationRequest;
use smallclaims::ivm::{Readiness, SourceCut, events};


pub fn views(store: &Store) -> Views {
    let views = Views::new(definitions()).unwrap();
    views.create_schema(&store.connection.write()).unwrap();
    views
}
