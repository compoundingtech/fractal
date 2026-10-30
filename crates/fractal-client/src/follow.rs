//! Retained, typed collection windows multiplexed over one reconnecting socket.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures_util::Stream;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio_stream::wrappers::WatchStream;

use crate::{Attention, Client, ClientError, CollectionStream, ErrorCode, Snapshot};

const MAX_WATCHES: usize = 16;
const MAX_ROWS: usize = 200;
const MAX_PENDING_WRITES: usize = MAX_WATCHES * 2;
const MAX_COMMAND_BYTES: usize = 16 * 1024;
const RECONNECT_DELAY: Duration = Duration::from_millis(250);
// Match the server collection send contract (COLLECTION_SEND_TIMEOUT). A command
// that cannot flush in eight seconds terminates the socket, releasing server slots.
const SEND_DEADLINE: Duration = Duration::from_secs(8);

/// Maps a collection selector to its decoded row. Implementations decode raw rows once,
/// without passing through `Resource::Unknown` or serializing a model back to JSON.
pub trait Descriptor: Send + Sync + 'static {
    type Row: Send + Sync + 'static;
    fn selector(&self) -> Value;
    fn decode(&self, row: Value) -> Result<Self::Row, ClientError>;
}

/// Existing attention collection; notice descriptors belong to their own wire contract.
#[derive(Clone, Debug, Default)]
pub struct AttentionDescriptor {
    pub person: Option<String>,
}

impl Descriptor for AttentionDescriptor {
    type Row = Attention;
    fn selector(&self) -> Value {
        serde_json::json!({"collection":"attention", "person":self.person})
    }
    fn decode(&self, row: Value) -> Result<Attention, ClientError> {
        decode_row(row)
    }
}

/// Serde decoder usable by descriptors whose row model is generated from the wire schema.
pub fn decode_row<T: DeserializeOwned>(row: Value) -> Result<T, ClientError> {
    serde_json::from_value(row).map_err(|error| ClientError::Protocol(error.to_string()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    Pending,
    Fresh,
    Stale,
}

/// Failure is independent of freshness: transport loss is stale, not an authorization error.
#[derive(Clone, Debug, PartialEq)]
pub enum FollowError {
    Subscription {
        code: Option<ErrorCode>,
        message: String,
    },
    Protocol(String),
}

#[derive(Debug)]
pub struct Window<T> {
    /// Server order, not arrival order. Values are shared across receiver clones.
    pub items: Vec<Arc<T>>,
    pub snapshot: Snapshot,
    pub has_more: bool,
}

#[derive(Debug)]
pub struct FollowState<T> {
    pub value: Option<Arc<Window<T>>>,
    pub freshness: Freshness,
    pub error: Option<FollowError>,
}

/// A latest-value stream: slow consumers coalesce intermediate states, never queue deltas.
/// Clones share a subscription. Dropping its last receiver releases the server slot.
pub struct FollowReceiver<T: Send + Sync + 'static> {
    retained: watch::Receiver<Arc<FollowState<T>>>,
    stream: WatchStream<Arc<FollowState<T>>>,
    dropped: NotifyDrop,
}
impl<T: Send + Sync + 'static> FollowReceiver<T> {
    pub fn current(&self) -> Arc<FollowState<T>> {
        self.retained.borrow().clone()
    }
}
impl<T: Send + Sync + 'static> Clone for FollowReceiver<T> {
    fn clone(&self) -> Self {
        Self {
            retained: self.retained.clone(),
            stream: WatchStream::new(self.retained.clone()),
            dropped: self.dropped.clone(),
        }
    }
}
// Declared after both receivers so notification occurs after their counts fall.
#[derive(Clone)]
struct NotifyDrop(Arc<Notify>);
impl Drop for NotifyDrop {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
impl<T: Send + Sync + 'static> Stream for FollowReceiver<T> {
    type Item = Arc<FollowState<T>>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

/// Clone this handle to compose models on the same socket and reconnect loop.
#[derive(Clone)]
pub struct FollowClient {
    commands: mpsc::Sender<Add>,
    dropped: Arc<Notify>,
}

impl FollowClient {
    pub async fn connect(client: Client) -> Result<Self, ClientError> {
        let socket = client.collection_stream().await?;
        let (commands, receiver) = mpsc::channel(MAX_WATCHES);
        let dropped = Arc::new(Notify::new());
        tokio::spawn(run(
            client,
            Connection::new(socket),
            receiver,
            dropped.clone(),
        ));
        Ok(Self { commands, dropped })
    }

