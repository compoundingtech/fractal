//! Leaf-local read attribution is deliberately separate from typed query arguments.
//! All argv consumers use the enriched graph; handlers cannot use this actor as a filter.
use clap::{Arg, ArgMatches, Command};

const PATHS: &[&[&str]] = &[
    &["actor-contract"],
    &["agents", "lane"],
    &["agents", "rename-id", "proof", "verify"],
    &["changes", "list"],
    &["changes", "show"],
    &["conversations", "export"],
    &["fleet", "anchor", "show"],
    &["gate", "ci-passed"],
    &["gate", "merged"],
    &["missions", "blocked"],
    &["missions", "waiting-on"],
    &["missions", "rename-id", "proof", "verify"],
    &["recorder", "report"],
    &["subject", "history"],
    &["terminals", "peek"],
    &["trace", "show"],
    &["validate"],
];
const ID: &str = "inspection_actor";

pub(super) fn augment(mut command: Command) -> Command {
    fn add(command: Command, path: &[&str]) -> Command {
        if let Some((head, rest)) = path.split_first() {
            command.mut_subcommand(head, |child| add(child, rest))
        } else {
            command.arg(Arg::new(ID).long("as").value_name("ACTOR")
                .value_parser(super::parse_actor_subject)
                .help("Attribute this read to an actor; does not change its subject or filters"))
        }
    }
    for path in PATHS {
        command = add(command, path);
    }
    command
}

fn guard(actor: &str, own: Option<&str>, environment_present: bool) -> anyhow::Result<()> {
    anyhow::ensure!(own.is_some() || !environment_present,
        "agent identity is empty or conflicting; refusing read attribution");
    if let Some(own) = own.filter(|own| own.starts_with("agent/")) {
        anyhow::ensure!(actor == own, "actor flag must equal ST_AGENT literally ({own})");
    }
    Ok(())
}

pub(super) fn attribute(matches: &ArgMatches) -> anyhow::Result<()> {
    let mut leaf = matches;
    while let Some((_, child)) = leaf.subcommand() {
        leaf = child;
    }
    let Some(actor) = leaf.try_get_one::<String>(ID).ok().flatten() else {
        return Ok(());
    };
    let own = fractal_drivers::contracts::env("ST_AGENT");
    guard(actor, own.as_deref(), super::agent_environment_present())?;
    let mut path = String::from("fractal");
    let mut node = matches;
    while let Some((name, child)) = node.subcommand() {
        path.push(' ');
        path.push_str(name);
        node = child;
    }
    // Offline leaves have no daemon request to carry attribution. Use the same explicit
    // stderr audit record for all reads, preserving machine-readable result stdout.
    super::stderr_message("st", format_args!("read attribution: actor={actor} command={path}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::FromArgMatches as _;

    #[test]
    fn operational_reads_parse_leading_actor_without_changing_query_arguments() {
        let inputs: &[&[&str]] = &[
            &[], &["agent/subject"],
            &["proof.json", "--fleet-anchor", "anchor"],
            &["--repository", "demo"], &["change", "--repository", "demo"],
            &["mailbox"], &[], &["check", "--repo", "example/demo", "--ref", "main"],
            &["example/demo#1"], &[], &["mission-run/demo"],
            &["proof.json", "--fleet-anchor", "anchor"],
            &[], &["agent/subject"], &["terminal/demo"], &["agent/subject"], &["input.kdl"],
        ];
        assert_eq!(inputs.len(), PATHS.len());
        let contract = super::super::actor_contract::generate(super::super::command_graph());
        for (path, input) in PATHS.iter().zip(inputs) {
            let key = format!("st {}", path.join(" "));
            assert_eq!(contract["contract"]["commands"][&key]["actor"], "optional");
            assert_eq!(contract["contract"]["commands"][&key]["as_role"], "actor");
            let mut argv = vec!["fractal"];
            argv.extend_from_slice(path);
            argv.extend_from_slice(input);
            let plain = super::super::command_graph().try_get_matches_from(&argv).unwrap();
            argv.splice(1 + path.len()..1 + path.len(), ["--as", "agent/reader"]);
            let attributed = super::super::command_graph().try_get_matches_from(&argv).unwrap();
            // Derived handlers receive exactly the same typed query, with no actor field.
            super::super::Cli::from_arg_matches(&attributed).unwrap();
            let (mut left, mut right) = (&plain, &attributed);
            for name in *path {
                left = left.subcommand_matches(name).unwrap();
                right = right.subcommand_matches(name).unwrap();
            }
            for id in left.ids() {
                if id.as_str() != ID {
                    assert_eq!(left.get_raw(id.as_str()).map(Iterator::collect::<Vec<_>>),
                        right.get_raw(id.as_str()).map(Iterator::collect::<Vec<_>>), "{path:?}: {id}");
                }
            }
            let actor = right.get_one::<String>(ID).unwrap();
            assert!(guard(actor, Some("agent/reader"), true).is_ok(), "{path:?}");
            for own in [Some("agent/peer"), None] {
                assert!(guard(actor, own, true).is_err(), "{path:?}");
            }
            for foreign in ["agent/peer", "person/operator"] {
                assert!(guard(foreign, Some("agent/reader"), true).is_err(), "{path:?}");
            }
        }
    }

    #[test]
    fn inspection_actor_does_not_expand_neighboring_reads() {
        for path in [
            &["fleet", "anchor"][..], &["replication", "checkpoint", "status"],
            &["subject", "basis"], &["agents", "ls"],
            &["launch", "preview"],
            &["agents", "rename-id", "proof", "export"], &["missions", "rename-id", "proof", "export"],
            &["replication", "checkpoint", "plan"],
        ] {
            let mut argv = vec!["fractal"];
            argv.extend_from_slice(path);
            argv.extend(["--as", "agent/reader"]);
            assert!(super::super::command_graph().try_get_matches_from(argv).is_err());
        }
    }
}
