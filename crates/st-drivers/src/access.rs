//! Install policy for st3-managed providers. Authored permission options remain authoritative.
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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

    pub fn from_environment() -> Result<Self> {
        match std::env::var(MODE_ENV).as_deref() {
            Ok("auto") => Ok(Self::Auto),
            Ok("bypass") | Err(std::env::VarError::NotPresent) => Ok(Self::Bypass),
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