    /// The bounded window limit is inclusive, from 1 through 200; at most 16 watches
    /// are admitted on a connection. A receiver clone does not consume another slot.
    pub async fn watch<D: Descriptor>(
        &self,
        descriptor: D,
        limit: usize,
    ) -> Result<FollowReceiver<D::Row>, ClientError> {
        if !(1..=MAX_ROWS).contains(&limit) {
            return Err(ClientError::Protocol(
                "follow limit must be between 1 and 200".into(),
            ));
        }
        let selector = descriptor.selector();
        if !selector.is_object() || selector.get("collection").and_then(Value::as_str).is_none() {
            return Err(ClientError::Protocol(
                "follow descriptor must select a collection".into(),
            ));
        }
        // Count serialized bytes without allocating a second command buffer.
        serde_json::to_writer(&mut CommandSize(0), &selector).map_err(|_| {
            ClientError::Protocol("follow selector exceeds its command byte bound".into())
        })?;
        let (sender, retained) = watch::channel(Arc::new(FollowState {
            value: None,
            freshness: Freshness::Pending,
            error: None,
        }));
        // Install cleanup before either await: cancellation can happen after admission
        // but before this future observes the receipt. Field drop order closes both
        // receivers before waking the worker to release the abandoned subscription.
        let receiver = FollowReceiver {
            stream: WatchStream::new(retained.clone()),
            retained,
            dropped: NotifyDrop(self.dropped.clone()),
        };
        let (accepted, receipt) = oneshot::channel();
        self.commands
            .send(Add {
                slot: Box::new(TypedSlot {
                    descriptor,
                    sender,
                    rows: HashMap::new(),
                    limit,
                }),
                selector,
                accepted,
            })
            .await
            .map_err(|_| ClientError::Transport("follow connection closed".into()))?;
        receipt
            .await
            .map_err(|_| ClientError::Transport("follow connection closed".into()))??;
        Ok(receiver)
    }
}

struct Add {
    slot: Box<dyn Slot>,
    selector: Value,
    accepted: oneshot::Sender<Result<(), ClientError>>,
}
struct Entry {
    slot: Box<dyn Slot>,
    selector: Value,
    wire_id: String,
}
trait Slot: Send + Sync {
    fn closed(&self) -> bool;
    fn stale(&mut self);
    fn frame(&mut self, frame: Value) -> Result<bool, ClientError>;
    fn protocol_error(&mut self, message: String);
    fn limit(&self) -> usize;
}
struct TypedSlot<D: Descriptor> {
    descriptor: D,
    sender: watch::Sender<Arc<FollowState<D::Row>>>,
    rows: HashMap<String, Arc<D::Row>>,
    limit: usize,
}
impl<D: Descriptor> TypedSlot<D> {
    fn publish(&self, freshness: Freshness, error: Option<FollowError>) {
        self.sender.send_modify(|state| {
            *state = Arc::new(FollowState {
                value: state.value.clone(),
                freshness,
                error,
            })
        });
    }
}
impl<D: Descriptor> Slot for TypedSlot<D> {
    fn limit(&self) -> usize {
        self.limit
    }
    fn closed(&self) -> bool {
        self.sender.is_closed()
    }
    fn stale(&mut self) {
        if self.sender.borrow().freshness == Freshness::Stale {
            return;
        }
        let error = self.sender.borrow().error.clone();
        self.publish(Freshness::Stale, error);
    }
    fn protocol_error(&mut self, message: String) {
        self.publish(Freshness::Stale, Some(FollowError::Protocol(message)));
    }
    fn frame(&mut self, mut frame: Value) -> Result<bool, ClientError> {
        fn field<T: DeserializeOwned>(frame: &mut Value, name: &str) -> Result<T, ClientError> {
            decode_row(frame.get_mut(name).map(Value::take).unwrap_or(Value::Null))
        }
        match frame["kind"].as_str() {
            Some("resync") => {
                self.stale();
                Ok(true)
            }
            Some("error") => {
                let code = field(&mut frame, "code")?;
                let message = field(&mut frame, "message")?;
                self.publish(
                    Freshness::Stale,
                    Some(FollowError::Subscription { code, message }),
                );
                Ok(false)
            }
            Some("snapshot" | "changes") => {
                let replace = frame["kind"] == "snapshot";
                // Deltas cannot establish freshness after a reconnect or resync.
                if !replace && self.sender.borrow().freshness != Freshness::Fresh {
                    return Ok(false);
                }
                let snapshot = field(&mut frame, "snapshot")?;
                let order: Vec<String> = field(&mut frame, "order")?;
                let has_more = field(&mut frame, "has_more")?;
                let raw: Vec<Value> = field(&mut frame, if replace { "items" } else { "upserts" })?;
                if order.len() > self.limit || raw.len() > MAX_ROWS {
                    return Err(ClientError::Protocol(
                        "follow window exceeds its row bound".into(),
                    ));
                }
                let mut updates = HashMap::with_capacity(raw.len());
                for row in raw {
                    let id = row["id"]
                        .as_str()
                        .ok_or_else(|| ClientError::Protocol("follow row has no id".into()))?
                        .to_owned();
                    if updates.contains_key(&id) {
                        return Err(ClientError::Protocol("duplicate follow row id".into()));
                    }
                    updates.insert(id, Arc::new(self.descriptor.decode(row)?));
                }
                let removes: Vec<String> = if replace {
                    Vec::new()
                } else {
                    field(&mut frame, "removes")?
                };
                let removed: HashSet<_> = removes.iter().collect();
                let mut seen = HashSet::with_capacity(order.len());
                let mut rows = HashMap::with_capacity(order.len());
                let mut items = Vec::with_capacity(order.len());
                for id in order {
                    if !seen.insert(id.clone()) {
                        return Err(ClientError::Protocol("duplicate follow order id".into()));
                    }
                    let row = updates
                        .remove(&id)
                        .or_else(|| {
                            (!replace && !removed.contains(&id))
                                .then(|| self.rows.get(&id).cloned())
                                .flatten()
                        })
                        .ok_or_else(|| {
                            ClientError::Protocol(format!(
                                "follow order references missing row {id}"
                            ))
                        })?;
                    items.push(row.clone());
                    rows.insert(id, row);
                }
                self.rows = rows;
                self.sender.send_replace(Arc::new(FollowState {
                    value: Some(Arc::new(Window {
                        items,
                        snapshot,
                        has_more,
                    })),
                    freshness: Freshness::Fresh,
                    error: None,
                }));
                Ok(false)
            }
            _ => Err(ClientError::Protocol("unexpected follow frame kind".into())),
        }
    }
}

// Reserve envelope space for generated subscription IDs, kind and limit.
struct CommandSize(usize);
impl std::io::Write for CommandSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        if self.0 > MAX_COMMAND_BYTES - 256 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "command too large",
            ));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn subscribe(entry: &Entry) -> Value {
    let mut command = entry.selector.clone();
    command["kind"] = "subscribe".into();
    command["id"] = entry.wire_id.clone().into();
    command["limit"] = entry.slot.limit().into();
    command
}

