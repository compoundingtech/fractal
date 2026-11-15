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
    assert_eq!(page.entries.iter().map(|e|e.claim_id.as_str()).collect::<Vec<_>>(),vec![newest.id.as_str(),old.id.as_str()]);
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
    let q=query(&store);
    store.append_claim(&sent("message/later","person/ada","agent/worker","session/one")).unwrap();
    store.connection.write().execute("INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)",[&second.id]).unwrap();
    let page=store.input_history_candidates(&q, |_|Ok(true)).unwrap();
    assert_eq!(page.entries.iter().map(|e|&e.claim_id).collect::<Vec<_>>(),vec![&first.id]);
    assert!(store.input_history_candidates(&q, |_|anyhow::bail!("source authorization unavailable")).is_err());
    assert!(store.input_history_candidates(&q, |_|Ok(false)).unwrap().entries.is_empty());
    let tiny=Query {limit:1,..q};
    let skipped=store.input_history_candidates(&tiny, |_|Ok(true)).unwrap();
    assert!(skipped.entries.is_empty());
    assert!(skipped.has_more_candidates);
    let next=Query {before:skipped.examined_through,..tiny};
    assert_eq!(store.input_history_candidates(&next, |_|Ok(true)).unwrap().entries[0].claim_id,first.id);
}

#[test]
fn indexed_seek_work_does_not_grow_with_other_conversations_or_receipts() {
    let store=Store::open_memory("history-bounded").unwrap();
    store.append_claim(&sent("message/kept","person/ada","agent/worker","session/kept")).unwrap();
    fn steps(store:&Store, conversation:Option<&str>) -> i32 {
        let connection=store.readers.get();
        let sql=&SQL[usize::from(conversation.is_some())];
        let mut statement=connection.prepare(sql).unwrap();
        let rows=statement.query_map(params!["person/ada","agent/worker",i64::MAX,i64::MAX,conversation,3],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(rows,vec!["message/kept"]);
        assert_eq!(statement.get_status(rusqlite::StatementStatus::FullscanStep),0);
        assert_eq!(statement.get_status(rusqlite::StatementStatus::Sort),0);
        statement.get_status(rusqlite::StatementStatus::VmStep)
    }
    let baseline=steps(&store,Some("session/kept"));
    for n in 0..64 {
        store.append_claim(&sent(&format!("message/noise-{n}"),"person/ada","agent/worker","session/noise")).unwrap();
    }
    assert_eq!(steps(&store,Some("session/kept")),baseline);
    // Receipts cannot create a second accepted input or change its identity/order.
    store.append_claim(&ClaimInput {subject:"message/kept".into(),kind:"message.delivered".into(),
        actor:Some("daemon/runtime".into()),fields:BTreeMap::from([("status".into(),json!("delivered"))]),
        evidence:vec![],expected_subject:None,idempotency_key:None}).unwrap();
    assert_eq!(steps(&store,Some("session/kept")),baseline);
}
