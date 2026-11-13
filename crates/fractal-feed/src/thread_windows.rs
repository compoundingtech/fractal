//! Thread interests use the ordinary authenticated collection socket.

use super::Update;
use fractal_client::{ClientError, CollectionStream, ThreadListScope};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ThreadInterest {
    Agent(String),
    Inbox { source: String, episode: String },
    Mission(String),
    RecentlyClosed,
    Timeline(String),
    Summary(String),
}

pub(super) struct ThreadSubscription {
    pub(super) interest: ThreadInterest,
    pub(super) id: String,
}

impl ThreadSubscription {
    pub(super) fn new(interest: ThreadInterest) -> Self {
        Self {
            interest,
            id: format!("thread-window/{}", uuid::Uuid::now_v7()),
        }
    }

    pub(super) async fn subscribe(&self, stream: &mut CollectionStream) -> Result<(), ClientError> {
        let scope = match &self.interest {
            ThreadInterest::Agent(id) => ThreadListScope::Agent(id),
            ThreadInterest::Inbox { source, episode } => ThreadListScope::Inbox { source, episode },
            ThreadInterest::Mission(id) => ThreadListScope::Mission(id),
            ThreadInterest::RecentlyClosed => ThreadListScope::RecentlyClosed,
            ThreadInterest::Summary(id) => {
                return stream.subscribe_thread_summary(&self.id, id).await;
            }
            ThreadInterest::Timeline(id) => {
                return stream.subscribe_thread_timeline(&self.id, id, 50).await;
            }
        };
        stream.subscribe_thread_list(&self.id, scope, 30).await
    }
}

// Same socket limit enforced by client_v0::COLLECTION_MAX_SUBSCRIPTIONS.
pub(super) const SOCKET_SLOTS: usize = 16;
const MAX_THREAD_IDS: usize = 4;
const MAX_CONTEXT_LISTS: usize = 2;
const PAUSED: &str = "Live thread updates paused: this view exceeds the shared live-window budget. Close another live view or collapse a thread; updates resume automatically.";

#[derive(Default)]
pub(super) struct ThreadWindows {
    pub(super) desired: Vec<ThreadInterest>,
    pub(super) active: Vec<ThreadSubscription>,
    paused: BTreeSet<ThreadInterest>,
}

pub(super) struct Plan {
    pub(super) admitted: Vec<ThreadInterest>,
    pub(super) paused: Vec<ThreadInterest>,
}

/// Context lists consume individual slots; an expanded thread's metadata/history
/// consume one indivisible pair. Retain all demand even when only part fits.
pub(super) fn plan(desired: &[ThreadInterest], ordinary_slots: usize) -> Plan {
    let mut available = SOCKET_SLOTS.saturating_sub(ordinary_slots);
    let mut admitted = Vec::new();
    let mut paused = Vec::new();
    let mut lists = Vec::new();
    let mut details = Vec::<(String, bool)>::new();
    for interest in desired {
        match interest {
            ThreadInterest::Summary(id) | ThreadInterest::Timeline(id) => {
                let timeline = matches!(interest, ThreadInterest::Timeline(_));
                if let Some((_, requested)) = details.iter_mut().find(|(known, _)| known == id) {
                    *requested |= timeline;
                } else {
                    details.push((id.clone(), timeline));
                }
            }
            _ if !lists.contains(interest) => lists.push(interest.clone()),
            _ => {}
        }
    }
    for (index, interest) in lists.into_iter().enumerate() {
        if index < MAX_CONTEXT_LISTS && available > 0 {
            available -= 1;
            admitted.push(interest);
        } else {
            paused.push(interest);
        }
    }
    for (index, (id, timeline)) in details.into_iter().enumerate() {
        let mut bundle = vec![ThreadInterest::Summary(id.clone())];
        if timeline {
            bundle.push(ThreadInterest::Timeline(id));
        }
        if index < MAX_THREAD_IDS && bundle.len() <= available {
            available -= bundle.len();
            admitted.extend(bundle);
        } else {
            paused.extend(bundle);
        }
    }
    Plan { admitted, paused }
}

impl ThreadWindows {
    pub(super) fn reconnect(&mut self) {
        self.active.clear();
        self.paused.clear();
    }

