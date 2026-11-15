use super::*;

fn sent(subject: &str, author: &str, agent: &str, session: &str) -> ClaimInput {
    ClaimInput { subject:subject.into(),kind:"message.sent".into(),actor:Some(author.into()),
        fields:serde_json::from_value(json!({"from":author,"to":agent,"content":"fixture input",
            "status":"sent","session_id":session,"tags":["dictated","secret"]})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:Some(subject.into()) }
}
fn query(store: &Store) -> Query<'static> {
    Query {person:"person/ada",agent:"agent/worker",conversation:None,
        through:store.index().unwrap(),before:None,limit:2}
}

#[test]
fn exact_author_agent_conversation_and_accepted_identity() {
    let store = Store::open_memory("history-source").unwrap();
    let old = store.append_claim(&sent("message/old","person/ada","agent/worker","session/one")).unwrap();
    let newest_input = sent("message/new","person/ada","agent/worker","session/two");
    let newest = store.append_claim(&newest_input).unwrap();
    assert_eq!(store.append_claim(&newest_input).unwrap().id,newest.id);
    for (id,author,agent) in [("other-author","person/other","agent/worker"),
        ("other-agent","person/ada","agent/other"),("harness","agent/harness","agent/worker")] {
        store.append_claim(&sent(&format!("message/{id}"),author,agent,"session/two")).unwrap();
    }
    let q=query(&store);
    let page=store.input_history_candidates(&q, |_| Ok(true)).unwrap();
    let mut expected=vec![&old,&newest];
    expected.sort_by(|a,b|b.accepted_at_unix_ms.cmp(&a.accepted_at_unix_ms).then_with(||b.subject.cmp(&a.subject)));
    assert_eq!(page.entries.iter().map(|e|e.claim_id.as_str()).collect::<Vec<_>>(),expected.iter().map(|e|e.id.as_str()).collect::<Vec<_>>());
    assert!(!page.has_more_candidates);
    let narrow=Query {conversation:Some("session/one"),..q};
    assert_eq!(store.input_history_candidates(&narrow, |_|Ok(true)).unwrap().entries[0].claim_id,old.id);
    // Dictation and a tag named secret are not exclusion facts or a classifier.
    assert_eq!(page.entries.len(),2);
    assert!(store.input_history_candidates(&Query {agent:"",..narrow}, |_|Ok(true)).is_err());
}

#[test]
fn frozen_membership_current_repairs_and_authorization_errors() {
    let store=Store::open_memory("history-revocation").unwrap();
    let first=store.append_claim(&sent("message/first","person/ada","agent/worker","session/one")).unwrap();
    let second=store.append_claim(&sent("message/second","person/ada","agent/worker","session/one")).unwrap();
    let q=Query {limit:200,..query(&store)};
    store.append_claim(&sent("message/later","person/ada","agent/worker","session/one")).unwrap();
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h2c2001d69ecf owner=fractal_store::store::input_history::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)",[&second.id]).unwrap();
    let page=store.input_history_candidates(&q, |_|Ok(true)).unwrap();
    assert_eq!(page.entries.iter().map(|e|&e.claim_id).collect::<Vec<_>>(),vec![&first.id]);
    assert!(store.input_history_candidates(&q, |_|anyhow::bail!("source authorization unavailable")).is_err());
    assert!(store.input_history_candidates(&q, |_|Ok(false)).unwrap().entries.is_empty());
    let tiny=Query {limit:1,..q};
    let skipped=store.input_history_candidates(&tiny, |_|Ok(true)).unwrap();
    assert!(skipped.entries.is_empty());
    assert!(skipped.has_more_candidates);
    let repaired=store.input_history_candidates(&Query {before:skipped.examined_through.as_ref(),..tiny}, |_|Ok(true)).unwrap();
    assert!(repaired.entries.is_empty());
    let next=Query {before:repaired.examined_through.as_ref(),..tiny};
    assert_eq!(store.input_history_candidates(&next, |_|Ok(true)).unwrap().entries[0].claim_id,first.id);
}

