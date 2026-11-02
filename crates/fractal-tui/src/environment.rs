use std::ffi::OsString;

use anyhow::Result;

/// Prefer the Fractal spelling without silently accepting ambiguous configuration.
pub(crate) fn read(new: &str, old: &str) -> Result<Option<OsString>> {
    resolve(new, old, &|name| std::env::var_os(name))
}

fn resolve(
    new: &str,
    old: &str,
    get: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Option<OsString>> {
    let preferred = get(new).filter(|value| !value.is_empty());
    let legacy = get(old).filter(|value| !value.is_empty());
    if let (Some(preferred), Some(legacy)) = (&preferred, &legacy) {
        anyhow::ensure!(
            preferred == legacy,
            "{new} and {old} have different values; unset one or set both to the same value"
        );
    }
    Ok(preferred.or(legacy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_aliases(new: &str, old: &str) {
        for (preferred, legacy, expected) in [
            (Some("same"), None, Some("same")),
            (None, Some("same"), Some("same")),
            (Some("same"), Some("same"), Some("same")),
            (Some(""), Some("same"), Some("same")),
            (Some("same"), Some(""), Some("same")),
            (Some(""), Some(""), None),
            (None, None, None),
        ] {
            let get = |name: &str| {
                if name == new { preferred } else if name == old { legacy } else { None }
                    .map(OsString::from)
            };
            assert_eq!(resolve(new, old, &get).unwrap(), expected.map(OsString::from));
        }
        let error = resolve(new, old, &|name| {
            Some(OsString::from(if name == new { "preferred-secret" } else { "legacy-secret" }))
        }).unwrap_err().to_string();
        assert!(error.contains(new) && error.contains(old));
        assert!(error.contains("unset one or set both to the same value"));
        assert!(!error.contains("preferred-secret") && !error.contains("legacy-secret"));
    }

    #[test]
    fn person_aliases() {
        assert_aliases("FRACTAL_PERSON", "ST3_PERSON");
    }

    #[test]
    fn endpoint_aliases() {
        assert_aliases("FRACTAL_ENDPOINT", "ST3_ENDPOINT");
    }

    #[test]
    fn downloads_aliases() {
        assert_aliases("FRACTAL_DOWNLOADS", "ST3_DOWNLOADS");
    }
}
