//! One local daemon stream per delivery component. Ownership and receipts are fenced in SQLite,
//! so an old channel reconnecting after a replacement cannot consume or acknowledge its mail.
use crate::SeatsStore;
use super::*;

/// Recover only recent mail which has never been offered. A prior staging claim is
/// already an offer attempt, even when its delivered receipt has not arrived yet.
pub fn never_offered(store: &Store, message: &crate::model::MessageView) -> anyhow::Result<bool> {
    Ok(message.status == "sent"
        && store
            .latest_claim(&message.subject, Some("message.staged"))?
            .is_none()
        && store
            .latest_claim(&message.subject, Some("message.delivered"))?
            .is_none())
}

/// Older drivers poll instead of subscribing. Preserve a recent message admitted
/// by this boot while its current staging attempt finishes; older attempts stay held.
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
    // A closed projection only removes old native inbox files. Explicit conversation
    // reads still see the original graph status, with no synthetic receipt or close.
    for message in messages {
        if crate::silent::waits_for_turn(message) {
            continue;
        }
        let Some(sent) = store.latest_claim(&message.subject, Some("message.sent"))? else {
            message.status = "closed".into();
            continue;
        };
        let after_boot = message.created_index > through && sent.accepted_at_unix_ms >= since;
        let recent = sent.accepted_at_unix_ms
            > since.saturating_sub(u128::from(super::mail_backlog::THRESHOLD_MS));
        let current_offer = recent
            && store
                .latest_claim(&message.subject, Some("message.staged"))?
                .is_some_and(|claim| {
                    claim.store_index > through
                        && claim.accepted_at_unix_ms >= since
                        && claim.actor.as_deref() == Some(peer.agent.as_str())
                });
        if !after_boot && !(recent && (never_offered(store, message)? || current_offer)) {
            message.status = "closed".into();
        }
    }
    Ok(())
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

    #[test]
    fn native_poll_recovers_unoffered_preboot_mail_until_its_current_offer_finishes() {
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
                ("hour-old", "sent"),
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
                if id == "hour-old" {
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

}