type Write = futures_util::future::BoxFuture<'static, (CollectionStream, Result<(), ClientError>)>;

/// Own the socket across a send without awaiting it outside the lifecycle select.
/// Cancelling this future drops the socket, including its buffered write.
struct Connection {
    socket: Option<CollectionStream>,
    writing: Option<Write>,
    outgoing: VecDeque<Value>,
    cleanup_pending: usize,
    writing_cleanup: bool,
    write_deadline: Option<tokio::time::Instant>,
}

impl Connection {
    fn new(socket: CollectionStream) -> Self {
        Self {
            socket: Some(socket),
            writing: None,
            outgoing: VecDeque::new(),
            cleanup_pending: 0,
            writing_cleanup: false,
            write_deadline: None,
        }
    }
    fn connected(&self) -> bool {
        self.socket.is_some() || self.writing.is_some()
    }
    fn reset(&mut self) {
        self.writing = None;
        self.socket = None;
        self.outgoing.clear();
        self.cleanup_pending = 0;
        self.writing_cleanup = false;
        self.write_deadline = None;
    }
    fn enqueue(&mut self, command: Value) {
        if !self.connected() {
            return;
        }
        if self.outgoing.len() == MAX_PENDING_WRITES {
            // A slow peer cannot grow pending writes or obstruct local lifecycle.
            // Reconnect reconciles the retained subscriptions instead of replaying
            // an arbitrarily long command queue.
            self.reset();
        } else {
            if command["kind"] == "unsubscribe" {
                self.cleanup_pending += 1;
            }
            self.outgoing.push_back(command);
        }
    }
    fn start_write(&mut self) {
        if self.socket.is_some() && !self.outgoing.is_empty() {
            let mut socket = self.socket.take().unwrap();
            let command = self.outgoing.pop_front().unwrap();
            self.writing_cleanup = command["kind"] == "unsubscribe";
            self.write_deadline = Some(tokio::time::Instant::now() + SEND_DEADLINE);
            self.writing = Some(Box::pin(async move {
                let sent = socket.send(&command).await;
                (socket, sent)
            }));
        }
    }
    fn finish_write(&mut self, socket: CollectionStream, sent: Result<(), ClientError>) {
        self.writing = None;
        self.write_deadline = None;
        if sent.is_ok() {
            if self.writing_cleanup {
                self.cleanup_pending -= 1;
            }
            self.writing_cleanup = false;
            self.socket = Some(socket);
        } else {
            self.reset();
        }
    }
}

