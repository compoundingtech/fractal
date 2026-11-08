//! Receipt validation and bounded deterministic continuation for readiness operations.
use super::*;
use crate::model::ClaimRecord;
use sha2::Digest as _;

const MAX_CONFLICTS: usize = 4;

pub(super) enum Lookup {
    Vacant(String),
    Recorded(ClaimRecord),
    Blocked,
}

fn conflict(store: &Store, root: &str, owner: &str, receipt: &ClaimRecord) -> Result<()> {
    let episode = format!("readiness-operation-conflict/{}", hex::encode(sha2::Sha256::digest(root)));
    eprintln!("fractal: readiness operation conflict: {episode}, occupied by {}", receipt.id);
    if store.operational_failure(&episode)?.is_some() {
        return Ok(());
    }
    let source = format!("daemon/{}", store.origin());
    // This diagnostic has no operation key: its native, system-only failure episode
    // records the conflict without relying on another globally occupied key.
    store.append_claim(&ClaimInput {
        subject: source.clone(), kind: "operational.failure".into(),
        actor: Some("daemon/runtime".into()),
        fields: BTreeMap::from([
            ("episode".into(), json!(episode)),
            ("condition".into(), json!("readiness-operation-conflict")),
            ("reviewer".into(), json!(owner)),
            ("title".into(), json!("Readiness operation conflict")),
            ("severity".into(), json!("warning")),
            ("reason".into(), json!(format!("A readiness operation is occupied by a different record: {}", receipt.id))),
            ("targets".into(), json!([source])),
        ]),
        evidence: vec![receipt.id.clone()], expected_subject: None, idempotency_key: None,
    })?;
    Ok(())
}

pub(super) fn lookup(
    store: &Store, root: &str, owner: &str,
    accepts: impl Fn(&ClaimRecord, &str) -> bool,
) -> Result<Lookup> {
    let mut key = root.to_owned();
    for depth in 0..=MAX_CONFLICTS {
        let Some(receipt) = store.operation_claim(&key)? else {
            return Ok(Lookup::Vacant(key));
        };
        if accepts(&receipt, &key) {
            return Ok(Lookup::Recorded(receipt));
        }
        conflict(store, root, owner, &receipt)?;
        if depth == MAX_CONFLICTS { return Ok(Lookup::Blocked); }
        key = format!("{root}:conflict:{}", receipt.id);
    }
    unreachable!("bounded key resolution returns at the limit")
}

pub(super) fn message_subject(key: &str) -> String {
    format!("message/{}", &hex::encode(sha2::Sha256::digest(key))[..16])
}

pub(super) fn matches(store: &Store, input: &ClaimInput, receipt: &ClaimRecord, key: &str) -> bool {
    receipt.kind == input.kind && receipt.actor == input.actor
        && receipt.origin == store.origin()
        && (if input.kind == "message.sent" {
            receipt.subject == message_subject(key)
        } else { receipt.subject == input.subject })
        && input.fields.iter().all(|(key, value)| receipt.body["fields"].get(key).is_some_and(|actual| actual == value))
        && receipt.body["evidence"] == json!(input.evidence)
}

pub(super) fn append(store: &Store, input: &mut ClaimInput, owner: &str) -> Result<Option<ClaimRecord>> {
    let root = input.idempotency_key.as_deref().context("readiness operation has no key")?.to_owned();
    // An append may race a newly occupied key. Each retry validates the full chain
    // again, and both the chain and append attempts are bounded.
    for _ in 0..=MAX_CONFLICTS {
        match lookup(store, &root, owner, |receipt, key| matches(store, input, receipt, key))? {
            Lookup::Recorded(receipt) => return Ok(Some(receipt)),
            Lookup::Blocked => return Ok(None),
            Lookup::Vacant(key) => {
                if input.kind == "message.sent" && input.idempotency_key.as_deref() != Some(key.as_str()) {
                    input.subject = message_subject(&key);
                }
                input.idempotency_key = Some(key);
            }
        }
        let result = if input.kind == "message.sent" {
            crate::store::message_routing::send(store, input)
        } else {
            store.append_claim(input)
        };
        match result {
            Ok(receipt) => {
                if matches(store, input, &receipt, input.idempotency_key.as_deref().unwrap()) {
                    return Ok(Some(receipt));
                }
                conflict(store, &root, owner, &receipt)?;
                return Ok(None);
            }
            Err(error) if error.code == "idempotency-mismatch" => {},
            Err(error) => return Err(error.into()),
        }
    }
    // Preserve the conflict as a blocked operation, never as accepted work.
    let _ = lookup(store, &root, owner, |receipt, key| matches(store, input, receipt, key))?;
    Ok(None)
}
