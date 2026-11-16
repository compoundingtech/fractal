//! One local daemon stream per delivery component. Ownership and receipts are fenced in SQLite,
//! so an old channel reconnecting after a replacement cannot consume or acknowledge its mail.
use crate::SeatsStore;
use crate::store::messages_shard::mailbox_reads;
use super::*;

pub const RECOVERY_CUTOFF_MS: u128 = 7 * 24 * 60 * 60 * 1_000;
pub const RECOVERY_BACKLOG_LIMIT: usize = 5;

/// None holds mail outside the recovery window; true consumes a backlog slot.
pub fn recovery_backlog(accepted_at: u128, since: u128) -> Option<bool> {
    (accepted_at >= since.saturating_sub(RECOVERY_CUTOFF_MS)).then_some(
        accepted_at < since.saturating_sub(u128::from(super::mail_backlog::THRESHOLD_MS)),
    )
}

/// Recover only mail which has never been offered. A prior staging claim is
/// already an offer attempt, even when its delivered receipt has not arrived yet.
pub fn never_offered(store: &Store, message: &crate::model::MessageView) -> anyhow::Result<bool> {
    // A stream item's offers are this member's own lifecycle records, not graph claims.
    if mailbox_reads::is_stream_item(message) {
        return Ok(message.status == "sent"
            && mailbox_reads::lifecycle_step(store, &message.subject, "message.staged")?.is_none()
            && mailbox_reads::lifecycle_step(store, &message.subject, "message.delivered")?.is_none());
    }
    Ok(message.status == "sent"
        && store
            .latest_claim(&message.subject, Some("message.staged"))?
            .is_none()
        && store
            .latest_claim(&message.subject, Some("message.delivered"))?
            .is_none())
}