async fn run(
    client: Client,
    mut connection: Connection,
    mut commands: mpsc::Receiver<Add>,
    dropped: Arc<Notify>,
) {
    let mut entries: HashMap<u64, Entry> = HashMap::new();
    let mut routes = HashMap::new();
    let mut next_id = 0_u64;
    // Every resubscribe fences publications from older generations.
    let mut generation = 0_u64;
    loop {
        // Preserve a completed send before deciding whether lifecycle cleanup must
        // cancel one that is genuinely backpressured.
        if let Some(write) = &mut connection.writing
            && let Poll::Ready((socket, sent)) = futures_util::poll!(write.as_mut())
        {
            connection.finish_write(socket, sent);
        }
        let closed: Vec<_> = entries
            .iter()
            .filter_map(|(id, entry)| entry.slot.closed().then_some(*id))
            .collect();
        if !closed.is_empty() && connection.writing.is_some() {
            // A dropped watcher must not wait behind a backpressured send.
            // Closing the socket releases every old subscription at the peer.
            connection.reset();
        }
        for id in closed {
            let entry = entries.remove(&id).unwrap();
            routes.remove(&entry.wire_id);
            connection.enqueue(serde_json::json!({"kind":"unsubscribe", "id":entry.wire_id}));
        }
        if !connection.connected() {
            for entry in entries.values_mut() {
                entry.slot.stale();
            }
        }
        connection.start_write();
        tokio::select! {
            _ = dropped.notified() => {
                // Cleanup remains cancellable after its entry leaves the routing
                // table, until the unsubscribe itself has actually flushed.
                if connection.cleanup_pending != 0 && connection.writing.is_some() {
                    connection.reset();
                }
            },
            _ = async { tokio::time::sleep_until(connection.write_deadline.unwrap()).await },
                if connection.write_deadline.is_some() => connection.reset(),
            add = commands.recv() => {
                // The last client handle is the connection's shutdown authority.
                // Drop, rather than flush/close-await, a potentially blocked socket.
                let Some(add) = add else { break; };
                if entries.len() == MAX_WATCHES {
                    let _ = add.accepted.send(Err(ClientError::Protocol("follow connection has 16 subscriptions".into())));
                    continue;
                }
                next_id += 1;
                generation += 1;
                let entry = Entry { slot: add.slot, selector: add.selector, wire_id: format!("follow-{next_id}-{generation}") };
                connection.enqueue(subscribe(&entry));
                routes.insert(entry.wire_id.clone(), next_id);
                entries.insert(next_id, entry);
                let _ = add.accepted.send(Ok(()));
            },
            (socket, sent) = async { connection.writing.as_mut().unwrap().await }, if connection.writing.is_some() => {
                connection.finish_write(socket, sent);
            },
            frame = async { connection.socket.as_mut().unwrap().next().await }, if connection.socket.is_some() => {
                match frame {
                    Ok(Some(frame)) => {
                        let Some(id) = frame["id"].as_str().and_then(|id| routes.get(id)).copied() else { continue; };
                        let entry = entries.get_mut(&id).unwrap();
                        match entry.slot.frame(frame) {
                            Ok(true) => {
                                routes.remove(&entry.wire_id);
                                connection.enqueue(serde_json::json!({"kind":"unsubscribe", "id":entry.wire_id}));
                                generation += 1;
                                entry.wire_id = format!("follow-{id}-{generation}");
                                routes.insert(entry.wire_id.clone(), id);
                                connection.enqueue(subscribe(entry));
                            }
                            Ok(false) => {},
                            Err(error) => entry.slot.protocol_error(error.to_string()),
                        }
                    }
                    Ok(None) | Err(_) => connection.reset(),
                }
            },
            connected = async {
                tokio::time::sleep(RECONNECT_DELAY).await;
                client.collection_stream().await
            }, if !connection.connected() => {
                if let Ok(active) = connected {
                    connection = Connection::new(active);
                    routes.clear();
                    for (id, entry) in &mut entries {
                        generation += 1;
                        entry.wire_id = format!("follow-{id}-{generation}");
                        routes.insert(entry.wire_id.clone(), *id);
                        connection.enqueue(subscribe(entry));
                    }
                }
            },
        }
    }
    // No async close handshake: shutdown cannot depend on peer read progress.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pending_watch_commands_apply_backpressure_at_capacity() {
        // Keep the real command channel unserviced to deterministically saturate
        // admission, independently of WebSocket buffer sizes and scheduler timing.
        let (commands, mut pending_commands) = mpsc::channel(MAX_WATCHES);
        let client = FollowClient {
            commands,
            dropped: Arc::new(Notify::new()),
        };
        assert!(
            client
                .watch(
                    AttentionDescriptor {
                        person: Some("x".repeat(MAX_COMMAND_BYTES))
                    },
                    1
                )
                .await
                .is_err()
        );
        assert_eq!(pending_commands.len(), 0);
        let mut pending = Vec::new();
        for _ in 0..=MAX_WATCHES {
            let mut request = Box::pin(client.watch(AttentionDescriptor::default(), 1));
            assert!(futures_util::poll!(request.as_mut()).is_pending());
            pending.push(request);
        }
        assert_eq!(pending_commands.len(), MAX_WATCHES);
        assert_eq!(client.commands.capacity(), 0);
        let admitted = pending_commands.recv().await.unwrap();
        // The extra request cannot enqueue until the worker consumes a slot.
        assert_eq!(pending_commands.len(), MAX_WATCHES - 1);
        assert!(futures_util::poll!(pending[MAX_WATCHES].as_mut()).is_pending());
        assert_eq!(pending_commands.len(), MAX_WATCHES);
        assert_eq!(client.commands.capacity(), 0);
        assert!(admitted.accepted.send(Ok(())).is_ok());
        assert!(futures_util::poll!(pending[0].as_mut()).is_ready());
    }

    async fn connection_with_filler() -> (
        Connection,
        tokio::net::UnixStream,
        std::os::unix::net::UnixStream,
    ) {
        use std::os::fd::AsRawFd;
        use tokio_tungstenite::tungstenite::protocol::Role;
        let (sender, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        sender.set_nonblocking(true).unwrap();
        peer.set_nonblocking(true).unwrap();
        let filler = sender.try_clone().unwrap();
        let send_bytes: libc::c_int = 1024;
        // SAFETY: sender owns a live socket fd; the option points to a correctly
        // sized initialized c_int for the duration of this synchronous syscall.
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    sender.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    std::ptr::from_ref(&send_bytes).cast(),
                    std::mem::size_of_val(&send_bytes) as libc::socklen_t,
                )
            },
            0
        );
        let sender = tokio::net::UnixStream::from_std(sender).unwrap();
        let peer = tokio::net::UnixStream::from_std(peer).unwrap();
        let socket = CollectionStream {
            socket: crate::TerminalSocket::Unix(
                tokio_tungstenite::WebSocketStream::from_raw_socket(sender, Role::Client, None)
                    .await,
            ),
            limit: MAX_COMMAND_BYTES,
            heard: crate::Heard::new(),
            subscribed: HashMap::new(),
        };
        (Connection::new(socket), peer, filler)
    }

    async fn backpressured_connection() -> (Connection, tokio::net::UnixStream) {
        let (mut connection, peer, _filler) = connection_with_filler().await;
        connection.enqueue(serde_json::json!({"kind":"subscribe", "id":"blocked", "collection":"attention", "person":"x".repeat(MAX_COMMAND_BYTES - 512)}));
        connection.start_write();
        // The peer deliberately never reads. Prove an actual pending socket
        // send, rather than relying on a sleep or guessing its buffer capacity.
        assert!(futures_util::poll!(connection.writing.as_mut().unwrap().as_mut()).is_pending());
        (connection, peer)
    }

    #[tokio::test]
    async fn unread_peer_cannot_strand_admission_drop_or_shutdown() {
        use futures_util::StreamExt;
        let (connection, peer) = backpressured_connection().await;
        let (commands, receiver) = mpsc::channel(MAX_WATCHES);
        let dropped = Arc::new(Notify::new());
        let client = FollowClient {
            commands,
            dropped: dropped.clone(),
        };
        let worker = tokio::spawn(run(
            Client::unix("unused-follow-test.sock"),
            connection,
            receiver,
            dropped,
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut watches = Vec::new();
            for _ in 0..MAX_WATCHES {
                watches.push(
                    client
                        .watch(AttentionDescriptor::default(), 1)
                        .await
                        .unwrap(),
                );
            }
            // Capacity errors do not wait behind the blocked send either.
            assert!(
                client
                    .watch(AttentionDescriptor::default(), 1)
                    .await
                    .is_err()
            );
            drop(watches.pop());
            // A drop cancels the blocked socket and marks remaining windows stale.
            loop {
                if watches[0].next().await.unwrap().freshness == Freshness::Stale {
                    break;
                }
            }
            drop(client);
            worker.await.unwrap();
            assert!(watches[0].next().await.is_none());
        })
        .await
        .unwrap();
        // Keep the peer alive and unread until every lifecycle assertion finishes.
        drop(peer);
    }

    #[tokio::test]
    async fn outgoing_write_queue_resets_at_its_bound() {
        let (mut connection, _peer) = backpressured_connection().await;
        for _ in 0..MAX_PENDING_WRITES {
            connection.enqueue(serde_json::json!({"kind":"unsubscribe", "id":"queued"}));
        }
        assert_eq!(connection.outgoing.len(), MAX_PENDING_WRITES);
        connection.enqueue(serde_json::json!({"kind":"unsubscribe", "id":"overflow"}));
        assert!(!connection.connected());
        assert!(connection.outgoing.is_empty());
    }

    #[tokio::test]
    async fn cleanup_started_unsubscribe_cannot_outlive_send_deadline() {
        use futures_util::StreamExt;
        use std::io::Write;
        use tokio_tungstenite::tungstenite::protocol::Role;
        let (connection, peer, mut filler) = connection_with_filler().await;
        let mut peer =
            tokio_tungstenite::WebSocketStream::from_raw_socket(peer, Role::Server, None).await;
        let (commands, receiver) = mpsc::channel(MAX_WATCHES);
        let dropped = Arc::new(Notify::new());
        let client = FollowClient {
            commands,
            dropped: dropped.clone(),
        };
        let worker = tokio::spawn(run(
            Client::unix("unused-follow-test.sock"),
            connection,
            receiver,
            dropped,
        ));
        let first = client
            .watch(AttentionDescriptor::default(), 1)
            .await
            .unwrap();
        peer.next().await.unwrap().unwrap(); // subscribe has flushed
        let mut retained = client
            .watch(AttentionDescriptor::default(), 1)
            .await
            .unwrap();
        peer.next().await.unwrap().unwrap(); // second subscribe has flushed
        assert_eq!(retained.next().await.unwrap().freshness, Freshness::Pending);
        // Fill the same socket's send buffer while the worker has no active send.
        // After this point the peer stays alive but never reads, so cleanup's
        // unsubscribe (not an earlier subscribe) is the write that blocks.
        loop {
            match filler.write(&[0; 1024]) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("fill send buffer: {error}"),
            }
        }
        drop(filler);
        drop(first);
        tokio::time::timeout(SEND_DEADLINE + Duration::from_secs(1), async {
            loop {
                if retained.next().await.unwrap().freshness == Freshness::Stale {
                    break;
                }
            }
        })
        .await
        .expect("cleanup must reset its blocked unsubscribe socket");
        // The client remains live throughout cleanup; shutdown is not what
        // released the server-side subscription.
        drop(client);
        worker.await.unwrap();
        drop(peer);
    }

    #[tokio::test]
    async fn stuck_unsubscribe_keeps_cleanup_state_until_deadline_reset() {
        use futures_util::StreamExt;
        let (mut connection, peer, mut filler) = connection_with_filler().await;
        use std::io::Write;
        loop {
            match filler.write(&[0; 1024]) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("fill send buffer: {error}"),
            }
        }
        drop(filler);
        connection.enqueue(serde_json::json!({"kind":"unsubscribe", "id":"released"}));
        connection.start_write();
        assert!(futures_util::poll!(connection.writing.as_mut().unwrap().as_mut()).is_pending());
        assert_eq!(connection.cleanup_pending, 1);
        assert!(connection.writing_cleanup);
        let deadline = connection.write_deadline.unwrap();
        let (commands, receiver) = mpsc::channel(MAX_WATCHES);
        let dropped = Arc::new(Notify::new());
        let client = FollowClient {
            commands,
            dropped: dropped.clone(),
        };
        let worker = tokio::spawn(run(
            Client::unix("unused-follow-test.sock"),
            connection,
            receiver,
            dropped,
        ));
        let mut retained = client
            .watch(AttentionDescriptor::default(), 1)
            .await
            .unwrap();
        assert_eq!(retained.next().await.unwrap().freshness, Freshness::Pending);
        tokio::time::timeout(SEND_DEADLINE + Duration::from_secs(1), async {
            loop {
                if retained.next().await.unwrap().freshness == Freshness::Stale {
                    break;
                }
            }
        })
        .await
        .expect("send deadline resets socket without another lifecycle event");
        assert!(tokio::time::Instant::now() >= deadline);
        drop(client);
        worker.await.unwrap();
        drop(peer);
    }
}
