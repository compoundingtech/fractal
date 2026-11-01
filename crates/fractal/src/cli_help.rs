//! Human entry points and the actions to take after creating durable work.
use std::fmt::Write as _;

use clap::CommandFactory as _;

use super::{Cli, presentation::shell_argument};

const GROUPS: &[(&str, &[&str])] = &[
    (
        "Everyday use",
        &[
            "ui",
            "web",
            "now",
            "agents",
            "conversations",
            "note",
            "missions",
            "schedules",
            "alerts",
            "usage",
            "slo",
            "terminals",
            "machines",
            "launch",
            "lanes",
            "devices",
            "clients",
            "sekrets",
            "demo",
        ],
    ),
    (
        "Inside an agent",
        &["work", "changes", "gh", "claim", "diagnostic", "trace", "skill"],
    ),
    (
        "Running fractal on a machine or fleet",
        &[
            "setup",
            "up",
            "service",
            "migrate",
            "fleet",
            "deploy",
            "replication",
            "apply",
            "validate",
            "sets",
            "backup",
            "doctor",
            "admission",
            "rules",
            "tiles",
            "repair",
            "persons",
            "store",
            "uninstall",
        ],
    ),
    (
        "Plumbing",
        &[
            "schema",
            "actor-contract",
            "subject",
            "harness-questions",
            "documents",
            "evidence",
            "blobs",
            "recorder",
            "activity",
            "import",
            "completions",
            "gate",
        ],
    ),
];

/// Only intercept root `help --all`. All other help paths remain clap's native help tree.
pub(super) fn all_help_requested(arguments: &[std::ffi::OsString]) -> bool {
    if !arguments.iter().any(|argument| argument == "--all") {
        return false;
    }
    // Use the real global options to find the root command, so option values such as a
    // device name or endpoint containing "help" cannot accidentally trigger this path.
    let parser = Cli::command().disable_help_subcommand(true).subcommand(
        clap::Command::new("help").arg(
            clap::Arg::new("all")
                .long("all")
                .action(clap::ArgAction::SetTrue),
        ),
    );
    parser
        .try_get_matches_from(arguments)
        .ok()
        .is_some_and(|matches| {
            matches
                .subcommand_matches("help")
                .is_some_and(|help| help.get_flag("all"))
        })
}

pub(super) fn root_help(all: bool) -> String {
    let command = Cli::command();
    let mut output = String::from(
        "See what needs you:\n  fractal now\n\nRun fractal in a terminal to open the interface:\n  fractal\n  fractal ui --space NAME\n\nStart an agent and talk to it:\n  fractal agents new NAME --harness claude --attach\n\nSee what is running:\n  fractal missions ls\n  fractal agents ls\n\nFrom an agent:\n  fractal skill\n  fractal work ls --as \"$ST_AGENT\"\n\nRecord an authorized job:\n  Claim an existing step first, or fractal work start TITLE --as \"$ST_AGENT\".\n  Use a finite mission for dependencies, verification gates or review.\n\nUsage: fractal [OPTIONS] <COMMAND>\n",
    );
    for (heading, names) in GROUPS {
        if *heading == "Plumbing" && !all {
            continue;
        }
        let _ = writeln!(output, "\n{heading}:");
        for name in *names {
            let subcommand = command
                .find_subcommand(name)
                .expect("help names a CLI command");
            let about = subcommand.get_about().expect("command has a purpose");
            let _ = writeln!(output, "  {name:14} {about}");
        }
    }
    let _ = writeln!(output, "\n{}", fractal_daemon::skill::message_sender_guidance());
    // Let clap keep the options, defaults and environment variables accurate.
    let mut options = command.clone().help_template("\nOptions:\n{options}");
    output.push('\n');
    output.push_str(&options.render_help().to_string());
    output.push_str("\nUse fractal help COMMAND for command help; fractal help --all also lists plumbing.\n");
    output.push_str(
        "\nIssues and suggestions are welcome. Please file an upstream issue:\n  gh issue create --repo compoundingtech/fractal\nPull requests are welcome too: https://github.com/compoundingtech/fractal\n",
    );
    output
}

pub(super) fn next_steps(state: &str, actions: &[(&str, String)]) -> String {
    let mut output = format!("{state}\n\nNext steps:\n");
    for (label, command) in actions {
        let _ = writeln!(output, "  {label}: {command}");
    }
    output
}