/// A batch settles when every member is delivered/read or terminally withdrawn (closed/expired).
/// Read outside recovery locks; withdrawal never manufactures a delivery receipt.
pub fn recovery_batch_settled(
    store: &Store,
    batch: &std::collections::BTreeSet<String>,
) -> anyhow::Result<bool> {
    if batch.is_empty() {
        return Ok(false);
    }
    for subject in batch {
        if store.message_expiry(subject)?.is_some_and(|expiry| !expiry.superseded) {
            continue;
        }
        let settled = if !crate::store::messages_shard::graph_sent(store, subject)? {
            mailbox_reads::lifecycle_step(store, subject, "message.delivered")?.is_some()
                || mailbox_reads::lifecycle_step(store, subject, "message.read")?.is_some()
                || mailbox_reads::lifecycle_step(store, subject, "message.closed")?.is_some()
        } else {
            crate::store::message_marks::recorded_at(
                &store.readers.get(), subject, &["message.delivered", "message.read", "message.closed"], None,
            )?.is_some()
                || store.latest_claim(subject, Some("message.delivered"))?.is_some()
                || store.latest_claim(subject, Some("message.read"))?.is_some()
                || store.latest_claim(subject, Some("message.closed"))?.is_some()
        };
        if !settled {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Store-local recovery: one latest boot floor and one outstanding batch per recipient.
#[derive(Default)]
struct PollRecovery(BTreeMap<String, PollBoot>);

#[derive(Default)]
struct PollBoot {
    floor: (u128, u64),
    admitted: std::collections::BTreeSet<String>,
}

fn poll_backlog(
    store: &Store,
    recovery: &Mutex<PollRecovery>,
    recipient: &str,
    floor: (u128, u64),
    messages: &[crate::model::MessageView],
) -> anyhow::Result<std::collections::BTreeSet<String>> {
    let mut has_backlog = false;
    for message in messages {
        if message.status == "sent" && !crate::silent::waits_for_turn(message)
            && let Some(sent) = sent_claim(store, message)?
            && recovery_backlog(sent.accepted_at_unix_ms, floor.0) == Some(true)
        {
            has_backlog = true;
            break;
        }
    }
    let previous = recovery.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .0.get(recipient).map(|boot| (boot.floor, boot.admitted.clone()));
    if previous.as_ref().is_some_and(|(current, _)| *current > floor) {
        return Ok(Default::default());
    }
    let completed = if let Some((current, admitted)) = &previous {
        if *current == floor && !admitted.is_empty() {
            if !recovery_batch_settled(store, admitted)? {
                return Ok(admitted.clone());
            }
            true
        } else { false }
    } else { false };
    if !has_backlog {
        return Ok(previous.filter(|(current, _)| *current == floor)
            .map(|(_, admitted)| admitted).unwrap_or_default());
    }
    // A page or filtered query must not choose newer mail ahead of older eligible mail.
    // Read before locking: the mutex protects only the five-slot admission decision.
    let mut candidates = Vec::new();
    for message in store.messages(Some(recipient), false)? {
        if !crate::silent::waits_for_turn(&message)
            && never_offered(store, &message)?
            && let Some(sent) = sent_claim(store, &message)?
            && recovery_backlog(sent.accepted_at_unix_ms, floor.0) == Some(true)
        {
            candidates.push((sent.accepted_at_unix_ms, message.subject));
        }
    }
    candidates.sort_unstable();
    let selected = candidates.into_iter().take(RECOVERY_BACKLOG_LIMIT)
        .map(|(_, subject)| subject).collect::<std::collections::BTreeSet<_>>();
    let selecting = messages.iter().any(|message| selected.contains(&message.subject));
    let mut recovery = recovery.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let PollBoot { floor: current, admitted } = recovery.0.entry(recipient.into()).or_default();
    if *current > floor {
        return Ok(Default::default()); // A concurrent poll from an obsolete boot cannot refill.
    }
    if *current != floor {
        *current = floor;
        admitted.clear();
    } else if completed && selecting && previous.as_ref().is_some_and(|(_, batch)| batch == admitted) {
        admitted.clear();
    }
    // Reserve the whole oldest batch across pages; a fast first page cannot refill early.
    // Keep a completed batch visible for receipt reconciliation until another is selected.
    if admitted.is_empty() && selecting {
        *admitted = selected;
    }
    Ok(admitted.clone())
}

/// Polling preserves this boot's admitted mail until its current offer finishes.
/// Previously offered mail from earlier boots stays held, regardless of age.
pub fn hold_pre_boot_mail(
    store: &Store,
    peer: Option<&NativeDeliveryPeer>,
    recipient: Option<&str>,
    messages: &mut Vec<crate::model::MessageView>,
) -> anyhow::Result<()> {
    let Some(peer) = peer.filter(|peer| recipient == Some(peer.agent.as_str())) else {
        return Ok(());
    };
    let Some((since, through)) = store.native_mail_boot_floor(&peer.agent)? else {
        for message in messages {
            if !crate::silent::waits_for_turn(message) {
                message.status = "closed".into();
            }
        }
        return Ok(());
    };
    // Domain-owned ephemeral state shares the daemon's Store lifetime without changing callers.
    let recovery = store.extension::<Mutex<PollRecovery>>();
    let admitted = poll_backlog(store, &recovery, &peer.agent, (since, through), messages)?;
    // A closed projection only removes old native inbox files. Explicit conversation
    // reads still see the original graph status, with no synthetic receipt or close.
    for message in messages {
        if crate::silent::waits_for_turn(message) {
            continue;
        }
        let Some(sent) = sent_claim(store, message)? else {
            message.status = "closed".into();
            continue;
        };
        // A stream item's index is past every graph index, so only its accepted time places
        // it against the boot; its staging is a local record without a graph index.
        let after_boot = message.created_index > through && sent.accepted_at_unix_ms >= since;
        let recent = recovery_backlog(sent.accepted_at_unix_ms, since) == Some(false);
        let recovered = admitted.contains(&message.subject);
        let current_offer = (recent || recovered)
            && if mailbox_reads::is_stream_item(message) {
                mailbox_reads::lifecycle_step(store, &message.subject, "message.staged")?
                    .is_some_and(|(actor, at)| at >= since && actor == peer.agent)
            } else if let Some((consumer, at)) = crate::store::message_marks::staged(&store.readers.get(), &message.subject)? {
                // A staging this member recorded as a mark has no graph index either.
                at >= since && consumer == peer.agent
            } else {
                store
                    .latest_claim(&message.subject, Some("message.staged"))?
                    .is_some_and(|claim| {
                        claim.store_index > through
                            && claim.accepted_at_unix_ms >= since
                            && claim.actor.as_deref() == Some(peer.agent.as_str())
                    })
            };
        if !after_boot && !((recent || recovered) && (never_offered(store, message)? || current_offer)) {
            message.status = "closed".into();
        }
    }
    Ok(())
}

/// A message's `message.sent` claim; a stream item's is in the messages shard.
pub fn sent_claim(
    store: &Store,
    message: &crate::model::MessageView,
) -> anyhow::Result<Option<ClaimRecord>> {
    if mailbox_reads::is_stream_item(message) {
        return mailbox_reads::sent_claim(store, &message.subject);
    }
    store.latest_claim(&message.subject, Some("message.sent"))
}

// Unit controls can pause snapshot processing while the real subscription and
// incoming-report handler remain live. Installed servers never have this gate.
#[cfg(any(test, feature = "test-support"))]
pub struct FixtureDirtyGate {
    pub allowed: std::sync::atomic::AtomicBool,
    pub notifications: std::sync::atomic::AtomicUsize,
    pub replay_proofs: std::sync::atomic::AtomicUsize,
    pub attachment_evaluations: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::mailbox_tests_shared::age_sent;

    /// A staging recorded as a mark is a current offer like a `message.staged` claim: it keeps a
    /// recent message open through this boot, and the next boot holds it.
    #[test]
    fn a_staging_mark_is_a_current_offer_until_the_next_boot() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("graph.db"), "node").unwrap();
        let seat = "agent/eval.worker";
        let peer = NativeDeliveryPeer { agent: seat.into(), transport: "omp-channel", pid: 37, archives_inbox: true };
        crate::store::mailbox_tests::ready(&store, "first-boot");
        store
            .append_claim(&ClaimInput {
                subject: "message/fresh".into(),
                kind: "message.sent".into(),
                actor: Some("person/eval".into()),
                fields: BTreeMap::from([
                    ("status".into(), json!("sent")),
                    ("from".into(), json!("person/eval")),
                    ("to".into(), json!(seat)),
                    ("content".into(), json!("QUARTZ SIGNAL")),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some("send:fresh".into()),
            })
            .unwrap();
        crate::store::mailbox_tests::ready(&store, "second-boot");
        crate::store::message_marks::record(&store, "message/fresh", "message.staged", seat).unwrap();
        let held = |store: &Store| {
            let mut messages = store.messages(Some(seat), false).unwrap();
            hold_pre_boot_mail(store, Some(&peer), Some(seat), &mut messages).unwrap();
            messages.into_iter().find(|m| m.subject == "message/fresh").unwrap().status
        };
        assert_eq!(held(&store), "staged", "the boot's own staging mark closed its offer");
        // A mark has no graph index: only its time places it before the next boot.
        std::thread::sleep(std::time::Duration::from_millis(2));
        crate::store::mailbox_tests::ready(&store, "third-boot");
        assert_eq!(held(&store), "closed");
        assert!(store.claims_for("message/fresh", Some("message.staged")).unwrap().is_empty());
    }

    #[test]
    fn native_poll_recovers_hour_old_unoffered_mail_until_its_current_offer_finishes() {
        for transport in [
            "claude-channel",
            "omp-channel",
            "pi-channel",
            "app-server",
            "opencode-server",
        ] {
            let root = tempfile::tempdir().unwrap();
            let database = root.path().join("graph.db");
            let store = Store::open(&database, "node").unwrap();
            let seat = "agent/eval.worker";
            let peer = NativeDeliveryPeer {
                agent: seat.into(),
                transport,
                pid: 37,
                archives_inbox: true,
            };
            for (id, phase) in [
                ("fresh", "sent"),
                ("offered", "staged"),
                ("accepted", "delivered"),
            ] {
                let subject = format!("message/{id}");
                store
                    .append_claim(&ClaimInput {
                        subject: subject.clone(),
                        kind: "message.sent".into(),
                        actor: Some("person/eval".into()),
                        fields: BTreeMap::from([
                            ("status".into(), json!("sent")),
                            ("from".into(), json!("person/eval")),
                            ("to".into(), json!(seat)),
                            ("content".into(), json!("QUARTZ SIGNAL")),
                        ]),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: Some(format!("send:{id}")),
                    })
                    .unwrap();
                if id == "fresh" {
                    age_sent(&store, &database, &subject);
                }
                if phase != "sent" {
                    store
                        .append_claim(&ClaimInput {
                            subject,
                            kind: format!("message.{phase}"),
                            actor: Some(seat.into()),
                            fields: BTreeMap::from([("status".into(), json!(phase))]),
                            evidence: Vec::new(),
                            expected_subject: None,
                            idempotency_key: Some(format!("offer:{id}")),
                        })
                        .unwrap();
                }
            }
            crate::store::mailbox_tests::ready(&store, "first-boot");
            let mut messages = store.messages(Some(seat), false).unwrap();
            hold_pre_boot_mail(&store, Some(&peer), Some(seat), &mut messages).unwrap();
            assert!(messages.iter().all(|m| m.status
                == if m.subject == "message/fresh" {
                    "sent"
                } else {
                    "closed"
                }));
            for phase in ["staged", "delivered"] {
                store
                    .append_claim(&ClaimInput {
                        subject: "message/fresh".into(),
                        kind: format!("message.{phase}"),
                        actor: Some(seat.into()),
                        fields: BTreeMap::from([("status".into(), json!(phase))]),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: Some(format!("fresh:{phase}")),
                    })
                    .unwrap();
                let mut messages = store.messages(Some(seat), false).unwrap();
                hold_pre_boot_mail(&store, Some(&peer), Some(seat), &mut messages).unwrap();
                assert!(
                    messages
                        .iter()
                        .find(|m| m.subject == "message/fresh")
                        .is_some_and(|m| m.status == phase),
                    "{transport} removed recovery during current {phase}"
                );
            }
            crate::store::mailbox_tests::ready(&store, "second-boot");
            let mut messages = store.messages(Some(seat), false).unwrap();
            hold_pre_boot_mail(&store, Some(&peer), Some(seat), &mut messages).unwrap();
            assert!(messages.iter().all(|m| m.status == "closed"));
            assert_eq!(
                store.message("message/fresh").unwrap().unwrap().status,
                "delivered"
            );
            assert!(
                store
                    .claims_for("message/fresh", Some("message.read"))
                    .unwrap()
                    .is_empty()
            );
            assert!(
                store
                    .claims_for("message/fresh", Some("message.closed"))
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn native_poll_reserves_oldest_batch_across_pages_and_replaces_obsolete_boots() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("graph.db");
        let store = Store::open(&database, "node").unwrap();
        let recovery = store.extension::<Mutex<PollRecovery>>();
        let seat = "agent/eval.worker";
        let peer = NativeDeliveryPeer { agent: seat.into(), transport: "omp-channel", pid: 37, archives_inbox: true };
        // Creation order deliberately disagrees with original send age.
        for id in (0..9).rev() {
            store.append_claim(&ClaimInput {
                subject: format!("message/{id}"), kind: "message.sent".into(),
                actor: Some("person/eval".into()),
                fields: BTreeMap::from([
                    ("status".into(), json!("sent")), ("from".into(), json!("person/eval")),
                    ("to".into(), json!(seat)), ("content".into(), json!("QUARTZ SIGNAL")),
                ]),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        crate::store::message_marks::record(&store, "message/0", "message.staged", seat).unwrap();
        crate::store::mailbox_tests::ready(&store, "first-boot");
        let (since, _) = store.native_mail_boot_floor(seat).unwrap().unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        crate::store::configure_projection_writer(&connection).unwrap();
        for id in 0..9 {
            let age = match id {
                0 | 1 => RECOVERY_CUTOFF_MS, // The cutoff itself is inclusive.
                8 => RECOVERY_CUTOFF_MS + 1,
                _ => (10 - id) * 3_600_000,
            };
            let sent = store.latest_claim(&format!("message/{id}"), Some("message.sent")).unwrap().unwrap();
            connection.execute(
                "UPDATE claims SET accepted_at_unix_ms=?1 WHERE id=?2",
                rusqlite::params![(since - age).to_string(), sent.id],
            ).unwrap();
        }
        let poll = |subjects: &[&str]| {
            let mut messages = store.messages(Some(seat), false).unwrap();
            if !subjects.is_empty() {
                messages.retain(|m| subjects.contains(&m.subject.as_str()));
            }
            hold_pre_boot_mail(&store, Some(&peer), Some(seat), &mut messages).unwrap();
            messages
        };
        assert_eq!(poll(&["message/7"])[0].status, "closed",
            "a filtered page must not admit newer backlog before the oldest five");
        assert!(recovery.lock().unwrap().0[seat].admitted.is_empty(), "a preview spent the budget");
        for id in 1..=5 {
            let subject = format!("message/{id}");
            assert_eq!(poll(&[&subject])[0].status, "sent");
            crate::store::message_marks::record(&store, &subject, "message.staged", seat).unwrap();
        }
        assert_eq!(recovery.lock().unwrap().0[seat].admitted.len(), 5);
        let held = poll(&[]);
        for message in &held {
            let admitted = ["message/1", "message/2", "message/3", "message/4", "message/5"]
                .contains(&message.subject.as_str());
            assert_eq!(message.status, if admitted { "staged" } else { "closed" });
        }
        // A single completed member cannot release the other staged members.
        crate::store::message_marks::record(&store, "message/1", "message.read", seat).unwrap();
        assert_eq!(poll(&["message/6"])[0].status, "closed");
        std::thread::sleep(std::time::Duration::from_millis(2));
        crate::store::mailbox_tests::ready(&store, "second-boot");
        for message in poll(&[]) {
            assert_eq!(message.status,
                if ["message/6", "message/7"].contains(&message.subject.as_str()) { "sent" } else { "closed" },
                "the next boot re-offered staged or expired mail");
        }
        assert_eq!(recovery.lock().unwrap().0.len(), 1, "old boot entries leaked");
    }

    #[test]
    fn native_poll_drains_twelve_in_settled_batches_including_expiry_and_rejects_stale_floors() {
        for phase in ["delivered", "closed", "expired"] {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("graph.db");
        let store = Store::open(&database, "node").unwrap();
        let seat = "agent/eval.worker";
        for id in (0..12).rev() {
            store.append_claim(&ClaimInput {
                subject: format!("message/batch-{id:02}"), kind: "message.sent".into(),
                actor: Some("person/eval".into()),
                fields: BTreeMap::from([
                    ("status".into(), json!("sent")), ("from".into(), json!("person/eval")),
                    ("to".into(), json!(seat)), ("content".into(), json!("QUARTZ SIGNAL")),
                ]),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        crate::store::mailbox_tests::ready(&store, "batch-boot");
        let floor = store.native_mail_boot_floor(seat).unwrap().unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        crate::store::configure_projection_writer(&connection).unwrap();
        for id in 0..12 {
            connection.execute("UPDATE claims SET accepted_at_unix_ms=?1 WHERE subject=?2",
                rusqlite::params![(floor.0 - 7_200_000 + id).to_string(), format!("message/batch-{id:02}")]).unwrap();
        }
        let peer = NativeDeliveryPeer { agent: seat.into(), transport: "omp-channel", pid: 37, archives_inbox: true };
        let poll = || {
            let mut messages = store.messages(Some(seat), false).unwrap();
            hold_pre_boot_mail(&store, Some(&peer), Some(seat), &mut messages).unwrap();
            messages.into_iter().filter(|m| m.status == "sent").map(|m| m.subject)
                .collect::<std::collections::BTreeSet<_>>()
        };
        for (start, end) in [(0, 5), (5, 10), (10, 12)] {
            let expected = (start..end).map(|id| format!("message/batch-{id:02}"))
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(poll(), expected);
            for (position, subject) in expected.iter().enumerate() {
                crate::store::message_marks::record(&store, subject,
                    if position == 0 { "message.staged" } else { "message.delivered" }, seat).unwrap();
            }
            assert!(poll().is_empty(), "a staged member must block the next batch");
            let withdraw = phase != "delivered";
            store.append_claim(&ClaimInput {
                subject: expected.first().unwrap().clone(), kind: format!("message.{phase}"),
                actor: Some(if withdraw { "daemon/runtime" } else { seat }.into()),
                fields: if phase == "expired" {
                    BTreeMap::from([("status".into(), json!(phase)), ("reason".into(), json!("cutoff-uncertain-attempt"))])
                } else { BTreeMap::from([("status".into(), json!(phase))]) },
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            }).unwrap();
            if withdraw {
                assert_eq!(store.message(expected.first().unwrap()).unwrap().unwrap().status, phase);
                assert!(crate::store::message_marks::recorded_at(
                    &store.readers.get(), expected.first().unwrap(), &["message.delivered", "message.read"], None,
                ).unwrap().is_none());
            }
            assert!(recovery_batch_settled(&store, &expected).unwrap());
        }
        assert!(poll().is_empty());
        let recovery = store.extension::<Mutex<PollRecovery>>();
        let newer = (floor.0 + 1, floor.1);
        recovery.lock().unwrap().0.insert(seat.into(), PollBoot { floor: newer, admitted: Default::default() });
        assert!(poll_backlog(&store, &recovery, seat, floor, &store.messages(Some(seat), false).unwrap()).unwrap().is_empty());
        assert_eq!(recovery.lock().unwrap().0[seat].floor, newer);
        assert_eq!(recovery.lock().unwrap().0.len(), 1);
        }
    }

}
