//! Local browser-session launching and installer alias ownership.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Only exact installer-created symlinks belong to this installation.
pub fn owns_alias(path: &Path) -> bool {
    std::fs::read_link(path)
        .is_ok_and(|target| target == Path::new("st3") || target == path.with_file_name("st3"))
}

pub fn may_remove(path: &Path) -> bool {
    path.file_name() != Some(OsStr::new("fractal")) || owns_alias(path)
}

/// Extend old setup manifests when an upgrade installed the new alias beside st3.
/// Recheck recorded aliases too: a foreign replacement never belongs to uninstall.
pub fn installed_binaries(mut files: Vec<PathBuf>) -> Vec<PathBuf> {
    let recorded = files.len();
    for index in 0..recorded {
        if files[index].file_name() == Some(OsStr::new("st3")) {
            let alias = files[index].with_file_name("fractal");
            if owns_alias(&alias) && !files.contains(&alias) {
                files.push(alias);
            }
        }
    }
    files.retain(|path| may_remove(path));
    files
}

pub fn web(explicit_state: Option<PathBuf>) -> ExitCode {
    let url = match std::env::var("FRACTAL_WEB_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            let path = explicit_state.or_else(|| {
                fractal_daemon::config::Config::load_unvalidated(None).ok().map(|config| config.state_dir)
            }).map(|state| state.join("web-launch-url"));
            let result = path.ok_or_else(|| anyhow::anyhow!("cannot resolve daemon state directory"))
                .and_then(|path| {
                    use std::io::Read as _;
                    let mut file = fractal_daemon::config::open_token_file(&path)?;
                    let mut url = String::new();
                    file.read_to_string(&mut url)?;
                    Ok(url)
                });
            match result {
                Ok(url) => url,
                Err(error) => {
                    eprintln!("fractal web: {error:#}; start the daemon with built web assets first");
                    return ExitCode::from(1);
                }
            }
        }
        Err(error) => {
            eprintln!("fractal web: FRACTAL_WEB_URL: {error}");
            return ExitCode::from(2);
        }
    };
    // A URL is an opener argument, never a shell command or an option.
    if !reqwest::Url::parse(&url)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
    {
        eprintln!("fractal web: FRACTAL_WEB_URL must be an HTTP or HTTPS URL");
        return ExitCode::from(2);
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let opened = Command::new(opener)
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !opened {
        fractal_daemon::cli_println!("{url}");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn parse(input: &[&str]) -> crate::Cli {
        use clap::Parser;
        crate::Cli::try_parse_from(input).unwrap()
    }

    #[test]
    fn fractal_dispatch_no_arguments_keeps_onboarding_and_terminal_guards() {
        let arguments = [OsString::from("/bin/fractal")];
        for (stdin, stdout, seat, expected) in [
            (true, true, false, true),
            (false, true, false, false),
            (true, false, false, false),
            (true, true, true, false),
        ] {
            assert_eq!(
                crate::plain_ui_requested(arguments.len(), stdin, stdout, seat),
                expected
            );
        }
    }

    #[test]
    fn tui_alias_reuses_ui_arguments_with_global_endpoint() {
        for arguments in [
            &["fractal", "tui", "--demo"][..],
            &["fractal", "--endpoint", "http://localhost:7410", "tui", "--demo"],
            &["fractal", "tui", "--endpoint", "http://localhost:7410", "--demo"],
        ] {
            let cli = parse(arguments);
            assert!(matches!(cli.command, crate::Command::Ui(options) if options.demo));
            if arguments.contains(&"--endpoint") {
                assert_eq!(cli.endpoint.as_deref(), Some("http://localhost:7410"));
            }
        }
    }

    #[test]
    fn installed_alias_is_discovered_for_old_manifests_and_only_removed_while_owned() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let st3 = directory.path().join("st3");
        let alias = directory.path().join("fractal");
        let legacy_files = vec![st3.clone(), directory.path().join("st")];
        for target in [PathBuf::from("st3"), st3] {
            symlink(&target, &alias).unwrap();
            let upgraded = installed_binaries(legacy_files.clone());
            assert_eq!(upgraded.last(), Some(&alias));
            assert!(may_remove(&alias));
            assert_eq!(installed_binaries(upgraded.clone()), upgraded);
            std::fs::remove_file(&alias).unwrap();
        }
        for target in ["foreign", "/missing/foreign"] {
            symlink(target, &alias).unwrap();
            let mut recorded = legacy_files.clone();
            recorded.push(alias.clone());
            assert_eq!(installed_binaries(recorded), legacy_files);
            assert!(!may_remove(&alias));
            std::fs::remove_file(&alias).unwrap();
        }
        std::fs::write(&alias, "foreign").unwrap();
        assert!(!may_remove(&alias));
        assert_eq!(installed_binaries(legacy_files.clone()), legacy_files);
        std::fs::remove_file(&alias).unwrap();
        std::fs::create_dir(&alias).unwrap();
        assert!(!may_remove(&alias));
    }
}
