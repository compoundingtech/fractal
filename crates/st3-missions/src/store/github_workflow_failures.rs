//! The part of `store::github_workflow_failures` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::resource::github_workflows::PerformanceFailure;
pub use st3_delivery::store::github_workflow_failures::*;

#[allow(clippy::too_many_arguments)]
pub fn deliver_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    batch: &str,
    observer: &str,
    resource: &str,
    subscription: &str,
    recipient: &str,
    repository_id: &Value,
    failures: &[Value],
    baseline: bool,
) -> Result<Vec<String>, St3Error> {
    let mut messages = Vec::new();
    // Each recipient establishes its own baseline, including when the first listing is
    // empty. Registering later cannot turn another recipient's baseline into a history flood.
    let baseline_identity = repository_id
        .as_u64()
        .map(Value::from)
        .unwrap_or_else(|| Value::String(resource.to_owned()));
    let baseline_key = canonical_hash(&(
        "main-performance-recipient-baseline",
        recipient,
        baseline_identity,
    ))
    .map_err(internal)?;
    let baseline_subject = format!("resource/github-workflow-baseline/{baseline_key}");
    let has_baseline = latest_actual(transaction, &baseline_subject)
        .map_err(internal)?
        .is_some();
    let baseline = baseline || !has_baseline;
    if !has_baseline {
        append_claim_tx(transaction, origin, &baseline_subject, "resource.observed", None,
            &json!({"fields": {"kind": "custom.github.workflow-failure-baseline", "observer": observer,
                "facts": {"recipient": recipient, "resource": resource, "repository_id": repository_id}}}),
            &[], Some(batch)).map_err(claim_append_error)?;
    }
    for facts in failures {
        let Some(failure) = PerformanceFailure::from_facts(facts) else {
            continue;
        };
        // The subscription and observer names are intentionally absent: replacement
        // registrations to the same recipient cannot redeliver a known attempt/head.
        let repository_identity = repository_id
            .as_u64()
            .map(Value::from)
            .unwrap_or_else(|| Value::String(failure.repository.clone()));
        let key = canonical_hash(&(
            "main-performance-failure",
            recipient,
            repository_identity,
            failure.run_id,
            failure.run_attempt,
            &failure.head_sha,
        ))
        .map_err(internal)?;
        let receipt = format!("resource/github-workflow-failure/{key}");
        if latest_actual(transaction, &receipt)
            .map_err(internal)?
            .is_some()
        {
            continue;
        }
        let record = append_claim_tx(
            transaction,
            origin,
            &receipt,
            "resource.observed",
            None,
            &json!({"fields": {
                "kind": "custom.github.workflow-failure",
                "observer": observer,
                "facts": {"failure": failure, "recipient": recipient, "baseline": baseline},
            }}),
            &[],
            Some(batch),
        )
        .map_err(claim_append_error)?;
        if baseline {
            continue;
        }
        let subject = format!("message/workflow-failure-{}", &key[..20]);
        let mut content = serde_json::to_value(&failure).map_err(internal)?;
        content["priority"] = Value::String("P0".into());
        append_claim_tx(transaction, origin, &subject, "message.sent", None,
            &json!({"fields": {
                "from": format!("daemon/{origin}"),
                "to": recipient,
                "title": format!("P0: main Performance failed: {} run {}/{}", failure.repository, failure.run_id, failure.run_attempt),
                "content": serde_json::to_string(&content).map_err(internal)?,
                "status": "sent",
                "tags": ["P0", "github-workflow-failure", subscription, resource],
            }, "evidence": [record.id]}), &[], Some(batch)).map_err(claim_append_error)?;
        messages.push(subject);
    }
    Ok(messages)
}
