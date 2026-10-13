//! One offline contract for observed GitHub PR evidence URLs. A normalized URL is
//! a citation key, never proof of creation, ownership, work identity or current head.
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrEvidenceUrl {
    canonical: String,
    repository: String,
    number: u64,
}

impl PrEvidenceUrl {
    pub fn canonical(&self) -> &str {
        &self.canonical
    }
    pub fn repository(&self) -> &str {
        &self.repository
    }
    pub fn number(&self) -> u64 {
        self.number
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrUrlError {
    UnsupportedAuthority,
    InvalidPath,
    InvalidNumber,
    UnsupportedQuery,
}

impl fmt::Display for PrUrlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never copy supplied URLs, usernames, credentials or query values into errors.
        formatter.write_str(match self {
            Self::UnsupportedAuthority => {
                "expected an HTTP or HTTPS github.com PR URL without userinfo or a nondefault port"
            }
            Self::InvalidPath => {
                "unsupported GitHub PR URL path or embedded whitespace/control character"
            }
            Self::InvalidNumber => "expected a positive decimal PR number within u64",
            Self::UnsupportedQuery => {
                "unsupported PR URL query; only known display parameters are accepted"
            }
        })
    }
}
impl std::error::Error for PrUrlError {}

/// Preserve original evidence in its existing immutable claim. Callers use this
/// key for lookup/association only; unsupported attachments retain their existing
/// generic evidence behavior. No network request or repository inference occurs.
pub fn normalize_pr_url(input: &str) -> Result<PrEvidenceUrl, PrUrlError> {
    if input.chars().any(char::is_control) {
        return Err(PrUrlError::InvalidPath);
    }
    let input = input.trim();
    if input.is_empty()
        || input.len() > 4096
        || input.chars().any(|c| c.is_control() || c.is_whitespace())
        || input.contains('\\')
    {
        return Err(PrUrlError::InvalidPath);
    }
    let (scheme, rest) = input
        .split_once("://")
        .ok_or(PrUrlError::UnsupportedAuthority)?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(PrUrlError::UnsupportedAuthority);
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(host, port)| (host, Some(port)));
    if !host.eq_ignore_ascii_case("github.com") || authority.contains('@') {
        return Err(PrUrlError::UnsupportedAuthority);
    }
    if let Some(port) = port {
        let expected = if scheme.eq_ignore_ascii_case("http") {
            80
        } else {
            443
        };
        if port.is_empty()
            || !port.bytes().all(|byte| byte.is_ascii_digit())
            || port.parse::<u16>().ok() != Some(expected)
        {
            return Err(PrUrlError::UnsupportedAuthority);
        }
    }
    let suffix = &rest[authority_end..];
    let path_end = suffix.find(['?', '#']).unwrap_or(suffix.len());
    let raw_path = &suffix[..path_end];
    // URL parsers discard dot segments and decode some path spellings. Reject
    // those inputs before parsing, so normalization cannot change their identity.
    if raw_path.contains('%') || raw_path.split('/').any(|part| matches!(part, "." | "..")) {
        return Err(PrUrlError::InvalidPath);
    }
    let url = reqwest::Url::parse(input).map_err(|_| PrUrlError::UnsupportedAuthority)?;
    if url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(PrUrlError::UnsupportedAuthority);
    }
    let path = raw_path.strip_prefix('/').ok_or(PrUrlError::InvalidPath)?;
    let path = path.strip_suffix('/').unwrap_or(path);
    let parts: Vec<_> = path.split('/').collect();
    if !(4..=5).contains(&parts.len())
        || !matches!(parts[2], "pull" | "pulls")
        || (parts.len() == 5 && !matches!(parts[4], "files" | "commits" | "checks"))
    {
        return Err(PrUrlError::InvalidPath);
    }
    for name in &parts[..2] {
        if name.is_empty()
            || name.len() > 100
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        {
            return Err(PrUrlError::InvalidPath);
        }
    }
    if parts[3].is_empty() || !parts[3].bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PrUrlError::InvalidNumber);
    }
    let number: u64 = parts[3].parse().map_err(|_| PrUrlError::InvalidNumber)?;
    if number == 0 {
        return Err(PrUrlError::InvalidNumber);
    }
    let mut seen = std::collections::BTreeSet::new();
    for (key, value) in url.query_pairs() {
        if !seen.insert(key.clone())
            || !match key.as_ref() {
                "w" => matches!(value.as_ref(), "0" | "1"),
                "tab" => matches!(
                    value.as_ref(),
                    "conversation" | "files" | "commits" | "checks"
                ),
                _ => false,
            }
        {
            return Err(PrUrlError::UnsupportedQuery);
        }
    }
    let repository = format!(
        "{}/{}",
        parts[0].to_ascii_lowercase(),
        parts[1].to_ascii_lowercase()
    );
    Ok(PrEvidenceUrl {
        canonical: format!("https://github.com/{repository}/pull/{number}"),
        repository,
        number,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Case {
        input: String,
        expected: Option<String>,
        why: String,
    }
    #[derive(serde::Deserialize)]
    struct Table {
        cases: Vec<Case>,
    }

    #[test]
    fn approved_table_normalizes_only_explicit_pr_url_aliases() {
        let table: Table =
            serde_json::from_str(include_str!("../tests/fixtures/pr-work-evidence-urls.json"))
                .unwrap();
        assert_eq!(table.cases.len(), 42);
        for case in table.cases {
            let normalized = normalize_pr_url(&case.input);
            assert_eq!(
                normalized
                    .as_ref()
                    .ok()
                    .map(|url| url.canonical().to_owned()),
                case.expected,
                "{}",
                case.why
            );
            if let Ok(normalized) = normalized {
                assert_eq!(
                    normalize_pr_url(normalized.canonical()).unwrap(),
                    normalized
                );
            }
        }
    }

    #[test]
    fn malformed_authorities_sensitive_queries_and_ambiguity_are_not_inferred() {
        for value in [
            "https://@github.com/fixture/garden/pull/12",
            "https://github.com\\fixture\\garden\\pull\\12",
            "https:github.com/fixture/garden/pull/12",
            "https://github.com/fixture/garden/pull/12?w=fixture-token",
            "https://github.com/fixture/garden/pull/12?w=1&w=0",
            "https://github.com/fixture/garden/pull/12/tab/other",
            "https://github.com/fixture/garden/pull/12//",
            "https://github.com./fixture/garden/pull/12",
            "https://%67ithub.com/fixture/garden/pull/12",
            "https://ＧＩＴＨＵＢ．ＣＯＭ/fixture/garden/pull/12",
            "https://github.com:/fixture/garden/pull/12",
            "\nhttps://github.com/fixture/garden/pull/12",
        ] {
            let error = normalize_pr_url(value).unwrap_err();
            assert!(!error.to_string().contains("fixture-token"));
        }
        let maximum =
            normalize_pr_url("https://github.com/fixture/garden/pull/18446744073709551615")
                .unwrap();
        assert_eq!(maximum.number(), u64::MAX);
        assert_eq!(maximum.repository(), "fixture/garden");
        assert_ne!(
            maximum,
            normalize_pr_url("https://github.com/other/garden/pull/18446744073709551615").unwrap()
        );
    }
}
