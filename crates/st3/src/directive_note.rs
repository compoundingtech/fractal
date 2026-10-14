use st3::cli_println as println;

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::json;
use st3::client::{Client, Endpoint};
use st3::store::directive_notes::DirectiveNote;

#[derive(Args)]
#[command(after_help = "Without a subcommand, read person directive notes; set and clear remain person-only. Add, ls and archive manage subject notes. --for me is private; --for agent also delivers non-waking agent context. Notes never assign work, approve a gate or grant authority.")]
pub(super) struct NoteArgs {
    /// Act as this person or agent; defaults to the harness identity, else configured person.
    #[arg(long, global = true, value_parser = super::parse_actor_subject)]
    actor: Option<String>,
    #[command(subcommand)]
    command: Option<NoteCommand>,
}

#[derive(Subcommand)]
enum NoteCommand {
    /// Replace your current note (nonblank, at most 4096 UTF-8 bytes).
    Set {
        text: String,
        /// Optional UTC RFC3339 expiry; expired notes disappear from reads.
        #[arg(long, value_name = "RFC3339")]
        expires_at: Option<String>,
    },
    /// Clear your current note. Only persons may clear their own note.
    Clear,
    /// Attach short context to an agent, mission, mission run or step run.
    Add {
        target: String,
        text: String,
        /// Audience defaults to me for a person and agent for an agent.
        #[arg(long = "for", value_parser = ["me", "agent"])]
        audience: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        expires_at: Option<String>,
    },
    /// Read current subject notes on a target, or authored by you.
    Ls { target: Option<String> },
    /// Archive one of your subject notes.
    Archive { id: String },
}

#[derive(Deserialize, serde::Serialize)]
struct NotesResponse {
    notes: Vec<DirectiveNote>,
}

#[derive(Deserialize, serde::Serialize)]
struct NoteResponse {
    note: Option<DirectiveNote>,
}

pub(super) async fn run(
    endpoint: &Endpoint,
    configured_person: Option<&str>,
    args: NoteArgs,
    json_output: bool,
) -> Result<()> {
    let actor = super::configured_actor(args.actor.as_deref(), configured_person, "note")?;
    if let Some(own) = std::env::var("ST_AGENT").ok().filter(|own| !own.is_empty()) {
        anyhow::ensure!(actor == own, "an agent must read notes as its own harness identity");
        anyhow::ensure!(!matches!(args.command, Some(NoteCommand::Set { .. } | NoteCommand::Clear)),
            "only a person outside an agent seat can set or clear a directive note");
    }
    if matches!(args.command, Some(NoteCommand::Set { .. } | NoteCommand::Clear)) {
        super::parse_person_subject(&actor).map_err(anyhow::Error::msg)?;
    }
    let Endpoint::Unix(socket) = endpoint else {
        anyhow::bail!("st note requires the authenticated local Unix socket");
    };
    let client = if actor.starts_with("agent/") {
        Client::unix_agent(socket, &actor)?
    } else {
        Client::unix_as(socket, &actor)?
    };
    if matches!(args.command, Some(NoteCommand::Add { .. } | NoteCommand::Ls { .. } | NoteCommand::Archive { .. })) {
        return run_subject_note(&client, &actor, args.command.expect("subject command"), json_output).await;
    }
    match args.command {
        None => {
            let response: NotesResponse = client
                .get(&format!("/v1/client/notes?actor={}", urlencoding::encode(&actor)))
                .await?;
            if json_output {
                return super::print_value(&response, true);
            }
            if response.notes.is_empty() {
                println!("No current notes.");
            }
            for note in response.notes {
                println!("{} — {} at {}", note.person, note.author, note.time);
                if let Some(expiry) = note.expires_at {
                    println!("Expires: {expiry}");
                }
                println!("Revision: {}\n{}", note.revision, note.text);
            }
        }
        Some(command) => {
            let (text, expires_at) = match command {
                NoteCommand::Set { text, expires_at } => (Some(text), expires_at),
                NoteCommand::Clear => (None, None),
                NoteCommand::Add { .. } | NoteCommand::Ls { .. } | NoteCommand::Archive { .. } => unreachable!("subject command handled above"),
            };
            let response: NoteResponse = client
                .request("PUT", "/v1/notes", Some(&json!({
                    "person": actor,
                    "actor": actor,
                    "text": text,
                    "expires_at": expires_at,
                })))
                .await
                .context("could not update your directive note")?;
            if json_output {
                return super::print_value(&response, true);
            }
            println!("{}", if text.is_some() { "Note set." } else { "Note cleared." });
        }
    }
    Ok(())
}

