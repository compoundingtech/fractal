#![allow(unused_imports)]
use super::*;
use st3_store::store::custom::tests_shared_delivery::*;
use st3_store::store::custom::tests_shared_delivery::{DECIDER, FLEET, RAW_KINDS, SEAT, TREE, decision_tree, empty_input, example, legacy_sync, registration};
use st3_store::store::custom::tests_shared_delivery::{request, signed_pair, signed_sync, tree_claim, tree_document, tree_status};


fn reply(store: &Store, id: &str) -> ReplyRequest {
    let v = store.custom_subject(id).unwrap().unwrap();
    ReplyRequest {
        subject: id.into(),
        registration: v["registration"].as_str().unwrap().into(),
        revision: v["revision"].as_str().unwrap().into(),
        episode: v["attention"]["episode"].as_str().unwrap().into(),
        fields: serde_json::from_value(json!({"selection":"keep"})).unwrap(),
        actor: "person/lichen".into(),
        idempotency_key: "garden-review-answer-001".into(),
    }
}

#[test]
fn custom_round_trip_replicates_restarts_and_retains_human_actor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replica.db");
    let a = Store::open_memory("alder").unwrap();
    let b = Store::open(&path, "birch").unwrap();
    registration(&a);
    let id = "custom/garden/review/v1/review-001";
    let q = request(&a, id);
    legacy_sync(&a, &b);
    assert_eq!(a.custom_subject(id).unwrap(), b.custom_subject(id).unwrap());
    let cards = b.attention_items(Some("person/lichen")).unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].episode, q.id);
    let r = reply(&b, id);
    let answer = b.reply_custom_subject(&r).unwrap();
    assert_eq!(answer.actor.as_deref(), Some("person/lichen"));
    assert!(b.attention_items(Some("person/lichen")).unwrap().is_empty());
    assert_eq!(b.reply_custom_subject(&r).unwrap().id, answer.id);
    let mut changed = r.clone();
    changed.fields.insert("selection".into(), json!("discard"));
    assert_eq!(
        b.reply_custom_subject(&changed).unwrap_err().code,
        "idempotency-conflict"
    );
    legacy_sync(&b, &a);
    assert_eq!(
        a.reply_custom_subject(&r).unwrap().id,
        answer.id,
        "a retry on another member recovers the replicated receipt"
    );
    assert_eq!(a.custom_subject(id).unwrap(), b.custom_subject(id).unwrap());
    assert_eq!(
        projection_digest::oracle(&a.readers.get()).unwrap(),
        projection_digest::oracle(&b.readers.get()).unwrap()
    );
    drop(b);
    let b = Store::open(&path, "birch").unwrap();
    assert_eq!(
        b.custom_subject(id).unwrap().unwrap()["fields"]["selection"],
        "keep"
    );
    assert!(b.attention_items(Some("person/lichen")).unwrap().is_empty());
    assert_eq!(b.claims_for(id, None).unwrap().len(), 2);
}

#[test]
fn custom_raw_writes_enforce_schema_namespace_and_human_authority() {
    let s = Store::open_memory("alder").unwrap();
    registration(&s);
    let id = "custom/garden/review/v1/review-002";
    let q = request(&s, id);
    let mut input = ClaimInput {
        subject: id.into(),
        kind: "custom.garden.review.v1.answered".into(),
        actor: Some("agent/garden/seed".into()),
        fields: serde_json::from_value(json!({"selection":"keep","request":q.id})).unwrap(),
        ..empty_input()
    };
    assert!(s.append_claim(&input).is_err());
    input.actor = Some("person/lichen".into());
    input.fields.insert("selection".into(), json!("bad"));
    assert!(s.append_claim(&input).is_err());
    input.kind = "custom.other.answered".into();
    assert!(s.append_claim(&input).is_err());
    input.subject = format!("{}fake", schema::REGISTRY_PREFIX);
    input.kind = schema::REGISTERED.into();
    assert_eq!(
        s.append_claim(&input).unwrap_err().code,
        "custom-registration-only"
    );
    assert_eq!(s.attention_items(Some("person/lichen")).unwrap().len(), 1);
    let mut r = reply(&s, id);
    r.actor = "agent/garden/seed".into();
    assert_eq!(s.reply_custom_subject(&r).unwrap_err().code, "forbidden");
    r.actor = "person/lichen".into();
    r.revision = "obsolete".into();
    assert_eq!(s.reply_custom_subject(&r).unwrap_err().code, "stale-fence");
}

