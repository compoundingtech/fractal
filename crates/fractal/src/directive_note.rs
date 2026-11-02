use fractal_daemon::cli_println as println;

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::json;
use fractal_daemon::client::{Client, Endpoint};
use fractal_daemon::store::directive_notes::DirectiveNote;

#[derive(Args)]
#[command(after_help = "Add, ls and archive manage subject notes. Bare note and note set|clear are deprecated aliases for directive show|set|clear. Every subject note reaches its target agent as non-waking context. Notes never assign work, approve a gate or grant authority.")]
pub(super) struct NoteArgs {
    /// Act as this person or agent; defaults to the harness identity, else configured person.
    #[arg(long, global = true, value_parser = super::parse_actor_subject)]
    actor: Option<String>,
    #[command(subcommand)]
    command: Option<NoteCommand>,
}

#[derive(Args)]
pub(super) struct DirectiveArgs {
    /// Act as this person or agent; defaults to the harness identity, else configured person.
    #[arg(long, global = true, value_parser = super::parse_actor_subject)]
    actor: Option<String>,
    #[command(subcommand)]
    command: DirectiveCommand,
}

#[derive(Subcommand)]
enum DirectiveCommand {
    /// Replace your directive note (nonblank, at most 4096 UTF-8 bytes).
    Set {
        text: String,
        /// Optional UTC RFC3339 expiry; expired notes disappear from reads.
        #[arg(long, value_name = "RFC3339")]
        expires_at: Option<String>,
    },
    /// Clear your directive note. Only persons may clear their own note.
    Clear,
    /// Read current person directive notes.
    Show,
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
    /// Attach short context to an agent, session, mission, mission run or step run.
    Add {
        text: String,
        #[arg(long = "on", value_name = "TARGET")]
        target: String,
        /// Target this agent's current running incarnation instead of the agent.
        #[arg(long, value_parser = ["current"])]
        session: Option<String>,
        #[arg(long)]
        expires_at: Option<String>,
    },
    /// Read current subject notes on a target, or authored by you.
    Ls {
        #[arg(long = "on", value_name = "TARGET")]
        target: Option<String>,
    },
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

pub(super) async fn run_directive(
    endpoint: &Endpoint,
    configured_person: Option<&str>,
    args: DirectiveArgs,
    json_output: bool,
) -> Result<()> {
    let command = match args.command {
        DirectiveCommand::Set { text, expires_at } => Some(NoteCommand::Set { text, expires_at }),
        DirectiveCommand::Clear => Some(NoteCommand::Clear),
        DirectiveCommand::Show => None,
    };
    run_note(endpoint, configured_person, NoteArgs { actor: args.actor, command }, json_output).await
}

fn deprecated_directive(command: Option<&NoteCommand>) -> Option<&'static str> {
    match command {
        None => Some("show"),
        Some(NoteCommand::Set { .. }) => Some("set"),
        Some(NoteCommand::Clear) => Some("clear"),
        Some(NoteCommand::Add { .. } | NoteCommand::Ls { .. } | NoteCommand::Archive { .. }) => None,
    }
}

pub(super) async fn run(
    endpoint: &Endpoint,
    configured_person: Option<&str>,
    args: NoteArgs,
    json_output: bool,
) -> Result<()> {
    if let Some(command) = deprecated_directive(args.command.as_ref()) {
        eprintln!("Deprecated: use `fractal directive {command}` instead.");
    }
    run_note(endpoint, configured_person, args, json_output).await
}

