//! The part of `store::mailbox_wakes` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::DeliveryStore;
use super::*;
use crate::model::DoctorCheck;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::watch;
pub use fractal_store::store::mailbox_wakes::*;
pub use fractal_store::store::mailbox_wakes::{Dependency, Registry, Subscription, Wakes};

const RETRY: std::time::Duration = std::time::Duration::from_secs(5);

async fn dispatch(
    store: std::sync::Weak<Store>,
    wakes: Arc<Wakes>,
    mut events: watch::Receiver<u64>,
    mut cursor: Option<(u64, u64)>,
) {
    let mut retrying = cursor.is_none();
    let mut retry = tokio::time::interval_at(tokio::time::Instant::now() + RETRY, RETRY);
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            result = events.changed(), if !retrying => {
                if result.is_err() { return; }
                events.borrow_and_update();
            },
            _ = retry.tick(), if retrying => {},
        }
        if cursor.is_none() {
            let Some(store) = store.upgrade() else {
                return;
            };
            match tokio::task::spawn_blocking(move || crate::task_cpu::blocking("mailbox.wake_dispatch", || store.mailbox_wake_cursor())).await {
                Ok(Ok(at)) => {
                    // Start at the current head, never at zero. Re-read every subscriber once
                    // to cover writes skipped while initial cursor acquisition was unavailable.
                    cursor = Some(at);
                    wakes.resync();
                }
                Ok(Err(error)) => {
                    wakes.failure("initialize", error);
                    continue;
                }
                Err(error) => {
                    wakes.failure("initialize-worker", error);
                    continue;
                }
            }
        }
        loop {
            let Some(store) = store.upgrade() else {
                return;
            };
            let at = cursor.expect("cursor acquired before dispatch");
            let result = tokio::task::spawn_blocking(move || {
                crate::task_cpu::profiled_blocking("mailbox.wake_dispatch", || {
                    store.mailbox_wake_batch(at)
                })
            })
            .await;
            match result {
                Ok(Ok((next, dependencies, more))) => {
                    wakes.notify(dependencies);
                    cursor = Some(next);
                    retrying = false;
                    wakes.degraded.store(false, Ordering::Relaxed);
                    if !more {
                        break;
                    }
                }
                Ok(Err(error)) => {
                    wakes.failure("read", error);
                    retrying = true;
                    break;
                }
                Err(error) => {
                    wakes.failure("worker", error);
                    retrying = true;
                    break;
                }
            }
        }
        if retrying {
            retry.reset_after(RETRY);
        }
    }
}

pub trait MailboxWakesStore {
    fn subscribe_mailbox(
            self: &Arc<Self>,
            fence: &crate::mailbox::Fence,
            events: &watch::Sender<u64>,
        ) -> Subscription;
    /// Local telemetry uses the existing doctor check shape; never claims or replicated state.
    fn mailbox_wake_health(&self) -> DoctorCheck;
    fn mailbox_wake_cursor(&self) -> Result<(u64, u64)>;
    fn mailbox_wake_batch(
            &self,
            cursor: (u64, u64),
        ) -> Result<((u64, u64), HashSet<Dependency>, bool)>;
}

impl MailboxWakesStore for Store {

    fn subscribe_mailbox(
            self: &Arc<Self>,
            fence: &crate::mailbox::Fence,
            events: &watch::Sender<u64>,
        ) -> Subscription {
        // Subscribe to the post-commit feed before establishing the dispatcher's cursor, and
        // register this stream before its first durable read. No replay-to-live gap.
        let wakes = self.smalltalk.mailbox_wakes.get_or_init(|| {
            let events = events.subscribe();
            let cursor = self.mailbox_wake_cursor();
            let weak = Arc::downgrade(self);
            let wakes = Arc::new(Wakes::default());
            let cursor = match cursor {
                Ok(cursor) => Some(cursor),
                Err(error) => {
                    wakes.failure("initialize", error);
                    None
                }
            };
            let routed = wakes.clone();
            tokio::spawn(crate::task_cpu::account("mailbox.wake_dispatch", async move {
                dispatch(weak, routed, events, cursor).await;
            }));
            wakes
        });
        wakes.subscribe(fence)
    }

