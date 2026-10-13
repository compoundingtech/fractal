//! Install policy for st3-managed providers. Authored permission options remain authoritative.
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MODE_ENV: &str = "ST_ACCESS_MODE";
pub const CODEX_PROFILE: &str = "st-managed";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AccessMode {
    Auto,
    #[default]
    Bypass,
}

impl AccessMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Bypass => "bypass",
        }
    }

    /// The daemon always supplies the resolved install mode, including legacy bypass.
    /// A hand-run driver has no install policy and preserves its complete provider argv.
    pub fn from_environment() -> Result<Option<Self>> {
        match std::env::var(MODE_ENV).as_deref() {
            Ok("auto") => Ok(Some(Self::Auto)),
            Ok("bypass") => Ok(Some(Self::Bypass)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            _ => anyhow::bail!("ST_ACCESS_MODE must be auto or bypass"),
        }
    }
}

/// One inline table avoids CLI dotted-key splitting of socket paths containing periods.
pub fn codex_profile(socket: &Path, workspace: &Path) -> Result<String> {
    anyhow::ensure!(
        socket.is_absolute() && workspace.is_absolute(),
        "the managed Codex profile needs an absolute st socket and workspace"
    );
    let quoted = |path: &Path| -> Result<String> {
        Ok(toml::Value::String(
            path.to_str()
                .context("managed Codex path is not UTF-8")?
                .into(),
        )
        .to_string())
    };
    Ok(format!(
        "permissions.{CODEX_PROFILE}={{extends=\":workspace\",workspace_roots={{{}=true}},network={{enabled=true,unix_sockets={{{}=\"allow\"}}}}}}",
        quoted(workspace)?,
        quoted(socket)?,
    ))
}

pub fn codex_defaults(
    mode: AccessMode,
    socket: Option<&Path>,
    workspace: &Path,
    explicit_permissions: bool,
) -> Result<Vec<String>> {
    let mut defaults = vec!["--dangerously-bypass-hook-trust".into()];
    if !explicit_permissions {
        match mode {
            AccessMode::Bypass => {
                defaults.push("--dangerously-bypass-approvals-and-sandbox".into())
            }
            AccessMode::Auto => defaults.extend([
                "-c".into(),
                codex_profile(
                    socket.context("managed auto Codex needs ST3_ENDPOINT")?,
                    workspace,
                )?,
                "-c".into(),
                format!("default_permissions=\"{CODEX_PROFILE}\""),
                // The remote TUI that owns the thread flattens the named profile and drops its
                // network table, so the daemon socket gets EPERM. Only this legacy key carries
                // network through (verified against Codex 0.160.1 through server plus remote TUI).
                "-c".into(),
                "sandbox_workspace_write.network_access=true".into(),
                // Live search is Codex's default only with no sandbox; an agent without it is not useful.
                "-c".into(),
                "web_search=\"live\"".into(),
                "-a".into(),
                "on-request".into(),
                "-c".into(),
                "approvals_reviewer=\"auto_review\"".into(),
            ]),
        }
    }
    Ok(defaults)
}

pub fn claude_defaults(mode: AccessMode, explicit_permissions: bool, workspace: &Path) -> Vec<String> {
    if explicit_permissions {
        return vec![];
    }
    match mode {
        AccessMode::Bypass => vec!["--dangerously-skip-permissions".into()],
        // Auto mode's classifier treats a read in another repository as scope escalation and asks a
        // person, which stalls an unattended seat (a release builder reading a sibling worktree
        // overnight). The seat may read its sibling worktrees and scratch; everything else,
        // including the rest of the home directory, and every write and command stay with the
        // classifier.
        AccessMode::Auto => vec![
            "--permission-mode".into(),
            "auto".into(),
            "--settings".into(),
            auto_read_settings(workspace, std::env::var_os("HOME").map(PathBuf::from).as_deref()),
        ],
    }
}

/// Credential and login paths an auto-mode Claude seat is never allowed to read, whatever else
/// allows it: SSH keys, GitHub tokens, the fabric identity, harness logins and configuration,
/// package-registry tokens, st's key material and secret profiles. `~` is the home directory.
pub const AUTO_READ_DENIED: &[&str] = &[
    "~/.ssh/**",
    "~/.config/gh/**",
    "~/.config/fabric/**",
    "~/.config/sekrets/**",
    "~/.sekrets/**",
    "~/.claude-accounts/**",
    "~/.claude/.credentials.json",
    "~/.claude.json",
    "~/.codex/auth.json",
    "~/.codex/config.toml",
    "~/.npmrc",
    "~/.local/state/st3/keys/**",
    "~/.local/state/st3/accounts/**",
    "~/.config/st3/**",
];

