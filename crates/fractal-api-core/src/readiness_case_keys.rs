//! Receipt validation and bounded deterministic continuation for readiness operations.
use super::*;
use crate::model::ClaimRecord;
use sha2::Digest as _;

const MAX_CONFLICTS: usize = 4;
pub(super) const MAX_VISITS: usize = 64;

pub(super) enum Lookup {
    Vacant(String),
    Recorded(Box<ClaimRecord>),
    Blocked,
}

fn conflict(store: &Store, root: &str, owner: &str, receipt: Option<&ClaimRecord>) -> Result<()> {
    let episode = format!("readiness-operation-conflict/{}", hex::encode(sha2::Sha256::digest(root)));
    if store.operational_failure(&episode)?.is_some() {
        return Ok(());
    }
    let reason = receipt.map_or_else(
        || "Readiness operation resolution exceeded its bounded work allowance".to_owned(),
        |receipt| format!("A readiness operation is occupied by a different record: {}", receipt.id),
    );
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
            ("reason".into(), json!(reason)),
            ("targets".into(), json!([source])),
        ]),
        evidence: receipt.into_iter().map(|receipt| receipt.id.clone()).collect(),
        expected_subject: None, idempotency_key: None,
    })?;
    eprintln!("fractal: readiness operation conflict: {episode}: {reason}");
    Ok(())
}

pub(super) fn lookup(
    store: &Store, root: &str, owner: &str,
    accepts: impl Fn(&ClaimRecord, &str) -> bool,
) -> Result<Lookup> {
    let mut remaining = MAX_VISITS;
    resolve(store, root, root, owner, 0, &accepts, &mut remaining)
}

pub(super) fn resolve(
    store: &Store, root: &str, key: &str, owner: &str, depth: usize,
    accepts: &impl Fn(&ClaimRecord, &str) -> bool,
    remaining: &mut usize,
) -> Result<Lookup> {
    if *remaining == 0 {
        conflict(store, root, owner, None)?;
        return Ok(Lookup::Blocked);
    }
    *remaining -= 1;
    let (receipts, overflow) = store.operation_claims(key)?;
    if overflow {
        conflict(store, root, owner, None)?;
    }
    if receipts.is_empty() {
        return Ok(Lookup::Vacant(key.to_owned()));
    }
    let mut expected = None;
    for receipt in &receipts {
        if *remaining == 0 {
            conflict(store, root, owner, None)?;
            return Ok(Lookup::Blocked);
        }
        *remaining -= 1;
        if accepts(receipt, key) {
            expected.get_or_insert(receipt);
        } else {
            conflict(store, root, owner, Some(receipt))?;
        }
    }
    if let Some(receipt) = expected {
        return Ok(Lookup::Recorded(Box::new(receipt.clone())));
    }
    if overflow || depth == MAX_CONFLICTS { return Ok(Lookup::Blocked); }
    let mut selected = None;
    // Search occupied alternative continuations before creating any new receipt.
    // A later replicated occupant can sort before the occupant originally used.
    for receipt in &receipts {
        let next = format!("{root}:conflict:{}", receipt.id);
        let found = resolve(store, root, &next, owner, depth + 1, accepts, remaining)?;
        if *remaining == 0 {
            conflict(store, root, owner, None)?;
            return Ok(Lookup::Blocked);
        }
        if matches!(found, Lookup::Recorded(_)) {
            return Ok(found);
        }
        selected.get_or_insert(found);
    }
    Ok(selected.unwrap_or(Lookup::Blocked))
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
            Lookup::Recorded(receipt) => return Ok(Some(*receipt)),
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
                conflict(store, &root, owner, Some(&receipt))?;
                return Ok(None);
            }
            Err(error) if matches!(error.code, "idempotency-mismatch" | "idempotency-conflict") => {},
            Err(error) => return Err(error.into()),
        }
    }
    // Preserve the conflict as a blocked operation, never as accepted work.
    let _ = lookup(store, &root, owner, |receipt, key| matches(store, input, receipt, key))?;
    Ok(None)
}