async fn run_note(
    endpoint: &Endpoint,
    configured_person: Option<&str>,
    args: NoteArgs,
    json_output: bool,
) -> Result<()> {
    let actor = super::configured_actor(args.actor.as_deref(), configured_person, "note")?;
    if let Some(own) = fractal_drivers::contracts::env("ST_AGENT").filter(|own| !own.is_empty()) {
        anyhow::ensure!(actor == own, "an agent must read notes as its own harness identity");
        anyhow::ensure!(!matches!(args.command, Some(NoteCommand::Set { .. } | NoteCommand::Clear)),
            "only a person outside an agent seat can set or clear a directive note");
    }
    if matches!(args.command, Some(NoteCommand::Set { .. } | NoteCommand::Clear)) {
        super::parse_person_subject(&actor).map_err(anyhow::Error::msg)?;
    }
    let Endpoint::Unix(socket) = endpoint else {
        anyhow::bail!("fractal note requires the authenticated local Unix socket");
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


async fn run_subject_note(client: &Client, actor: &str, command: NoteCommand, json_output: bool) -> Result<()> {
    match command {
        NoteCommand::Add { target, text, session, expires_at } => {
            let target = if session.is_some() {
                anyhow::ensure!(target.starts_with("agent/"), "--session current requires an agent target");
                let status = super::status_for(client, &target).await?;
                let actual = status.subjects.first().and_then(|subject| subject.actual.as_ref());
                current_session_target(&target, actual)?
            } else {
                target
            };
            let response: serde_json::Value = client.post("/v1/subject-notes", &json!({
                "target": target, "text": text,
                "expires_at": expires_at,
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
            let notes: Vec<fractal_store::store::subject_notes::SubjectNote> = serde_json::from_value(response["notes"].clone())?;
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

fn current_session_target(target: &str, actual: Option<&serde_json::Value>) -> Result<String> {
    anyhow::ensure!(target.starts_with("agent/"), "--session current requires an agent target");
    let fields = actual.map(|actual| actual.get("fields").unwrap_or(actual));
    let incarnation = fields
        .filter(|fields| fields["status"].as_str() == Some("running"))
        .and_then(|fields| fields["incarnation_id"].as_str())
        .filter(|incarnation| !incarnation.trim().is_empty())
        .with_context(|| format!("`{target}` has no current running incarnation"))?;
    fractal_schema::subject_notes::session_subject(target, incarnation).map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;
    use super::*;

    #[test]
    fn directive_parser_exposes_explicit_commands_and_global_actor() {
        let cli = super::super::Cli::try_parse_from([
            "fractal", "directive", "set", "Review this first",
            "--expires-at", "2099-01-01T00:00:00Z", "--actor", "person/example",
        ]).unwrap();
        let super::super::Command::Directive(args) = cli.command else { panic!("expected directive") };
        assert_eq!(args.actor.as_deref(), Some("person/example"));
        let DirectiveCommand::Set { text, expires_at } = args.command else { panic!("expected set") };
        assert_eq!(text, "Review this first");
        assert_eq!(expires_at.as_deref(), Some("2099-01-01T00:00:00Z"));
        for command in ["clear", "show"] {
            assert!(super::super::Cli::try_parse_from(["st3", "directive", command]).is_ok());
        }
        assert!(super::super::Cli::try_parse_from(["st", "directive"]).is_err());
        assert!(super::super::Cli::try_parse_from(["st", "directive", "set"]).is_err());
    }

    #[test]
    fn only_legacy_directive_aliases_request_a_deprecation_notice() {
        assert_eq!(deprecated_directive(None), Some("show"));
        assert_eq!(deprecated_directive(Some(&NoteCommand::Set {
            text: "Context".into(), expires_at: None,
        })), Some("set"));
        assert_eq!(deprecated_directive(Some(&NoteCommand::Clear)), Some("clear"));
        for command in [
            NoteCommand::Add {
                target: "agent/example/worker".into(), text: "Context".into(),
                session: None, expires_at: None,
            },
            NoteCommand::Ls { target: None },
            NoteCommand::Archive { id: "note/person/example/synthetic".into() },
        ] {
            assert_eq!(deprecated_directive(Some(&command)), None);
        }
    }

    #[test]
    fn current_session_requires_running_authoritative_incarnation() {
        let target = "agent/example/worker";
        let incarnation = "1234:2026-10-14T15:57:40.016Z:bound:synthetic/%";
        let expected = fractal_schema::subject_notes::session_subject(target, incarnation).unwrap();
        for actual in [
            json!({"status": "running", "incarnation_id": incarnation}),
            json!({"fields": {"status": "running", "incarnation_id": incarnation}}),
        ] {
            assert_eq!(current_session_target(target, Some(&actual)).unwrap(), expected);
        }
        assert!(current_session_target(target, None).is_err());
        for actual in [
            json!({"status": "running"}),
            json!({"status": "running", "incarnation_id": ""}),
            json!({"status": "running", "incarnation_id": " "}),
            json!({"incarnation_id": incarnation}),
            json!({"status": "ready", "session_id": "provider-native"}),
        ] {
            assert!(current_session_target(target, Some(&actual)).is_err());
        }
        for status in ["absent", "stopped", "exited", "vanished", "starting"] {
            assert!(current_session_target(target, Some(&json!({
                "status": status, "incarnation_id": incarnation,
            }))).is_err());
        }
        for target in ["mission/example", expected.as_str()] {
            assert!(current_session_target(target, Some(&json!({
                "status": "running", "incarnation_id": incarnation,
            }))).is_err());
        }
    }

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
            "st", "note", "add", "short context", "--on", "agent/example/worker", "--session", "current",
        ]).unwrap();
        let super::super::Command::Note(args) = cli.command else { panic!("expected note") };
        let Some(NoteCommand::Add { target, text, session, .. }) = args.command else { panic!("expected add") };
        assert_eq!(target, "agent/example/worker");
        assert_eq!(text, "short context");
        assert_eq!(session.as_deref(), Some("current"));
        for argv in [
            vec!["st", "note", "ls"],
            vec!["st", "note", "ls", "--on", "mission/example"],
            vec!["st", "note", "archive", "note/person/example/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9d"],
        ] {
            assert!(super::super::Cli::try_parse_from(argv).is_ok());
        }
        assert!(super::super::Cli::try_parse_from([
            "st", "note", "add", "context", "--on", "agent/example/worker", "--for", "agent",
        ]).is_err());
        let session_target = fractal_schema::subject_notes::session_subject(
            "agent/example/worker", "synthetic:bound:incarnation",
        ).unwrap();
        let cli = super::super::Cli::try_parse_from([
            "st", "note", "add", "Session context", "--on", &session_target,
        ]).unwrap();
        let super::super::Command::Note(args) = cli.command else { panic!("expected note") };
        let Some(NoteCommand::Add { target, session, .. }) = args.command else { panic!("expected add") };
        assert_eq!(target, session_target);
        assert!(session.is_none());
        for argv in [
            vec!["st", "note", "add", "agent/example/worker", "context"],
            vec!["st", "note", "add", "context"],
            vec!["st", "note", "ls", "mission/example"],
            vec!["st", "note", "add", "context", "--on", "agent/example/worker", "--session", "provider-id"],
        ] {
            assert!(super::super::Cli::try_parse_from(argv).is_err());
        }
    }

    #[tokio::test]
    async fn subject_note_cli_roundtrip_over_authenticated_unix_socket() {
        use std::sync::Arc;
        use fractal_daemon::{SubjectNotesStore, api::AppState, model::ClaimInput, store::Store};
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("example").unwrap());
        let fleet = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let key = Arc::new(fractal_daemon::fleet::MemberKey::generate().unwrap().0);
        store.bind_fleet(fleet).unwrap();
        store.pin_fleet_anchor(key.public()).unwrap();
        store.set_member_key(Some(key.clone())).unwrap();
        store.admit_fleet_anchor(fleet, key.public(), "listening").unwrap();
        for (subject, kind, fields) in [
            ("daemon/example", "daemon.started", json!({"status":"running","features":{"subject_notes":1}})),
            ("agent/example/worker", "harness.observed", json!({"state":"ready"})),
            ("agent/example/worker", "runtime.observed", json!({
                "status": "running", "incarnation_id": "1234:synthetic:bound:invocation",
            })),
        ] {
            store.append_claim(&ClaimInput { subject: subject.into(), kind: kind.into(), actor: None,
                fields: serde_json::from_value(fields).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None }).unwrap();
        }
        let state = AppState {
            seat_admission: Default::default(), owning_person: None,
            harness_question_sink: None, assistant_receipts: Default::default(), private_notes: Default::default(),
            store: store.clone(), notify: Arc::new(tokio::sync::Notify::new()),
            event_notify: tokio::sync::watch::channel(0_u64).0, node: "example".into(),
            state_dir: root.path().into(), pty_root: root.path().join("pty"), pty_binary: "pty".into(),
            fleet_id: Some(fleet.into()), configured_peers: vec![], client_relay: None,
            native_session_home: None, planner_default: fractal_daemon::model::PlannerSpec::default(),
        };
        let socket = root.path().join("st.sock");
        let server_socket = socket.clone();
        let (ready, listening) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            fractal_daemon::api::serve_unix_with_ready(&server_socket, fractal_daemon::api::router(state), || { let _ = ready.send(()); }).await.unwrap();
        });
        listening.await.unwrap();
        let client = Client::unix_as(&socket, "person/example").unwrap();
        run_subject_note(&client, "person/example", NoteCommand::Add {
            target: "agent/example/worker".into(), text: "Remember the context".into(),
            session: None, expires_at: None,
        }, true).await.unwrap();
        let notes = store.list_subject_notes(None, Some("person/example"), "person/example").unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text, "Remember the context");
        assert_eq!(notes[0].audience, "agent");
        assert!(store.messages(Some("agent/example/worker"), false).unwrap().iter()
            .any(|message| message.tags.iter().any(|tag| tag == fractal_daemon::notes_digest::TAG)));
        run_subject_note(&client, "person/example", NoteCommand::Ls { target: None }, true).await.unwrap();
        run_subject_note(&client, "person/example", NoteCommand::Archive { id: notes[0].id.clone() }, true).await.unwrap();
        assert!(store.list_subject_notes(None, Some("person/example"), "person/example").unwrap().is_empty());
        let session_target = fractal_schema::subject_notes::session_subject(
            "agent/example/worker", "1234:synthetic:bound:invocation",
        ).unwrap();
        for (target, session) in [
            ("agent/example/worker".to_owned(), Some("current".to_owned())),
            (session_target.clone(), None),
        ] {
            run_subject_note(&client, "person/example", NoteCommand::Add {
                target, text: "Session context".into(), session, expires_at: None,
            }, true).await.unwrap();
        }
        let notes = store.list_subject_notes(Some(&session_target), None, "person/example").unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|note| note.target == session_target));
        run_subject_note(&client, "person/example", NoteCommand::Ls {
            target: Some(session_target),
        }, true).await.unwrap();
        for note in notes {
            run_subject_note(&client, "person/example", NoteCommand::Archive {
                id: note.id,
            }, true).await.unwrap();
        }
        store.append_claim(&ClaimInput {
            subject: "agent/example/worker".into(), kind: "runtime.observed".into(), actor: None,
            fields: serde_json::from_value(json!({
                "status": "stopped", "incarnation_id": "1234:synthetic:bound:invocation",
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        for target in ["agent/example/worker", "mission/example"] {
            let error = run_subject_note(&client, "person/example", NoteCommand::Add {
                target: target.into(), text: "Must not fall back".into(),
                session: Some("current".into()), expires_at: None,
            }, true).await.unwrap_err();
            assert!(error.to_string().contains(if target.starts_with("agent/") {
                "no current running incarnation"
            } else {
                "requires an agent target"
            }));
        }
        assert!(store.list_subject_notes(None, Some("person/example"), "person/example").unwrap().is_empty());
        server.abort();
    }
}
