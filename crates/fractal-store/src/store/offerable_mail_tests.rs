//! A delivery stream's first read lists only the mail it can offer. A long-lived seat's mailbox
//! is mostly read mail nobody closed; building each of those views reads every claim body of the
//! message, its content included, only for the stream to drop it.
use super::*;

const READER: &str = "agent/example/reader";
const READ_MAIL: usize = 300;

fn claim(store: &Store, subject: &str, kind: &str, actor: &str, fields: Value) {
    let Value::Object(fields) = fields else { unreachable!() };
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: Some(actor.into()),
            fields: fields.into_iter().collect(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn send(store: &Store, subject: &str) {
    claim(store, subject, "message.sent", "person/example", json!({
        "status": "sent", "from": "person/example", "to": READER,
        // A real message's content is most of its bytes.
        "content": "invented words ".repeat(64),
    }));
}

/// Move a sent message through `statuses`, as its reader does.
fn advance(store: &Store, subject: &str, statuses: &[&str]) {
    for status in statuses {
        claim(store, subject, &format!("message.{status}"), READER, json!({"status": status}));
    }
}

/// A mailbox of `READ_MAIL` read messages, one closed, and three the stream can offer.
fn mailbox() -> Store {
    let store = Store::open_memory("node").unwrap();
    for index in 0..READ_MAIL {
        let subject = format!("message/read-{index}");
        send(&store, &subject);
        advance(&store, &subject, &["delivered", "read"]);
    }
    send(&store, "message/closed");
    advance(&store, "message/closed", &["delivered", "read", "closed"]);
    for subject in ["message/new-1", "message/new-2"] {
        send(&store, subject);
    }
    send(&store, "message/delivered");
    advance(&store, "message/delivered", &["delivered"]);
    // A local mark ranks above the claims' status.
    send(&store, "message/marked");
    message_marks::record(&store, "message/marked", "message.read", READER).unwrap();
    store
}

fn decoded(read: impl FnOnce() -> Vec<MessageView>) -> (Vec<MessageView>, usize) {
    ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.set(0));
    let messages = read();
    (messages, ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.get()))
}

#[test]
fn a_first_delivery_read_builds_views_only_for_offerable_mail() {
    let store = mailbox();
    let through = store.index().unwrap();
    // Cold: no view of this mailbox is cached yet.
    let (offerable, offerable_bodies) =
        decoded(|| store.offerable_messages_for_delivery_through(READER, through).unwrap());
    let (all, all_bodies) = decoded(|| store.messages_through_inner(Some(READER), false, through, true).unwrap());

    // Exactly the views the full listing gives, less what a stream drops by status.
    let expected = all
        .into_iter()
        .filter(|message| matches!(message.status.as_str(), "sent" | "staged" | "delivered"))
        .collect::<Vec<_>>();
    assert_eq!(serde_json::to_value(&offerable).unwrap(), serde_json::to_value(&expected).unwrap());
    let subjects = offerable.iter().map(|message| message.subject.as_str()).collect::<Vec<_>>();
    assert_eq!(subjects, ["message/new-1", "message/new-2", "message/delivered"]);

    // A read message costs at most its newest claim, recorded as a claim or as a local mark,
    // against every claim of it, its send included, in the full listing.
    assert!(all_bodies >= READ_MAIL, "the full listing decodes {all_bodies}");
    assert!(
        offerable_bodies <= READ_MAIL + 20 && 2 * offerable_bodies < all_bodies,
        "{offerable_bodies} bodies decoded for {READ_MAIL} read messages, against {all_bodies}"
    );
}