pub(super) fn agent_state(
    state: &str,
    harness: Option<&str>,
    fault: Option<&str>,
    reachability: &str,
) -> String {
    match state {
        "retired" => "Retired — this agent identity is permanent history.".into(),
        "running" => "Ready — the agent is running.".into(),
        "failed" => format!(
            "Could not start — {}.",
            fault.unwrap_or("the agent process failed")
        ),
        "stopped" => "Stopped — the agent is no longer running.".into(),
        "down" => "Down — st sees no running session for the agent; `fractal agents show` says since when.".into(),
        "suspended" => {
            "Suspended — the agent stopped at a quiet moment; `fractal agents resume` brings back its session.".into()
        }
        "waiting" if reachability == "indeterminate" => {
            "Unknown — st cannot tell which host runs this agent; its processes may be running. `fractal terminals` says the same until the claims agree.".into()
        }
        "waiting" if reachability != "reachable" => {
            "Still starting — the agent process cannot currently be reached.".into()
        }
        "waiting" => match harness {
            Some("unauthenticated" | "needs-login") => {
                "Waiting for you — attach to sign in to the agent's provider.".into()
            }
            // st withdraws an idle report it has not heard renewed in 90 s (idle seats renew every
            // five minutes): nothing is known to be waiting on a person.
            Some("indeterminate") => {
                "Quiet — st has no current report from the agent's harness; an idle agent reports every few minutes. Attach to inspect.".into()
            }
            _ => "Waiting for you — attach to inspect what the agent needs.".into(),
        },
        _ => match harness {
            None | Some("unobserved") => {
                "Still starting — waiting for the agent process to appear.".into()
            }
            Some("starting") => "Still starting — the agent process is initializing.".into(),
            _ => "Still starting — waiting for the agent to become ready.".into(),
        },
    }
}

pub(super) fn agent_next_steps(subject: &str, actor: &str, state: &str) -> String {
    let subject = shell_argument(subject);
    if state.starts_with("Retired") { return next_steps(state, &[("Show history", format!("fractal agents show {subject}"))]); }
    let attach_actor = if actor.starts_with("person/") {
        format!(" --as {}", shell_argument(actor))
    } else {
        String::new()
    };
    let actor = shell_argument(actor);
    let mut output = next_steps(
        state,
        &[
            (
                "Attach",
                format!("fractal terminals attach {subject}{attach_actor}"),
            ),
            (
                "Send a message",
                format!("fractal conversations send {subject} --from {actor} --body 'Hello'"),
            ),
            ("Show", format!("fractal agents show {subject}")),
            ("Stop", format!("fractal agents stop {subject} --as {actor}")),
        ],
    );
    output.push_str("\nUse fractal agents new NAME --attach to attach as soon as the agent starts. Detach with Ctrl+\\.\n");
    output
}

pub(super) fn mission_next_steps(run: &super::MissionRunView) -> String {
    let subject = shell_argument(&run.subject);
    let actor = shell_argument(if run.requester.starts_with("person/") {
        &run.requester
    } else {
        "person/NAME"
    });
    let state = match run.status.as_str() {
        "running" => "Started — the mission is running.".to_owned(),
        "desired" | "starting" => "Still starting — waiting for the mission's agents.".to_owned(),
        "waiting" | "blocked" => run.after.as_ref().map_or_else(
            || "Waiting — inspect the run to see what it needs.".to_owned(),
            |after| format!("Waiting — work starts after {after} completes."),
        ),
        "standing" => "Ready — the mission's agents are standing by.".to_owned(),
        "failed" => "Failed — inspect the run for the reason.".to_owned(),
        "completed" => "Completed — the mission's work has finished.".to_owned(),
        "cancelled" => "Cancelled — the mission has stopped.".to_owned(),
        _ => format!("Mission state: {}.", run.status),
    };
    next_steps(
        &state,
        &[
            ("Follow", format!("fractal missions show {subject} --follow")),
            ("Show", format!("fractal missions show {subject}")),
            (
                "Stop",
                format!("fractal missions cancel {subject} --as {actor} --reason 'No longer needed'"),
            ),
        ],
    )
}

pub(super) fn launch_next_steps(launch: &super::PlanningSessionView) -> String {
    let subject = shell_argument(&launch.subject);
    let requester = shell_argument(&launch.requester);
    let planner = shell_argument(&launch.planner);
    next_steps(
        "Started — the planner is preparing a mission for your review.",
        &[
            ("Show", format!("fractal launch show {subject}")),
            (
                "Send a message",
                format!("fractal conversations send {planner} --from {requester} --body 'Hello'"),
            ),
            ("Preview", format!("fractal launch preview {subject}")),
            (
                "Stop",
                format!("fractal launch cancel {subject} --as {requester}"),
            ),
        ],
    )
}