#[test]
fn custom_basis_invalidation_guard_revival_and_reframe_are_graph_facts() {
    let a = Store::open_memory("alder").unwrap();
    let b = Store::open_memory("birch").unwrap();
    let mut m = example();
    let derived = "custom.garden.review.v1.derived";
    let field: schema::Field = serde_json::from_value(
        json!({"value_type":"string","required":true,"values":["pending","gated","moot"]}),
    )
    .unwrap();
    m.claims.insert(
        derived.into(),
        schema::ClaimSchema {
            authority: Authority::Owner,
            fields: BTreeMap::from([("status".into(), field)]),
            additional_fields: false,
        },
    );
    m.slots.insert(
        "state".into(),
        schema::Slot {
            kind: derived.into(),
            select: Select::Last,
        },
    );
    m.attention.as_mut().unwrap().when = Predicate::Eq {
        left: Expr::Field {
            slot: "state".into(),
            field: "status".into(),
        },
        right: Expr::Constant {
            value: json!("pending"),
        },
    };
    m.attention.as_mut().unwrap().episode = Expr::ClaimId {
        slot: "state".into(),
    };
    // Zero/multiple selections and free text are ordinary typed answer fields.
    let selection = m
        .claims
        .get_mut("custom.garden.review.v1.answered")
        .unwrap()
        .fields
        .get_mut("selection")
        .unwrap();
    selection.value_type = st3_schema::ValueType::Array;
    m.attention
        .as_mut()
        .unwrap()
        .reply
        .fields
        .get_mut("selection")
        .unwrap()
        .value_type = st3_schema::ValueType::Array;
    a.register_custom_kind(&RegistrationRequest {
        manifest: m,
        actor: "agent/garden/seed".into(),
    })
    .unwrap();
    let parent = "custom/garden/review/v1/parent";
    let child = "custom/garden/review/v1/child";
    request(&a, parent);
    request(&a, child);
    let kinds = vec![
        "custom.garden.review.v1.requested".to_owned(),
        "custom.garden.review.v1.answered".to_owned(),
    ];
    let derive = |s: &Store, status: &str| {
        let basis = s.custom_basis_revision(parent, &kinds).unwrap();
        s.append_claim(&ClaimInput{subject:child.into(),kind:derived.into(),actor:Some("agent/garden/seed".into()),fields:serde_json::from_value(json!({"status":status,"_basis":[{"subject":parent,"kinds":kinds,"revision":basis},{"subject":child,"kinds":kinds,"revision":s.custom_basis_revision(child,&kinds).unwrap()}]})).unwrap(),..empty_input()}).unwrap()
    };
    derive(&a, "pending");
    assert_eq!(a.attention_items(Some("person/lichen")).unwrap().len(), 1);
    let episode = a.custom_subject(child).unwrap().unwrap()["attention"]["episode"].clone();
    // The person's raw answer is preserved and invalidates the descendant's derived view.
    a.append_claim(&ClaimInput{subject:parent.into(),kind:"custom.garden.review.v1.answered".into(),actor:Some("person/lichen".into()),fields:serde_json::from_value(json!({"request":a.claims_for(parent,None).unwrap()[0].id,"selection":[],"text":"Reframe: retain only recent history"})).unwrap(),..empty_input()}).unwrap();
    assert_eq!(a.custom_subject(child).unwrap().unwrap()["state"], "stale");
    assert!(a.attention_items(Some("person/lichen")).unwrap().is_empty());
    derive(&a, "gated");
    assert!(a.attention_items(Some("person/lichen")).unwrap().is_empty());
    derive(&a, "pending");
    assert_eq!(a.attention_items(Some("person/lichen")).unwrap().len(), 1);
    assert_ne!(
        a.custom_subject(child).unwrap().unwrap()["attention"]["episode"],
        episode
    );
    legacy_sync(&a, &b);
    assert_eq!(
        a.custom_subject(child).unwrap(),
        b.custom_subject(child).unwrap()
    );
    assert_eq!(
        b.claims_for(parent, None).unwrap().last().unwrap().body["fields"]["text"],
        "Reframe: retain only recent history"
    );
    let mut answer = reply(&b, child);
    answer.fields =
        serde_json::from_value(json!({"selection":[],"text":"Reframe the descendant"})).unwrap();
    let human = b.reply_custom_subject(&answer).unwrap();
    assert_eq!(human.actor.as_deref(), Some("person/lichen"));
    assert_eq!(human.body["fields"]["selection"], json!([]));
    assert_eq!(b.custom_subject(child).unwrap().unwrap()["state"], "stale");
    assert!(b.attention_items(Some("person/lichen")).unwrap().is_empty());
}