fn subject_audience(actor: &str, requested: Option<&str>) -> &'static str {
    match requested {
        Some("me") => "self",
        Some("agent") => "agent",
        _ if actor.starts_with("agent/") => "agent",
        _ => "self",
    }
}

async fn run_subject_note(client: &Client, actor: &str, command: NoteCommand, json_output: bool) -> Result<()> {
    match command {
        NoteCommand::Add { target, text, audience, session, expires_at } => {
            let response: serde_json::Value = client.post("/v1/subject-notes", &json!({
                "target": target, "text": text,
                "audience": subject_audience(actor, audience.as_deref()),
                "session": session, "expires_at": expires_at,
            })).await.context("could not add subject note")?;
            if json_output { return super::print_value(&response, true); }
            println!("{}", response["id"].as_str().context("note response has no id")?);
        }
        NoteCommand::Ls { target } => {
            let query = target.map_or_else(
                || format!("author={}", urlencoding::encode(actor)),
                |target| format!("target={}", urlencoding::encode(&target)),
            );
            let response: serde_json::Value = client.get(&format!("/v1/client/subject-notes?{query}")).await?;
            if json_output { return super::print_value(&response, true); }
            let notes: Vec<st3::store::subject_notes::SubjectNote> = serde_json::from_value(response["notes"].clone())?;
            if notes.is_empty() { println!("No current subject notes."); }
            for note in notes {
                println!("{} — {} on {} at {}\n{}", note.id, note.author, note.target, note.updated_at, note.text);
            }
        }
        NoteCommand::Archive { id } => {
            let response: serde_json::Value = client.request("PUT",
                &format!("/v1/subject-notes/{}", urlencoding::encode(&id)), Some(&json!({"text": null}))).await?;
            if json_output { return super::print_value(&response, true); }
            println!("Note archived.");
        }
        NoteCommand::Set { .. } | NoteCommand::Clear => unreachable!("directive command handled separately"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;
    use super::*;

    #[test]
    fn note_parser_reads_and_accepts_actor_after_subcommand() {
        let cli = super::super::Cli::try_parse_from(["st", "note", "--actor", "agent/helper"]).unwrap();
        let super::super::Command::Note(args) = cli.command else { panic!("expected note") };
        assert_eq!(args.actor.as_deref(), Some("agent/helper"));
        assert!(args.command.is_none());
        let cli = super::super::Cli::try_parse_from(["st", "note", "clear", "--actor", "person/avery"]).unwrap();
        let super::super::Command::Note(args) = cli.command else { panic!("expected note") };
        assert!(matches!(args.command, Some(NoteCommand::Clear)));
    }

    #[test]
    fn note_parser_preserves_text_and_expiry_without_person_override() {
        let cli = super::super::Cli::try_parse_from([
            "st", "note", "set", "Review before merging", "--expires-at", "2099-01-01T00:00:00Z",
        ]).unwrap();
        let super::super::Command::Note(args) = cli.command else { panic!("expected note") };
        let Some(NoteCommand::Set { text, expires_at }) = args.command else { panic!("expected set") };
        assert_eq!(text, "Review before merging");
        assert_eq!(expires_at.as_deref(), Some("2099-01-01T00:00:00Z"));
        assert!(super::super::Cli::try_parse_from(["st", "note", "clear", "--person", "person/other"]).is_err());
        assert!(super::super::Cli::try_parse_from(["st", "note", "set"]).is_err());
    }

    #[test]
    fn subject_note_parser_preserves_directive_commands_and_scope() {
        let cli = super::super::Cli::try_parse_from([
            "st", "note", "add", "agent/example/worker", "short context", "--for", "agent", "--session", "current",
        ]).unwrap();
        let super::super::Command::Note(args) = cli.command else { panic!("expected note") };
        let Some(NoteCommand::Add { target, text, audience, session, .. }) = args.command else { panic!("expected add") };
        assert_eq!(target, "agent/example/worker");
        assert_eq!(text, "short context");
        assert_eq!(audience.as_deref(), Some("agent"));
        assert_eq!(session.as_deref(), Some("current"));
        for argv in [
            vec!["st", "note", "ls"],
            vec!["st", "note", "ls", "mission/example"],
            vec!["st", "note", "archive", "note/person/example/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9d"],
        ] {
            assert!(super::super::Cli::try_parse_from(argv).is_ok());
        }
        assert_eq!(subject_audience("person/example", None), "self");
        assert_eq!(subject_audience("agent/example/worker", None), "agent");
        assert_eq!(subject_audience("agent/example/worker", Some("me")), "self");
    }

    #[tokio::test]
    async fn subject_note_cli_roundtrip_over_authenticated_unix_socket() {
        use std::sync::Arc;
        use st3::{api::AppState, model::ClaimInput, store::Store};
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("example").unwrap());
        let fleet = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let key = Arc::new(st3::fleet::MemberKey::generate().unwrap().0);
        store.bind_fleet(fleet).unwrap();
        store.pin_fleet_anchor(key.public()).unwrap();
        store.set_member_key(Some(key.clone())).unwrap();
        store.admit_fleet_anchor(fleet, key.public(), "listening").unwrap();
        for (subject, kind, fields) in [
            ("daemon/example", "daemon.started", json!({"status":"running","features":{"subject_notes":1}})),
            ("agent/example/worker", "harness.observed", json!({"state":"ready"})),
        ] {
            store.append_claim(&ClaimInput { subject: subject.into(), kind: kind.into(), actor: None,
                fields: serde_json::from_value(fields).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None }).unwrap();
        }
        let state = AppState {
            store: store.clone(), notify: Arc::new(tokio::sync::Notify::new()),
            event_notify: tokio::sync::watch::channel(0_u64).0, node: "example".into(),
            state_dir: root.path().into(), pty_root: root.path().join("pty"), pty_binary: "pty".into(),
            fleet_id: Some(fleet.into()), configured_peers: vec![], client_relay: None,
            native_session_home: None, planner_default: st3::model::PlannerSpec::default(),
        };
        let socket = root.path().join("st.sock");
        let server_socket = socket.clone();
        let (ready, listening) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            st3::api::serve_unix_with_ready(&server_socket, st3::api::router(state), || { let _ = ready.send(()); }).await.unwrap();
        });
        listening.await.unwrap();
        let client = Client::unix_as(&socket, "person/example").unwrap();
        run_subject_note(&client, "person/example", NoteCommand::Add {
            target: "agent/example/worker".into(), text: "Remember the context".into(), audience: Some("agent".into()),
            session: None, expires_at: None,
        }, true).await.unwrap();
        let notes = store.list_subject_notes(None, Some("person/example"), "person/example").unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text, "Remember the context");
        assert!(store.messages(Some("agent/example/worker"), false).unwrap().iter()
            .any(|message| message.tags.iter().any(|tag| tag == st3::notes_digest::TAG)));
        run_subject_note(&client, "person/example", NoteCommand::Ls { target: None }, true).await.unwrap();
        run_subject_note(&client, "person/example", NoteCommand::Archive { id: notes[0].id.clone() }, true).await.unwrap();
        assert!(store.list_subject_notes(None, Some("person/example"), "person/example").unwrap().is_empty());
        server.abort();
    }
}