    fn mailbox_wake_health(&self) -> DoctorCheck {
        let (failures, degraded) = self
            .smalltalk
            .mailbox_wakes
            .get()
            .map_or((0, false), |wakes| {
                (
                    wakes.failures.load(Ordering::Relaxed),
                    wakes.degraded.load(Ordering::Relaxed),
                )
            });
        DoctorCheck {
            name: "mailbox-wake-dispatcher".into(),
            status: if degraded { "warn" } else { "pass" }.into(),
            message: format!(
                "{failures} failures since startup; {}; thirty-second safety reads remain active",
                if degraded {
                    "retrying; routing degraded"
                } else {
                    "routing healthy or not yet subscribed"
                }
            ),
        }
    }

    fn mailbox_wake_cursor(&self) -> Result<(u64, u64)> {
        let connection = self.readers.get();
        Ok(connection
            .prepare_cached(
                "SELECT COALESCE((SELECT MAX(store_index) FROM claims),0),
                    COALESCE((SELECT MAX(id) FROM local_observations),0)",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-hb895da732cc2 owner=fractal_delivery::store::mailbox_wakes TODO=trace-statement-and-value-representation */ [], |r| Ok((r.get(0)?, r.get(1)?)))?)
    }

    fn mailbox_wake_batch(
            &self,
            cursor: (u64, u64),
        ) -> Result<((u64, u64), HashSet<Dependency>, bool)> {
        let connection = self.readers.get();
        let mut dependencies = HashSet::new();
        let mut next = cursor;
        // Bound each read and route once per batch. Only new subjects are read; this cost does
        // not multiply by the number of mailbox streams, or by the size of the graph.
        let rows = connection.prepare_cached(
            "SELECT store_index,subject FROM claims WHERE store_index>?1 ORDER BY store_index LIMIT 1024"
        )?.query_map([cursor.0], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let graph_full = rows.len() == 1024;
        let mut messages = HashSet::new();
        for (index, subject) in rows {
            next.0 = index;
            if subject.starts_with("message/") {
                messages.insert(subject.clone());
            }
            dependencies.insert(Dependency::Subject(subject));
        }
        for subject in messages {
            // Include historical sent recipients and the current declaration. Retargeting
            // wakes both owners; subject subscribers see removals as well as additions.
            let recipients = connection.prepare_cached(
                "SELECT json_extract(body,'$.fields.to') FROM claims WHERE subject=?1 AND kind='message.sent'
                 UNION SELECT json_extract(child.value,'$.arguments[0]')
                 FROM desired,json_each(desired.body,'$.children') child
                 WHERE desired.subject=?1 AND desired.kind='message' AND json_extract(child.value,'$.name')='to'"
            )?.query_map([&subject], |r| r.get::<_, Option<String>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for recipient in recipients.into_iter().flatten() {
                dependencies.insert(Dependency::Recipient(normalize_message_party(&recipient)));
            }
        }
        let rows = connection
            .prepare_cached(
                "SELECT id,subject FROM local_observations WHERE id>?1 ORDER BY id LIMIT 1024",
            )?
            .query_map([cursor.1], |r| {
                Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let local_full = rows.len() == 1024;
        for (id, subject) in rows {
            next.1 = id;
            dependencies.insert(Dependency::Subject(subject));
        }
        Ok((next, dependencies, graph_full || local_full))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailbox::Fence;
    use fractal_store::store::mailbox_wakes_tests_shared_delivery::*;
    use fractal_store::store::mailbox_wakes_tests_shared_delivery::{sent};

    #[tokio::test]
    async fn unrelated_writes_do_not_wake_a_stream_and_dependencies_are_released() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&root.path().join("graph.db"), "node").unwrap());
        let events = watch::channel(0).0;
        let fence = Fence::new("agent/quartz", "boot", "delivery");
        let mut subscription = store.subscribe_mailbox(&fence, &events);
        let other =
            store.subscribe_mailbox(&Fence::new("agent/other", "boot", "delivery"), &events);
        let append = |subject: &str, to: &str| {
            store
                .append_claim(&ClaimInput {
                    subject: subject.into(),
                    kind: "message.sent".into(),
                    actor: Some("person/fixture".into()),
                    fields: BTreeMap::from([
                        ("status".into(), json!("sent")),
                        ("from".into(), json!("person/fixture")),
                        ("to".into(), json!(to)),
                        ("content".into(), json!("A note.")),
                    ]),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
            events.send_modify(|generation| *generation += 1);
        };
        append("message/other", "agent/other");
        let mut other_changed = other.changed.clone();
        tokio::time::timeout(std::time::Duration::from_secs(2), other_changed.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(
            !subscription.changed.has_changed().unwrap(),
            "unrelated durable writes must leave the stream asleep"
        );
        append("message/mine", "agent/quartz");
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            subscription.changed.changed(),
        )
        .await
        .unwrap()
        .unwrap();
        subscription.changed.borrow_and_update();
        subscription.messages(&["message/mine".into()]);
        // An owner replacement is local (no graph claim) and wakes only its component.
        let title = store.subscribe_mailbox(&Fence::new("agent/quartz", "boot", "title"), &events);
        let hub = subscription.wakes.clone();
        hub.owner_changed("agent/quartz", "delivery");
        assert!(subscription.changed.has_changed().unwrap());
        assert!(!title.changed.has_changed().unwrap());
        drop(subscription);
        drop(other);
        drop(title);
        assert!(hub.registry.lock().unwrap().subscribers.is_empty());
    }

    #[tokio::test]
    async fn replicated_mail_and_local_observations_wake_the_recipient() {
        let root = tempfile::tempdir().unwrap();
        let target = Arc::new(Store::open(&root.path().join("graph.db"), "target").unwrap());
        let source = Store::open_memory("source").unwrap();
        let events = watch::channel(0).0;
        let mut subscription =
            target.subscribe_mailbox(&Fence::new("agent/quartz", "boot", "delivery"), &events);
        source
            .append_claim(&ClaimInput {
                subject: "message/replicated".into(),
                kind: "message.sent".into(),
                actor: Some("person/fixture".into()),
                fields: BTreeMap::from([
                    ("status".into(), json!("sent")),
                    ("from".into(), json!("person/fixture")),
                    ("to".into(), json!("agent/quartz")),
                    ("content".into(), json!("A note.")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        target
            .import_replication("source", &source.export_replication(0).unwrap())
            .unwrap();
        events.send_modify(|generation| *generation += 1);
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            subscription.changed.changed(),
        )
        .await
        .unwrap()
        .unwrap();
        subscription.changed.borrow_and_update();
        target
            .append_claim(&ClaimInput {
                subject: "agent/quartz".into(),
                kind: "harness.observed".into(),
                actor: Some("agent/quartz".into()),
                fields: BTreeMap::from([
                    ("state".into(), json!("idle")),
                    ("driver".into(), json!("codex")),
                    ("incarnation_id".into(), json!("boot")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        events.send_modify(|generation| *generation += 1);
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            subscription.changed.changed(),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[test]
    fn routing_covers_declarations_lifecycle_and_local_observations() {
        let store = Store::open_memory("node").unwrap();
        let cursor = store.mailbox_wake_cursor().unwrap();
        store.apply(&crate::graph::parse_intent(
            "version 2\nmessage \"declared\" { from \"person/fixture\"; to \"quartz\"; content \"A note.\"; }\n", "fixture").unwrap(), &BTreeMap::from([("message/declared".into(), vec![])]), "fixture").unwrap();
        let (next, deps, _) = store.mailbox_wake_batch(cursor).unwrap();
        assert!(deps.contains(&Dependency::Recipient("agent/quartz".into())));
        store
            .append_claim(&ClaimInput {
                subject: "message/declared".into(),
                kind: "message.delivered".into(),
                actor: Some("agent/quartz".into()),
                fields: BTreeMap::from([("status".into(), json!("delivered"))]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let (next, deps, _) = store.mailbox_wake_batch(next).unwrap();
        assert!(deps.contains(&Dependency::Subject("message/declared".into())));
        assert!(deps.contains(&Dependency::Recipient("agent/quartz".into())));
        store
            .append_claim(&ClaimInput {
                subject: "agent/quartz".into(),
                kind: "harness.observed".into(),
                actor: Some("agent/quartz".into()),
                fields: BTreeMap::from([
                    ("state".into(), json!("idle")),
                    ("driver".into(), json!("codex")),
                    ("incarnation_id".into(), json!("boot")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let (_, deps, _) = store.mailbox_wake_batch(next).unwrap();
        assert!(deps.contains(&Dependency::Subject("agent/quartz".into())));
    }

    #[test]
    fn routing_covers_the_durable_changed_since_dependencies() {
        for case in [
            "seat declaration",
            "recipient",
            "legacy bare recipient",
            "snapshot message",
            "message declaration",
            "local seat",
            "owner",
            "other owner component",
            "unrelated mail",
            "unrelated declaration",
        ] {
            let store = Store::open_memory("node").unwrap();
            crate::store::mailbox_tests::ready(&store, "boot");
            let fence = store
                .bind_mailbox(&Fence::new("agent/eval.worker", "boot", "delivery"))
                .unwrap();
            // The snapshot subject is deliberately addressed elsewhere: this exercises the
            // subject dependency independently of recipient routing.
            sent(&store, "message/observed", "agent/other");
            let messages = vec!["message/observed".into()];
            let hub = Arc::new(Wakes::default());
            let mut subscription = hub.subscribe(&fence);
            subscription.messages(&messages);
            let mark = store.mailbox_watermark(&fence).unwrap();
            let cursor = store.mailbox_wake_cursor().unwrap();
            match case {
                "recipient" | "legacy bare recipient" => {
                    sent(&store, "message/new", "agent/eval.worker");
                    if case == "legacy bare recipient" {
                        // Old replicated data can contain bare recipients; new inputs require references.
                        store.connection.write().execute("UPDATE claims SET body=json_set(body,'$.fields.to','eval.worker') WHERE subject='message/new'", []).unwrap();
                    }
                }
                "snapshot message" => {
                    store
                        .append_claim(&ClaimInput {
                            subject: "message/observed".into(),
                            kind: "message.staged".into(),
                            actor: Some("agent/other".into()),
                            fields: BTreeMap::from([
                                ("status".into(), json!("staged")),
                                ("recipient".into(), json!("agent/other")),
                            ]),
                            evidence: vec![],
                            expected_subject: None,
                            idempotency_key: None,
                        })
                        .unwrap();
                }
                "local seat" => {
                    store.append_local_observations_for_test(&[ClaimInput {
                        subject: "agent/eval.worker".into(),
                        kind: "harness.telemetry".into(),
                        actor: Some("agent/eval.worker".into()),
                        fields: BTreeMap::from([
                            ("driver".into(), json!("claude")),
                            ("unit".into(), json!("hook")),
                            ("incarnation_id".into(), json!("boot")),
                            ("signals".into(), json!({})),
                        ]),
                        evidence: vec![],
                        expected_subject: None,
                        idempotency_key: None,
                    }]);
                    assert_eq!(
                        store.index().unwrap(),
                        mark.index,
                        "local branch must not move the graph index"
                    );
                }
                "owner" | "other owner component" => {
                    let component = if case == "owner" { "delivery" } else { "title" };
                    store
                        .bind_mailbox(&Fence::new("agent/eval.worker", "boot", component))
                        .unwrap();
                    hub.owner_changed("agent/eval.worker", component);
                }
                "unrelated mail" => sent(&store, "message/other", "agent/other"),
                _ => {
                    let (text, subject) = if case == "seat declaration" {
                        (
                            "agent \"eval.worker\" { workspace \"/tmp/fixture\"; harness \"codex\" {} }",
                            "agent/eval.worker",
                        )
                    } else if case == "message declaration" {
                        (
                            "message \"declared\" { from \"person/fixture\"; to \"eval.worker\"; content \"Note.\"; }",
                            "message/declared",
                        )
                    } else {
                        (
                            "message \"declared\" { from \"person/fixture\"; to \"other\"; content \"Note.\"; }",
                            "message/declared",
                        )
                    };
                    store
                        .apply(
                            &crate::graph::parse_intent(&format!("version 2\n{text}\n"), "fixture")
                                .unwrap(),
                            &BTreeMap::from([(subject.into(), vec![])]),
                            "fixture",
                        )
                        .unwrap();
                }
            }
            let (_, dependencies, _) = store.mailbox_wake_batch(cursor).unwrap();
            hub.notify(dependencies);
            let changed = store
                .mailbox_changed_since(&fence, &mark, &messages)
                .unwrap();
            let routed = subscription.changed.has_changed().unwrap();
            let relevant = !matches!(
                case,
                "unrelated mail" | "unrelated declaration" | "other owner component"
            );
            assert_eq!(routed, relevant, "{case}: dispatcher");
            // changed-since deliberately treats every message declaration as potentially
            // relevant. Routing can omit an unrelated declaration while retaining that
            // conservative guard for the streams which actually depend on it.
            assert_eq!(
                changed,
                relevant || case == "unrelated declaration",
                "{case}: durable guard"
            );
        }
    }

    #[tokio::test]
    async fn failed_initial_cursor_is_visible_and_recovers_without_zero_replay() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        sent(&store, "message/history", "agent/other");
        store
            .connection
            .write()
            .execute_batch("ALTER TABLE claims RENAME TO unavailable_claims")
            .unwrap();
        let events = watch::channel(0).0;
        let mut subscription =
            store.subscribe_mailbox(&Fence::new("agent/quartz", "boot", "delivery"), &events);
        assert_eq!(subscription.wakes.failures.load(Ordering::Relaxed), 1);
        assert_eq!(store.mailbox_wake_health().status, "warn");
        store
            .connection
            .write()
            .execute_batch("ALTER TABLE unavailable_claims RENAME TO claims")
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            subscription.changed.changed(),
        )
        .await
        .unwrap()
        .unwrap();
        // Initial-cursor recovery resyncs subscribers once at the current head rather than
        // enumerating historical rows. Future writes still route normally.
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while subscription.wakes.degraded.load(Ordering::Relaxed) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        subscription.changed.borrow_and_update();
        sent(&store, "message/new", "agent/quartz");
        events.send_modify(|generation| *generation += 1);
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            subscription.changed.changed(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(store.mailbox_wake_health().status, "pass");
        assert_eq!(subscription.wakes.failures.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn failed_dispatch_retains_its_cursor_and_retries_without_another_write_wake() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let events = watch::channel(0).0;
        let mut subscription =
            store.subscribe_mailbox(&Fence::new("agent/quartz", "boot", "delivery"), &events);
        store
            .connection
            .write()
            .execute_batch("ALTER TABLE claims RENAME TO unavailable_claims")
            .unwrap();
        events.send_modify(|generation| *generation += 1);
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while !subscription.wakes.degraded.load(Ordering::Relaxed) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(store.mailbox_wake_health().status, "warn");
        assert_eq!(subscription.wakes.failures.load(Ordering::Relaxed), 1);
        store
            .connection
            .write()
            .execute_batch("ALTER TABLE unavailable_claims RENAME TO claims")
            .unwrap();
        sent(&store, "message/retry", "agent/quartz");
        // Deliberately omit the global notification: recovery must scan from the retained
        // cursor after its retry timer, rather than starting from the new head or zero.
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            subscription.changed.changed(),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while subscription.wakes.degraded.load(Ordering::Relaxed) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(store.mailbox_wake_health().status, "pass");
        assert_eq!(subscription.wakes.failures.load(Ordering::Relaxed), 1);
    }
}
