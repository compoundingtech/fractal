//! JSON framing and message content for the Claude MCP watcher.
//!
//! The watcher ([`crate::claude_mcp`]) uses these helpers for newline-delimited JSON over stdio.

use std::io::Write;

use anyhow::Result;
use serde_json::Value;

/// The envelope a delivered inbox message is handed to the model in. A message with no subject is
/// its body verbatim: an empty `Subject:` line would be noise the model has to read past.
pub(crate) fn channel_content(subject: Option<&str>, body: &str) -> String {
    match subject.filter(|value| !value.is_empty()) {
        Some(subject) => format!("Subject: {subject}\n\n{body}"),
        None => body.to_owned(),
    }
}

/// One frame on the wire: compact JSON followed by the newline that terminates it.
pub(crate) fn write_json(out: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    out.write_all(b"\n")?;
    Ok(())
}

/// The channel's incarnation-scoped journal. An older channel with no timestamp has no
/// certificate: successful delivery status alone must never manufacture a historical clock.
pub fn input_acceptances(dir: &std::path::Path) -> Result<std::collections::BTreeMap<String, u64>> {
    match std::fs::read(dir.join("input-acceptances.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error.into()),
    }
}

/// Commit the first native acceptance before publishing it to the daemon.
pub fn record_input_acceptance(
    dir: &std::path::Path, message: &str, accepted_at_unix_ms: u64,
) -> Result<u64> {
    anyhow::ensure!(crate::push_mailbox::is_delivery_key(message) && accepted_at_unix_ms > 0,
        "invalid native input acceptance");
    let mut receipts = input_acceptances(dir)?;
    if let Some(first) = receipts.get(message) {
        return Ok(*first);
    }
    receipts.insert(message.to_owned(), accepted_at_unix_ms);
    std::fs::create_dir_all(dir)?;
    crate::fsatomic::replace(
        &dir.join("input-acceptances.json"), &serde_json::to_vec(&receipts)?,
        crate::fsatomic::Staging::new(".input-acceptances"),
        crate::fsatomic::Durability::FsyncFileAndDir,
    )?;
    Ok(accepted_at_unix_ms)
}

pub fn input_attempts(dir: &std::path::Path) -> Result<std::collections::BTreeMap<String, Option<u64>>> {
    match std::fs::read(dir.join("input-attempts.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error.into()),
    }
}

/// Persist before writing the frame. `None` carries an older unclocked handoff honestly.
pub fn record_input_attempt(dir: &std::path::Path, message: &str, timestamp: Option<u64>) -> Result<()> {
    let mut attempts = input_attempts(dir)?;
    if attempts.contains_key(message) { return Ok(()); }
    attempts.insert(message.to_owned(), timestamp);
    persist_input_attempts(dir, &attempts)
}

pub fn resolve_input_attempt(dir: &std::path::Path, message: &str) -> Result<()> {
    let mut attempts = input_attempts(dir)?;
    if attempts.remove(message).is_some() {
        persist_input_attempts(dir, &attempts)?;
    }
    Ok(())
}

fn persist_input_attempts(
    dir: &std::path::Path, attempts: &std::collections::BTreeMap<String, Option<u64>>,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    crate::fsatomic::replace(
        &dir.join("input-attempts.json"), &serde_json::to_vec(attempts)?,
        crate::fsatomic::Staging::new(".input-attempts"),
        crate::fsatomic::Durability::FsyncFileAndDir,
    )?;
    Ok(())
}

/// A source seal, not a heartbeat: every unresolved native handoff limits coverage to
/// strictly before its durable start, even if its acceptance frame is buffered in stdio.
pub fn input_acceptance_frontier(dir: &std::path::Path, cut: u64) -> Result<Option<u64>> {
    let attempts = input_attempts(dir)?;
    let mut through = cut;
    for timestamp in attempts.into_values() {
        let Some(timestamp) = timestamp else { return Ok(None) };
        through = through.min(timestamp.saturating_sub(1));
    }
    Ok(Some(through))
}

/// All incarnation journals belong to this recipient. Restart does not establish native absence.
pub fn recipient_input_acceptance_frontier(dir: &std::path::Path, cut: u64) -> Result<Option<u64>> {
    let parent = dir.parent().ok_or_else(|| anyhow::anyhow!("acceptance journal has no recipient"))?;
    let mut through = cut;
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let Some(bound) = input_acceptance_frontier(&entry.path(), through)? else { return Ok(None) };
            through = through.min(bound);
        }
    }
    Ok(Some(through))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_frontier_cannot_pass_a_buffered_or_unclocked_native_handoff() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(input_acceptance_frontier(root.path(), 200).unwrap(), Some(200));
        record_input_attempt(root.path(), "message/buffered", Some(100)).unwrap();
        assert_eq!(input_acceptance_frontier(root.path(), 200).unwrap(), Some(99));
        // Reopening and retrying cannot move the unresolved native onset forward.
        record_input_attempt(root.path(), "message/buffered", Some(150)).unwrap();
        assert_eq!(input_acceptance_frontier(root.path(), 300).unwrap(), Some(99));
        record_input_acceptance(root.path(), "message/buffered", 110).unwrap();
        assert_eq!(input_acceptance_frontier(root.path(), 300).unwrap(), Some(99),
            "a local acceptance is not a daemon receipt ACK");
        resolve_input_attempt(root.path(), "message/buffered").unwrap();
        assert_eq!(input_acceptance_frontier(root.path(), 300).unwrap(), Some(300));
        record_input_attempt(root.path(), "message/historical", None).unwrap();
        assert_eq!(input_acceptance_frontier(root.path(), 400).unwrap(), None);
        resolve_input_attempt(root.path(), "message/historical").unwrap();
        assert_eq!(input_acceptance_frontier(root.path(), 400).unwrap(), Some(400));
    }

    #[test]
    fn replacement_incarnation_cannot_discard_a_previous_unacknowledged_handoff() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old-incarnation");
        let current = root.path().join("current-incarnation");
        std::fs::create_dir_all(&current).unwrap();
        record_input_attempt(&old, "message/old", Some(100)).unwrap();
        assert_eq!(recipient_input_acceptance_frontier(&current, 300).unwrap(), Some(99));
        record_input_acceptance(&old, "message/old", 110).unwrap();
        assert_eq!(recipient_input_acceptance_frontier(&current, 300).unwrap(), Some(99),
            "stranded acceptance still needs its receipt ACK");
        resolve_input_attempt(&old, "message/old").unwrap();
        assert_eq!(recipient_input_acceptance_frontier(&current, 300).unwrap(), Some(300));
    }

    #[test]
    fn acceptance_journal_retains_first_clock_across_replay_and_reopen() {
        let root = tempfile::tempdir().unwrap();
        let message = "message/fixture";
        assert!(input_acceptances(root.path()).unwrap().is_empty());
        assert_eq!(record_input_acceptance(root.path(), message, 123).unwrap(), 123);
        assert_eq!(record_input_acceptance(root.path(), message, 999).unwrap(), 123);
        assert_eq!(input_acceptances(root.path()).unwrap().get(message), Some(&123));
        assert!(record_input_acceptance(root.path(), message, 0).is_err());
    }


    #[test]
    fn channel_content_reuses_one_subject_and_body_envelope() {
        assert_eq!(
            channel_content(Some("subject"), "body"),
            "Subject: subject\n\nbody"
        );
        assert_eq!(channel_content(None, "body"), "body");
        assert_eq!(channel_content(Some(""), "body"), "body");
    }

    /// The wire is newline-delimited: a reader splitting on newlines must see exactly one frame
    /// per value, and a body carrying its own newline must not be able to forge a second one.
    #[test]
    fn each_frame_is_one_newline_terminated_line() {
        let mut out = Vec::new();
        write_json(&mut out, &serde_json::json!({"a": 1})).unwrap();
        write_json(
            &mut out,
            &serde_json::json!({"content": "line one\nline two"}),
        )
        .unwrap();
        let framed = String::from_utf8(out).unwrap();
        assert_eq!(framed, "{\"a\":1}\n{\"content\":\"line one\\nline two\"}\n");
        assert_eq!(framed.lines().count(), 2);
    }
}
