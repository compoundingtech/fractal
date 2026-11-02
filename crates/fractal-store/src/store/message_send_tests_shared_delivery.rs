#![allow(unused_imports)]
use crate::store::*;
use smallclaims::principal::{ClaimSignature, FIELDS_FORMAT, Judged, fields_signing_bytes};

pub fn request() -> ClaimInput {
    ClaimInput {
        subject: "message/local-signed-send".into(),
        kind: "message.sent".into(),
        actor: Some("person/eval".into()),
        fields: BTreeMap::from([
            ("from".into(), json!("person/eval")),
            ("to".into(), json!("agent/eval.recipient")),
            ("content".into(), json!("One durable local send")),
            ("status".into(), json!("sent")),
        ]),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: Some("local-signed-send-request".into()),
    }
}

pub fn signed(input: &ClaimInput) -> ClaimSignature {
    let (key, _) = smallclaims::fleet::MemberKey::generate().unwrap();
    let mut signature = ClaimSignature::sign(
        &key,
        "",
        input.actor.as_deref().unwrap(),
        None,
        vec![],
        now_ms() as u64,
    );
    signature.format = Some(FIELDS_FORMAT.into());
    signature.signed_fields = vec!["content".into(), "from".into(), "to".into()];
    let fields = serde_json::to_value(&input.fields).unwrap();
    signature.signature = key.sign(&fields_signing_bytes(
        &input.subject,
        &input.kind,
        input.actor.as_deref(),
        &fields,
        &signature.signed_fields,
        &signature.signer,
        None,
        &signature.key,
        &signature.chain,
        &signature.nonce,
        signature.signed_at_unix_ms,
    ));
    signature
}

pub fn operation_rows(connection: &Connection) -> Vec<(String, String, String, String)> {
    connection
        .prepare("SELECT id,request_digest,canonical_claim_id,state FROM operations ORDER BY id")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

pub fn assert_operation_replay_parity(store: &Store) {
    let before = {
        let connection = store.readers.get();
        (
            operation_rows(&connection),
            graph_digest(&connection).unwrap(),
            projection_digest::tables(&connection).unwrap(),
            projection_digest::oracle(&connection).unwrap(),
        )
    };
    store.replay_replication_graph().unwrap();
    let connection = store.readers.get();
    assert_eq!(operation_rows(&connection), before.0);
    assert_eq!(graph_digest(&connection).unwrap(), before.1);
    assert_eq!(projection_digest::tables(&connection).unwrap(), before.2);
    assert_eq!(projection_digest::oracle(&connection).unwrap(), before.3);
}
