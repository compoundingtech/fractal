//! Workspace pre-trust (R12) for an explicit provider runtime or operator command. The eval runner
//! and the reconciler never infer a provider from an opaque command.
//!
//! The claude config is a JSON object at `$CLAUDE_CONFIG_DIR/.claude.json` (else `$HOME/.claude.json`)
//! whose `projects` map is keyed by absolute workspace path. A workspace is trusted when its entry has
//! `"hasTrustDialogAccepted": true`; st also sets `"hasCompletedProjectOnboarding": true` to skip
//! onboarding friction. We merge into any existing entry and never clobber the
//! other per-project fields — and write ALL requested dirs in ONE atomic read-modify-write.
//!
//! The same write completes Claude's own first launch on a config that has never run it: the
//! text-style picker (and the welcome that follows it) is shown while the top-level
//! `hasCompletedOnboarding` is not `true`, and finishing it writes that key and the chosen `theme`
//! (see [`complete_first_launch`]). The seat then reaches the login prompt, or its first usable
//! screen, and a missing login is the only thing left for the person to do.
//!
//! A booted Claude process periodically flushes `.claude.json`. One explicit batch prevents sibling
//! trust writes from losing updates. Each Claude process re-reads and replaces the file under its own
//! lock, so the trust write takes that lock too — see [`ClaudeConfigLock`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use sha2::Digest;

use crate::flock::{self, FileLock};

/// The claude config file: `$CLAUDE_CONFIG_DIR/.claude.json` if set, else `$HOME/.claude.json`.
pub(crate) fn config_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        Ok(PathBuf::from(dir).join(".claude.json"))
    } else {
        let home =
            std::env::var_os("HOME").context("neither $CLAUDE_CONFIG_DIR nor $HOME is set")?;
        Ok(PathBuf::from(home).join(".claude.json"))
    }
}

/// Pre-trust `dirs` for BOTH harnesses — claude (`~/.claude.json`) AND codex
/// (`~/.codex/config.toml`). Writing the other harness's entry is harmless because each harness
/// reads only its own config. Returns the number of dirs written.
pub fn pretrust(dirs: &[PathBuf]) -> Result<usize> {
    let n = pretrust_at(&config_path()?, dirs)?;
    // Codex has its OWN "Do you trust this directory?" prompt (confirmed: bypass-approvals does not
    // skip it), stored separately — mark the dirs trusted there too so a codex seat never hangs.
    pretrust_codex_at(&codex_config_path()?, dirs)?;
    Ok(n)
}

/// Admit workspaces for Claude in the provider runtime's selected config. A typed Claude driver
/// calls this after task environment selection and before it starts Claude.
pub fn pretrust_claude(dirs: &[PathBuf]) -> Result<usize> {
    pretrust_at(&config_path()?, dirs)
}

/// Admit workspaces for Codex in the provider runtime's selected config, `$CODEX_HOME/config.toml`
/// or `~/.codex/config.toml`: the file the seat's own Codex reads. A Codex seat start calls this
/// for its work root before it starts Codex, so Codex does not ask "Do you trust this directory?".
pub fn pretrust_codex(dirs: &[PathBuf]) -> Result<usize> {
    pretrust_codex_at(&codex_config_path()?, dirs)
}

/// The codex config file: `$CODEX_HOME/config.toml` if set, else `~/.codex/config.toml`.
pub(crate) fn codex_config_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME") {
        Ok(PathBuf::from(dir).join("config.toml"))
    } else {
        let home = std::env::var_os("HOME").context("neither $CODEX_HOME nor $HOME is set")?;
        Ok(PathBuf::from(home).join(".codex").join("config.toml"))
    }
}

