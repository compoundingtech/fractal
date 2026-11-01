//! The plugin store contract (docs/st3/plugins.md). A plugin owns `<state>/plugins/<name>/` and
//! one SQLite file there, `<name>.sqlite3`, which the daemon opens and migrates. The file never
//! syncs and is not in the graph's backup: it is a cache its plugin rebuilds from its source, so a
//! file this build cannot use is set aside and replaced by an empty one. A plugin writes into the
//! graph only the small keyed kinds its manifest lists, each with a subject rule and a body bound;
//! anything else, a snapshot above all, is refused before it reaches the store. Other members read
//! a plugin's data from the member that runs it, through the client read relay.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use rusqlite::Connection;
use serde_json::Value;

/// One graph kind a plugin may write.
#[derive(Clone, Copy, Debug)]
pub struct GraphKind {
    pub kind: &'static str,
    /// Every subject the plugin writes this kind on starts with this.
    pub subject_prefix: &'static str,
    /// The most bytes the claim's fields may take as JSON.
    pub max_fields_bytes: usize,
}

/// What a plugin is and the only graph writes it may make.
#[derive(Clone, Copy, Debug)]
pub struct Manifest {
    pub name: &'static str,
    pub graph_kinds: &'static [GraphKind],
}

/// Why a plugin's graph write was refused.
#[derive(Debug, PartialEq, Eq)]
pub struct Refused {
    pub plugin: &'static str,
    pub kind: String,
    pub reason: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "plugin {} may not write {}: {}", self.plugin, self.kind, self.reason)
    }
}

impl std::error::Error for Refused {}

impl Manifest {
    /// Whether this plugin may write a `kind` claim with `fields` on `subject`: the kind is on its
    /// list, the subject is under the kind's prefix, the fields fit the kind's bound and the
    /// registry's fixed field set. Snapshots, such as `resource.observed`, are never on a list.
    pub fn admit(
        &self,
        subject: &str,
        kind: &str,
        fields: &BTreeMap<String, Value>,
    ) -> std::result::Result<(), Refused> {
        let refuse = |reason: String| Refused { plugin: self.name, kind: kind.to_owned(), reason };
        let Some(allowed) = self.graph_kinds.iter().find(|allowed| allowed.kind == kind) else {
            return Err(refuse("the kind is not in the plugin's graph allowlist".into()));
        };
        if !subject.starts_with(allowed.subject_prefix) {
            return Err(refuse(format!("the subject is not under {}", allowed.subject_prefix)));
        }
        let size = serde_json::to_vec(fields).map_or(usize::MAX, |bytes| bytes.len());
        if size > allowed.max_fields_bytes {
            return Err(refuse(format!(
                "the fields take {size} bytes; the bound is {}",
                allowed.max_fields_bytes
            )));
        }
        fractal_schema::registry()
            .validate_claim(subject, kind, fields)
            .map(|_| ())
            .map_err(|error| refuse(error.message))
    }
}

/// The directory a plugin owns beneath `state_dir`.
pub fn directory(state_dir: &Path, plugin: &str) -> PathBuf {
    state_dir.join("plugins").join(plugin)
}

