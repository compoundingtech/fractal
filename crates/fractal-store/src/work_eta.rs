//! Advisory, self-reported estimates embedded in ordinary progress notes.
use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};

const PREFIX: &str = "ETA (self-reported by ";
const SUFFIX: &str = ". Not a deadline.";

/// Append an attributed estimate and its report time without introducing graph fields.
pub fn summary(summary: Option<&str>, eta: &str, author: &str, now: u128) -> Result<String> {
    anyhow::ensure!(
        !author.trim().is_empty() && !author.contains(['\r', '\n']),
        "ETA author must be a nonempty single line"
    );
    let at = if let Ok(at) = DateTime::parse_from_rfc3339(eta) {
        u128::try_from(at.timestamp_millis()).context("ETA must be after the Unix epoch")?
    } else {
        let duration = crate::graph::parse_duration(eta, true)
            .context("use a positive duration such as 2h or a future RFC3339 timestamp")?;
        now.checked_add(u128::from(duration)).context("ETA is too large")?
    };
    anyhow::ensure!(at > now, "ETA must be in the future");
    let timestamp = timestamp_at(at)?;
    let reported = timestamp_at(now)?;
    let annotation = format!(
        "{PREFIX}{author}): {timestamp}; reported at {reported}{SUFFIX}"
    );
    Ok(match summary.filter(|text| !text.trim().is_empty()) {
        Some(text) => format!("{text}\n{annotation}"),
        None => annotation,
    })
}

fn timestamp_at(at: u128) -> Result<String> {
    let at = i64::try_from(at).context("ETA timestamp is out of range")?;
    let at = DateTime::<Utc>::from_timestamp_millis(at).context("ETA timestamp is out of range")?;
    Ok(at.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn is_annotation(line: &str) -> bool {
    let Some(body) = line.strip_prefix(PREFIX).and_then(|line| line.strip_suffix(SUFFIX)) else {
        return false;
    };
    let Some((author, times)) = body.rsplit_once("): ") else {
        return false;
    };
    if author.trim().is_empty() || author.contains(['\r', '\n']) {
        return false;
    }
    let Some((estimate, reported)) = times.split_once("; reported at ") else {
        return false;
    };
    let canonical = |value: &str| {
        let bytes = value.as_bytes();
        let len = bytes.len();
        if len < 5 || !value.contains('T') || bytes[len - 5] != b'.'
            || bytes[len - 1] != b'Z'
            || !bytes[len - 4..len - 1].iter().all(u8::is_ascii_digit)
        {
            return None;
        }
        DateTime::parse_from_rfc3339(value).ok().filter(|at| {
            at.timestamp_millis() >= 0 && at.timestamp_subsec_nanos() < 1_000_000_000
        })
    };
    matches!((canonical(estimate), canonical(reported)), (Some(at), Some(report)) if at > report)
}

/// Withdraw only annotations emitted by `summary`, leaving ordinary progress intact.
/// A nonempty replacement is necessary because empty progress does not replace a note.
pub fn without_eta(summary: &str) -> Option<String> {
    without_eta_at(summary, "ETA withdrawn at handoff.")
}

/// Withdraw estimates inherited from a predecessor step without discarding material progress.
pub fn without_eta_on_revision(summary: &str) -> Option<String> {
    without_eta_at(summary, "ETA withdrawn at revision.")
}

fn without_eta_at(summary: &str, withdrawal: &str) -> Option<String> {
    let first_annotation = summary.split('\n').position(is_annotation)?;
    let mut material = String::with_capacity(summary.len());
    let mut lines = summary.split('\n').enumerate()
        .filter(|(index, line)| *index != first_annotation
            && (*index < first_annotation || !is_annotation(line)))
        .map(|(_, line)| line)
        .peekable();
    while let Some(line) = lines.next() {
        material.push_str(line);
        if lines.peek().is_some() {
            material.push('\n');
        }
    }
    if material.trim().is_empty() {
        material.clear();
        material.push_str(withdrawal);
    }
    Some(material)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u128 = 1_700_000_000_123;
    const AUTHOR: &str = "agent/example/alder";

    #[test]
    fn duration_and_offset_timestamp_normalize_with_author_and_report_time() {
        let expected = "Material checkpoint\nETA (self-reported by agent/example/alder): 2023-11-15T00:13:20.123Z; reported at 2023-11-14T22:13:20.123Z. Not a deadline.";
        assert_eq!(summary(Some("Material checkpoint"), "2h", AUTHOR, NOW).unwrap(), expected);
        assert_eq!(summary(Some("Material checkpoint"), "2023-11-15T02:13:20.123+02:00", AUTHOR, NOW).unwrap(), expected);
        assert!(summary(None, "1ms", "person/nathan", NOW).unwrap().starts_with("ETA (self-reported by person/nathan):"));
    }

    #[test]
    fn rejects_invalid_past_zero_and_overflow_estimates() {
        for eta in ["0s", "-1h", "soon", "2023-11-14T22:13:20.123Z", "1969-12-31T23:59:59Z", "18446744073709551615d"] {
            assert!(summary(None, eta, AUTHOR, NOW).is_err(), "{eta}");
        }
        assert!(summary(None, "1ms", AUTHOR, u128::MAX).is_err());
        assert!(summary(None, "1ms", AUTHOR, i64::MAX as u128).is_err());
        assert!(summary(None, "1ms", "", NOW).is_err());
        assert!(summary(None, "1ms", "agent/example\nforged", NOW).is_err());
    }

    #[test]
    fn strips_only_valid_annotations_and_preserves_material_text() {
        let note = summary(Some("  Checkpoint\n\nETA is discussed here."), "2h", AUTHOR, NOW).unwrap();
        assert_eq!(without_eta(&note).as_deref(), Some("  Checkpoint\n\nETA is discussed here."));
        let annotation = summary(None, "2h", AUTHOR, NOW).unwrap();
        assert_eq!(without_eta(&annotation).as_deref(), Some("ETA withdrawn at handoff."));
        let mixed = format!("First checkpoint\n{annotation}\n\nSecond checkpoint\n");
        assert_eq!(without_eta(&mixed).as_deref(), Some("First checkpoint\n\nSecond checkpoint\n"));
        for invalid in [
            "ETA (self-reported): 2023-11-15T00:13:20.123Z; reported at 2023-11-14T22:13:20.123Z. Not a deadline.",
            "ETA (self-reported by ): 2023-11-15T00:13:20.123Z; reported at 2023-11-14T22:13:20.123Z. Not a deadline.",
            "ETA (self-reported by agent/example/alder): tomorrow; reported at today. Not a deadline.",
        ] {
            assert_eq!(without_eta(invalid), None);
        }
        assert_eq!(without_eta(&annotation.replace(".123Z", "Z")), None);
        assert_eq!(without_eta(&annotation.replace("2023-11-15T00:13:20.123Z", "2023-11-14T22:13:20.123Z")), None);
        assert_eq!(without_eta(&format!("Quoted {annotation}")), None);
    }
}
