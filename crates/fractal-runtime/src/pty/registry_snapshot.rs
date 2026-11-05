//! Incremental observation only: control paths must use a fresh keyed lookup.
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use pty_core::registry::{self, DaemonIdentityCheck, SessionInfo, SessionMetadata, SessionStatus};
use serde::Deserialize;

#[derive(Default)]
pub(super) struct RegistrySnapshot {
    metadata: BTreeMap<PathBuf, (Fingerprint, Option<SessionMetadata>)>,
    #[cfg(test)]
    reads: usize,
}

#[derive(PartialEq, Eq)]
struct Fingerprint(u64, u64, i64, i64, u64, i64, i64);
impl From<&std::fs::Metadata> for Fingerprint {
    fn from(row: &std::fs::Metadata) -> Self {
        Self(
            row.dev(),
            row.ino(),
            row.mtime(),
            row.mtime_nsec(),
            row.len(),
            row.ctime(),
            row.ctime_nsec(),
        )
    }
}

// Ignore retained output, environments and unknown payloads while deserializing.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Projection {
    daemon_pid: Option<i32>,
    daemon_start_token: Option<String>,
    daemon_boot_id: Option<String>,
    recovery: Option<serde_json::Value>,
    created_at: String,
    display_name: Option<String>,
    tags: Option<registry::TagMap>,
    exit_code: Option<i32>,
    exited_at: Option<String>,
}
impl From<Projection> for SessionMetadata {
    fn from(row: Projection) -> Self {
        Self {
            daemon_pid: row.daemon_pid,
            daemon_start_token: row.daemon_start_token,
            daemon_boot_id: row.daemon_boot_id,
            recovery: row.recovery,
            created_at: row.created_at,
            display_name: row.display_name,
            tags: row.tags,
            exit_code: row.exit_code,
            exited_at: row.exited_at,
            ..Self::default()
        }
    }
}

