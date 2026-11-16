//! Seed the isolated preview daemon's store with durable thread history.
//!
//! Ignored test: run only for the scratch preview daemon, with the daemon STOPPED:
//!
//!   THREAD_PREVIEW_ROOT=~/tmp/threads-delivery/tui-preview/scratch-root \
//!     cargo test -p fractal-tui --locked --test thread_preview_seed -- --ignored --nocapture
//!
//! It writes through the production store paths (thread_metadata, thread_delivery),
//! the same functions the daemon's API layer calls, so the running daemon later
//! publishes these threads through its ordinary client-v0 windows.

use std::sync::Arc;

use fractal_daemon::store::{self, Store};
use fractal_daemon::SeatsStoreCore as _;

fn input(kind: &str, actor: &str, key: &str, mut fields: serde_json::Value) -> smallclaims::ClaimInput {
    fields["status"] = serde_json::json!("sent");
    serde_json::from_value(serde_json::json!({
        "subject": format!("message/preview/{key}"),
        "kind": kind,
        "actor": actor,
        "idempotency_key": key,
        "fields": fields,
    })).unwrap()
}

#[test]
#[ignore = "seeds the isolated preview scratch store; see the module docs"]
fn seed_scratch_threads() {
    let root = std::env::var("THREAD_PREVIEW_ROOT").expect("THREAD_PREVIEW_ROOT names the scratch root");
    let store = Arc::new(Store::open(&std::path::PathBuf::from(&root).join("store/claims.sqlite3"), "previewnode").unwrap());
    // The scratch config enables thread writes at runtime; mirror that here.
    store.configure_thread_write_rollout([]);
    let source = std::fs::read_to_string(std::path::PathBuf::from(&root).join("seed.kdl")).unwrap();
    let intent = fractal_daemon::graph::parse_intent(&source, store.origin()).unwrap();
    store.apply_internal(&intent, "preview-declare-agents").unwrap();

    let person = "person/fixture";
    let botanist = "agent/glasshouse/botanist";
    let surveyor = "agent/glasshouse/surveyor";

    // --- Thread 1: open, renamed once, several replies with a quote chain.
    let open_thread = "thread/preview-humidity-calibration";
    store::thread_metadata::apply(
        &store,
        person,
        open_thread,
        "preview-humidity-calibration",
        store::thread_metadata::Edit::Create { agent: botanist, title: "Humidity calibration" },
    )
    .unwrap();
    let observed = store::thread_metadata::summary(&store, person, open_thread).unwrap();
    let rename_claim = if observed.title == "Humidity calibration before the frost" {
        observed.title_revision
    } else {
        store::thread_metadata::apply(
            &store,
            person,
            open_thread,
            "preview-humidity-calibration-rename",
            store::thread_metadata::Edit::Rename {
                title: "Humidity calibration before the frost",
                title_revision: &observed.title_revision,
                revision: &observed.revision,
            },
        ).unwrap().0.id
    };
    let first = store::thread_delivery::append_reply(
        &store,
        &input("message.sent", person, "seed/calib-1", serde_json::json!({
            "thread_id": open_thread, "from": person, "to": botanist,
            "content": "The greenhouse humidity sensors drift after the door fix. Can you calibrate before the first frost?"
        })),
        store::thread_metadata::PersonAuthority::TrustedLocal,
        None,
    )
    .unwrap();
    let first_id = first.0.subject.clone();
    store::thread_delivery::append_reply(
        &store,
        &input("message.sent", botanist, "seed/calib-2", serde_json::json!({
            "thread_id": open_thread, "from": botanist, "to": person,
            "thread_quote_id": first_id,
            "content": "Drift confirmed: bay 3 reads 8% low. I will recalibrate all six sensors this afternoon."
        })),
        store::thread_metadata::PersonAuthority::TrustedLocal,
        None,
    )
    .unwrap();

    // --- Thread 2: closed after a decision; stays in Recently closed with history.
    let closed_thread = "thread/preview-frost-cover";
    store::thread_metadata::apply(
        &store,
        person,
        closed_thread,
        "preview-frost-cover",
        store::thread_metadata::Edit::Create { agent: surveyor, title: "Frost cover survey" },
    )
    .unwrap();
    store::thread_delivery::append_reply(
        &store,
        &input("message.sent", surveyor, "seed/frost-1", serde_json::json!({
            "thread_id": closed_thread, "from": surveyor, "to": person,
            "content": "The north beds need cover fabric before Thursday night. Should I order the 12m roll?"
        })),
        store::thread_metadata::PersonAuthority::TrustedLocal,
        None,
    )
    .unwrap();
    let observed = store::thread_metadata::summary(&store, person, closed_thread).unwrap();
    let close = store::thread_metadata::apply(
        &store,
        person,
        closed_thread,
        "preview-frost-cover-close",
        store::thread_metadata::Edit::Close { revision: &observed.revision },
    )
    .unwrap();

    // --- Thread 3: open on the same agent as thread 1, so the agent pane lists
    // several groups with independent disclosure.
    let sibling_thread = "thread/preview-bay3-sensor";
    store::thread_metadata::apply(
        &store,
        person,
        sibling_thread,
        "preview-bay3-sensor",
        store::thread_metadata::Edit::Create { agent: botanist, title: "Replace the bay 3 sensor" },
    )
    .unwrap();
    store::thread_delivery::append_reply(
        &store,
        &input("message.sent", person, "seed/bay3-1", serde_json::json!({
            "thread_id": sibling_thread, "from": person, "to": botanist,
            "content": "Bay 3 drifts the most; replace that sensor first."
        })),
        store::thread_metadata::PersonAuthority::TrustedLocal,
        None,
    )
    .unwrap();
    let observed = store::thread_metadata::summary(&store, person, open_thread).unwrap();
    store::thread_metadata::apply(
        &store, person, open_thread, "preview-calibration-close",
        store::thread_metadata::Edit::Close { revision: &observed.revision },
    ).unwrap();
    let observed = store::thread_metadata::summary(&store, person, open_thread).unwrap();
    store::thread_metadata::apply(
        &store, person, open_thread, "preview-calibration-reopen",
        store::thread_metadata::Edit::Reopen { revision: &observed.revision },
    ).unwrap();
    // More than four expanded IDs exercise the shared live-window admission limit.
    for n in 1..=4 {
        let id = format!("thread/preview-extra-{n}");
        let title = format!("Sensor follow-up {n}");
        store::thread_metadata::apply(
            &store, person, &id, &format!("preview-extra-{n}"),
            store::thread_metadata::Edit::Create { agent: botanist, title: &title },
        ).unwrap();
    }

    println!(
        "SEEDED open={open_thread} (rename claim {rename_claim}) closed={closed_thread} (close claim {}) sibling={sibling_thread}",
        close.0.id
    );
}

