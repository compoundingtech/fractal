//! Read-only, owner-local runtime-start diagnostics. Never expose launch bodies or errors.
use super::*;

// These are provenance identifiers and typed facts, not arbitrary retained JSON.
const PROVENANCE: &[&str] = &[
    "operation_id", "subject", "owner", "origin", "runtime_id", "terminal", "runtime_kind",
    "selected_token", "desired_token", "expected_incarnation", "incarnation", "verification",
    "start_request", "start_request_position", "control_request", "requester_restart",
];

fn scalars(source: &Value, keys: &[&str]) -> Value {
    Value::Object(keys.iter().map(|key| {
        let value = &source[*key];
        ((*key).to_owned(), if value.is_string() || value.is_boolean() || value.is_number() {
            value.clone()
        } else {
            Value::Null
        })
    }).collect())
}

fn receipt(source: &Value) -> Value {
    if !source.is_object() {
        return Value::Null;
    }
    let mut value = scalars(source, &[
        "id", "subject", "kind", "actor", "origin", "accepted_at_unix_ms",
        "store_index", "batch_id", "operation_id", "request_digest",
    ]);
    value["fields"] = scalars(&source["body"]["fields"], &[
        "action", "operation", "runtime_id", "terminal", "incarnation_id",
    ]);
    // Retain only identifier arrays needed to inspect the exact request chain.
    for (key, source) in [
        ("evidence", &source["body"]["evidence"]),
        ("predecessors", &source["predecessors"]),
    ] {
        value[key] = source.as_array().map(|items| {
            Value::Array(items.iter().filter(|item| item.is_string()).cloned().collect())
        }).unwrap_or(Value::Null);
    }
    value
}

fn proof(source: &Value) -> Value {
    if !source.is_object() {
        return Value::Null;
    }
    let mut value = scalars(source, PROVENANCE);
    value["version"] = source["version"].as_u64().into();
    for key in ["start_request_receipt", "control_request_receipt"] {
        value[key] = receipt(&source[key]);
    }
    value["witness"] = scalars(&source["witness"], &[
        "kind", "status", "incarnation", "retirement_request", "retirement_result",
    ]);
    value["witness"]["retirement_request_receipt"] =
        receipt(&source["witness"]["retirement_request_receipt"]);
    value
}

pub trait RuntimeStartDiagnosticsStore {
    /// All local ledger states, including gates and retired declarations. Paging is
    /// performed by the client API's snapshot cache, not public-agent enumeration.
    /// Only SELECTs on the reader connection; this must never repair or close rows.
    fn runtime_start_diagnostics(&self) -> Result<Vec<Value>>;
}

impl RuntimeStartDiagnosticsStore for Store {
    fn runtime_start_diagnostics(&self) -> Result<Vec<Value>> {
        let connection = self.readers.get();
        // Project before decoding: launch arguments/environment and free-form error
        // text are never even copied out of SQLite into the diagnostic response path.
        let mut statement = connection.prepare(
            "SELECT id,subject,owner,revoked_by,finished,applied,cleanup_verified,dispatched,receipted,
             json_object('runtime_id',json_extract(body,'$.member.runtime_id'),
                'terminal',body -> '$.member.terminal',
                'selected_token',json_extract(body,'$.selected_token'),
                'desired_token',json_extract(body,'$.desired_token'),
                'expected_incarnation',json_extract(body,'$.expected_incarnation'),
                'start_request',json_extract(body,'$.start_request'),
                'start_request_position',json_extract(body,'$.start_request_position'),
                'control_request',json_extract(body,'$.control_request'),
                'requester_restart',json_extract(body,'$.request'),
                'start_request_receipt',json_extract(body,'$.start_request_receipt'),
                'control_request_receipt',json_extract(body,'$.control_request_receipt')),
             json_object('phase',json_extract(result,'$.phase'),
                'verification',json_extract(result,'$.verification'),
                'incarnation',json_extract(result,'$.incarnation'),
                'producer_closed',result -> '$.producer_closed',
                'closure_kind',json_extract(result,'$.closure_kind'),
                'closure_proof_id',json_extract(result,'$.closure_proof_id'),
                'closure_proof',json_extract(result,'$.closure_proof')),
             result IS NOT NULL, COALESCE(json_type(result,'$.error') <> 'null',0)
             FROM local_runtime_starts ORDER BY id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?, row.get::<_, bool>(4)?, row.get::<_, bool>(5)?,
                row.get::<_, bool>(6)?, row.get::<_, bool>(7)?, row.get::<_, bool>(8)?,
                row.get::<_, String>(9)?, row.get::<_, String>(10)?,
                row.get::<_, bool>(11)?, row.get::<_, bool>(12)?))
        })?;
        rows.map(|row| {
            let (id, subject, owner, revoked_by, finished, applied, cleanup_verified,
                dispatched, receipted, body, result, has_result, has_error) = row?;
            let body: Value = serde_json::from_str(&body)?;
            let result: Value = serde_json::from_str(&result)?;
            let created_at_unix_ms = Uuid::parse_str(&id).ok()
                .filter(|uuid| uuid.get_version_num() == 7)
                .and_then(|uuid| uuid.get_timestamp())
                .map(|timestamp| {
                    let (seconds, nanos) = timestamp.to_unix();
                    seconds * 1_000 + u64::from(nanos / 1_000_000)
                });
            let phase = if revoked_by.is_some() && !cleanup_verified {
                "revoked"
            } else {
                match result["phase"].as_str() {
                    Some(phase @ ("failed" | "completed" | "unknown" | "ended" | "retired")) => phase,
                    _ if finished => "finished",
                    _ if dispatched => "dispatched",
                    _ => "admitted",
                }
            };
            let mut provenance = scalars(&body, PROVENANCE);
            provenance["operation_id"] = json!(id);
            provenance["subject"] = json!(subject);
            provenance["owner"] = json!(owner);
            provenance["origin"] = json!(self.origin());
            for key in ["start_request_receipt", "control_request_receipt"] {
                provenance[key] = receipt(&body[key]);
            }
            Ok(json!({
                "id": id, "kind": "runtime-start", "operation_id": id,
                "subject": subject, "owner": owner, "phase": phase,
                "dispatched": dispatched, "finished": finished, "applied": applied,
                "receipted": receipted, "revoked_by": revoked_by,
                "result_kind": if !has_result { "none" } else if has_error { "error" } else { "result" },
                "result_phase": result["phase"].as_str(),
                "producer_closed": result["producer_closed"].as_bool(),
                "closure_kind": result["closure_kind"].as_str(),
                "closure_proof_id": result["closure_proof_id"].as_str(),
                "verification": result["verification"].as_str(),
                "incarnation": result["incarnation"].as_str(),
                "provenance": provenance, "closure_proof": proof(&result["closure_proof"]),
                "cleanup_verified": cleanup_verified,
                "created_at_unix_ms": created_at_unix_ms,
                "requested_at_unix_ms": body["start_request_receipt"]["accepted_at_unix_ms"].as_u64(),
                "dispatched_at_unix_ms": null, "finished_at_unix_ms": null,
                "timestamp_sources": {
                    "created_at_unix_ms": "operation-id-uuidv7",
                    "requested_at_unix_ms": "start_request_receipt.accepted_at_unix_ms",
                    "dispatched_at_unix_ms": "unavailable", "finished_at_unix_ms": "unavailable"
                }
            }))
        }).collect()
    }
}
