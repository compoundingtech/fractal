#![allow(unused_imports)]
use crate::store::*;
use crate::store::trace_principal::*;

pub fn store() -> Store {
        let store = Store::open_memory("alder").unwrap();
        store
            .set_node_key(Arc::new(crate::fleet::MemberKey::generate().unwrap().0))
            .unwrap();
        store
    }

