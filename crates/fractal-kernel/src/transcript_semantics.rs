//! Native classification contract; wire negotiation and presentation consume this independently.
//! See docs/st3/conversation-normalization-design.md. No source payload is copied or discarded.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Conversation,
    ToolActivity,
    ActionableNotice,
    Usage,
    InternalActivity,
    SourceDiagnostic,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition { ReadingLane, Indicator, Inspector }

impl Category {
    pub const fn disposition(self) -> Disposition {
        match self {
            Self::Conversation | Self::ToolActivity | Self::ActionableNotice | Self::SourceDiagnostic => Disposition::ReadingLane,
            Self::Usage => Disposition::Indicator,
            Self::InternalActivity | Self::Unknown => Disposition::Inspector,
        }
    }
}

fn word<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Classify a record, or one independently identified part from that record.
/// A record-level category does not classify its nested parts: callers must visit them.
/// Unknown is a supported runtime outcome, but observed fixture inventory must classify it.
/// Native display:false overrides disposition, not category, at projection time.
pub fn classify(harness: &str, record: &Value, part: Option<&Value>) -> Category {
    use Category::{ActionableNotice as Notice, Conversation as Chat, InternalActivity as Internal,
        SourceDiagnostic as Diagnostic, ToolActivity as Tool, Unknown, Usage};
    if !matches!(harness, "omp" | "pi" | "claude" | "codex" | "opencode") { return Unknown; }
    if let Some(part) = part {
        let parent = classify(harness, record, None);
        if matches!(parent, Unknown | Internal | Usage | Diagnostic) { return parent; }
        let category = match (harness, word(part, "type")) {
            ("omp" | "pi", "text" | "thinking" | "image") |
            ("claude", "text" | "thinking" | "redacted_thinking" | "image" | "document") |
            ("codex", "input_text" | "output_text" | "summary_text" | "input_image" | "output_image" | "refusal") => Chat,
            ("omp" | "pi", "toolCall" | "toolUse") |
            ("claude", "tool_use" | "tool_result") => Tool,
            (_, "") => Diagnostic,
            _ => Unknown,
        };
        return if parent == Tool && category == Chat { Tool } else { category };
    }
    match (harness, word(record, "type")) {
        (_, "") => Diagnostic,
        ("omp" | "pi", "session" | "session_init" | "title_change" | "title" | "credential_pin" | "label" | "session_info") => Internal,
        ("omp" | "pi", "model_usage") => Usage,
        ("omp" | "pi", "ttsr_injection" | "model_change" | "thinking_level_change" | "reset_boundary") => Notice,
        ("omp" | "pi", "compaction" | "branch_summary") => Chat,
        ("omp" | "pi", "custom_message") => match word(record, "customType") {
            "launch-completion" | "async-result" => Tool,
            "skill-prompt" => Notice,
            _ => Chat, // Native extension messages are an explicitly open conversation family.
        },
        ("omp" | "pi", "custom") => match word(record, "customType") {
            "tool_execution_start" => Tool,
            "session_exit" => Notice,
            _ => Unknown,
        },
        ("omp" | "pi", "message") => match word(&record["message"], "role") {
            "assistant" if matches!(word(&record["message"], "stopReason"), "error" | "aborted") => Notice,
            "user" | "assistant" => Chat,
            "toolResult" => Tool,
            "system" | "developer" => Notice,
            _ => Unknown,
        },
        ("claude", "user" | "assistant") => match word(&record["message"], "role") {
            "user" | "assistant" => Chat,
            _ => Unknown,
        },
        ("claude", "attachment") => match word(&record["attachment"], "type") {
            "queued_command" => Chat,
            "directory" => Internal,
            _ => Unknown,
        },
        ("claude", "system") => match word(record, "subtype") {
            "api_error" => Notice,
            "turn_duration" => Internal,
            _ => Unknown,
        },
        ("claude", "permission-mode" | "mode") => Notice,
        ("claude", "progress" | "file-history-snapshot" | "file-history-delta" | "queue-operation" |
            "atis-latch" | "last-prompt" | "ai-title" | "custom-title" | "cost-state" |
            "agent-name" | "tag" | "pr-link" | "bridge-session" | "fork-context-ref") => Internal,
        ("codex", "session_meta" | "turn_context" | "world_state") => Internal,
        ("codex", "token_usage_record") => Usage,
        ("codex", "event_msg") => match word(&record["payload"], "type") {
            "token_count" => Usage,
            _ => Unknown, // Do not duplicate event mirrors whose semantics are unproven.
        },
        ("codex", "response_item") => match word(&record["payload"], "type") {
            "message" => match word(&record["payload"], "role") {
                "user" | "assistant" => Chat,
                "system" | "developer" => Internal,
                _ => Unknown,
            },
            "function_call" | "function_call_output" | "custom_tool_call" | "custom_tool_call_output" => Tool,
            "reasoning" => Chat,
            "ghost_snapshot" => Internal,
            _ => Unknown,
        },
        // OpenCode SQLite message/part records use their own row shape, not JSONL envelopes.
        ("opencode", _) => Unknown,
        _ => Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const VECTORS: &str = include_str!("../../fractal-daemon/fixtures/native-records/semantic-vectors.jsonl");

    #[test]
    fn semantic_projection_vectors() {
        for line in VECTORS.lines() {
            let vector: Value = serde_json::from_str(line).unwrap();
            let category = classify(word(&vector, "harness"), &vector["record"], vector.get("part"));
            assert_eq!(serde_json::to_value(category).unwrap(), vector["category"], "{vector}");
            assert_eq!(serde_json::to_value(category.disposition()).unwrap(), vector["disposition"], "{vector}");
        }
    }

    #[test]
    fn semantic_projection_observed_discriminators_are_classified() {
        let fixtures = [
            ("omp", include_str!("../../fractal-daemon/fixtures/omp-admin-events.jsonl")),
            ("omp", include_str!("../../fractal-daemon/fixtures/omp-native-events.jsonl")),
            ("omp", include_str!("../../fractal-daemon/fixtures/omp-tool-results.jsonl")),
            ("omp", include_str!("../../fractal-daemon/fixtures/native-records/omp-parity.jsonl")),
            ("claude", include_str!("../../fractal-daemon/fixtures/native-records/claude.jsonl")),
            ("codex", include_str!("../../fractal-daemon/fixtures/native-records/codex.jsonl")),
        ];
        for (harness, fixture) in fixtures {
            for line in fixture.lines() {
                let record: Value = serde_json::from_str(line).unwrap();
                assert_ne!(classify(harness, &record, None), Category::Unknown, "unclassified {harness}: {record}");
                let content = record.pointer("/message/content").or_else(|| record.pointer("/payload/content"));
                if let Some(parts) = content.and_then(Value::as_array) {
                    for part in parts {
                        assert_ne!(classify(harness, &record, Some(part)), Category::Unknown, "unclassified {harness} part: {part}");
                    }
                }
            }
        }
    }

    #[test]
    fn semantic_projection_evolution_and_mixed_parts() {
        let record = json!({"type":"message","message":{"role":"assistant"},"future_field":42});
        assert_eq!(classify("omp", &record, None), Category::Conversation);
        assert_eq!(classify("omp", &record, Some(&json!({"type":"toolCall"}))), Category::ToolActivity);
        assert_eq!(classify("omp", &record, Some(&json!({"type":"future_part"}))), Category::Unknown);
        assert_eq!(classify("omp", &json!({"type":"future_event"}), None), Category::Unknown);
        assert_eq!(classify("omp", &json!({}), None), Category::SourceDiagnostic);
        assert_eq!(serde_json::from_value::<Category>(json!("future_category")).unwrap(), Category::Unknown);
        assert_eq!(record["future_field"], 42);
    }
}