/// Agent-authored Inbox sources for the person's final inline-thread trial.
/// Run only with the existing scratch daemon stopped, after seed_scratch_threads.
#[test]
#[ignore = "seeds agent asks in the isolated preview scratch store"]
fn seed_scratch_trial_asks() {
    use fractal_daemon::PersonWorkStore as _;
    let root = std::path::PathBuf::from(
        std::env::var("THREAD_PREVIEW_ROOT").expect("THREAD_PREVIEW_ROOT names the scratch root"),
    );
    assert!(
        std::os::unix::net::UnixStream::connect(root.join("run/st3.sock")).is_err(),
        "stop the scratch daemon before seeding its store",
    );
    let store = Store::open(&root.join("store/claims.sqlite3"), "previewnode").unwrap();
    for (agent, title, key) in [
        ("agent/glasshouse/botanist", "Preview trial: humidity calibration decision", "preview-trial-ask-calibration-v1"),
        ("agent/glasshouse/surveyor", "Preview trial: frost cover decision", "preview-trial-ask-frost-v1"),
    ] {
        let input = fractal_daemon::model::PersonAskRequest {
            legacy_request: None,
            person: "person/fixture".into(),
            title: title.into(),
            reason: "Scratch-only TUI trial. Start a thread to discuss this decision; leave the ask unanswered while testing rename, close and reopen.".into(),
            actor: agent.into(),
            step: None,
            new_run: None,
            incarnation: None,
            request: None,
            deadline_ms: None,
            default_answer: None,
            priority: None,
            idempotency_key: key.into(),
        };
        let ask = store.ask_person(&input).unwrap();
        assert_eq!(store.ask_person(&input).unwrap().subject, ask.subject, "reseed is idempotent");
        assert!(store.attention_items(Some("person/fixture")).unwrap().iter().any(|item| item.subject == ask.subject));
        println!("TRIAL ASK subject={} requester={agent} idempotency_key={key} title={title}", ask.subject);
    }
}

