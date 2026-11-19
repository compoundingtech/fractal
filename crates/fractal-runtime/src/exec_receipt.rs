//! Immutable payload identity and optional repository observations, never credentials or grants.
use std::path::Path;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// NUL-separated argv, with no trailing NUL. OS arguments cannot contain NUL.
pub fn argv_sha256(argv: &[String]) -> String {
    let mut digest = Sha256::new();
    for (index, argument) in argv.iter().enumerate() {
        if index != 0 { digest.update([0]); }
        digest.update(argument.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PayloadLaunchSnapshot {
    pub argv: Vec<String>,
    pub argv_sha256: String,
    pub cwd: String,
    pub repository: RepositoryObservation,
}

impl PayloadLaunchSnapshot {
    pub(crate) fn capture(launch: &crate::Launch, cwd: &Path) -> Self {
        let argv = match launch {
            crate::Launch::Argv(argv) => argv.clone(),
            crate::Launch::Shell(source) => vec!["sh".into(), "-c".into(), source.clone()],
        };
        Self { argv_sha256: argv_sha256(&argv), argv, cwd: cwd.to_string_lossy().into_owned(),
            repository: RepositoryObservation::capture(cwd) }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryObservation {
    pub head: Option<String>,
    pub clean: Option<bool>,
}

impl RepositoryObservation {
    pub(crate) fn capture(cwd: &Path) -> Self {
        let git = |args: &[&str]| std::process::Command::new("git").current_dir(cwd).args(args)
            .output().ok().filter(|output| output.status.success());
        Self {
            head: git(&["rev-parse", "HEAD"]).and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|head| head.trim().to_owned()),
            clean: git(&["status", "--porcelain", "--untracked-files=normal"]).map(|output| output.stdout.is_empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn argv_digest_preserves_argument_boundaries_and_has_no_trailing_nul() {
        let argv = vec!["echo".into(), "hello".into()];
        assert_eq!(argv_sha256(&argv), format!("{:x}", Sha256::digest(b"echo\0hello")));
        assert_ne!(argv_sha256(&argv), argv_sha256(&["echohello".into()]));
        assert_ne!(argv_sha256(&argv), argv_sha256(&["echo".into(), "hello".into(), "".into()]));
    }
}