/// Open the plugin's database beneath `state_dir`, created on first use, and run `migrate` on it.
/// A file `migrate` refuses, damaged or from a newer build, is set aside beside it as
/// `<name>.sqlite3.set-aside-*` and replaced by an empty one, with a warning. It is an error only
/// when the directory cannot be made, the file cannot be moved aside, or no database opens in its
/// place.
pub fn open(
    state_dir: &Path,
    plugin: &str,
    migrate: impl Fn(&Connection) -> Result<()>,
) -> Result<Connection> {
    let directory = directory(state_dir, plugin);
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("create {}", directory.display()))?;
    let file = format!("{plugin}.sqlite3");
    let path = directory.join(&file);
    let open_at = |path: &Path| -> Result<Connection> {
        let connection =
            Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&connection)?;
        Ok(connection)
    };
    match open_at(&path) {
        Ok(connection) => Ok(connection),
        Err(error) => {
            let at = super::now_ms();
            let aside = (0..)
                .map(|n| directory.join(format!("{file}.set-aside-{at}-{n}")))
                .find(|aside| !aside.exists())
                .expect("an unused name");
            for suffix in ["", "-wal", "-shm"] {
                let from = directory.join(format!("{file}{suffix}"));
                if from.exists() {
                    let to = aside.with_file_name(format!(
                        "{}{suffix}",
                        aside.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    std::fs::rename(&from, &to).with_context(|| {
                        format!("set aside {} after: {error:#}", from.display())
                    })?;
                }
            }
            tracing::warn!(
                plugin,
                path = %path.display(),
                aside = %aside.display(),
                "a plugin database could not be used and was set aside; the plugin rebuilds it from its source: {error:#}"
            );
            open_at(&path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE: Manifest = Manifest {
        name: "fixture",
        graph_kinds: &[GraphKind {
            kind: fractal_schema::github_plugin::EVENT,
            subject_prefix: "github-event/",
            max_fields_bytes: 400,
        }],
    };

    fn event(detail: Value) -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("version".into(), json!(1)),
            ("item".into(), json!("resource/github/acme/garden/pull-request/7")),
            ("event".into(), json!("checks-failed")),
            ("observer".into(), json!("observer/github/acme/garden")),
            ("at".into(), json!("2026-10-30T12:00:00.000Z")),
            ("detail".into(), detail),
        ])
    }

    #[test]
    fn a_plugin_writes_only_its_listed_keyed_kinds_within_their_bounds() {
        let subject = "github-event/github/acme/garden/pull-request/7/checks/abc/fail";
        FIXTURE.admit(subject, "github.event", &event(json!({}))).unwrap();
        let refused = |subject: &str, kind: &str, fields: &BTreeMap<String, Value>| {
            FIXTURE.admit(subject, kind, fields).unwrap_err().reason
        };
        assert!(refused("resource/github/acme/garden/pull-request/7", "resource.observed",
            &BTreeMap::from([("kind".into(), json!("vcs.pull-request")), ("facts".into(), json!({}))])
        ).contains("allowlist"));
        assert!(refused("github-events/acme/garden/7", "github.event", &event(json!({}))).contains("not under"));
        let big = json!({"note": "x".repeat(400)});
        assert!(refused(subject, "github.event", &event(big)).contains("bound is 400"));
        let mut extra = event(json!({}));
        extra.insert("facts".into(), json!({"title": "Garden paths"}));
        assert!(!refused(subject, "github.event", &extra).is_empty(), "a field outside the kind's set");
    }

    #[test]
    fn an_unusable_database_is_set_aside_and_a_missing_directory_is_an_error() {
        let state = tempfile::tempdir().unwrap();
        let refuse_newer = |connection: &Connection| -> Result<()> {
            let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            anyhow::ensure!(version <= 1, "version {version}");
            connection.pragma_update(None, "user_version", 1)?;
            Ok(())
        };
        drop(open(state.path(), "fixture", refuse_newer).unwrap());
        let path = directory(state.path(), "fixture").join("fixture.sqlite3");
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 9).unwrap();
        let reopened = open(state.path(), "fixture", refuse_newer).unwrap();
        let version: i64 = reopened.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, 1);
        assert_eq!(
            std::fs::read_dir(directory(state.path(), "fixture")).unwrap()
                .filter(|entry| entry.as_ref().unwrap().file_name().to_string_lossy()
                    .starts_with("fixture.sqlite3.set-aside-")
                    && !entry.as_ref().unwrap().file_name().to_string_lossy().ends_with("-wal")
                    && !entry.as_ref().unwrap().file_name().to_string_lossy().ends_with("-shm"))
                .count(),
            1
        );
        std::fs::write(state.path().join("plugins/blocked"), b"a file").unwrap();
        assert!(open(state.path(), "blocked", refuse_newer).is_err());
    }
}