#[test]
fn indexed_seek_work_does_not_grow_with_other_conversations_or_receipts() {
    let store=Store::open_memory("history-bounded").unwrap();
    store.append_claim(&sent("message/kept","person/ada","agent/worker","session/kept")).unwrap();
    fn steps(store:&Store, conversation:Option<&str>) -> i32 {
        let connection=store.readers.get();
        let sql=&SQL[usize::from(conversation.is_some())];
        let mut statement=connection.prepare(/* sqlite-large-value: allow-for-now lv-h5a5cc8c639cc owner=fractal_store::store::input_history::tests TODO=trace-statement-and-value-representation */ sql).unwrap();
        let rows=statement.query_map(params!["person/ada","agent/worker",i64::MAX,"\u{10ffff}","","",conversation,3],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(rows,vec!["message/kept"]);
        assert_eq!(statement.get_status(rusqlite::StatementStatus::FullscanStep),0);
        statement.get_status(rusqlite::StatementStatus::VmStep)
    }
    for n in 0..64 {
        store.append_claim(&sent(&format!("message/noise-{n}"),"person/ada","agent/worker","session/noise")).unwrap();
    }
    let small=steps(&store,Some("session/kept"));
    for n in 64..1024 {
        store.append_claim(&sent(&format!("message/noise-{n}"),"person/ada","agent/worker","session/noise")).unwrap();
    }
    let large=steps(&store,Some("session/kept"));
    eprintln!("history seek VM steps: 64 neighbors={small}; 1024 neighbors={large}");
    // Compare populated index ranges: the empty-tree endpoint has a different
    // constant cursor-exit cost. A scan growing with 16x more history still fails.
    assert!(large.abs_diff(small)<=2,"unrelated history increased VM work: {small} -> {large}");
    // Receipts cannot create a second accepted input or change its identity/order.
    store.append_claim(&ClaimInput {subject:"message/kept".into(),kind:"message.delivered".into(),
        actor:Some("daemon/runtime".into()),fields:BTreeMap::from([("status".into(),json!("delivered"))]),
        evidence:vec![],expected_subject:None,idempotency_key:None}).unwrap();
    assert!(steps(&store,Some("session/kept")).abs_diff(large)<=2);
}

#[test]
fn canonical_subject_winners_and_order_survive_opposite_replication() {
    use crate::store::glasses_tests_shared_delivery::sync;
    let left=Store::open_memory("history-a").unwrap();
    let right=Store::open_memory("history-b").unwrap();
    left.append_claim(&sent("message/same","person/ada","agent/worker","session/left")).unwrap();
    left.append_claim(&sent("message/left","person/ada","agent/worker","session/left")).unwrap();
    right.append_claim(&sent("message/same","person/other","agent/worker","session/right")).unwrap();
    right.append_claim(&sent("message/right","person/ada","agent/worker","session/right")).unwrap();
    let forward=Store::open_memory("history-forward").unwrap();
    let reverse=Store::open_memory("history-reverse").unwrap();
    sync(&left,&forward); sync(&right,&forward);
    sync(&right,&reverse); sync(&left,&reverse);
    let rows=|store:&Store|store.input_history_candidates(&Query {limit:200,..query(store)}, |_|Ok(true)).unwrap().entries;
    assert_eq!(rows(&forward),rows(&reverse));
    let winner=forward.readers.get().query_row(/* sqlite-large-value: allow-for-now lv-hb40613939937 owner=fractal_store::store::input_history::tests TODO=trace-statement-and-value-representation */ &message_sent::graph_sent_claim(),["message/same"],claim_from_row).unwrap();
    for store in [&forward,&reverse] {
        let page=store.input_history_candidates(&Query {person:winner.actor.as_deref().unwrap(),limit:200,..query(store)}, |_|Ok(true)).unwrap();
        assert_eq!(page.entries.iter().filter(|e|e.message_id=="message/same").map(|e|&e.claim_id).collect::<Vec<_>>(),vec![&winner.id]);
        store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hcf630123788a owner=fractal_store::store::input_history::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)",[&winner.id]).unwrap();
        for person in ["person/ada","person/other"] {
            assert!(store.input_history_candidates(&Query {person,limit:200,..query(store)}, |_|Ok(true)).unwrap().entries.iter().all(|e|e.message_id!="message/same"));
        }
    }
}
