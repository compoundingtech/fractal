//! Store-free eligibility rules shared by cached runtime facts and portable proofs.
use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde_json::Value;

use super::{ClaimSignature, Judged, content_digest, message_body_entry};
use crate::fleet::Membership;

/// Root eligibility contexts for cached facts and authenticated portable records.
/// Portable node signatures are bound to the signer's own membership window.
pub enum RootContext<'a> {
    Runtime(&'a BTreeSet<(String, String)>),
    Record { membership: &'a Membership, writer: &'a str, sequence: u64, claim_id: &'a str },
}

/// Apply the root policy for the caller's authenticated evidence context.
pub fn root_eligible(subject: &str, key: &str, context: RootContext<'_>) -> bool {
    match context {
        RootContext::Runtime(roots) => roots.contains(&(subject.to_owned(), key.to_owned())),
        RootContext::Record { membership, writer, sequence, claim_id } => {
            subject.strip_prefix("host/") == Some(writer)
                && membership.incarnations().any(|incarnation| {
                    incarnation.name == writer && incarnation.member_key == key
                        && ((sequence >= incarnation.start
                            && incarnation.end.is_none_or(|end| sequence <= end))
                            // The pinned anchor authenticates its own admission bootstrap.
                            || (membership.anchor() == Some(key)
                                && incarnation.admitted_claim == claim_id))
                })
        }
    }
}

/// Canonical SQL order components, without importing a database value type. Unknown legacy
/// value kinds preserve the runtime comparator's existing neutral comparison.
#[derive(Clone, Copy)]
pub enum OrderComponent<'a> {
    Null,
    Integer(i128),
    Text(&'a str),
    /// A decoded timestamp: SQL's decimal length/text pair has the same numeric ordering.
    Unsigned(u128),
    Other,
}

/// Compare canonical order components without a store or database dependency.
pub fn compare_order<'a, 'b>(
    left: impl IntoIterator<Item = OrderComponent<'a>>,
    right: impl IntoIterator<Item = OrderComponent<'b>>,
) -> Ordering {
    use OrderComponent::{Integer, Null, Text};
    for (left, right) in left.into_iter().zip(right) {
        let ordering = match (left, right) {
            (Integer(left), Integer(right)) => left.cmp(&right),
            (Text(left), Text(right)) => left.cmp(right),
            (Null, Null) => Ordering::Equal,
            (OrderComponent::Unsigned(left), OrderComponent::Unsigned(right)) => left.cmp(&right),
            (Null, _) => Ordering::Less,
            (_, Null) => Ordering::Greater,
            (Integer(_), _) => Ordering::Less,
            (_, Integer(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        };
        if ordering != Ordering::Equal { return ordering; }
    }
    Ordering::Equal
}

/// The message attachment supplies the virtual fields signed by devices, not stored fields.
pub fn signature_fields(kind: &str, body: &Value) -> Value {
    let mut fields = body.get("fields").cloned().unwrap_or(Value::Null);
    if kind == "message.sent"
        && let Some(body) = fields.get("attachments").and_then(message_body_entry).cloned()
    {
        fields["body_sha256"] = body.get("sha256").cloned().unwrap_or(Value::Null);
        fields["body_bytes"] = body.get("size").cloned().unwrap_or(Value::Null);
    }
    fields
}

/// Only a cryptographically matching candidate consumes a nonce, regardless of authorization.
/// Untrusted signature metadata alone must never poison a later legitimate claim.
pub fn nonce_eligible(
    id: &str, subject: &str, kind: &str, actor: Option<&str>, body: &Value,
    signature: &ClaimSignature,
) -> bool {
    let fields = signature_fields(kind, body);
    signature.verifies(&Judged {
        id, subject, kind, actor, content: content_digest(subject, kind, actor, body), fields: &fields,
    })
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
