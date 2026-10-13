//! Install policy for st3-managed providers. Authored permission options remain authoritative.
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MODE_ENV: &str = "ST_ACCESS_MODE";
pub const CODEX_PROFILE: &str = "st-managed";
/// Set to `1` by the daemon when the setup screen's "Let Codex work without asking" box is
/// unticked: Codex then keeps its own approval defaults instead of the install access mode's.
pub const CODEX_ASKS_ENV: &str = "ST_CODEX_ASKS";

pub fn codex_asks() -> bool {
    std::env::var(CODEX_ASKS_ENV).as_deref() == Ok("1")
}

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

pub fn claude_defaults(mode: AccessMode, explicit_permissions: bool) -> Vec<String> {
    if explicit_permissions {
        return vec![];
    }
    match mode {
        AccessMode::Bypass => vec!["--dangerously-skip-permissions".into()],
        AccessMode::Auto => vec!["--permission-mode".into(), "auto".into()],
    }
}
