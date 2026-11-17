//! Native mission continuity previews and explicit, hash-bound recovery.
use super::*;

#[derive(Subcommand)]
pub(super) enum RunCommand {
    /// Inspect or recover completed evidence without fabricating completions.
    Continuity {
        #[command(subcommand)]
        command: ContinuityCommand,
    },
}

impl RunCommand {
    pub(super) fn actor(&self) -> Option<&str> {
        match self {
            Self::Continuity { command: ContinuityCommand::Recover(args) } => Some(&args.actor),
            _ => None,
        }
    }
}

#[derive(Subcommand)]
pub(super) enum ContinuityCommand {
    /// Read eligible source evidence, current workspace, and the recovery preview hash.
    Preview {
        #[arg(add = ArgValueCompleter::new(Complete(Entity::MissionRun { unfinished_only: false })))]
        run: String,
    },
    /// Recover evidence by reference using an unchanged preview.
    Recover(RecoverArgs),
    /// Read which runs startup would supersede and which require review.
    StartupPreview,
}

#[derive(Args)]
pub(super) struct RecoverArgs {
    #[arg(add = ArgValueCompleter::new(Complete(Entity::MissionRun { unfinished_only: false })))]
    run: String,
    /// Exact hash returned by continuity preview; stale previews are refused.
    #[arg(long, value_name = "HASH")]
    preview_hash: String,
    /// Confirm legacy person evidence using the exact source step-run subject or
    /// root gate.requested claim ID from preview (repeatable; authenticated person only).
    #[arg(long = "confirm-legacy-evidence", value_name = "EVIDENCE")]
    confirmed_legacy_evidence: Vec<String>,
    #[arg(long = "as", value_parser = parse_publication_actor)]
    actor: String,
}

fn recovery_body(args: &RecoverArgs) -> Result<Value> {
    if !args.confirmed_legacy_evidence.is_empty() {
        parse_person_subject(&args.actor).map_err(anyhow::Error::msg)?;
    }
    Ok(json!({
        "run": args.run,
        "preview_hash": args.preview_hash,
        "confirmed_legacy_evidence": args.confirmed_legacy_evidence,
    }))
}

pub(super) async fn run(endpoint: &Endpoint, command: RunCommand, json_output: bool) -> Result<()> {
    let RunCommand::Continuity { command } = command;
    let client = cli_client(endpoint);
    let result: Value = match command {
        ContinuityCommand::Preview { run } => client.post(
            "/v1/mission-runs/continuity/preview", &json!({ "run": run }),
        ).await?,
        ContinuityCommand::Recover(args) => {
            reject_foreign_agent_actor(&args.actor)?;
            let body = recovery_body(&args)?;
            let attributed = execution_client(endpoint, args.actor)?;
            attributed.post("/v1/mission-runs/continuity/recover", &body).await?
        }
        ContinuityCommand::StartupPreview => client.get(
            "/v1/mission-runs/continuity/startup-preview",
        ).await?,
    };
    print_value(&result, json_output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_command(arguments: &[&str]) -> RunCommand {
        let cli = Cli::try_parse_from(arguments).expect("valid continuity command");
        let Command::Missions { command: MissionViewCommand::Run { command } } = cli.command else {
            panic!("expected mission run command");
        };
        command
    }

    #[test]
    fn continuity_previews_are_read_only_and_need_no_actor() {
        let command = parse_command(&[
            "fractal", "missions", "run", "continuity", "preview", "mission-run/example",
        ]);
        assert!(command.actor().is_none());
        let RunCommand::Continuity { command: ContinuityCommand::Preview { run } } = command else {
            panic!("expected continuity preview");
        };
        assert_eq!(run, "mission-run/example");
        let command = parse_command(&[
            "fractal", "missions", "run", "continuity", "startup-preview",
        ]);
        assert!(command.actor().is_none());
    }

    #[test]
    fn recovery_requires_hash_and_actor() {
        let base = ["fractal", "missions", "run", "continuity", "recover", "mission-run/example"];
        assert!(Cli::try_parse_from(base).is_err());
        assert!(Cli::try_parse_from(base.into_iter().chain(["--as", "person/example"])).is_err());
        assert!(Cli::try_parse_from(base.into_iter().chain(["--preview-hash", "snapshot"])).is_err());
        assert!(Cli::try_parse_from(base.into_iter().chain([
            "--as", "person/example", "--preview-hash", "snapshot",
            "--confirm-legacy-step", "step-run/example",
        ])).is_err());
    }

    #[test]
    fn recovery_preserves_explicit_legacy_confirmations_in_native_body() {
        let command = parse_command(&[
            "fractal", "missions", "run", "continuity", "recover", "mission-run/example",
            "--preview-hash", "snapshot", "--as", "person/example",
            "--confirm-legacy-evidence", "step-run/example", "--confirm-legacy-evidence", "claim/example",
        ]);
        assert_eq!(command.actor(), Some("person/example"));
        let RunCommand::Continuity { command: ContinuityCommand::Recover(mut args) } = command else {
            panic!("expected continuity recovery");
        };
        assert_eq!(recovery_body(&args).expect("person confirmation"), json!({
            "run": "mission-run/example",
            "preview_hash": "snapshot",
            "confirmed_legacy_evidence": ["step-run/example", "claim/example"],
        }));
        args.actor = "agent/example".to_owned();
        assert!(recovery_body(&args).is_err());
        args.confirmed_legacy_evidence.clear();
        assert!(recovery_body(&args).is_ok());
    }

    #[test]
    fn continuity_help_exposes_review_and_hash_bound_recovery() {
        let help = Cli::try_parse_from([
            "fractal", "missions", "run", "continuity", "--help",
        ]).err().expect("help response").to_string();
        for command in ["preview", "recover", "startup-preview"] {
            assert!(help.contains(command), "missing {command}");
        }
        let help = Cli::try_parse_from([
            "fractal", "missions", "run", "continuity", "recover", "--help",
        ]).err().expect("help response").to_string();
        for option in ["--preview-hash <HASH>", "--confirm-legacy-evidence <EVIDENCE>", "--as"] {
            assert!(help.contains(option), "missing {option}");
        }
    }
}