pub(super) fn pairing_next_steps(person: &str) -> String {
    let person = shell_argument(person);
    next_steps(
        "Waiting for your device — enter the pairing code in its st client before it expires.",
        &[
            (
                "Complete on the device",
                "fractal devices complete MEMBER_URL PAIRING_ID --fingerprint SHA256".to_owned(),
            ),
            ("Show paired devices", format!("fractal devices --as {person}")),
            (
                "Pair again",
                format!("fractal devices pair DEVICE_NAME --as {person}"),
            ),
        ],
    )
}

pub(super) fn fleet_next_steps(services: bool, sync_state: Option<&str>) -> String {
    let state = if sync_state == Some("verified") {
        "Ready — this machine joined the fleet and completed its first sync."
    } else if sync_state.is_some_and(|state| matches!(state, "failed" | "mismatch")) {
        "Sync failed — this machine joined the fleet; inspect its sync status for the reason."
    } else if services {
        "Still starting — this machine joined the fleet; check its sync status."
    } else {
        "Still starting — this machine joined the fleet; start its services to begin syncing."
    };
    let mut actions = vec![
        ("Show", "fractal fleet status".into()),
        ("Wait for sync", "fractal fleet wait".into()),
        ("Check machines", "fractal machines".into()),
    ];
    if !services {
        actions.push(("Start services", "fractal service install".into()));
    }
    next_steps(state, &actions)
}

