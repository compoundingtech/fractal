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

#[test]
fn inbox_fallback_changes_invalidate_materialized_associations() {
    use super::*;
    let store = Store::open_memory("example").unwrap();
    custom(&store, "custom/garden/review/v1/agent-origin", "person/example");
    let subject = "custom/garden/review/v1/person-origin";
    store.append_claim(&input(subject, "custom.garden.review.v1.requested", "person/example",
        json!({"title":"Review my work","detail":"A person-owned review","recipient":"person/example"}))).unwrap();
    let views = Views::new(definitions()).unwrap();
    views.create_schema(&store.connection.write()).unwrap();
    install(&store, &views);
    for fallback in [Some("agent/example/first"), Some("agent/example/second"), None] {
        store.configure_inbox_fallback_agent(fallback).unwrap();
        store.connection.batched(|tx| -> Result<()> {
            while !flush(tx, &views, now_ms(), 2)? {}
            let rows = window(tx, &views, CUSTOM_VIEW, "person/example", u128::MAX, 10)?;
            let cached = rows.iter().find(|row| row["source_id"] == subject).unwrap();
            assert_eq!(cached["conversation_id"].as_str(), fallback);
            assert_eq!(rows.iter().find(|row| row["source_id"] == "custom/garden/review/v1/agent-origin").unwrap()["conversation_id"], "agent/garden/seed");
            Ok(())
        }).unwrap().unwrap();
        let direct = store.attention_items(Some("person/example")).unwrap().into_iter()
            .find(|item| item.subject == subject).unwrap();
        assert_eq!(direct.conversation.as_deref(), fallback);
    }
}
