//! Opt-in Linux seat admission from an owner-written envelope state file; never reads cgroups.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub state_file: PathBuf,
    pub batch_paused_min_secs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Decision {
    pub mode: String,
    pub reason: String,
    pub queue: bool,
    pub observed_at: Option<u64>,
}

/// The OS hostname used by the envelope writer, never a daemon node alias or environment value.
#[cfg(unix)]
pub fn local_host() -> anyhow::Result<String> {
    let mut bytes = [0_u8; 256];
    // SAFETY: bytes points to a writable buffer of the length passed to gethostname.
    if unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let end = bytes.iter().position(|byte| *byte == 0)
        .ok_or_else(|| anyhow::anyhow!("OS hostname exceeds buffer"))?;
    anyhow::ensure!(end != 0, "OS hostname is empty");
    Ok(std::str::from_utf8(&bytes[..end])?.to_owned())
}

#[cfg(not(unix))]
pub fn local_host() -> anyhow::Result<String> {
    anyhow::bail!("seat admission hostname lookup requires Unix")
}

/// Read only the configured file. Invalid or unavailable observations always admit.
#[cfg(target_os = "linux")]
pub fn read(config: &Config, host: &str, now: u64) -> Decision {
    match linux::read_state(config).and_then(|state| linux::evaluate(state, config, host, now)) {
        Ok(decision) => decision,
        Err(error) => Decision {
            mode: "fail-open".into(), reason: format!("{error:#}"), queue: false, observed_at: None,
        },
    }
}