/// Synthetic scratch authority only: the sole scope is read.projections.
/// Stop the scratch daemon before running; never use a production state root.
#[tokio::test]
#[ignore = "issues a read-only synthetic scratch pairing; never prints the credential"]
async fn seed_scratch_read_only_pairing() {
    use axum::{Extension, Json, extract::{Path, State}};
    use fractal_daemon::api::{AppState, client_v0};
    let root = std::path::PathBuf::from(
        std::env::var("THREAD_PREVIEW_ROOT").expect("THREAD_PREVIEW_ROOT"),
    );
    assert!(root.ends_with("tui-preview/scratch-root"), "scratch preview root only");
    let state_dir = root.join("state");
    let store = Arc::new(Store::open(&root.join("store/claims.sqlite3"), "previewnode").unwrap());
    store.set_node_key(Arc::new(
        fractal_daemon::fleet::join::standalone_node_key(&state_dir).unwrap(),
    )).unwrap();
    store.configure_thread_write_rollout([]);
    let state = AppState {
        browser_pairing_login: None,
        seat_admission: Default::default(),
        owning_person: Some("person/fixture".into()),
        store,
        notify: Arc::new(tokio::sync::Notify::new()),
        event_notify: tokio::sync::watch::channel(0).0,
        node: "previewnode".into(),
        api_socket: root.join("run/st3.sock"),
        state_dir,
        pty_root: root.join("pty"),
        pty_binary: root.join("unused-pty"),
        fleet_id: None,
        configured_peers: vec![],
        client_relay: None,
        native_session_home: None,
        harness_question_sink: None,
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
        planner_default: Default::default(),
    };
    let local = client_v0::ClientSession::local(Some("person/fixture")).unwrap();
    let Json(challenge) = client_v0::pairing_begin(
        State(state.clone()), Extension(local.clone()),
        Json(serde_json::from_value(serde_json::json!({
            "api_version": fractal_client::API_VERSION,
            "person_id": "person/fixture",
            "device_name": "Synthetic read-only threads preview",
            "scopes": ["read.projections"],
        })).unwrap()),
    ).await.unwrap();
    let id = challenge["pairing_id"].as_str().unwrap().trim_start_matches("pairing/");
    let public = smallclaims::fleet::MemberKey::generate().unwrap().0.public().to_string();
    let Json(paired) = client_v0::pairing_complete(
        State(state), Extension(local), Path(id.into()),
        Json(serde_json::from_value(serde_json::json!({
            "api_version": fractal_client::API_VERSION,
            "code": challenge["code"], "device_public_key": public,
        })).unwrap()),
    ).await.unwrap();
    assert_eq!(paired["scopes"], serde_json::json!(["read.projections"]));
    let session: fractal_client::PersonPairedSession = serde_json::from_value(paired).unwrap();
    let profile = fractal_client::device::Profile {
        devices: vec![fractal_client::device::Device {
            endpoint: "http://127.0.0.1:17410".into(),
            allow_public_http: false,
            person_root_fingerprint: None,
            signing_key: None,
            session,
        }],
    };
    let path = root.join("paired-config/st3/stui-devices.json");
    profile.save(&path).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    println!("Read-only synthetic preview pairing saved privately; scope=read.projections");
}