/// Where an auto-mode Claude seat may read without asking: the source tree, scratch and the Nix
/// store (`/private/var/tmp` is where macOS resolves `/var/tmp`), plus, for a workspace under a
/// directory literally named `github.com` that is not under `~/src` (a Mac's
/// `~/Developer/github.com/org/repo`), that `github.com` tree. The tree must be strictly inside the
/// home directory. Any other workspace layout adds nothing.
pub fn auto_read_allowed(workspace: &Path, home: Option<&Path>) -> Vec<String> {
    let mut allowed = vec![
        "Read(~/src/**)".to_string(),
        "Read(//var/tmp/**)".into(),
        "Read(//private/var/tmp/**)".into(),
        "Read(//nix/store/**)".into(),
    ];
    if let Some(home) = home {
        let tree = workspace
            .ancestors()
            .skip(1)
            .find(|path| path.file_name().is_some_and(|name| name == "github.com"));
        if let Some(tree) = tree.filter(|tree| tree.starts_with(home) && *tree != home && !tree.starts_with(home.join("src"))) {
            allowed.push(format!("Read(/{}/**)", tree.display()));
        }
    }
    allowed
}

/// The `--settings` JSON that carries those rules (`//` marks an absolute path in Claude's rule
/// syntax; deny wins over allow). They merge with any authored `--settings`.
pub fn auto_read_settings(workspace: &Path, home: Option<&Path>) -> String {
    let deny: Vec<String> = AUTO_READ_DENIED.iter().map(|path| format!("Read({path})")).collect();
    serde_json::json!({"permissions": {"allow": auto_read_allowed(workspace, home), "deny": deny}}).to_string()
}

#[cfg(test)]
mod claude_default_tests {
    use super::*;

    #[test]
    fn auto_claude_reads_the_source_tree_and_scratch_but_never_credentials() {
        let home = Path::new("/home/example");
        let fixed = ["Read(~/src/**)", "Read(//var/tmp/**)", "Read(//private/var/tmp/**)", "Read(//nix/store/**)"];
        let in_src = Path::new("/home/example/src/github.com/org/repo--work");
        assert_eq!(auto_read_allowed(in_src, Some(home)), fixed);
        // A workspace under a github.com directory outside ~/src (a Mac's ~/Developer) adds that tree.
        let mac = Path::new("/home/example/Developer/github.com/org/app");
        assert_eq!(auto_read_allowed(mac, Some(home)).last().unwrap(), "Read(//home/example/Developer/github.com/**)");
        // Every other layout adds nothing: the default agent location, an instance root with its own
        // login and state, a shallow workspace, an application-support tree, and no home at all.
        for other in [
            "/home/example/st/agents/assistant",
            "/home/example/.st-instance/a/agents/x",
            "/home/example/work/proj",
            "/home/example/Library/Application Support/foo/bar",
            "/srv/elsewhere/a/b",
            "/home/example/github.com",
        ] {
            assert_eq!(auto_read_allowed(Path::new(other), Some(home)), fixed, "{other}");
        }
        assert_eq!(auto_read_allowed(mac, None), fixed);
        // The home directory itself is never opened.
        assert_eq!(auto_read_allowed(Path::new("/home/example/github.com/org/repo"), Some(home)).len(), 5);
        let settings: serde_json::Value = serde_json::from_str(&auto_read_settings(in_src, Some(home))).unwrap();
        // Only Read rules: writes and commands stay with the auto-mode classifier.
        let permissions = settings["permissions"].as_object().unwrap();
        assert_eq!(permissions.len(), 2);
        let deny: Vec<&str> = permissions["deny"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        for credential in ["Read(~/.ssh/**)", "Read(~/.config/gh/**)", "Read(~/.claude-accounts/**)", "Read(~/.codex/auth.json)"] {
            assert!(deny.contains(&credential), "{credential}");
        }
        assert!(permissions["allow"].as_array().unwrap().iter().all(|rule| rule.as_str().unwrap().starts_with("Read(")));
    }

    #[test]
    fn auto_defaults_carry_the_read_rules_while_bypass_and_explicit_permissions_do_not() {
        let workspace = Path::new("/var/tmp/example/work");
        let auto = claude_defaults(AccessMode::Auto, false, workspace);
        assert_eq!(&auto[..3], ["--permission-mode", "auto", "--settings"]);
        assert!(auto[3].contains("Read(//var/tmp/**)") && auto[3].contains("Read(~/.ssh/**)"));
        assert_eq!(claude_defaults(AccessMode::Bypass, false, workspace), ["--dangerously-skip-permissions"]);
        assert!(claude_defaults(AccessMode::Auto, true, workspace).is_empty());
    }
}
