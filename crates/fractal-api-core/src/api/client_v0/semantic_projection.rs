use crate::transcript_semantics::{Category, PROJECTION_REVISION};
use serde_json::{Value, json};

/// Select the negotiated semantic lane without changing owner-retained native records.
pub(super) fn prepare(item: &mut Value, enabled: bool) {
    if !enabled {
        if let Some(blocks) = item["body"]["blocks"].as_array_mut() {
            for block in blocks {
                if let Some(block) = block.as_object_mut() { block.remove("semantic"); }
            }
        }
        return;
    }
    let id = item["id"].as_str().unwrap_or("entry").to_owned();
    let Some(body) = item["body"].as_object_mut() else { return };
    let text = body.remove("text");
    let had_text = text.is_some();
    let error_text = body.get("message").and_then(Value::as_str).map(str::to_owned);
    let Some(blocks) = body.get_mut("blocks").and_then(Value::as_array_mut) else {
        if let Some(text) = text { body.insert("text".into(), text); }
        return;
    };
    let presentation_count = blocks.iter().filter(|block| block["kind"] != "source_record").count();
    let mut text = text;
    for block in blocks {
        if block.get("semantic").is_none() {
            let category = match block["kind"].as_str().unwrap_or("") {
                "text" | "reasoning" | "image" | "image_link" | "document" | "irc" => Category::Conversation,
                "tool_call" | "tool_output" | "job" | "subagent" | "ask" => Category::ToolActivity,
                "error" | "raw_text" => Category::SourceDiagnostic,
                "source_record" => Category::InternalActivity,
                _ => Category::Unknown,
            };
            let part = block["id"].as_str().unwrap_or(&id);
            let disposition = if matches!(block["visibility"].as_str(), Some("internal" | "hidden-by-harness")) {
                crate::transcript_semantics::Disposition::Inspector
            } else { category.disposition() };
            block["semantic"] = json!({"revision":PROJECTION_REVISION,"category":category,
                "disposition":disposition,"source_record_id":id,"source_part_id":part});
        }
        let reading_text = block["kind"] == "text" && matches!(block["semantic"]["category"].as_str(), Some("conversation" | "actionable_notice"));
        if reading_text && block["semantic"].get("text").is_none() {
            // Per-block native text takes precedence; a whole-body text is safe only alone.
            let own_text = block["payload"].as_object_mut().and_then(|payload| payload.remove("text"));
            if let Some(own) = own_text.or_else(|| (presentation_count == 1).then(|| text.take()).flatten()) {
                block["semantic"]["text"] = own;
            } else {
                block["semantic"]["category"] = json!("source_diagnostic");
                block["semantic"]["disposition"] = json!("reading_lane");
                block["semantic"]["text"] = json!("Mixed content has no independent text; inspect the source");
            }
        } else if block["kind"] == "error" && let Some(message) = &error_text {
            block["semantic"]["text"] = json!(message);
        }
    }
    // Content's legacy required text field remains structurally valid but is not a raw fallback.
    if had_text || body.get("media_type").is_some() {
        body.insert("text".into(), json!(""));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_projection_capability_isolation_and_mixed_text() {
        let original = json!({"id":"one","body":{"media_type":"text/plain","text":"legacy",
            "blocks":[{"id":"part","kind":"text","payload":{"body_ref":true},
                "semantic":{"revision":1,"category":"conversation","disposition":"reading_lane","source_record_id":"source","source_part_id":"part"}}]}});
        let mut legacy = original.clone();
        prepare(&mut legacy, false);
        assert_eq!(legacy["body"]["text"], "legacy");
        assert!(legacy["body"]["blocks"][0].get("semantic").is_none());
        let mut semantic = original;
        prepare(&mut semantic, true);
        assert_eq!(semantic["body"]["blocks"][0]["semantic"]["text"], "legacy");
        assert_eq!(semantic["body"]["text"], "");
        let prepared = semantic.clone();
        prepare(&mut semantic, true);
        assert_eq!(semantic, prepared, "owner preparation plus relay finalization is idempotent");
        let mut mixed = json!({"id":"mixed","body":{"text":"never use whole-body JSON",
            "blocks":[{"id":"prose","kind":"text","payload":{"text":"real prose"}},
                {"id":"future","kind":"unknown","payload":{"raw":{"type":"future"}}}]}});
        prepare(&mut mixed, true);
        assert_eq!(mixed["body"]["blocks"][0]["semantic"]["text"], "real prose");
        assert_eq!(mixed["body"]["blocks"][1]["semantic"]["category"], "unknown");
        assert_eq!(mixed["body"]["text"], "");
        let mut ambiguous = json!({"id":"mixed-old","body":{"text":"whole body cannot be split",
            "blocks":[{"id":"text","kind":"text","payload":{"body_ref":true}},
                {"id":"internal","kind":"unknown","payload":{"raw":{}}}]}});
        prepare(&mut ambiguous, true);
        assert_eq!(ambiguous["body"]["blocks"][0]["semantic"]["category"], "source_diagnostic");
        assert_eq!(ambiguous["body"]["text"], "");
    }

    #[test]
    fn semantic_projection_unknown_never_uses_raw_text_as_prose() {
        let mut item = json!({"id":"future","body":{"text":"[unrecognized omp entry] {raw}",
            "blocks":[{"id":"unknown","kind":"unknown","payload":{"raw":{"type":"future"}}}]}});
        prepare(&mut item, true);
        assert_eq!(item["body"]["blocks"][0]["semantic"]["disposition"], "inspector");
        assert_eq!(item["body"]["text"], "");
        assert!(item["body"]["blocks"][0]["semantic"].get("text").is_none());
        assert_eq!(item["body"]["blocks"][0]["payload"]["raw"]["type"], "future");
    }
}
