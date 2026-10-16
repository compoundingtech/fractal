//! Local mode status does not connect to a daemon or expose a device credential.
use anyhow::Result;
use clap::{Args as ClapArgs, Subcommand};
use serde::Serialize;
use st3_client::client_mode::{State, Store};

#[derive(ClapArgs)]
pub(crate) struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read mode state without starting or contacting a daemon.
    Status,
}

#[derive(Serialize)]
struct Status {
    version: u32,
    outcome: &'static str,
    authority_path: std::path::PathBuf,
    authority_enrolled: bool,
    local_start_inhibited: bool,
    state: State,
}

pub(crate) fn run(args: &Args, json: bool) -> Result<()> {
    match args.command {
        Command::Status => {
            let store = Store::configured()?;
            let state = store.read()?;
            let status = Status {
                version: 1,
                outcome: "read-succeeded",
                authority_path: store.path().to_owned(),
                authority_enrolled: store.enrolled()?,
                local_start_inhibited: state.require_local().is_err(),
                state,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                let mode = match &status.state {
                    State::Local { .. } => "local",
                    State::Client { .. } => "client",
                    State::Transition { .. } => "transition (recovery required)",
                };
                println!("Mode: {mode}");
                println!("Authority enrolled: {}", status.authority_enrolled);
                println!("Generation: {}", status.state.generation());
                println!("Local start inhibited: {}", status.local_start_inhibited);
            }
            Ok(())
        }
    }
}
