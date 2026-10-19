#![allow(unused_imports)]
pub use st3_store::store::mission_ivm_tests_shared_seats::*;
use crate::store::*;
use smallclaims::ivm::{Definition, LocalChange, Readiness, View, Views, source_cut};
use crate::store::mission_ivm::*;
use smallclaims::ivm::{SourceCut, events};
use crate::store::attention_ivm::tests_shared_seats::*;
use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;


pub fn register(store: &Store) -> Arc<Views> {
        let views = Arc::new(Views::new(definitions()).unwrap());
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        assert_eq!(
            current_index_tx(&tx).unwrap(),
            0,
            "fixture registration is empty-source only"
        );
        views.create_schema(&tx).unwrap();
        views
            .initialize_empty(
                &tx,
                SourceCut {
                    epoch: 1,
                    admitted: 0,
                    projected: 0,
                    local_generation: 0,
                },
            )
            .unwrap();
        events::install(&tx, 1024).unwrap();
        tx.commit().unwrap();
        views
    }

pub fn fixture() -> (Store, Arc<Views>) {
        let store = Store::open_memory("birch").unwrap();
        let views = register(&store);
        (store, views)
    }

// Test-only coverage certificate: every real Store action above has returned, or a real
    // replication projection pass has completed. This is not a production MAX-index adapter.
    pub fn checkpoint(store: &Store, views: &Views, at: u128) {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let index = current_index_tx(&tx).unwrap();
        let previous = source_cut(&tx).unwrap().unwrap();
        views
            .publish_cut(
                &tx,
                SourceCut {
                    admitted: index,
                    projected: index,
                    ..previous
                },
            )
            .unwrap();
        while flush(&tx, views, at, 2).unwrap() != 0 {}
        while clock_page(&tx, views, at, 2).unwrap() != 0 {}
        tx.commit().unwrap();
    }

pub fn parity(store: &Store, views: &Views, at: u128) {
        checkpoint(store, views, at);
        store.read_snapshot(|_| {
            let connection=store.readers.get();
            for history in [false,true] {
                let expected=store.mission_collection_page(history,0,501,None)?;
                assert_eq!(window(&connection,views,history,at,None,None,501)?,expected);
                let mut paged=Vec::new();
                let mut after=None;
                loop {
                    let page=window(&connection,views,history,at,after.as_ref(),None,2)?;
                    if page.is_empty(){break;}
                    let last=page.last().unwrap();
                    after=Some((last.1,last.0.clone()));
                    paged.extend(page);
                }
                assert_eq!(paged,expected,"existing full-sort seek tuple");
            }
            // Independent SQL count oracle, not the incremental counters.
            let counts=connection.prepare("SELECT mission_id,status,COUNT(*) FROM mission_runs GROUP BY mission_id,status ORDER BY mission_id,status")?
                .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,u64>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let actual=connection.prepare("SELECT mission_id,status,count FROM local_mission_counts ORDER BY mission_id,status")?
                .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,u64>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            assert_eq!(actual,counts);
            Ok(())
        }).unwrap();
    }
