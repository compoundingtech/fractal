use super::ExternalDriver;
use crate::transcript_semantics::{Category, PROJECTION_REVISION, classify};
use serde_json::{Value, json};

/// Owner-local annotations are removed unless the reader negotiates semantic projection.
pub(super) fn annotate(driver: ExternalDriver, record: &Value, sequence: u64, items: &mut [Value]) {
    let root = classify(driver.as_str(), record, None);
    let source_id = format!("native-{sequence}/source");
    let mut assistant_usage_attached = false;
    for item in items {
        let error = item["type"] == "error";
        let part = native_part(driver, record, sequence, item["sequence"].as_u64());
        let Some(blocks) = item["body"]["blocks"].as_array_mut() else { continue };
        for block in blocks {
            let category = match block["kind"].as_str().unwrap_or("") {
                "source_record" => Category::InternalActivity,
                "raw_text" => Category::SourceDiagnostic,
                "error" if error && record.is_null() => Category::SourceDiagnostic,
                "error" => Category::ActionableNotice,
                _ if part.is_some() => classify(driver.as_str(), record, part),
                "tool_call" | "tool_output" | "job" | "subagent" | "ask" => Category::ToolActivity,
                "text" | "reasoning" | "image" if record.pointer("/message/role").and_then(Value::as_str) == Some("assistant") => Category::Conversation,
                "unknown" if block.pointer("/payload/raw/type") != record.get("type") => {
                    classify(driver.as_str(), record, block.pointer("/payload/raw"))
                }
                _ => root,
            };
            let category = if block["kind"] == "unknown" && matches!(category,
                Category::Conversation | Category::ToolActivity | Category::ActionableNotice) {
                Category::SourceDiagnostic
            } else { category };
            let disposition = if matches!(block["visibility"].as_str(), Some("internal" | "hidden-by-harness")) {
                crate::transcript_semantics::Disposition::Inspector
            } else { category.disposition() };
            let part_id = block["id"].as_str().unwrap_or(&source_id);
            let mut semantic = json!({"revision":PROJECTION_REVISION,"category":category,
                "disposition":disposition,"source_record_id":source_id,"source_part_id":part_id});
            if record["type"] == "ttsr_injection" && block["kind"] != "source_record" {
                semantic["text"] = json!("Rule applied");
            }
            if category == Category::SourceDiagnostic && block["kind"] != "error" {
                semantic["text"] = json!("Native event has no valid typed projection; inspect source");
            }
            if category == Category::Usage {
                semantic["usage"] = usage(record, false);
            } else if !assistant_usage_attached && block["kind"] != "source_record"
                && record.pointer("/message/usage").is_some()
            {
                semantic["usage"] = usage(record, true);
                assistant_usage_attached = true;
            }
            block["semantic"] = semantic;
        }
    }
}

/// Normalizers reserve the root sequence for the message, then one sequence per native part.
/// Use that source offset, never the normalized block kind, to classify open content arrays.
fn native_part(driver: ExternalDriver, record: &Value, sequence: u64, item_sequence: Option<u64>) -> Option<&Value> {
    let (content, first) = match (driver.as_str(), record["type"].as_str()) {
        ("codex", Some("response_item")) if record["payload"]["type"] == "message" =>
            (&record["payload"]["content"], sequence.checked_add(1)?),
        ("claude", Some("user" | "assistant")) =>
            (&record["message"]["content"], sequence.checked_add(1)?),
        ("omp" | "pi", Some("message")) if record["message"]["role"] != "toolResult" =>
            (&record["message"]["content"], sequence.checked_add(1)?),
        ("omp" | "pi", Some("custom_message")) => (&record["content"], sequence),
        _ => return None,
    };
    let offset = usize::try_from(item_sequence?.checked_sub(first)?).ok()?;
    content.as_array()?.get(offset)
}