impl RegistrySnapshot {
    pub(super) fn sessions(&mut self, root: &Path) -> Result<Vec<SessionInfo>> {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.metadata.clear();
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(error).with_context(|| format!("list PTYs in {}", root.display()));
            }
        };
        let mut names = BTreeSet::new();
        let mut sockets = BTreeSet::new();
        for entry in entries {
            let entry = entry.context("read PTY registry entry")?;
            let Ok(filename) = entry.file_name().into_string() else {
                continue;
            };
            if registry::is_tmp_name(&filename) {
                continue;
            }
            if let Some(name) = filename
                .strip_suffix(".json")
                .or_else(|| filename.strip_suffix(".pid"))
                .filter(|name| !name.starts_with('@'))
            {
                names.insert(name.to_owned());
            } else if let Some(name) = filename.strip_suffix(".sock") {
                sockets.insert(filename.clone());
                if !name.starts_with('@') && registry::socket_path_in(root, name) == entry.path() {
                    names.insert(name.to_owned());
                }
            }
        }
        self.metadata.retain(|path, _| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| names.contains(name))
        });
        let mut sessions = Vec::with_capacity(names.len());
        for name in names {
            let path = root.join(format!("{name}.json"));
            let metadata = match std::fs::metadata(&path) {
                Ok(stat) => {
                    let fingerprint = Fingerprint::from(&stat);
                    if !self
                        .metadata
                        .get(&path)
                        .is_some_and(|(old, _)| *old == fingerprint)
                    {
                        #[cfg(test)]
                        {
                            self.reads += 1;
                        }
                        let row = std::fs::read(&path)
                            .ok()
                            .and_then(|bytes| serde_json::from_slice::<Projection>(&bytes).ok())
                            .map(Into::into);
                        self.metadata.insert(path.clone(), (fingerprint, row));
                    }
                    self.metadata.get(&path).and_then(|(_, row)| row.clone())
                }
                Err(_) => {
                    self.metadata.remove(&path);
                    None
                }
            };
            let socket_path = registry::socket_path_in(root, &name);
            if metadata.is_none()
                && !socket_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| sockets.contains(name))
            {
                continue;
            }
            let exited = metadata
                .as_ref()
                .is_some_and(|row| row.has_exited() || row.exit_code.is_some());
            // Exited records need no sidecar reads or process probes. Their immutable
            // observation is retained for exit-code consumers, not control authority.
            let (identity, pid) = if exited {
                (DaemonIdentityCheck::Gone, None)
            } else {
                let sidecar = registry::with_root(root, || registry::read_session_pid(&name));
                let identity =
                    registry::check_record_identity_in(root, &name, sidecar, metadata.as_ref());
                let retained = matches!(
                    identity,
                    DaemonIdentityCheck::Match | DaemonIdentityCheck::Unknown
                );
                (
                    identity,
                    retained
                        .then(|| {
                            sidecar.or_else(|| metadata.as_ref().and_then(|row| row.daemon_pid))
                        })
                        .flatten(),
                )
            };
            let status = if exited {
                SessionStatus::Exited
            } else if matches!(
                identity,
                DaemonIdentityCheck::Match | DaemonIdentityCheck::Unknown
            ) {
                SessionStatus::Running
            } else {
                SessionStatus::Vanished
            };
            sessions.push(SessionInfo {
                name,
                socket_path,
                metadata,
                identity,
                pid,
                status,
            });
        }
        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exit_code(rows: &[SessionInfo], name: &str) -> Option<i32> {
        rows.iter()
            .find(|row| row.name == name)
            .unwrap()
            .metadata
            .as_ref()
            .unwrap()
            .exit_code
    }

    #[test]
    fn unchanged_exited_records_are_not_read_again() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..885 {
            std::fs::write(
                root.path().join(format!("session-{index}.json")),
                r#"{"exitCode":0,"createdAt":"one"}"#,
            )
            .unwrap();
        }
        assert_eq!(
            registry::list_sessions_in(root.path(), &registry::ListOptions::default()).len(),
            885
        );
        let mut snapshot = RegistrySnapshot::default();
        assert_eq!(snapshot.sessions(root.path()).unwrap().len(), 885);
        assert_eq!(snapshot.reads, 885);
        snapshot.reads = 0;
        assert_eq!(snapshot.sessions(root.path()).unwrap().len(), 885);
        assert_eq!(snapshot.reads, 0);
        std::fs::write(
            root.path().join("replacement"),
            r#"{"exitCode":2,"createdAt":"two"}"#,
        )
        .unwrap();
        std::fs::rename(
            root.path().join("replacement"),
            root.path().join("session-0.json"),
        )
        .unwrap();
        std::fs::write(root.path().join("new.json"), r#"{"exitCode":3}"#).unwrap();
        let rows = snapshot.sessions(root.path()).unwrap();
        assert_eq!(snapshot.reads, 2);
        assert_eq!(rows.len(), 886);
        assert_eq!(exit_code(&rows, "session-0"), Some(2));
        assert!(rows.iter().any(|row| row.name == "new"));
        println!("metadata reads: baseline=885 cold=885 unchanged=0 changed-and-new=2");
        std::fs::write(root.path().join("new.json"), r#"{"exitCode":123}"#).unwrap();
        assert_eq!(
            exit_code(&snapshot.sessions(root.path()).unwrap(), "new"),
            Some(123)
        );
        assert_eq!(snapshot.reads, 3);
        std::fs::remove_file(root.path().join("new.json")).unwrap();
        assert_eq!(snapshot.sessions(root.path()).unwrap().len(), 885);
        assert_eq!(snapshot.metadata.len(), 885);
        let runtime = super::super::PtyRuntime::new(root.path().to_owned());
        runtime.snapshot().unwrap();
        std::fs::write(
            root.path().join("session-0.json"),
            r#"{"createdAt":"fresh","exitCode":4}"#,
        )
        .unwrap();
        assert_eq!(
            runtime
                .session("session-0")
                .unwrap()
                .metadata
                .unwrap()
                .created_at,
            "fresh"
        );
    }
}
