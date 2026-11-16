#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::attention_ivm::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::attention_ivm::tests_shared_seats::{custom, input, install};


    

    

    


#[test]
fn notice_legacy_cached_attention_does_not_hide_unrelated_items() {
    use super::*;
    let store = Store::open_memory("example").unwrap();
    let ordinary = "custom/garden/review/v1/ordinary";
    custom(&store, ordinary, "person/example");
    let reserved = "custom/attention/source";
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h162cd91a047b owner=fractal_store::store::attention_ivm::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO custom_sources SELECT ?1,registration,kind,revision,state,
            json_set(body,'$.id',?1),person,episode,active,requested_at FROM custom_sources WHERE subject=?2",
            params![reserved, ordinary])?;
        assert!(custom_row(tx, reserved)?.is_none());
        Ok(())
    }).unwrap().unwrap();
    let views = Views::new(definitions()).unwrap();
    views.create_schema(&store.connection.write()).unwrap();
    install(&store, &views);
    store.connection.batched(|tx| -> Result<()> {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h01805b1bf25a owner=fractal_store::store::attention_ivm::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_attention_open SELECT family,?1,'attention/legacy',person,
            priority,requested,eligible,episode,item,json_set(body,'$.id','attention/legacy') FROM local_attention_open WHERE source=?2",
            params![reserved, ordinary])?;
        let rows = window(tx, &views, CUSTOM_VIEW, "person/example", u128::MAX, 1)?;
        assert_eq!(rows.len(), 1);
        assert_ne!(rows[0]["id"], "attention/legacy");
        assert!(row(tx, &views, CUSTOM_VIEW, "person/example", "attention/legacy", u128::MAX)?.is_none());
        Ok(())
    }).unwrap().unwrap();
}