/// Mark each dir trusted in codex's `config.toml`: a `[projects."<abs-dir>"]` table with
/// `trust_level = "trusted"`. Preserves the existing config by APPENDING the table only when absent
/// (a full TOML round-trip would drop the user's comments/formatting), and is idempotent — a dir
/// already `trusted` is skipped. Returns dirs newly written.
pub fn pretrust_codex_at(config: &Path, dirs: &[PathBuf]) -> Result<usize> {
    let existing = std::fs::read_to_string(config).unwrap_or_default();
    // Parse read-only just to see which dirs are already trusted (never rewrites the file).
    let parsed: toml::Value =
        toml::from_str(&existing).unwrap_or(toml::Value::Table(Default::default()));
    let already = |dir: &str| -> bool {
        parsed
            .get("projects")
            .and_then(|p| p.get(dir))
            .and_then(|e| e.get("trust_level"))
            .and_then(|t| t.as_str())
            == Some("trusted")
    };

    let mut appended = String::new();
    let mut n = 0;
    for dir in dirs {
        let key = canonical_key(dir);
        if already(&key) {
            continue;
        }
        appended.push_str(&format!(
            "\n[projects.{}]\ntrust_level = \"trusted\"\n",
            toml_key(&key)
        ));
        n += 1;
    }
    if !appended.is_empty() {
        let mut out = existing;
        out.push_str(&appended);
        write_atomic_str(config, &out)?;
    }
    Ok(n)
}

/// Quote a path as a TOML basic string (escape `\` and `"`), for a `[projects."<dir>"]` header.
fn toml_key(dir: &str) -> String {
    format!("\"{}\"", dir.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The core, taking the config path explicitly so it is testable without touching the real config or
/// the process environment. Idempotent: re-trusting an already-trusted dir is a no-op merge.
pub fn pretrust_at(config: &Path, dirs: &[PathBuf]) -> Result<usize> {
    let _lock = ConfigLock::acquire(config)?;
    let _claude = ClaudeConfigLock::acquire(config)?;
    // Read the existing config, or start from an empty object if it is absent/blank.
    let mut root: Value = match std::fs::read_to_string(config) {
        Ok(s) if !s.trim().is_empty() => {
            serde_json::from_str(&s).with_context(|| format!("parsing {}", config.display()))?
        }
        _ => json!({}),
    };
    let obj = root
        .as_object_mut()
        .with_context(|| format!("{} is not a JSON object", config.display()))?;
    complete_first_launch(obj);
    let projects = obj
        .entry("projects")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("`projects` is not a JSON object")?;

    for dir in dirs {
        let key = canonical_key(dir);
        let entry = projects
            .entry(key)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("a `projects` entry is not a JSON object")?;
        // Merge — set only the trust flags and preserve every other field Claude wrote.
        entry.insert("hasTrustDialogAccepted".into(), Value::Bool(true));
        entry.insert("hasCompletedProjectOnboarding".into(), Value::Bool(true));
        for (name, hash) in approve_project_mcp_servers(dir, entry) {
            // Claude approves by name, so a later edit to the command is not asked again; this line
            // is how a changed definition is noticed after the fact.
            eprintln!(
                "st: approved project MCP server {name} in {} (definition sha256 {hash})",
                dir.display()
            );
        }
    }

    write_atomic(config, &root)?;
    Ok(dirs.len())
}

/// Complete Claude's first launch in a config that has never finished it, so the text-style picker
/// and the welcome are not shown to a new person. Claude shows them while the top-level
/// `hasCompletedOnboarding` is not `true`; finishing them writes `hasCompletedOnboarding: true`
/// (plus the version it ran, which nothing reads back) and the chosen `theme`, whose default is
/// `"dark"`. Read from Claude Code 2.1.296.
///
/// Only an ABSENT `hasCompletedOnboarding` is completed. A key that is present, true or false, was
/// written by Claude or by the person and is left alone, and so is every `theme` that is already
/// set: st writes the default theme only together with the onboarding flag it adds. A login is never
/// touched: `oauthAccount` and every other field are neither read nor written, so a signed-out
/// config stays signed out and Claude still asks for the login.
fn complete_first_launch(config: &mut serde_json::Map<String, Value>) {
    if config.contains_key("hasCompletedOnboarding") {
        return;
    }
    config.insert("hasCompletedOnboarding".into(), Value::Bool(true));
    config
        .entry("theme")
        .or_insert_with(|| Value::String("dark".into()));
}

/// The writer lock for one foreign harness config. Read only by `Drop`: the guard's job is to
/// outlive the read-merge-publish cycle, and closing the descriptor is what releases it.
struct ConfigLock {
    _held: FileLock,
    _predecessor: Option<FileLock>,
}

impl ConfigLock {
    fn acquire(config: &Path) -> Result<Self> {
        if let Some(parent) = config.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut path = config.as_os_str().to_owned();
        path.push(".st-trust.lock");
        let path = PathBuf::from(path);
        let file = flock::open(&path, flock::Open::Create)
            .with_context(|| format!("opening {}", path.display()))?;
        let held = FileLock::hold_blocking(file, flock::Mode::Exclusive)
            .map_err(|error| anyhow::anyhow!("locking {}: {error}", path.display()))?;
        // Existing writers may still hold the historical lock. Keep that inode and join
        // its lock domain when present, but never create a legacy path for a fresh config.
        // Both generations also take ClaudeConfigLock before reading or publishing.
        let mut predecessor = config.as_os_str().to_owned();
        predecessor.push(".st2trust.lock");
        let predecessor = PathBuf::from(predecessor);
        let old = match flock::open(&predecessor, flock::Open::Existing) {
            Ok(file) => Some(
                FileLock::hold_blocking(file, flock::Mode::Exclusive)
                    .with_context(|| format!("locking {}", predecessor.display()))?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("opening {}", predecessor.display()));
            }
        };
        Ok(Self {
            _held: held,
            _predecessor: old,
        })
    }
}

/// Claude treats its own config lock as abandoned once the directory's mtime is this old; a live
/// holder refreshes it at half this interval.
const CLAUDE_LOCK_STALE: Duration = Duration::from_secs(10);

/// How long a trust write waits for live Claude writers before it fails the seat start.
const CLAUDE_LOCK_WAIT: Duration = Duration::from_secs(30);

/// Claude's own writer lock for `.claude.json`: the `proper-lockfile` directory `<config>.lock`,
/// taken with `mkdir` and released with `rmdir`. Every Claude process re-reads, merges, and
/// replaces the config while it holds this lock.
///
/// The trust file lock excludes only other driver writers. A Claude process that read the config under
/// its lock before a trust write and replaced it afterwards published its older copy and dropped
/// the new workspace entry. On 2026-09-27 that left two of three seats started together at Claude's
/// trust prompt. Holding this lock serializes the trust write with every Claude writer.
struct ClaudeConfigLock {
    path: PathBuf,
}

impl ClaudeConfigLock {
    fn acquire(config: &Path) -> Result<Self> {
        let mut path = config.as_os_str().to_owned();
        path.push(".lock");
        let path = PathBuf::from(path);
        let deadline = Instant::now() + CLAUDE_LOCK_WAIT;
        let mut pause = Duration::from_millis(5);
        loop {
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("creating {}", path.display()));
                }
            }
            match std::fs::symlink_metadata(&path) {
                // `mkdir` never creates anything else, so a file or symlink here is not Claude's.
                Ok(metadata) if !metadata.is_dir() => anyhow::bail!(
                    "{} is not a directory, so it is not Claude's config lock",
                    path.display()
                ),
                Ok(metadata) if lock_is_stale(&metadata) => {
                    // Claude's own rule for an abandoned lock: remove it, then try again.
                    match std::fs::remove_dir(&path) {
                        Ok(()) => continue,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => {
                            return Err(error)
                                .with_context(|| format!("removing stale {}", path.display()));
                        }
                    }
                }
                Ok(_) => {}
                // Released between the `mkdir` and the `stat`.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("inspecting {}", path.display()));
                }
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "Claude held its config lock {} for {} seconds; the workspace trust was not written",
                path.display(),
                CLAUDE_LOCK_WAIT.as_secs()
            );
            std::thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(100));
        }
    }
}