#[test]
fn custom_signed_round_trip_late_manifest_partition_answers_and_checkpoint_replay() {
    let (a, b) = signed_pair();
    registration(&a);
    let id = "custom/garden/review/v1/signed-example";
    let q = request(&a, id);
    let seq: u64 = a
        .readers
        .get()
        .query_row(
            "SELECT replica_sequence FROM batches WHERE id=?1",
            [&q.batch_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut early = a
        .export_replication_exchange_answering(
            FLEET,
            &b.replication_inventory().unwrap(),
            &b.replication_signature_requests().unwrap(),
        )
        .unwrap();
    early
        .envelopes
        .retain(|e| e.writer == a.origin && e.sequence >= seq);
    b.receive_replication_exchange(&a.origin, FLEET, &early)
        .unwrap();
    b.validate_replication_backlog().unwrap();
    b.project_replication_backlog().unwrap();
    assert_eq!(
        b.custom_subject(id).unwrap().unwrap()["state"],
        "pending-dependencies"
    );
    assert!(b.attention_items(Some("person/lichen")).unwrap().is_empty());
    signed_sync(&a, &b);
    assert_eq!(a.custom_subject(id).unwrap(), b.custom_subject(id).unwrap());
    assert_eq!(b.attention_items(Some("person/lichen")).unwrap().len(), 1);
    // Both isolated members may accept the same current card. Preserve both answers.
    let ar = reply(&a, id);
    let mut br = reply(&b, id);
    br.fields.insert("selection".into(), json!("discard"));
    br.idempotency_key = "partition-answer-birch".into();
    let ac = a.reply_custom_subject(&ar).unwrap();
    let bc = b.reply_custom_subject(&br).unwrap();
    signed_sync(&a, &b);
    signed_sync(&b, &a);
    assert_eq!(a.claims_for(id, None).unwrap().len(), 3);
    assert!(
        a.claims_for(id, None)
            .unwrap()
            .iter()
            .any(|c| c.id == ac.id)
    );
    assert!(
        a.claims_for(id, None)
            .unwrap()
            .iter()
            .any(|c| c.id == bc.id)
    );
    assert_eq!(a.custom_subject(id).unwrap(), b.custom_subject(id).unwrap());
    let digest = projection_digest::oracle(&a.readers.get()).unwrap();
    assert_eq!(digest, projection_digest::oracle(&b.readers.get()).unwrap());
    assert_eq!(a.custom_subject(id).unwrap().unwrap()["state"], "conflict");
    assert_eq!(
        a.custom_subject(id).unwrap().unwrap()["reply_conflicts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let before = a.custom_subject(id).unwrap();
    let plan = checkpoint_rules::plan_drops(&a.checkpoint_sealed_set(now_ms() + 1000).unwrap());
    let custom_ids = a
        .claims_for(id, None)
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect::<BTreeSet<_>>();
    assert!(plan.claims.iter().all(|c| !custom_ids.contains(&c.id)));
    a.apply_checkpoint_drop("checkpoint/garden", &plan.envelopes, &plan.claims)
        .unwrap();
    a.connection
        .batched(replay_graph_from_nothing_tx)
        .unwrap()
        .unwrap();
    assert_eq!(before, a.custom_subject(id).unwrap());
    assert_eq!(digest, projection_digest::oracle(&a.readers.get()).unwrap());
}

#[test]
fn decision_tree_manifest_replicates_restarts_and_surfaces_damage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replica.db");
    let a = Store::open_memory("alder").unwrap();
    let b = Store::open(&path, "birch").unwrap();
    let m = decision_tree();
    assert!(serde_json::to_vec(&m).unwrap().len() <= schema::MAX_BYTES);
    a.register_custom_kind(&RegistrationRequest {
        manifest: m,
        actor: SEAT.into(),
    })
    .unwrap();
    tree_claim(&a, TREE, "opened", SEAT, json!({"seat":SEAT,"recipient":DECIDER})).unwrap();
    let body = tree_document(&a, "doc/decision/example/q1", b"## Options\n### keep\nKeep it.\n### drop\nDrop it.\n");
    let ask = json!({"question":"Keep the seed history?","kind":"blocker","body":body,"q":1,"legacy_id":"k3x9qa"});
    // Only the tree owner asks, assumes, promotes and derives; only the person answers.
    assert!(tree_claim(&a, TREE, "requested", "agent/example/other", ask.clone()).is_err());
    let request = tree_claim(&a, TREE, "requested", SEAT, ask).unwrap();
    let answer = json!({"request":request.id,"selection":["keep"]});
    assert!(tree_claim(&a, TREE, "answered", SEAT, answer.clone()).is_err());
    assert!(tree_claim(&a, TREE, "assumed", DECIDER, json!({"request":request.id,"text":"Keep."})).is_err());
    let bad_kind = json!({"question":"Bad","kind":"urgent","body":body});
    assert!(tree_claim(&a, TREE, "requested", SEAT, bad_kind).is_err());
    assert!(a.attention_items(Some(DECIDER)).unwrap().is_empty());
    let status = tree_status(&a, TREE, SEAT, json!({"state":"pending","title":"Q1: Keep the seed history?","detail":"Options: keep, drop.","request":request.id,"q":1,"pending":1}));
    legacy_sync(&a, &b);
    assert_eq!(a.custom_subject(TREE).unwrap(), b.custom_subject(TREE).unwrap());
    let cards = b.attention_items(Some(DECIDER)).unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].episode, status.id);
    assert_eq!(cards[0].requester_id.as_deref(), Some(SEAT));

    let v = b.custom_subject(TREE).unwrap().unwrap();
    let mut r = ReplyRequest {
        subject: TREE.into(),
        registration: v["registration"].as_str().unwrap().into(),
        revision: v["revision"].as_str().unwrap().into(),
        episode: v["attention"]["episode"].as_str().unwrap().into(),
        fields: serde_json::from_value(json!({"selection":[],"text":"Reframe: keep only recent history"}))
            .unwrap(),
        actor: SEAT.into(),
        idempotency_key: "decision-q1-answer".into(),
    };
    assert_eq!(b.reply_custom_subject(&r).unwrap_err().code, "forbidden");
    r.actor = DECIDER.into();
    let mut stale = r.clone();
    stale.revision = "obsolete".into();
    assert_eq!(b.reply_custom_subject(&stale).unwrap_err().code, "stale-fence");
    let human = b.reply_custom_subject(&r).unwrap();
    assert_eq!(human.actor.as_deref(), Some(DECIDER));
    assert_eq!(human.body["fields"]["request"], json!(request.id));
    assert_eq!(human.body["fields"]["selection"], json!([]));
    assert_eq!(b.reply_custom_subject(&r).unwrap().id, human.id);
    let mut changed = r.clone();
    changed.fields.insert("selection".into(), json!(["drop"]));
    assert_eq!(
        b.reply_custom_subject(&changed).unwrap_err().code,
        "idempotency-conflict"
    );
    // The raw answer makes the derived status stale; the old card cannot be answered again.
    assert_eq!(b.custom_subject(TREE).unwrap().unwrap()["state"], "stale");
    assert!(b.attention_items(Some(DECIDER)).unwrap().is_empty());
    let mut again = r.clone();
    again.idempotency_key = "decision-q1-second".into();
    assert_eq!(b.reply_custom_subject(&again).unwrap_err().code, "stale-fence");

    legacy_sync(&b, &a);
    assert_eq!(a.reply_custom_subject(&r).unwrap().id, human.id);
    let assumption = tree_claim(&a, TREE, "assumed", SEAT, json!({"request":request.id,"text":"Assume recent history only."})).unwrap();
    let raw = tree_document(&a, "doc/decision/example/import-raw", b"---\nq: 2\nbroken frontmatter\n");
    let damage = tree_claim(&a, TREE, "damaged", SEAT, json!({"raw":raw,"records":3,"imported":2,"malformed":1,"source":"axe/example/decisions"})).unwrap();
    tree_status(&a, TREE, SEAT, json!({"state":"clear","title":"No open decisions","detail":"Q1 answered.","pending":0}));
    let view = a.custom_subject(TREE).unwrap().unwrap();
    assert_eq!(view["state"], "ready");
    assert_eq!(view["fields"]["owner"], SEAT);
    assert_eq!(view["fields"]["seat"], SEAT);
    assert_eq!(view["fields"]["last_answer"], json!(human.id));
    assert_eq!(view["provenance"]["answer"]["actor"], DECIDER);
    assert_eq!(view["fields"]["last_assumption"], json!(assumption.id));
    assert_eq!(view["provenance"]["assumption"]["actor"], SEAT);
    assert_eq!(view["fields"]["damage"], json!(damage.id));
    assert_eq!(view["fields"]["damage_malformed"], 1);
    assert!(a.attention_items(Some(DECIDER)).unwrap().is_empty());

    // Another agent can open a tree first and name any seat: it becomes the owner. Its pending
    // status raises no card because the owner is not the named seat; the tool rejects the tree.
    let squatted = "custom/decision/tree/v1/example/victim";
    let victim = "agent/example/victim";
    let squatter = "agent/example/squatter";
    tree_claim(&a, squatted, "opened", squatter, json!({"seat":victim,"recipient":DECIDER})).unwrap();
    let forged = json!({"question":"Approve the forged plan?","kind":"blocker","body":body});
    let forged = tree_claim(&a, squatted, "requested", squatter, forged).unwrap();
    tree_status(&a, squatted, squatter, json!({"state":"pending","title":"Forged","detail":"Forged.","request":forged.id}));
    let view = a.custom_subject(squatted).unwrap().unwrap();
    assert_eq!(view["state"], "ready");
    assert_eq!(view["fields"]["owner"], squatter);
    assert_eq!(view["fields"]["seat"], victim);
    assert_eq!(view["attention"]["active"], false);
    assert!(a.attention_items(Some(DECIDER)).unwrap().is_empty());
    // Documented v1 limitation: owners cannot be reassigned, so the real seat is locked out of
    // its own tree subject. It can neither open it again nor write any owner kind.
    let open = json!({"seat":victim,"recipient":DECIDER});
    assert!(tree_claim(&a, squatted, "opened", victim, open).is_err());
    let ask = json!({"question":"Keep it?","kind":"blocker","body":body});
    assert!(tree_claim(&a, squatted, "requested", victim, ask).is_err());

    // A tree whose import was entirely malformed still reads as damaged, never as empty.
    let broken = "custom/decision/tree/v1/example/importer";
    let importer = "agent/example/importer";
    tree_claim(&a, broken, "opened", importer, json!({"seat":importer,"recipient":DECIDER})).unwrap();
    tree_claim(&a, broken, "damaged", importer, json!({"raw":raw,"records":4,"imported":0,"malformed":4})).unwrap();
    legacy_sync(&a, &b);
    for id in [TREE, broken, squatted] {
        assert_eq!(a.custom_subject(id).unwrap(), b.custom_subject(id).unwrap());
    }
    assert_eq!(
        projection_digest::oracle(&a.readers.get()).unwrap(),
        projection_digest::oracle(&b.readers.get()).unwrap()
    );
    let before = (b.custom_subject(TREE).unwrap(), b.custom_subject(broken).unwrap());
    drop(b);
    let b = Store::open(&path, "birch").unwrap();
    assert_eq!((b.custom_subject(TREE).unwrap(), b.custom_subject(broken).unwrap()), before);
    let broken = b.custom_subject(broken).unwrap().unwrap();
    assert_eq!(broken["state"], "ready");
    assert_eq!(broken["fields"]["damage_malformed"], 4);
    assert_eq!(broken["fields"]["damage_imported"], 0);
    assert!(broken["fields"]["last_request"].is_null());
    assert!(b.attention_items(Some(DECIDER)).unwrap().is_empty());
    assert_eq!(b.custom_subject(squatted).unwrap().unwrap()["attention"]["active"], false);
    assert_eq!(b.reply_custom_subject(&r).unwrap().id, human.id);
}