    pub(super) async fn reconcile(
        &mut self,
        stream: &mut CollectionStream,
        updates: &std::sync::mpsc::Sender<Update>,
        ordinary_slots: usize,
        granted: bool,
    ) -> Result<(), ClientError> {
        if !granted {
            return Ok(());
        }
        let next = plan(&self.desired, ordinary_slots);
        let mut kept = Vec::new();
        // Release all old slots before admitting any new bundle or ordinary window.
        for window in std::mem::take(&mut self.active) {
            if next.admitted.contains(&window.interest) {
                kept.push(window);
            } else {
                stream.unsubscribe(&window.id).await?;
            }
        }
        for interest in next.admitted {
            if kept.iter().any(|window| window.interest == interest) {
                continue;
            }
            let window = ThreadSubscription::new(interest);
            window.subscribe(stream).await?;
            kept.push(window);
        }
        self.active = kept;
        let paused: BTreeSet<_> = next.paused.into_iter().collect();
        for interest in paused.difference(&self.paused) {
            updates
                .send(Update::ThreadWindowFailed {
                    interest: interest.clone(),
                    message: PAUSED.into(),
                    permanent: false,
                })
                .map_err(|_| ClientError::Protocol("Thread update receiver closed".into()))?;
        }
        self.paused = paused;
        Ok(())
    }
}

#[cfg(test)]
mod planning_tests {
    use super::*;

    fn details(ids: &[&str]) -> Vec<ThreadInterest> {
        ids.iter()
            .flat_map(|id| {
                [
                    ThreadInterest::Summary((*id).into()),
                    ThreadInterest::Timeline((*id).into()),
                ]
            })
            .collect()
    }

    #[test]
    fn two_expanded_threads_fit_normal_agent_mission_and_inbox_views() {
        for lists in [
            vec![ThreadInterest::Agent("agent/a".into())],
            vec![ThreadInterest::Mission("mission/a".into())],
            vec![
                ThreadInterest::Inbox {
                    source: "agent/a".into(),
                    episode: "claim/a".into(),
                },
                ThreadInterest::RecentlyClosed,
            ],
        ] {
            let mut desired = lists;
            desired.extend(details(&["thread/a", "thread/b"]));
            // Six core/optional windows, three conversations, and one terminal reserve ten.
            let selected = plan(&desired, 10);
            assert_eq!(selected.admitted, desired);
            assert!(selected.paused.is_empty());
        }
    }

    #[test]
    fn every_shared_socket_budget_admits_whole_thread_pairs_only() {
        let mut desired = vec![
            ThreadInterest::Inbox {
                source: "agent/a".into(),
                episode: "claim/a".into(),
            },
            ThreadInterest::RecentlyClosed,
        ];
        desired.extend(details(&[
            "thread/a", "thread/b", "thread/c", "thread/d", "thread/e",
        ]));
        for occupied in 0..=SOCKET_SLOTS {
            let selected = plan(&desired, occupied);
            assert!(selected.admitted.len() + occupied <= SOCKET_SLOTS);
            for id in ["thread/a", "thread/b", "thread/c", "thread/d", "thread/e"] {
                assert_eq!(
                    selected
                        .admitted
                        .contains(&ThreadInterest::Summary(id.into())),
                    selected
                        .admitted
                        .contains(&ThreadInterest::Timeline(id.into()))
                );
            }
            assert!(
                !selected
                    .admitted
                    .contains(&ThreadInterest::Timeline("thread/e".into())),
                "four DISTINCT IDs, not four raw interests, remain bounded"
            );
        }
    }

    #[test]
    fn ordinary_window_saturation_pauses_and_releasing_slots_readmits_all_retained_demand() {
        let mut windows = ThreadWindows::default();
        windows.desired = details(&["thread/a", "thread/b"]);
        let desired = windows.desired.clone();
        let saturated = plan(&windows.desired, SOCKET_SLOTS);
        assert!(saturated.admitted.is_empty());
        assert_eq!(saturated.paused, desired);
        windows.reconnect();
        assert_eq!(
            windows.desired, desired,
            "reconnect must not lose overflow demand"
        );
        let available = plan(&windows.desired, 12);
        assert_eq!(available.admitted, desired);
        assert!(available.paused.is_empty());
    }

    #[test]
    fn duplicate_interests_do_not_spend_slots_and_folded_summary_keeps_its_one_slot() {
        let mut desired = details(&["thread/a"]);
        desired.extend(details(&["thread/a"]));
        desired.push(ThreadInterest::Summary("thread/folded".into()));
        let selected = plan(&desired, 13);
        assert_eq!(
            selected.admitted,
            [
                ThreadInterest::Summary("thread/a".into()),
                ThreadInterest::Timeline("thread/a".into()),
                ThreadInterest::Summary("thread/folded".into())
            ]
        );
        assert!(selected.paused.is_empty());
        assert_eq!(
            plan(&details(&["thread/a"]), 15).paused.len(),
            2,
            "one available slot cannot strand a half-pair"
        );
    }
}