pub(super) fn fleet_created_next_steps(services: bool) -> String {
    let state = if services {
        "Ready — this machine is the fleet's first member."
    } else {
        "Still starting — the fleet was created; start this machine's services."
    };
    let mut actions = vec![
        ("Invite a machine", "fractal fleet invite NAME".into()),
        ("Show", "fractal fleet status".into()),
    ];
    if !services {
        actions.push(("Start services", "fractal service install".into()));
    }
    next_steps(state, &actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_help_uses_fractal_product_prose() {
        fn inspect(command: &mut clap::Command) {
            let help = command.render_long_help().to_string();
            for line in help.lines() {
                // These describe real installed compatibility aliases, not product prose.
                if line.contains(
                    "Keep the installed Fractal executables (fractal, st3, st, fractal-migrate, and pty)",
                ) || line.contains("Install Fractal (st3 and the st link) and pty")
                {
                    continue;
                }
                for word in line.split_whitespace() {
                    // The executable override is a protected option, not the product name.
                    if word == "--st" {
                        continue;
                    }
                    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
                    assert!(
                        !matches!(word, "st" | "st3" | "stui" | "Smalltalk" | "SmallTalk"),
                        "old product wording in {} help: {line}",
                        command.get_name()
                    );
                }
            }
            for child in command.get_subcommands_mut() {
                inspect(child);
            }
        }
        inspect(&mut Cli::command());
    }

    #[test]
    fn root_help_groups_every_public_command_once_and_opens_with_examples() {
        let default = root_help(false);
        let all = root_help(true);
        assert!(default.starts_with("See what needs you:\n  fractal now\n"));
        assert!(default.contains("fractal agents new NAME --harness claude --attach"));
        assert!(default.contains("fractal missions ls\n  fractal agents ls"));
        assert!(default.find("Everyday use:") < default.find("Inside an agent:"));
        assert!(
            default.find("Inside an agent:")
                < default.find("Running fractal on a machine or fleet:")
        );
        assert!(!default.contains("Plumbing:"));
        assert!(all.contains("Plumbing:"));
        let migration_row = format!("  {:14} ", "migrate");
        assert!(default.contains(&migration_row));
        assert!(GROUPS.iter().any(|(title, names)| {
            *title == "Running fractal on a machine or fleet" && names.contains(&"migrate")
        }));
        let sender_guidance = fractal_daemon::skill::message_sender_guidance();
        assert_eq!(default.matches(sender_guidance).count(), 1);
        assert_eq!(all.matches(sender_guidance).count(), 1);
        for command in Cli::command()
            .get_subcommands()
            .filter(|command| !command.is_hide_set())
        {
            let row = format!("  {:14} ", command.get_name());
            assert_eq!(all.matches(&row).count(), 1, "{}", command.get_name());
            let plumbing = GROUPS.last().unwrap().1.contains(&command.get_name());
            assert_eq!(default.contains(&row), !plumbing, "{}", command.get_name());
        }
        assert!(default.contains("--endpoint <ENDPOINT>"));
        assert!(default.contains("--daemon-wait <SECONDS>"));
    }

    #[test]
    fn agent_endings_explain_state_and_name_exact_actions() {
        for (state, harness, expected) in [
            ("running", Some("running"), "Ready"),
            ("desired", None, "waiting for the agent process to appear"),
            (
                "starting",
                Some("unobserved"),
                "waiting for the agent process to appear",
            ),
            ("starting", Some("starting"), "initializing"),
            ("waiting", Some("needs-login"), "Waiting for you"),
            ("waiting", Some("blocked"), "Waiting for you"),
            ("waiting", Some("indeterminate"), "Quiet — st has no current report"),
            ("failed", None, "Could not start"),
            ("down", None, "Down — st sees no running session"),
        ] {
            let state = agent_state(state, harness, None, "reachable");
            assert!(state.contains(expected), "{state}");
            let output = agent_next_steps("agent/builder.demo", "person/avery", &state);
            for command in [
                "fractal terminals attach agent/builder.demo",
                "fractal conversations send agent/builder.demo --from person/avery --body 'Hello'",
                "fractal agents show agent/builder.demo",
                "fractal agents stop agent/builder.demo --as person/avery",
                "--attach",
            ] {
                assert!(output.contains(command), "{output}");
            }
            assert!(!output.contains("unobserved"));
        }
    }

    #[test]
    fn fleet_endings_distinguish_sync_and_service_states() {
        let ready = fleet_next_steps(true, Some("verified"));
        assert!(ready.starts_with("Ready"));
        let syncing = fleet_next_steps(true, None);
        assert!(syncing.starts_with("Still starting"));
        assert!(syncing.contains("fractal fleet wait"));
        assert!(!syncing.contains("fractal service install"));
        let stopped = fleet_next_steps(false, None);
        assert!(stopped.contains("start its services"));
        assert!(stopped.contains("fractal service install"));
        assert!(fleet_next_steps(true, Some("failed")).starts_with("Sync failed"));
        assert!(fleet_created_next_steps(true).starts_with("Ready"));
        assert!(fleet_created_next_steps(false).contains("fractal service install"));
        assert!(fleet_created_next_steps(true).contains("fractal fleet invite NAME"));
    }

    #[test]
    fn human_next_step_commands_use_fractal_including_retired_and_pairing_states() {
        for output in [
            agent_next_steps("agent/example/worker", "person/avery", "Ready"),
            agent_next_steps("agent/example/worker", "person/avery", "Retired"),
            pairing_next_steps("person/avery"),
            fleet_next_steps(false, None),
            fleet_created_next_steps(false),
        ] {
            let actions = output.split_once("Next steps:\n").unwrap().1;
            for line in actions.lines().filter(|line| line.starts_with("  ")) {
                let (_, command) = line.split_once(": ").unwrap();
                assert!(command.starts_with("fractal "), "{line}");
            }
            assert!(!output.contains("Use st "), "{output}");
        }
        for (state, harness, reachability, command) in [
            ("down", None, "reachable", "fractal agents show"),
            ("suspended", None, "reachable", "fractal agents resume"),
            ("waiting", Some("indeterminate"), "indeterminate", "fractal terminals"),
        ] {
            assert!(agent_state(state, harness, None, reachability).contains(command));
        }
    }

    #[test]
    fn an_unreachable_agent_does_not_imply_a_login_prompt() {
        let state = agent_state("waiting", Some("ready"), None, "unreachable");
        assert!(state.contains("cannot currently be reached"));
    }

    #[test]
    fn an_agent_on_an_undecided_host_says_st_cannot_tell() {
        let state = agent_state("waiting", Some("indeterminate"), None, "indeterminate");
        assert!(state.contains("cannot tell which host runs this agent"));
        assert!(!state.contains("Still starting"));
        assert!(!state.contains("Waiting for you"));
        for other in ["unreachable", "remote", "unknown"] {
            assert!(agent_state("waiting", Some("ready"), None, other).starts_with("Still starting"), "{other}");
        }
    }

    #[test]
    fn commands_quote_subjects_and_actors() {
        let output = agent_next_steps("agent/demo's seat", "person/avery", "Ready");
        assert!(output.contains("'agent/demo'\"'\"'s seat'"));
    }
}
