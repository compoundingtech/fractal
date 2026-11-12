//! Shared transcript publication policy for driver events and exit evidence.

const MAX_STRING_CHARS: usize = 8_192;

/// Normalize conversation text, counting withheld bytes and truncated characters.
pub fn normalize_conversation_text(text: &str) -> (String, u64, u64) {
    let original_chars = text.chars().count();
    let bounded = text.chars().take(MAX_STRING_CHARS).collect::<String>();
    let mut redacted_bytes = 0_u64;
    let mut output = String::with_capacity(bounded.len());
    let mut cursor = 0;
    let mut redact_next = false;
    let words = bounded.match_indices(|character: char| !character.is_whitespace());
    for (start, _) in words {
        if start < cursor {
            continue;
        }
        let end = bounded[start..]
            .find(char::is_whitespace)
            .map_or(bounded.len(), |offset| start + offset);
        let word = &bounded[start..end];
        output.push_str(&bounded[cursor..start]);
        let lower = word.to_ascii_lowercase();
        let sensitive = redact_next
            || lower.starts_with("sk-")
            || lower.starts_with("ghp_")
            || word.starts_with("AKIA")
            || lower.starts_with("token=")
            || lower.starts_with("password=")
            || lower.starts_with("authorization=");
        if sensitive {
            redacted_bytes = redacted_bytes.saturating_add(word.len() as u64);
            if let Some((prefix, _)) = word.split_once('=') {
                output.push_str(prefix);
                output.push_str("=[REDACTED]");
            } else {
                output.push_str("[REDACTED]");
            }
            redact_next = false;
        } else {
            output.push_str(word);
            redact_next = lower == "bearer" || lower == "authorization:";
        }
        cursor = end;
    }
    output.push_str(&bounded[cursor..]);
    (
        output,
        redacted_bytes,
        original_chars.saturating_sub(MAX_STRING_CHARS) as u64,
    )
}

/// PTY output is not a structured transcript: refuse control/escape sequences
/// rather than let them obscure sensitive words from the shared token policy.
pub fn exit_output_tail(text: &str) -> Option<String> {
    if text.chars().any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t')) {
        return None;
    }
    let (mut safe, redacted_bytes, _) = normalize_conversation_text(text);
    // A terminal tail is less structured than prose. Withhold it entirely if
    // the transcript policy detects sensitive content, including tool payloads.
    if redacted_bytes > 0 {
        return Some("[REDACTED: sensitive terminal output]".into());
    }
    const MAX_TAIL_BYTES: usize = 8192;
    if safe.len() > MAX_TAIL_BYTES {
        let mut end = MAX_TAIL_BYTES;
        while !safe.is_char_boundary(end) { end -= 1; }
        safe.truncate(end);
    }
    Some(safe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_tail_uses_transcript_secret_policy_and_refuses_unsafe_controls() {
        let safe = exit_output_tail("fatal: failed token=fixture password=fixture sk-fixture ghp_fixture AKIAfixture Authorization: Bearer fixture").unwrap();
        assert_eq!(safe, "[REDACTED: sensitive terminal output]");
        assert!(!safe.contains("fixture"));
        assert_eq!(exit_output_tail("fatal: \u{1b}[31msecret"), None);
        assert!(exit_output_tail(&"界".repeat(8192)).unwrap().len() <= 8192);
    }
}
