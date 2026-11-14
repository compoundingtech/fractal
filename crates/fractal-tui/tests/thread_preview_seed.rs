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

fn input(kind: &str, actor: &str, key: &str, fields: serde_json::Value) -> smallclaims::ClaimInput {
    serde_json::from_value(serde_json::json!({
        "subject": format!("preview/{key}"),
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

    let person = "person/avery";
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
    let base = store::thread_metadata::apply(
        &store,
        person,
        open_thread,
        "preview-humidity-calibration-rename",
        store::thread_metadata::Edit::Rename {
            title: "Humidity calibration before the frost",
            title_revision: &observed.title_revision,
            revision: &observed.revision,
        },
    )
    .unwrap();
    let rename_claim = base.0.id.clone();
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
    let first_id = first.0.id.clone();
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
