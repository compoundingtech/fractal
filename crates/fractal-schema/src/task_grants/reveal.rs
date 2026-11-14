use super::browser::{TaskBrowserStepV1, TaskRevealSourceKindV1};
use super::{TaskManifestError, TaskProviderV1, invalid};

pub(super) fn bounded_text(value: &str, max: usize) -> Result<(), TaskManifestError> {
    if super::ecma_blank(value) || value.contains('\0')
        || value.encode_utf16().count() > max
    {
        return Err(invalid("text is blank, contains NUL, or exceeds its UTF-16 bound"));
    }
    Ok(())
}

pub(super) fn validate(
    step: &TaskBrowserStepV1,
    provider: &TaskProviderV1,
) -> Result<(), TaskManifestError> {
    let TaskBrowserStepV1::RevealOnce {
        locator, trigger, source_kind, text_node, intent, safe_url, ..
    } = step else {
        return Ok(());
    };
    locator.validate()?;
    trigger.locator.validate()?;
    if (*source_kind == Some(TaskRevealSourceKindV1::TextNode)) != text_node.is_some() {
        return Err(invalid("textNode is required iff sourceKind is textNode"));
    }
    if let Some(node) = text_node {
        bounded_text(&node.pattern, 256)?;
        if !valid_pattern(&node.pattern) {
            return Err(invalid("textNode pattern does not match the restricted grammar"));
        }
    }
    if let Some(intent) = intent {
        bounded_text(intent, 256)?;
    }
    if let Some(value) = safe_url {
        bounded_text(value, 4096)?;
        if value.len() > 4096 {
            return Err(invalid("safeUrl exceeds 4096 UTF-8 bytes"));
        }
        let parsed = super::validation::declared_url(value, provider, false)?;
        if parsed.fragment().is_some() {
            return Err(invalid("safeUrl must not have a fragment"));
        }
    }
    Ok(())
}

fn atom(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

/// A byte cursor suffices: the entire grammar is ASCII, including delimiters.
fn valid_pattern(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut at = 0;
    let mut atoms = 0;
    let mut quantified = false;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            if atoms == 0 || bytes.get(at + 1) != Some(&b'.') {
                return false;
            }
            at += 2;
            atoms = 0;
            quantified = false;
            continue;
        }
        if bytes[at] == b'[' {
            at += 1;
            let start = at;
            while bytes.get(at).is_some_and(|b| atom(*b)) {
                // Adjacent hyphens are excluded: JS range parsing otherwise
                // conflicts with the contract's literal edge-hyphen rule.
                if bytes[at] == b'-' && bytes.get(at + 1) == Some(&b'-') {
                    return false;
                }
                let low = bytes[at];
                at += 1;
                if bytes.get(at) == Some(&b'-')
                    && bytes.get(at + 1).is_some_and(|b| atom(*b))
                {
                    let high = bytes[at + 1];
                    if high < low {
                        return false;
                    }
                    at += 2;
                }
            }
            if at == start || bytes.get(at) != Some(&b']') {
                return false;
            }
            at += 1;
        } else if atom(bytes[at]) {
            at += 1;
        } else {
            return false;
        }
        atoms += 1;
        let has_quantifier = match bytes.get(at) {
            Some(b'+' | b'*' | b'?') => {
                at += 1;
                true
            }
            Some(b'{') => {
                at += 1;
                let Some(min) = bound(bytes, &mut at) else { return false };
                if bytes.get(at) == Some(&b',') {
                    at += 1;
                    if bytes.get(at) != Some(&b'}') {
                        let Some(max) = bound(bytes, &mut at) else { return false };
                        if max < min {
                            return false;
                        }
                    }
                }
                if bytes.get(at) != Some(&b'}') {
                    return false;
                }
                at += 1;
                true
            }
            _ => false,
        };
        if has_quantifier && quantified {
            return false;
        }
        quantified |= has_quantifier;
    }
    atoms > 0
}

fn bound(bytes: &[u8], at: &mut usize) -> Option<u16> {
    let start = *at;
    let mut value = 0;
    while let Some(byte) = bytes.get(*at).filter(|b| b.is_ascii_digit()) {
        if *at - start == 3 {
            return None;
        }
        value = value * 10 + u16::from(*byte - b'0');
        *at += 1;
    }
    (*at > start).then_some(value)
}

#[cfg(test)]
#[path = "reveal_tests.rs"]
mod tests;
