#![allow(unused_imports)]
use crate::DeliveryStore;
use crate::MailboxChangesStore;
use crate::MailboxLeaseStore;
use crate::MailboxWakesStore;
use crate::RolloutsStore;
use crate::SeatsStore;
use crate::SeatsStoreCore;
use crate::api::*;
use crate::WatchesStore;
use crate::mailbox::{Fence, Frame, Receipt};
use crate::api::mailbox::*;
use crate::client::{Client, Endpoint};
use tokio_tungstenite::tungstenite::Message;

/// Backdate a send past the recovery window of the recipient's current boot. The
    /// window is measured from the boot floor, so age from it (or from now, before any
    /// boot): a slow runner between boot and this call must not make the mail recent.
    pub fn age_sent(store: &Store, database: &Path, subject: &str) {
        let claim = store
            .latest_claim(subject, Some("message.sent"))
            .unwrap()
            .unwrap();
        let recipient = claim.body.get("fields").unwrap_or(&claim.body)["to"]
            .as_str()
            .unwrap()
            .to_owned();
        let anchor = store
            .native_mail_boot_floor(&recipient)
            .unwrap()
            .map_or(client_now_ms(), |(since, _)| since.min(client_now_ms()));
        let connection = rusqlite::Connection::open(database).unwrap();
        crate::store::configure_projection_writer(&connection).unwrap();
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-hb967428e5ded owner=fractal_api_core::api::mailbox_tests_shared TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=?1 WHERE id=?2",
                rusqlite::params![
                    (anchor - u128::from(crate::api::mail_backlog::THRESHOLD_MS) - 1_000)
                        .to_string(),
                    claim.id
                ],
            )
            .unwrap();
    }

/// No mail frame arrives for a while: nothing was offered to the seat.
    pub async fn no_mail(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>) {
        let quiet = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if let Frame::Mailbox { messages } = serde_json::from_str(&text).unwrap() {
                            return messages;
                        }
                    }
                    Some(Ok(Message::Ping(bytes))) => socket.send(Message::Pong(bytes)).await.unwrap(),
                    other => panic!("unexpected {other:?}"),
                }
            }
        })
        .await;
        if let Ok(messages) = quiet {
            panic!("held mail was offered: {messages:?}");
        }
    }

/// The seat reads each message, as its channel does.
    pub async fn read_all(client: &Client, fence: &Fence, messages: &[crate::model::MessageView]) {
        for message in messages {
            for lifecycle in ["staged", "delivered", "read"] {
                let _: ClaimRecord = client
                    .post(
                        "/v1/mailbox/receipts",
                        &Receipt {
                            fence: fence.clone(),
                            message: message.subject.clone(),
                            lifecycle: lifecycle.into(),
                        },
                    )
                    .await
                    .unwrap();
            }
        }
    }

pub struct ControlledStream {
        pub dirty_gate: Arc<FixtureDirtyGate>,
        pub socket: tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>,
        pub recheck: tokio::sync::mpsc::UnboundedSender<()>,
        pub heartbeat: tokio::sync::mpsc::UnboundedSender<()>,
        pub reads: Arc<std::sync::atomic::AtomicUsize>,
        pub server: tokio::task::JoinHandle<()>,
    }

impl Drop for ControlledStream {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

pub async fn expect_pong(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>, payload: &[u8]) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match socket.next().await.expect("stream closed before report acknowledgement").unwrap() {
                    Message::Pong(bytes) if bytes.as_ref() == payload => return,
                    Message::Ping(_) | Message::Pong(_) => {},
                    other => panic!("expected post-report Pong, received {other:?}"),
                }
            }
        }).await.expect("report processing was not observed");
    }
