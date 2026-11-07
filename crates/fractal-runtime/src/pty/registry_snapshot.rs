//! Incremental inventory only: control observations remain uncached.
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use pty_core::registry::{self, DaemonIdentityCheck, SessionInfo, SessionMetadata, SessionStatus};
use serde::Deserialize;

#[derive(Default)]
pub(super) struct RegistrySnapshot {
    metadata: BTreeMap<String, (Fingerprint, Option<SessionMetadata>)>,
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
        self.scan(root, || {})
    }

    fn scan(&mut self, root: &Path, after_list: impl FnOnce()) -> Result<Vec<SessionInfo>> {
        use DaemonIdentityCheck::{Match, Unknown};
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
        after_list();
        self.metadata.retain(|name, _| names.contains(name));
        let mut sessions = Vec::with_capacity(names.len());
        for name in names {
            let path = root.join(format!("{name}.json"));
            let metadata = match std::fs::metadata(&path) {
                Ok(stat) => {
                    let fingerprint = Fingerprint::from(&stat);
                    if !self
                        .metadata
                        .get(&name)
                        .is_some_and(|(old, _)| *old == fingerprint)
                    {
                        #[cfg(test)]
                        {
                            self.reads += 1;
                        }
                        let row = std::fs::File::open(&path).ok().and_then(|mut file| {
                            use std::io::Read as _;
                            let opened = Fingerprint::from(&file.metadata().ok()?);
                            let mut bytes = Vec::new();
                            file.read_to_end(&mut bytes).ok()?;
                            let parsed = serde_json::from_slice::<Projection>(&bytes)
                                .ok()
                                .map(Into::into);
                            (opened == Fingerprint::from(&file.metadata().ok()?))
                                .then_some((opened, parsed))
                        });
                        if let Some(row) = row {
                            self.metadata.insert(name.clone(), row);
                        } else {
                            self.metadata.remove(&name);
                        }
                    }
                    self.metadata.get(&name).and_then(|(_, row)| row.clone())
                }
                Err(_) => {
                    self.metadata.remove(&name);
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
            // Sidecars can appear after enumeration or conflict with recorded ownership.
            let recorded = metadata.as_ref().and_then(|row| row.daemon_pid);
            let sidecar = registry::with_root(root, || registry::read_session_pid(&name));
            let named_pid = sidecar.or(recorded);
            let identity =
                registry::check_record_identity_in(root, &name, sidecar, metadata.as_ref());
            let retained = matches!(identity, Match | Unknown);
            let pid = retained.then_some(named_pid).flatten();
            let status = if exited {
                SessionStatus::Exited
            } else if retained {
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
    fn write(root: &Path, name: &str, json: &str) {
        std::fs::write(root.join(name), json).unwrap();
    }
    fn exit_code(rows: &[SessionInfo], name: &str) -> Option<i32> {
        let row = rows.iter().find(|row| row.name == name)?;
        row.metadata.as_ref()?.exit_code
    }

    #[test]
    fn unchanged_exited_records_are_not_read_again() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..885 {
            write(root.path(), &format!("s-{index}.json"), r#"{"exitCode":0}"#);
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
        write(root.path(), "replacement", r#"{"exitCode":2}"#);
        std::fs::rename(
            root.path().join("replacement"),
            root.path().join("s-0.json"),
        )
        .unwrap();
        write(root.path(), "new.json", r#"{"exitCode":3}"#);
        let rows = snapshot.sessions(root.path()).unwrap();
        assert_eq!(snapshot.reads, 2);
        assert_eq!(rows.len(), 886);
        assert_eq!(exit_code(&rows, "s-0"), Some(2));
        assert!(rows.iter().any(|row| row.name == "new"));
        println!("metadata reads: baseline=885 cold=885 unchanged=0 changed-and-new=2");
        write(root.path(), "new.json", r#"{"exitCode":123}"#);
        assert_eq!(
            exit_code(&snapshot.sessions(root.path()).unwrap(), "new"),
            Some(123)
        );
        assert_eq!(snapshot.reads, 3);
        std::fs::remove_file(root.path().join("new.json")).unwrap();
        assert_eq!(snapshot.sessions(root.path()).unwrap().len(), 885);
        assert_eq!(snapshot.metadata.len(), 885);
    }

    #[test]
    fn an_exited_live_daemon_still_fences_scope_teardown() {
        use sha2::Digest as _;
        let root = tempfile::tempdir().unwrap();
        let runtime = super::super::PtyRuntime::new(root.path().to_owned());
        let pid = i32::try_from(std::process::id()).unwrap();
        write(root.path(), "live.pid", &pid.to_string());
        let owner = runtime
            .spawn_state_path("live", "producers")
            .join(format!("{:x}", super::super::Sha256::digest(b"op")));
        std::fs::create_dir_all(&owner).unwrap();
        std::fs::write(owner.join("owner.json"), b"invalid ownership").unwrap();
        for (legacy, recorded_pid) in [(false, pid), (true, pid), (false, pid + 1)] {
            let mut record = serde_json::json!({
                "exitCode": 0, "daemonPid": recorded_pid,
                "daemonStartToken": registry::read_process_start_token(pid).unwrap(),
                "daemonBootId": registry::read_boot_id().unwrap(),
                "tags": {"st3.start-operation": "op", "st3.scope-unit": "work.scope"}
            });
            if legacy {
                record["daemonPid"] = serde_json::Value::Null;
                record["daemonStartToken"] = serde_json::Value::Null;
            }
            write(root.path(), "live.json", &record.to_string());
            for _ in 0..2 {
                std::fs::remove_file(root.path().join("live.pid")).unwrap();
                let mut cache = runtime.registry_snapshot.lock().unwrap();
                let publish = || write(root.path(), "live.pid", &pid.to_string());
                let mut rows = cache.scan(root.path(), publish).unwrap();
                let row = super::super::observation(rows.remove(0)).unwrap();
                assert_eq!(row.status, "exited");
                assert_eq!(row.pid, Some(std::process::id()));
                let apart = runtime.server_apart("live", "work.scope", Some(&row));
                assert!(apart.is_err());
            }
            runtime.snapshot().unwrap();
            record["createdAt"] = "fresh".into();
            write(root.path(), "live.json", &record.to_string());
            let fresh = runtime.session("live").unwrap().metadata.unwrap();
            assert_eq!(fresh.created_at, "fresh");
        }
    }
}
