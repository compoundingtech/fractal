//! The argv contract is derived from the same clap graph used to parse commands.
use std::collections::BTreeMap;

use clap::CommandFactory as _;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

fn actor_argument(command: &clap::Command) -> Option<&clap::Arg> {
    command.get_arguments().find(|argument| argument.get_long() == Some("as"))
}

// Probe the real value parser, without other arguments, environment defaults, or conflicts.
// This describes CLI syntax, not the authority a daemon will grant to a caller.
fn accepts_person(argument: &clap::Arg) -> bool {
    clap::Command::new("actor-probe")
        .arg(clap::Arg::new("actor").long("as").value_parser(argument.get_value_parser().clone()))
        .try_get_matches_from(["actor-probe", "--as", "person/contract-probe"])
        .is_ok()
}

fn walk(command: &clap::Command, path: &str, commands: &mut BTreeMap<String, Value>) {
    let children = command.get_subcommands().filter(|child| child.get_name() != "help").collect::<Vec<_>>();
    if path != "st" && (children.is_empty() || !command.is_subcommand_required_set()) {
        let argument = actor_argument(command);
        let requirement = match argument {
            None => "none",
            Some(argument) if argument.is_required_set() => "required",
            Some(_) => "optional",
        };
        commands.insert(path.to_owned(), json!({
            "actor": requirement,
            "accepts_person": argument.is_some_and(accepts_person),
            "actor_environment": argument.and_then(|argument| argument.get_env()).map(|name| name.to_string_lossy()),
            "ignores_actor": argument.is_some_and(|argument| argument.get_id() == "ignored_actor"),
        }));
    }
    for child in children {
        walk(child, &format!("{path} {}", child.get_name()), commands);
    }
}

pub(super) fn generate(mut command: clap::Command) -> Value {
    command.build();
    let mut commands = BTreeMap::new();
    walk(&command, "st", &mut commands);
    let inventory = super::read_contract_inventory_for_command(command);
    let contract = json!({
        "kind": "st3.actor-contract.v1",
        "version": 1,
        "semantics": {
            "actor": "Whether this command accepts --as in argv; required can also be supplied by actor_environment. Optional does not mean that the operation needs no identity.",
            "accepts_person": "Whether the --as value parser accepts person/NAME. This is not a grant of person authority; runtime and daemon checks still apply.",
            "scope": "Canonical clap command paths, including hidden commands and callable parents; aliases resolve to their canonical path.",
        },
        "commands": commands,
        "aliases": inventory["aliases"],
    });
    let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&contract).expect("contract JSON")));
    json!({"contract": contract, "contract_sha256": digest})
}

pub(super) fn print(effective: bool) -> anyhow::Result<()> {
    let mut value = generate(super::Cli::command());
    if effective {
        let (config, _) = st3::config::Config::load_client_unvalidated(None)?;
        value["effective"] = json!({
            "instance": st3::instance::current().map(|instance| instance.name().to_owned()),
            "config_path": st3::config::Config::default_path(),
            "state_dir": config.state_dir,
        });
    }
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_contract_has_no_unclassified_commands() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/st3/actor-contract.json");
        let published: Value = serde_json::from_slice(&std::fs::read(path).expect("generate with st actor-contract --json")).unwrap();
        assert_eq!(generate(super::super::Cli::command()), published,
            "the CLI actor surface changed; review every changed command and regenerate docs/st3/actor-contract.json with st actor-contract --json");
    }

    #[test]
    fn actor_contract_new_command_invalidates_published_classification() {
        let original = generate(super::super::Cli::command());
        let changed = generate(super::super::Cli::command().subcommand(clap::Command::new("unclassified-probe")));
        assert_ne!(original, changed);
        assert!(changed["contract"]["commands"].get("st unclassified-probe").is_some());
    }

    #[test]
    fn actor_contract_matches_actual_as_acceptance() {
        fn check(command: &clap::Command, path: &str, contract: &Value) {
            if let Some(entry) = contract["contract"]["commands"].get(path) {
                // Keep the real argument's required bit and value parser. Remove unrelated
                // arguments so a missing positional or conflicting option cannot hide --as.
                let mut parser = clap::Command::new("probe");
                if let Some(argument) = actor_argument(command) {
                    parser = parser.arg(clap::Arg::new("actor").long("as")
                        .required(argument.is_required_set())
                        .value_parser(argument.get_value_parser().clone()));
                }
                let supplied = parser.clone().try_get_matches_from(["probe", "--as", "person/contract-probe"]);
                if entry["actor"] == "none" {
                    assert_eq!(supplied.unwrap_err().kind(), clap::error::ErrorKind::UnknownArgument, "{path}");
                } else {
                    assert_eq!(supplied.is_ok(), entry["accepts_person"].as_bool().unwrap(), "{path}");
                    let absent = parser.try_get_matches_from(["probe"]);
                    if entry["actor"] == "required" {
                        assert_eq!(absent.unwrap_err().kind(), clap::error::ErrorKind::MissingRequiredArgument, "{path}");
                    } else {
                        assert!(absent.is_ok(), "{path}");
                    }
                }
            }
            for child in command.get_subcommands().filter(|child| child.get_name() != "help") {
                check(child, &format!("{path} {}", child.get_name()), contract);
            }
        }
        let mut command = super::super::Cli::command();
        command.build();
        check(&command, "st", &generate(command.clone()));
    }
}
