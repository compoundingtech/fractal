//! Additive program-name routing; the existing CLI and UI keep their own identity.
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, ExitCode};

#[derive(Debug, Eq, PartialEq)]
pub enum Dispatch {
    Cli,
    Web,
}

/// Rewrite only the new program's UI spelling. All legacy invocations pass through untouched.
pub fn dispatch(arguments: &mut Vec<OsString>) -> Dispatch {
    let fractal = arguments
        .first()
        .and_then(|program| Path::new(program).file_name())
        == Some(OsStr::new("fractal"));
    // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases — DELETE at contraction — https://github.com/compoundingtech/smalltalk/issues/3032
    // st and st3 retain their existing CLI parser, display strings and bare-program behavior.
    if !fractal {
        return Dispatch::Cli;
    }
    // LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases
    match arguments.get(1).and_then(|argument| argument.to_str()) {
        None => arguments.push("ui".into()),
        Some("tui") => arguments[1] = "ui".into(),
        Some("web") => return Dispatch::Web,
        _ => {}
    }
    Dispatch::Cli
}

pub fn web(arguments: &[OsString]) -> ExitCode {
    if arguments.len() > 2 {
        if arguments.len() == 3 && matches!(arguments[2].to_str(), Some("--help" | "-h")) {
            st3::cli_println!("Usage: fractal web\n\nOpen the running web UI (FRACTAL_WEB_URL; default http://127.0.0.1:5173). This command does not start a server.");
            return ExitCode::SUCCESS;
        }
        eprintln!("Usage: fractal web");
        return ExitCode::from(2);
    }
    let url = match std::env::var("FRACTAL_WEB_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => "http://127.0.0.1:5173".into(),
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
        st3::cli_println!("{url}");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(input: &[&str]) -> (Dispatch, Vec<OsString>) {
        let mut arguments = input.iter().map(OsString::from).collect();
        (dispatch(&mut arguments), arguments)
    }

    #[test]
    fn fractal_dispatch_no_arguments_opens_tui() {
        assert_eq!(
            route(&["/bin/fractal"]),
            (Dispatch::Cli, vec!["/bin/fractal".into(), "ui".into()])
        );
    }

    #[test]
    fn fractal_dispatch_tui_reuses_ui_arguments() {
        assert_eq!(
            route(&["fractal", "tui", "--help"]),
            (
                Dispatch::Cli,
                vec!["fractal".into(), "ui".into(), "--help".into()]
            )
        );
    }

    #[test]
    fn fractal_dispatch_web() {
        assert_eq!(route(&["fractal", "web"]).0, Dispatch::Web);
    }

    #[test]
    fn fractal_dispatch_cli_passthrough_and_legacy_are_unchanged() {
        for input in [
            &["fractal", "work", "ls"][..],
            &["fractal", "agents", "ls"],
            &["st"],
            &["st3"],
            &["st", "web"],
            &["st3", "tui"],
        ] {
            assert_eq!(
                route(input),
                (Dispatch::Cli, input.iter().map(OsString::from).collect())
            );
        }
    }
}