/// Admission policy is Linux-only; other platforms never open the configured file.
#[cfg(not(target_os = "linux"))]
pub fn read(_config: &Config, _host: &str, _now: u64) -> Decision {
    Decision {
        mode: "off".into(), reason: "seat admission is Linux-only".into(),
        queue: false, observed_at: None,
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use anyhow::{Context as _, Result, ensure};
    use std::fs::{File, Metadata, OpenOptions};
    use std::io::Read as _;
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};

    const MAX_BYTES: u64 = 64 * 1024;

    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Mode { Observe, Enforce }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct State {
        schema: String,
        host: String,
        observed_at: u64,
        // These fields are required and type-checked, but are not admission predicates.
        #[serde(rename = "envelopeCgroup")]
        _envelope_cgroup: String,
        #[serde(rename = "envelopeBytes")]
        _envelope_bytes: u64,
        #[serde(rename = "usageBytes")]
        _usage_bytes: u64,
        exceeded: bool,
        #[serde(rename = "exceededSince", deserialize_with = "nullable_timestamp")]
        _exceeded_since: Option<u64>,
        batch_paused: bool,
        #[serde(deserialize_with = "nullable_timestamp")]
        batch_paused_since: Option<u64>,
        mode: Mode,
    }

    // deserialize_with makes even nullable timestamps required rather than implicitly defaulted.
    fn nullable_timestamp<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<u64>, D::Error> {
        Option::<u64>::deserialize(input)
    }

    fn validate_metadata(metadata: &Metadata, effective_uid: u32) -> Result<()> {
        ensure!(metadata.is_file(), "admission state must be a regular file");
        ensure!(metadata.uid() == effective_uid, "admission state owner must match effective uid");
        ensure!(metadata.len() <= MAX_BYTES, "admission state exceeds 64 KiB");
        Ok(())
    }

    pub(super) fn read_state(config: &Config) -> Result<State> {
        let file: File = OpenOptions::new().read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&config.state_file).context("open admission state")?;
        // SAFETY: geteuid takes no arguments and has no preconditions.
        validate_metadata(&file.metadata().context("inspect admission state")?, unsafe { libc::geteuid() })?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).context("read admission state")?;
        ensure!(bytes.len() as u64 <= MAX_BYTES, "admission state exceeds 64 KiB");
        serde_json::from_slice(&bytes).context("parse admission state")
    }

    pub(super) fn evaluate(state: State, config: &Config, host: &str, now: u64) -> Result<Decision> {
        ensure!(state.schema == "interactive-envelope.state/v1", "unsupported admission state schema");
        ensure!(state.host == host, "admission state host mismatch");
        ensure!(now.saturating_sub(state.observed_at) <= 120, "admission state is stale");
        ensure!(state.observed_at.saturating_sub(now) <= 60, "admission state is too far in the future");
        let paused_long_enough = state.batch_paused_since
            .and_then(|since| now.checked_sub(since))
            .is_some_and(|elapsed| elapsed >= config.batch_paused_min_secs);
        let enforce = matches!(state.mode, Mode::Enforce);
        let queue = enforce && state.exceeded && state.batch_paused && paused_long_enough;
        let reason = if !enforce { "observe mode admits" }
            else if !state.exceeded { "envelope is not exceeded" }
            else if !state.batch_paused { "batch is not paused" }
            else if !paused_long_enough { "batch pause is missing, future, or below minimum age" }
            else { "envelope exceeded and batch pause reached minimum age" };
        Ok(Decision {
            mode: if enforce { "enforce" } else { "observe" }.into(),
            reason: reason.into(), queue, observed_at: Some(state.observed_at),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::{Value, json};

        fn config(path: PathBuf) -> Config { Config { state_file: path, batch_paused_min_secs: 30 } }
        fn state() -> Value {
            json!({"schema":"interactive-envelope.state/v1", "host":"node-a", "observedAt":1000,
                "envelopeCgroup":"/interactive", "envelopeBytes":100, "usageBytes":101,
                "exceeded":true, "exceededSince":900, "batchPaused":true,
                "batchPausedSince":970, "mode":"enforce"})
        }
        fn decision(value: Value, now: u64) -> Result<Decision> {
            evaluate(serde_json::from_value(value)?, &config(PathBuf::new()), "node-a", now)
        }

        #[test]
        fn writer_observe_fixture_is_byte_compatible() {
            // Writer field order, compact separators, nullable timestamps, and no final newline.
            let bytes = br#"{"schema":"interactive-envelope.state/v1","host":"node-a","observedAt":1000,"envelopeCgroup":"/interactive","envelopeBytes":100,"usageBytes":100,"exceeded":false,"exceededSince":null,"batchPaused":false,"batchPausedSince":null,"mode":"observe"}"#;
            let parsed = serde_json::from_slice(bytes).unwrap();
            let result = evaluate(parsed, &config(PathBuf::new()), "node-a", 1000).unwrap();
            assert_eq!(result.mode, "observe");
            assert!(!result.queue);
            assert_eq!(result.observed_at, Some(1000));
        }

        #[test]
        fn strict_schema_requires_every_field_and_rejects_unknowns() {
            let valid = state();
            for field in valid.as_object().unwrap().keys() {
                let mut value = valid.clone();
                value.as_object_mut().unwrap().remove(field);
                assert!(decision(value, 1000).is_err(), "missing {field}");
            }
            for (field, value) in [("extra", json!(0)), ("schema", json!("other")),
                ("host", json!("node-b")), ("observedAt", json!(-1)), ("usageBytes", json!(1.5)),
                ("envelopeBytes", json!("100")), ("exceeded", json!(1)),
                ("batchPausedSince", json!("970")), ("mode", json!("off"))] {
                let mut bad = valid.clone();
                bad[field] = value;
                assert!(decision(bad, 1000).is_err(), "invalid {field}");
            }
            let mut nullable = valid;
            nullable["exceededSince"] = Value::Null;
            nullable["batchPausedSince"] = Value::Null;
            assert!(!decision(nullable, 1000).unwrap().queue);
        }

        #[test]
        fn admission_requires_every_predicate_and_pause_age() {
            assert!(decision(state(), 1000).unwrap().queue);
            assert!(!decision(state(), 999).unwrap().queue);
            for (field, value) in [("mode", json!("observe")), ("exceeded", json!(false)),
                ("batchPaused", json!(false)), ("batchPausedSince", json!(1001)),
                ("batchPausedSince", Value::Null)] {
                let mut value_state = state();
                value_state[field] = value;
                assert!(!decision(value_state, 1000).unwrap().queue, "{field}");
            }
            let mut future = state();
            future["batchPausedSince"] = json!(1001);
            let mut zero_min = config(PathBuf::new());
            zero_min.batch_paused_min_secs = 0;
            assert!(!evaluate(serde_json::from_value(future).unwrap(), &zero_min, "node-a", 1000).unwrap().queue);
        }

        #[test]
        fn observation_freshness_has_inclusive_boundaries_without_overflow() {
            for observed in [880, 1060] {
                let mut value = state(); value["observedAt"] = json!(observed);
                assert!(decision(value, 1000).is_ok());
            }
            for observed in [879, 1061, u64::MAX] {
                let mut value = state(); value["observedAt"] = json!(observed);
                assert!(decision(value, 1000).is_err());
            }
            let result = decision(state(), 1000).unwrap();
            assert_eq!(result.mode, "enforce");
            assert_eq!(result.observed_at, Some(1000));
        }

        #[test]
        fn file_reader_fails_open_for_unsafe_missing_and_invalid_files() {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("state");
            let cfg = config(path.clone());
            let assert_open = || {
                let result = read(&cfg, "node-a", 1000);
                assert_eq!(result.mode, "fail-open");
                assert!(!result.queue);
                assert!(!result.reason.is_empty());
            };
            assert_open();
            std::fs::create_dir(&path).unwrap(); assert_open();
            std::fs::remove_dir(&path).unwrap();
            let target = root.path().join("target");
            std::fs::write(&target, state().to_string()).unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap(); assert_open();
            std::fs::remove_file(&path).unwrap();
            let cpath = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            // SAFETY: cpath is NUL-terminated and lives throughout the call.
            assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
            assert_open(); std::fs::remove_file(&path).unwrap();
            for bytes in [vec![b' '; MAX_BYTES as usize + 1], b"{}".to_vec(),
                b"null".to_vec(), b"\xff".to_vec(), format!("{} trailing", state()).into_bytes()] {
                std::fs::write(&path, bytes).unwrap(); assert_open();
            }
            std::fs::write(&path, state().to_string()).unwrap();
            assert!(read(&cfg, "node-a", 1000).queue);
            assert_eq!(read(&cfg, "node-b", 1000).mode, "fail-open");
            assert_eq!(read(&cfg, "node-a", 1121).mode, "fail-open");
            let meta = std::fs::metadata(path).unwrap();
            assert!(validate_metadata(&meta, meta.uid().wrapping_add(1)).is_err());
        }

        #[test]
        fn config_uses_strict_camel_case_keys() {
            let cfg: Config = serde_json::from_value(json!({"stateFile":"state", "batchPausedMinSecs":30})).unwrap();
            assert_eq!(cfg, config(PathBuf::from("state")));
            assert!(serde_json::from_value::<Config>(json!({"stateFile":"state"})).is_err());
            assert!(serde_json::from_value::<Config>(json!({"stateFile":"state", "batchPausedMinSecs":30, "extra":true})).is_err());
        }
    }
}