impl Drop for ClaudeConfigLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

fn lock_is_stale(metadata: &std::fs::Metadata) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > CLAUDE_LOCK_STALE)
}

/// The absolute path claude keys a project by: the canonical (symlink-resolved) path when the dir
/// exists — matching `getcwd` for a booted agent whose cwd is this workspace — else the dir made
/// absolute against the current directory, else the path verbatim.
fn canonical_key(dir: &Path) -> String {
    std::fs::canonicalize(dir)
        .or_else(|_| std::env::current_dir().map(|c| c.join(dir)))
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Approve the servers the workspace's own `.mcp.json` declares, as the workspace trust above
/// already admits the code in the same directory.
///
/// Claude asks once per project server ("New MCP server found in .mcp.json") unless it is in the
/// project's `enabledMcpjsonServers`. The dialog stops startup before any MCP server connects, so
/// the st channel never attaches and mail is held; `--dangerously-skip-permissions` skips it, and
/// `--permission-mode auto` does not. A server the project lists in `disabledMcpjsonServers` stays
/// disabled, and an unreadable `.mcp.json` approves nothing.
///
/// Claude approves by NAME: once a name is enabled, a later edit to that server's command in
/// `.mcp.json` runs without a dialog. Returns each newly approved name with a hash of its
/// definition so the caller can log it.
fn approve_project_mcp_servers(
    dir: &Path,
    entry: &mut serde_json::Map<String, Value>,
) -> Vec<(String, String)> {
    let Some(declared) = std::fs::read(dir.join(".mcp.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|config| config.get("mcpServers").and_then(Value::as_object).cloned())
    else {
        return Vec::new();
    };
    let names = |key: &str| -> Vec<String> {
        entry
            .get(key)
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(|value| value.as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    };
    let disabled = names("disabledMcpjsonServers");
    let mut enabled = names("enabledMcpjsonServers");
    let mut approved = Vec::new();
    for (name, definition) in &declared {
        if !disabled.contains(name) && !enabled.contains(name) {
            enabled.push(name.clone());
            let digest = sha2::Sha256::digest(definition.to_string().as_bytes());
            approved.push((name.clone(), digest.iter().map(|byte| format!("{byte:02x}")).collect()));
        }
    }
    if !approved.is_empty() {
        entry.insert(
            "enabledMcpjsonServers".into(),
            Value::Array(enabled.into_iter().map(Value::String).collect()),
        );
    }
    approved
}

/// Write `value` to `config` atomically, so a crashed write never corrupts the real config.
fn write_atomic(config: &Path, value: &Value) -> Result<()> {
    let rendered = serde_json::to_string_pretty(value).context("serializing claude config")?;
    write_atomic_str(config, &rendered)
}

/// Stage-and-rename `contents` over `config`. The staging name carries the pid so concurrent
/// pretrusts do not collide.
///
/// Deliberately NOT the shared `fsatomic` primitive, and the reason is the same one that keeps
/// this module out of it: these are files st does not own — `~/.claude.json` and
/// `~/.codex/config.toml` — so the staged file must inherit the umask a harness's own config
/// carries rather than the driver's `0600`, and the staging path must sit beside the config under a name
/// its owner will recognize.
fn write_atomic_str(config: &Path, contents: &str) -> Result<()> {
    let mut tmp = config.as_os_str().to_owned();
    tmp.push(format!(".st-trust.{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    if let Some(parent) = config.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&tmp, contents).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, config)
        .with_context(|| format!("renaming {} into {}", tmp.display(), config.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretrust_approves_the_workspaces_project_mcp_servers_but_not_disabled_ones() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("work");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join(".mcp.json"),
            r#"{"mcpServers":{"one":{"command":"a"},"two":{"command":"b"},"three":{"command":"c"}}}"#,
        )
        .unwrap();
        let config = temp.path().join(".claude.json");
        let key = canonical_key(&workspace);
        std::fs::write(
            &config,
            serde_json::to_string(&json!({"projects": {key.clone(): {
                "enabledMcpjsonServers": ["one"], "disabledMcpjsonServers": ["two"], "lastCost": 1
            }}}))
            .unwrap(),
        )
        .unwrap();
        pretrust_at(&config, std::slice::from_ref(&workspace)).unwrap();
        let read = || -> Value { serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap() };
        let entry = read()["projects"][&key].clone();
        assert_eq!(entry["enabledMcpjsonServers"], json!(["one", "three"]));
        assert_eq!(entry["disabledMcpjsonServers"], json!(["two"]));
        assert_eq!(entry["lastCost"], 1, "other fields are preserved");
        let first = read();
        pretrust_at(&config, std::slice::from_ref(&workspace)).unwrap();
        assert_eq!(read(), first, "approving again changes nothing");
    }

    #[test]
    fn an_approved_name_stays_approved_when_its_command_changes_and_nothing_is_written() {
        // Claude approves by name, so st pins this rather than leaving it implicit: approving again
        // after the definition changed reports nothing new and does not touch the config.
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("work");
        std::fs::create_dir_all(&workspace).unwrap();
        let config = temp.path().join(".claude.json");
        std::fs::write(workspace.join(".mcp.json"), r#"{"mcpServers":{"one":{"command":"safe"}}}"#).unwrap();
        pretrust_at(&config, std::slice::from_ref(&workspace)).unwrap();
        let first = std::fs::read(&config).unwrap();
        std::fs::write(workspace.join(".mcp.json"), r#"{"mcpServers":{"one":{"command":"changed"}}}"#).unwrap();
        let mut entry = serde_json::from_slice::<Value>(&first).unwrap()["projects"][canonical_key(&workspace)]
            .as_object()
            .unwrap()
            .clone();
        assert!(approve_project_mcp_servers(&workspace, &mut entry).is_empty());
        pretrust_at(&config, std::slice::from_ref(&workspace)).unwrap();
        let after: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        assert_eq!(after["projects"][canonical_key(&workspace)]["enabledMcpjsonServers"], json!(["one"]));
    }

    #[test]
    fn a_newly_approved_server_is_reported_with_a_hash_of_its_definition() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".mcp.json"), r#"{"mcpServers":{"one":{"command":"a"}}}"#).unwrap();
        let mut entry = serde_json::Map::new();
        let approved = approve_project_mcp_servers(temp.path(), &mut entry);
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].0, "one");
        assert_eq!(approved[0].1.len(), 64);
        assert!(approve_project_mcp_servers(temp.path(), &mut entry).is_empty());
    }

    #[test]
    fn pretrust_leaves_mcp_approval_alone_without_a_readable_project_config() {
        let temp = tempfile::tempdir().unwrap();
        let none = temp.path().join("none");
        let broken = temp.path().join("broken");
        std::fs::create_dir_all(&none).unwrap();
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(".mcp.json"), "{ not json").unwrap();
        let config = temp.path().join(".claude.json");
        pretrust_at(&config, &[none.clone(), broken.clone()]).unwrap();
        let root: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        for dir in [&none, &broken] {
            let entry = &root["projects"][canonical_key(dir)];
            assert_eq!(entry["hasTrustDialogAccepted"], true);
            assert!(entry.get("enabledMcpjsonServers").is_none(), "{entry}");
        }
    }

    #[test]
    fn pretrust_creates_a_missing_config_and_trusts_the_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude.json");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();

        let n = pretrust_at(&cfg, std::slice::from_ref(&ws)).unwrap();
        assert_eq!(n, 1);

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let key = std::fs::canonicalize(&ws)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(v["projects"][&key]["hasTrustDialogAccepted"], json!(true));
        assert_eq!(
            v["projects"][&key]["hasCompletedProjectOnboarding"],
            json!(true)
        );
    }

    #[test]
    fn pretrust_completes_claudes_first_launch_on_a_clean_config_and_touches_no_login() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude.json");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();

        pretrust_at(&cfg, std::slice::from_ref(&ws)).unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(v["hasCompletedOnboarding"], json!(true));
        assert_eq!(v["theme"], json!("dark"));
        // A clean config stays signed out: no account, key or token field appears.
        let mut names: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["hasCompletedOnboarding", "projects", "theme"]);
    }

    #[test]
    fn pretrust_completes_first_launch_beside_a_login_and_unrelated_fields_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude.json");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let account = json!({ "emailAddress": "avery@example.com", "accountUuid": "account-a" });
        let seed = json!({ "oauthAccount": account, "numStartups": 3, "verbose": true });
        std::fs::write(&cfg, serde_json::to_string(&seed).unwrap()).unwrap();

        pretrust_at(&cfg, std::slice::from_ref(&ws)).unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(v["hasCompletedOnboarding"], json!(true));
        assert_eq!(v["oauthAccount"], account, "the login was touched");
        assert_eq!(v["numStartups"], json!(3));
        assert_eq!(v["verbose"], json!(true));
    }

    #[test]
    fn pretrust_leaves_a_present_onboarding_state_and_a_chosen_theme_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        // (seed, expected hasCompletedOnboarding, expected theme); None means the key is absent.
        let cases = [
            // Finished by the person with their own theme: nothing added or changed.
            (
                json!({ "hasCompletedOnboarding": true, "theme": "light-ansi" }),
                Some(json!(true)),
                Some(json!("light-ansi")),
            ),
            // Finished with no theme recorded: st does not invent one.
            (
                json!({ "hasCompletedOnboarding": true }),
                Some(json!(true)),
                None,
            ),
            // Present and false is Claude's or the person's state, not st's to flip.
            (
                json!({ "hasCompletedOnboarding": false, "theme": "light" }),
                Some(json!(false)),
                Some(json!("light")),
            ),
            // Not finished but a theme was already chosen: only the missing flag is added.
            (
                json!({ "theme": "dark-daltonized" }),
                Some(json!(true)),
                Some(json!("dark-daltonized")),
            ),
        ];
        for (index, (seed, onboarding, theme)) in cases.into_iter().enumerate() {
            let cfg = tmp.path().join(format!("case-{index}.json"));
            std::fs::write(&cfg, serde_json::to_string(&seed).unwrap()).unwrap();

            pretrust_at(&cfg, std::slice::from_ref(&ws)).unwrap();

            let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
            assert_eq!(v.get("hasCompletedOnboarding"), onboarding.as_ref(), "case {index}");
            assert_eq!(v.get("theme"), theme.as_ref(), "case {index}");
        }
    }

    #[test]
    fn completing_first_launch_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude.json");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();

        pretrust_at(&cfg, std::slice::from_ref(&ws)).unwrap();
        let first = std::fs::read_to_string(&cfg).unwrap();
        pretrust_at(&cfg, std::slice::from_ref(&ws)).unwrap();

        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), first);
    }

    #[test]
    fn pretrust_merges_into_an_existing_entry_without_clobbering_other_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude.json");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let key = std::fs::canonicalize(&ws)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        // Seed a config with a top-level key AND an existing project entry carrying extra fields.
        let seed = json!({
            "oauthAccount": { "keep": "me" },
            "projects": {
                &key: { "lastCost": 1.23, "hasTrustDialogAccepted": false },
                "/other/dir": { "hasTrustDialogAccepted": true }
            }
        });
        std::fs::write(&cfg, serde_json::to_string(&seed).unwrap()).unwrap();

        pretrust_at(&cfg, std::slice::from_ref(&ws)).unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        // The trust flag flipped true; the sibling field and the other top-level/project keys survive.
        assert_eq!(v["projects"][&key]["hasTrustDialogAccepted"], json!(true));
        assert_eq!(
            v["projects"][&key]["hasCompletedProjectOnboarding"],
            json!(true)
        );
        assert_eq!(
            v["projects"][&key]["lastCost"],
            json!(1.23),
            "existing field clobbered"
        );
        assert_eq!(
            v["oauthAccount"]["keep"],
            json!("me"),
            "top-level key clobbered"
        );
        assert_eq!(
            v["projects"]["/other/dir"]["hasTrustDialogAccepted"],
            json!(true),
            "sibling clobbered"
        );
    }

    #[test]
    fn pretrust_codex_appends_trust_preserving_config_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config.toml");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let key = std::fs::canonicalize(&ws)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        // Existing codex config with a COMMENT and an unrelated project entry — must survive.
        std::fs::write(&cfg, "# my codex config\nmodel = \"gpt-5\"\n\n[projects.\"/other\"]\ntrust_level = \"trusted\"\n").unwrap();

        assert_eq!(
            pretrust_codex_at(&cfg, std::slice::from_ref(&ws)).unwrap(),
            1
        );
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("# my codex config"), "comment clobbered");
        assert!(
            text.contains("model = \"gpt-5\""),
            "top-level setting clobbered"
        );
        assert!(
            text.contains("[projects.\"/other\"]"),
            "existing project clobbered"
        );
        // The new dir is trusted, and the whole file still parses as TOML.
        let v: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(v["projects"][&key]["trust_level"].as_str(), Some("trusted"));

        // Idempotent: re-trusting writes nothing (already trusted) and doesn't duplicate the table.
        assert_eq!(
            pretrust_codex_at(&cfg, std::slice::from_ref(&ws)).unwrap(),
            0
        );
        let again = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(
            again
                .matches(&format!("[projects.{}]", toml_key(&key)))
                .count(),
            1,
            "duplicated table"
        );
    }

    #[test]
    fn pretrust_is_batch_and_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude.json");
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();

        // Batch: both dirs in one write.
        assert_eq!(pretrust_at(&cfg, &[a.clone(), b.clone()]).unwrap(), 2);
        // Idempotent: re-trusting is a clean no-op merge (still trusted, no error).
        pretrust_at(&cfg, std::slice::from_ref(&a)).unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        for d in [&a, &b] {
            let key = std::fs::canonicalize(d)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            assert_eq!(v["projects"][&key]["hasTrustDialogAccepted"], json!(true));
        }
    }

    fn trusted(config: &Path, workspace: &Path) -> bool {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
        let key = std::fs::canonicalize(workspace)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        v["projects"][&key]["hasTrustDialogAccepted"] == json!(true)
    }

    /// Replace the config the way a Claude process does after its locked re-read.
    fn claude_replace(config: &Path, value: &Value) {
        let staged = config.with_extension("json.claude-save");
        std::fs::write(&staged, serde_json::to_string(value).unwrap()).unwrap();
        std::fs::rename(&staged, config).unwrap();
    }

    /// The 2026-09-27 lost update: a Claude process read the config under its own lock, the trust
    /// write landed, and Claude's replace published the older copy without the new workspace.
    #[test]
    fn a_trust_write_waits_for_claudes_config_lock_instead_of_being_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join(".claude.json");
        std::fs::write(&config, r#"{"numStartups":1,"projects":{}}"#).unwrap();
        let workspace = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();

        let claude_lock = tmp.path().join(".claude.json.lock");
        std::fs::create_dir(&claude_lock).unwrap();
        let mut read_under_lock: Value =
            serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();

        let trust = std::thread::spawn({
            let (config, workspace) = (config.clone(), workspace.clone());
            move || pretrust_at(&config, &[workspace]).unwrap()
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !trust.is_finished(),
            "the trust write must wait while Claude holds its config lock"
        );

        read_under_lock["numStartups"] = json!(2);
        claude_replace(&config, &read_under_lock);
        std::fs::remove_dir(&claude_lock).unwrap();
        trust.join().unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v["numStartups"], json!(2), "Claude's own save was lost");
        assert!(trusted(&config, &workspace), "the workspace trust was lost");
        assert!(
            !claude_lock.exists(),
            "the trust write must release Claude's lock"
        );
    }

    /// Several seats start at the same moment while a running Claude keeps saving the config under
    /// its own lock. Every seat's workspace must still be trusted when they have all started.
    #[test]
    fn seats_starting_together_keep_every_trust_entry_while_claude_saves_its_config() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Barrier};

        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join(".claude.json");
        std::fs::write(&config, r#"{"projects":{}}"#).unwrap();
        let claude_lock = tmp.path().join(".claude.json.lock");
        let workspaces = (0..6)
            .map(|seat| {
                let workspace = tmp.path().join(format!("seat-{seat}"));
                std::fs::create_dir_all(&workspace).unwrap();
                workspace
            })
            .collect::<Vec<_>>();

        let stop = Arc::new(AtomicBool::new(false));
        let (writer_started, writer_running) = std::sync::mpsc::sync_channel(0);
        let claude = std::thread::spawn({
            let (config, claude_lock, stop) = (config.clone(), claude_lock.clone(), stop.clone());
            move || {
                let mut saves = 0_u64;
                while !stop.load(Ordering::Relaxed) {
                    if std::fs::create_dir(&claude_lock).is_err() {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    if saves == 0 {
                        // Start the seats only after Claude has taken its lock. The writer is
                        // now guaranteed to finish at least one save before it sees `stop`.
                        writer_started.send(()).unwrap();
                    }
                    let mut current: Value =
                        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
                    std::thread::sleep(Duration::from_millis(5));
                    saves += 1;
                    current["numStartups"] = json!(saves);
                    claude_replace(&config, &current);
                    std::fs::remove_dir(&claude_lock).unwrap();
                    std::thread::sleep(Duration::from_millis(2));
                }
                saves
            }
        });

        writer_running.recv().unwrap();
        let start = Arc::new(Barrier::new(workspaces.len()));
        let seats = workspaces
            .iter()
            .cloned()
            .map(|workspace| {
                let (config, start) = (config.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    pretrust_at(&config, &[workspace]).unwrap();
                })
            })
            .collect::<Vec<_>>();
        for seat in seats {
            seat.join().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        assert!(claude.join().unwrap() > 0, "the Claude writer never saved");

        let lost = workspaces
            .iter()
            .filter(|workspace| !trusted(&config, workspace))
            .map(|workspace| {
                workspace
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert!(lost.is_empty(), "Claude's saves dropped trust for {lost:?}");
    }

    /// A Claude process that died holding its lock leaves the directory behind. Claude takes over a
    /// lock older than its stale threshold, and a seat start must not hang behind one either.
    #[test]
    fn an_abandoned_claude_config_lock_is_taken_over() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join(".claude.json");
        let claude_lock = tmp.path().join(".claude.json.lock");
        std::fs::create_dir(&claude_lock).unwrap();
        let abandoned = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::open(&claude_lock)
            .unwrap()
            .set_modified(abandoned)
            .unwrap();
        let workspace = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();

        pretrust_at(&config, std::slice::from_ref(&workspace)).unwrap();

        assert!(trusted(&config, &workspace));
        assert!(!claude_lock.exists());
    }

    #[test]
    fn a_symlinked_claude_config_lock_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join(".claude.json");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, tmp.path().join(".claude.json.lock")).unwrap();
        let workspace = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();

        let error = pretrust_at(&config, std::slice::from_ref(&workspace)).unwrap_err();
        assert!(
            format!("{error:#}").contains("not Claude's config lock"),
            "{error:#}"
        );
        assert!(
            outside.is_dir(),
            "a refused lock must leave its target alone"
        );
        assert!(!config.exists(), "a refused lock must publish nothing");
    }

    /// The transport hardened this lock file with `O_NOFOLLOW`; nothing pinned it before.
    #[test]
    fn a_symlinked_trust_lock_is_refused_instead_of_locking_its_target() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join(".claude.json");
        let outside = tmp.path().join("outside");
        std::fs::write(&outside, "unchanged").unwrap();
        std::os::unix::fs::symlink(&outside, tmp.path().join(".claude.json.st2trust.lock"))
            .unwrap();
        let workspace = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();

        let error = pretrust_at(&config, std::slice::from_ref(&workspace)).unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .and_then(std::io::Error::raw_os_error),
            Some(libc::ELOOP),
            "a symlinked trust lock must be refused, got {error:#}"
        );
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "unchanged");
        assert!(!config.exists(), "a refused lock must publish nothing");
    }

    #[test]
    fn the_trust_lock_is_created_private_and_excludes_a_second_holder() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join(".claude.json");
        let lock_path = tmp.path().join(".claude.json.st-trust.lock");
        let held = ConfigLock::acquire(&config).unwrap();
        assert_eq!(
            std::fs::metadata(&lock_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // `flock` locks the open file description, so a second open in this process contends.
        let probe = || {
            let file = flock::open(&lock_path, flock::Open::Existing).unwrap();
            FileLock::hold(file, flock::Mode::Exclusive, flock::Wait::Now)
                .unwrap()
                .is_some()
        };
        assert!(
            !probe(),
            "a live trust-lock holder must exclude a second writer"
        );
        drop(held);
        assert!(probe(), "dropping the guard must release the trust lock");
    }

    #[test]
    fn fresh_trust_writes_use_neutral_paths_and_join_existing_predecessor_locks() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join(".claude.json");
        let workspace = tmp.path().join("ws");
        std::fs::create_dir(&workspace).unwrap();
        pretrust_at(&config, &[workspace]).unwrap();
        assert!(tmp.path().join(".claude.json.st-trust.lock").exists());
        assert!(
            std::fs::read_dir(tmp.path())
                .unwrap()
                .all(|entry| { !entry.unwrap().file_name().to_string_lossy().contains("st2") })
        );

        let legacy = tmp.path().join(".claude.json.st2trust.lock");
        std::fs::write(&legacy, "historical inode").unwrap();
        let held = ConfigLock::acquire(&config).unwrap();
        let probe = || {
            let file = flock::open(&legacy, flock::Open::Existing).unwrap();
            FileLock::hold(file, flock::Mode::Exclusive, flock::Wait::Now)
                .unwrap()
                .is_some()
        };
        assert!(!probe(), "a new writer must exclude a predecessor writer");
        assert_eq!(
            std::fs::read_to_string(&legacy).unwrap(),
            "historical inode"
        );
        drop(held);
        assert!(probe());
    }
}
