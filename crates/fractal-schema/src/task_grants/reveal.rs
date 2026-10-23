//! RevealOnce source/trigger contract mirrors http-capture browser-control.ts.
use super::{Locator, TaskBrowserStep, TaskGrantError, TaskProvider, invalid};
use super::locator::bounded_text;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRevealTrigger {
    pub locator: Locator,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskRevealSourceKind { Input, MaskedText, TextNode }

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskTextNode {
    pub pattern: String,
}

// Capture permits ASCII literal/class atoms, with at most one quantified atom
// per escaped-dot-separated segment. This is deliberately not arbitrary regex.
fn safe_pattern(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut index = 0;
    let mut quantified = false;
    let mut previous_atom = false;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"\\.") {
            if !previous_atom || index + 2 == bytes.len() { return false; }
            quantified = false;
            previous_atom = false;
            index += 2;
            continue;
        }
        if bytes[index] == b'[' {
            let start = index + 1;
            let Some(end) = bytes[start..].iter().position(|value| *value == b']').map(|offset| start + offset) else { return false; };
            if end == start || !bytes[start..end].iter().all(|value| value.is_ascii_alphanumeric() || matches!(*value, b'_' | b'-')) { return false; }
            // The only possible JS Unicode-regex error in these ASCII classes
            // is a descending range; leading/trailing hyphens remain literals.
            let mut atom = start;
            while atom < end {
                if atom + 2 < end && bytes[atom + 1] == b'-' {
                    if bytes[atom] > bytes[atom + 2] { return false; }
                    atom += 3;
                } else { atom += 1; }
            }
            index = end + 1;
        } else if bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-') {
            index += 1;
        } else { return false; }
        previous_atom = true;
        let quantifier_start = index;
        match bytes.get(index) {
            Some(b'+' | b'*' | b'?') => index += 1,
            Some(b'{') => {
                index += 1;
                let start = index;
                while bytes.get(index).is_some_and(u8::is_ascii_digit) && index - start < 3 { index += 1; }
                if index == start { return false; }
                let minimum = bytes[start..index].iter().fold(0u16, |value, digit| value * 10 + u16::from(digit - b'0'));
                if bytes.get(index) == Some(&b',') {
                    index += 1;
                    let start = index;
                    while bytes.get(index).is_some_and(u8::is_ascii_digit) && index - start < 3 { index += 1; }
                    if index > start {
                        let maximum = bytes[start..index].iter().fold(0u16, |value, digit| value * 10 + u16::from(digit - b'0'));
                        if maximum < minimum { return false; }
                    }
                }
                if bytes.get(index) != Some(&b'}') { return false; }
                index += 1;
            }
            _ => {}
        }
        if index != quantifier_start {
            if quantified { return false; }
            quantified = true;
        }
    }
    previous_atom
}

pub(super) fn validate(step: &TaskBrowserStep, provider: &TaskProvider) -> Result<(), TaskGrantError> {
    // Source and trigger each have their own capture locator budget.
    step.trigger.as_ref().ok_or_else(|| invalid("revealOnce requires trigger"))?.locator.validate()?;
    match (&step.source_kind, &step.text_node) {
        (Some(TaskRevealSourceKind::TextNode), Some(node)) => {
            bounded_text(&node.pattern, 256)?;
            if !safe_pattern(&node.pattern) { return Err(invalid("textNode.pattern must use safe dot-delimited literal or character-class segments")); }
        }
        (Some(TaskRevealSourceKind::TextNode), None) => return Err(invalid("sourceKind textNode requires textNode")),
        (_, Some(_)) => return Err(invalid("textNode requires sourceKind textNode")),
        _ => {}
    }
    if let Some(intent) = &step.intent { bounded_text(intent, 256)?; }
    if let Some(safe_url) = &step.safe_url {
        bounded_text(safe_url, 4096)?;
        if safe_url.len() > 4096 { return Err(invalid("safeUrl exceeds 4096 UTF-8 bytes")); }
        let url = url::Url::parse(safe_url).map_err(|_| invalid("safeUrl must be HTTPS"))?;
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
            || url.fragment().is_some() || !provider.origins.iter().any(|origin| {
                url::Url::parse(origin).is_ok_and(|origin| origin.origin() == url.origin())
            }) {
            return Err(invalid("safeUrl must be credential-free HTTPS without a fragment and within provider origins"));
        }
    }
    Ok(())
}