#[cfg(test)]
mod socket_budget_tests {
    use super::*;
    use axum::extract::ws::{Message, WebSocketUpgrade};
    use serde_json::{Value, json};
    use tokio::sync::mpsc;

    async fn wait_for(
        receiver: &mut mpsc::UnboundedReceiver<Value>,
        mut done: impl FnMut(&Value) -> bool,
    ) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let command = receiver.recv().await.expect("controlled socket remains available");
                assert!(command["_held"].as_u64().unwrap() <= 16, "subscription admission must never transiently exceed the real shared limit: {command}");
                if done(&command) { break; }
            }
        }).await.expect("production feed processes the requested transition");
    }

    #[tokio::test]
    async fn production_feed_reclaims_thread_pairs_before_ordinary_adds_and_readmits_without_new_demand()
     {
        let (recorded, mut records) = mpsc::unbounded_channel::<Value>();
        let app = axum::Router::new()
            .route(
                "/v1/client/capabilities",
                axum::routing::get(|| async {
                    let mut caps: Value = serde_json::from_str(include_str!(
                        "../../../docs/st3/client-v0/fixtures/capabilities.json"
                    ))
                    .unwrap();
                    caps["api_version"] = json!(fractal_client::API_VERSION);
                    caps["value"]["capabilities"] = json!([
                        {"id":"collections","version":1,"state":"granted"},
                        {"id":"threads-v1","version":2,"state":"granted"},
                        {"id":"thread-summary-feed","version":1,"state":"granted"},
                        {"id":"inventory-collections","version":1,"state":"granted"},
                        {"id":"summary","version":1,"state":"granted"},
                        {"id":"arrangements","version":1,"state":"granted"},
                        {"id":"glasses","version":2,"state":"granted"}
                    ]);
                    axum::Json(caps)
                }),
            )
            .route(
                "/v1/client/collections/stream",
                axum::routing::get(move |upgrade: WebSocketUpgrade| {
                    let recorded = recorded.clone();
                    async move {
                        upgrade
                            .protocols(["fractal.client.collections.v0"])
                            .on_upgrade(move |mut socket| async move {
                                let mut held = BTreeSet::new();
                                while let Some(Ok(message)) = socket.recv().await {
                                    match message {
                                        Message::Text(text) => {
                                            let mut command: Value =
                                                serde_json::from_str(&text).unwrap();
                                            let Some(id) =
                                                command["id"].as_str().map(str::to_owned)
                                            else {
                                                continue;
                                            };
                                            if command["kind"] == "subscribe" {
                                                held.insert(id);
                                            } else if command["kind"] == "unsubscribe" {
                                                held.remove(&id);
                                            }
                                            command["_held"] = json!(held.len());
                                            if recorded.send(command).is_err() {
                                                return;
                                            }
                                        }
                                        Message::Ping(bytes) => {
                                            if socket.send(Message::Pong(bytes)).await.is_err() {
                                                return;
                                            }
                                        }
                                        Message::Close(_) => return,
                                        _ => {}
                                    }
                                }
                            })
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = fractal_client::Client::fabric_loopback(
            format!("http://{}", listener.local_addr().unwrap()),
            "controlled-transport-receiver",
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let (updates, received) = std::sync::mpsc::channel();
        let (commands, requests) = mpsc::unbounded_channel();
        let feed = tokio::spawn(crate::run_members_with_sidebar(
            vec![client],
            false,
            true,
            true,
            Some("person/avery".into()),
            updates,
            requests,
        ));
        let desired = vec![
            ThreadInterest::Agent("agent/a".into()),
            ThreadInterest::Summary("thread/a".into()),
            ThreadInterest::Timeline("thread/a".into()),
            ThreadInterest::Summary("thread/b".into()),
            ThreadInterest::Timeline("thread/b".into()),
        ];
        commands.send(crate::Command::Threads(desired)).unwrap();
        let mut timelines = 0;
        wait_for(&mut records, |command| {
            timelines += usize::from(command["collection"] == "thread-timeline");
            timelines == 2
        })
        .await;
        commands
            .send(crate::Command::Inventories(crate::Inventories {
                clients: true,
                usage_hours: Some(24),
                machines: true,
                devices: true,
                sessions: true,
                terminals: true,
            }))
            .unwrap();
        wait_for(&mut records, |command| command["collection"] == "terminals").await;
        commands
            .send(crate::Command::Converse {
                targets: vec!["agent/a".into(), "agent/b".into(), "agent/c".into()],
            })
            .unwrap();
        let mut conversations = 0;
        wait_for(&mut records, |command| {
            conversations += usize::from(command["collection"] == "conversation");
            conversations == 3
        })
        .await;
        let paused: BTreeSet<_> = received
            .try_iter()
            .filter_map(|update| match update {
                Update::ThreadWindowFailed {
                    interest: ThreadInterest::Timeline(id),
                    message,
                    permanent: false,
                } if message.contains("paused") => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(
            paused,
            BTreeSet::from(["thread/a".into(), "thread/b".into()])
        );
        // No new Threads command: releasing ordinary inventory slots must resume both pairs.
        commands
            .send(crate::Command::Inventories(crate::Inventories::default()))
            .unwrap();
        let mut resumed = 0;
        wait_for(&mut records, |command| {
            resumed += usize::from(command["collection"] == "thread-timeline");
            resumed == 2
        })
        .await;
        feed.abort();
        server.abort();
    }
}

/// Current connection grants. Read access never implies permission to mutate a thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThreadAccess {
    pub start: bool,
    pub reply: bool,
    pub rename: bool,
    pub close: bool,
    pub reopen: bool,
    pub mark_read: bool,
}

pub(super) fn access(
    capabilities: &fractal_client::Capabilities,
    has_signer: bool,
) -> ThreadAccess {
    use fractal_client::{CapabilityState, TransportKind};
    let granted = |id: &str| {
        capabilities
            .capabilities
            .iter()
            .any(|capability| capability.id == id && capability.state == CapabilityState::Granted)
    };
    let native_unsigned =
        capabilities.transport == TransportKind::Unix && capabilities.grant_id.is_none();
    let browser_send = capabilities.capabilities.iter().any(|capability| {
        capability.id == "thread-browser-send"
            && capability.version >= 1
            && capability.state == CapabilityState::Granted
    });
    let can_message = has_signer || native_unsigned || browser_send;
    ThreadAccess {
        start: can_message && granted("thread.start"),
        reply: can_message && granted("thread.reply"),
        rename: granted("thread.rename"),
        close: granted("thread.close"),
        reopen: granted("thread.reopen"),
        mark_read: granted("thread.read"),
    }
}

#[cfg(test)]
mod access_tests {
    use super::*;

    fn capabilities(paired: bool) -> fractal_client::Capabilities {
        serde_json::from_value(serde_json::json!({
            "kind":"capabilities","session_actor":"person/operator","transport":"unix",
            "capabilities":[],
            "limits":{"max_page_items":50,"max_event_items":50,"max_response_bytes":100000,"max_wait_ms":1000},
            "event_cursor":"","oldest_event_cursor":"","schemas":[],
            "grant_id":if paired { Some("claim/grant") } else { None },
        })).unwrap()
    }

    #[test]
    fn a_read_grant_never_becomes_a_send_or_metadata_grant() {
        let mut caps = capabilities(false);
        caps.capabilities = vec![fractal_client::Capability {
            id: "threads-v1".into(),
            version: 2,
            state: fractal_client::CapabilityState::Granted,
        }];
        assert_eq!(access(&caps, false), ThreadAccess::default());
    }

    #[test]
    fn paired_native_messages_need_a_signer_or_the_exact_browser_assertion() {
        let mut caps = capabilities(true);
        caps.capabilities = [
            "thread.start",
            "thread.reply",
            "thread.rename",
            "thread.close",
            "thread.reopen",
            "thread.read",
        ]
        .into_iter()
        .map(|id| fractal_client::Capability {
            id: id.into(),
            version: 0,
            state: fractal_client::CapabilityState::Granted,
        })
        .collect();
        let unsigned = access(&caps, false);
        assert!(!unsigned.start && !unsigned.reply);
        assert!(unsigned.rename && unsigned.close && unsigned.reopen && unsigned.mark_read);
        let signed = access(&caps, true);
        assert!(signed.start && signed.reply);
        caps.capabilities.push(fractal_client::Capability {
            id: "thread-browser-send".into(),
            version: 1,
            state: fractal_client::CapabilityState::Granted,
        });
        assert!(access(&caps, false).reply);
    }
}