fn usage(record: &Value, assistant: bool) -> Value {
    let source = if assistant { &record["message"] } else { record };
    let mut facts = json!({"semantics":"unclassified"});
    // Native response IDs establish response identity; a JSONL bookkeeping ID does not.
    if assistant && let Some(id) = source.get("id").and_then(Value::as_str) {
        facts["semantics"] = json!("response");
        facts["response_id"] = json!(id);
    }
    let counts = &source["usage"];
    for (native, alias, normalized) in [("input", "input_tokens", "input_tokens"),
        ("output", "output_tokens", "output_tokens"), ("cacheRead", "cache_read_input_tokens", "cache_read_tokens"),
        ("cacheWrite", "cache_creation_input_tokens", "cache_write_tokens"), ("totalTokens", "total_tokens", "total_tokens")] {
        if let Some(number) = counts.get(native).or_else(|| counts.get(alias)).and_then(Value::as_u64) {
            facts[normalized] = json!(number);
        }
    }
    if let Some(cost) = counts.pointer("/cost/total").filter(|value| value.as_f64().is_some_and(|n| n >= 0.0)) {
        facts["cost"] = cost.clone();
    }
    for key in ["currency", "provider", "model"] {
        if let Some(value) = source.get(key).filter(|value| value.is_string()) { facts[key] = value.clone(); }
    }
    // Codex names a cumulative session counter, not independent token-count deltas.
    if let Some(total) = record.pointer("/payload/info/total_token_usage") {
        facts["semantics"] = json!("session_cumulative");
        facts["counter_id"] = json!("codex/session_usage");
        for key in ["input_tokens", "output_tokens", "total_tokens"] {
            if let Some(number) = total.get(key).and_then(Value::as_u64) { facts[key] = json!(number); }
        }
        if let Some(number) = total.get("cached_input_tokens").and_then(Value::as_u64) {
            facts["cache_read_tokens"] = json!(number);
        }
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_sessions::normalize_native_line;

    fn normalized(record: Value) -> Vec<Value> {
        let mut items = Vec::new();
        normalize_native_line(ExternalDriver::Omp, &record, 100, "2026-10-04T00:00:00Z", &mut items);
        items
    }

    #[test]
    fn semantic_projection_admin_annotations_keep_legacy_text_and_source() {
        let fixture = include_str!("../../../fractal-daemon/fixtures/omp-admin-events.jsonl");
        for line in fixture.lines() {
            let record: Value = serde_json::from_str(line).unwrap();
            let items = normalized(record.clone());
            let block = &items[0]["body"]["blocks"][0];
            assert_eq!(block["payload"]["raw"], record);
            assert_eq!(block["semantic"]["revision"], PROJECTION_REVISION);
            assert_eq!(block["semantic"]["source_record_id"], "native-100/source");
            assert!(items[0]["body"]["text"].as_str().unwrap().starts_with("[omp "));
            if record["type"] == "model_usage" {
                assert_eq!(block["semantic"]["category"], "usage");
                assert_eq!(block["semantic"]["usage"]["total_tokens"], 0);
                assert_eq!(block["semantic"]["usage"]["semantics"], "unclassified");
            }
            if record["type"] == "ttsr_injection" { assert_eq!(block["semantic"]["text"], "Rule applied"); }
        }
    }

    #[test]
    fn semantic_projection_unknown_and_hidden_records_do_not_become_prose() {
        let items = normalized(json!({"type":"future_event"}));
        assert_eq!(items[0]["body"]["blocks"][0]["semantic"]["category"], "unknown");
        assert_eq!(items[0]["body"]["blocks"][0]["semantic"]["disposition"], "inspector");
        let hidden = normalized(json!({"type":"model_usage","display":false}));
        assert_eq!(hidden[0]["body"]["blocks"][0]["semantic"]["disposition"], "inspector");
    }

    #[test]
    fn semantic_projection_usage_identity_and_absence_are_explicit() {
        let response = json!({"message":{"id":"response-1","usage":{"input":0,"output":5}}});
        let facts = usage(&response, true);
        assert_eq!(facts["response_id"], "response-1");
        assert_eq!(facts["input_tokens"], 0);
        assert!(facts.get("total_tokens").is_none());
        assert!(facts.get("cost").is_none());
        assert_eq!(usage(&json!({"usage":{}}), false), json!({"semantics":"unclassified"}));
        let snapshot = json!({"payload":{"info":{"total_token_usage":{"total_tokens":10}}}});
        assert_eq!(usage(&snapshot, false), json!({"semantics":"session_cumulative","counter_id":"codex/session_usage","total_tokens":10}));
    }

    #[test]
    fn semantic_projection_malformed_source_is_diagnostic_and_usage_attaches_once() {
        let mut malformed = Vec::new();
        crate::external_sessions::normalize_native_bytes(ExternalDriver::Omp, b"{torn\n",
            100, "2026-10-04T00:00:00Z", 0, false, &mut malformed);
        assert_eq!(malformed[0]["body"]["blocks"][0]["semantic"]["category"], "source_diagnostic");
        assert_eq!(malformed[0]["body"]["blocks"][0]["payload"]["text"], "{torn\n");
        let items = normalized(json!({"type":"message","message":{"id":"response-one",
            "role":"assistant","usage":{"input":3},"content":[
                {"type":"thinking","thinking":"thought"},{"type":"text","text":"actual prose"},
                {"type":"future_part","content":"unclassified"}]}}));
        let blocks: Vec<_> = items.iter().filter_map(|item| item["body"]["blocks"].as_array())
            .flatten().collect();
        assert_eq!(blocks.iter().filter(|block| block.pointer("/semantic/usage").is_some()).count(), 1);
        assert!(blocks.iter().any(|block| block["semantic"]["category"] == "unknown"));
        let broken = normalized(json!({"type":"custom","customType":"tool_execution_start",
            "data":{"toolCallId":"call-one","toolName":"bash"}}));
        assert_eq!(broken[0]["body"]["blocks"][0]["semantic"]["category"], "source_diagnostic");
    }

    #[test]
    fn semantic_projection_native_part_discriminators_cannot_be_bypassed_by_text() {
        for (driver, record, expected) in [
            (ExternalDriver::Codex, json!({"type":"response_item","payload":{"type":"message",
                "role":"assistant","content":[{"type":"future_part","text":"opaque data"},
                    {"type":"output_text","text":"actual prose"}]}}), "unknown"),
            (ExternalDriver::Claude, json!({"type":"assistant","message":{"role":"assistant",
                "content":[{"text":"malformed data"},{"type":"text","text":"actual prose"}]}}), "source_diagnostic"),
            (ExternalDriver::Omp, json!({"type":"message","message":{"role":"assistant",
                "content":[{"text":"malformed data"},{"type":"text","text":"actual prose"}]}}), "source_diagnostic"),
        ] {
            let mut items = Vec::new();
            normalize_native_line(driver, &record, 100, "2026-10-04T00:00:00Z", &mut items);
            let part = items.iter().find(|item| item["sequence"] == 101).unwrap();
            assert_eq!(part["body"]["blocks"][0]["semantic"]["category"], expected);
            assert_ne!(part["body"]["blocks"][0]["semantic"]["disposition"], "reading_lane");
            let prose = items.iter().find(|item| item["sequence"] == 102).unwrap();
            assert_eq!(prose["body"]["blocks"][0]["semantic"]["category"], "conversation");
            assert!(items.iter().flat_map(|item| item["body"]["blocks"].as_array().unwrap())
                .any(|block| block["kind"] == "source_record" && block["payload"]["raw"] == record));
        }
    }
}
