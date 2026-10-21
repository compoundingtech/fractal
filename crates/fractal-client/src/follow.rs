//! Retained, typed collection windows multiplexed over one reconnecting socket.
use std::{
    collections::{HashMap, HashSet},
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
const RECONNECT_DELAY: Duration = Duration::from_millis(250);

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
    pub actor: Option<String>,
    pub status: Option<String>,
}

impl Descriptor for AttentionDescriptor {
    type Row = Attention;
    fn selector(&self) -> Value {
        serde_json::json!({"collection":"attention", "actor":self.actor, "status":self.status})
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
        tokio::spawn(run(client, socket, receiver, dropped.clone()));
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

async fn subscribe(socket: &mut CollectionStream, entry: &Entry) -> Result<(), ClientError> {
    let mut command = entry.selector.clone();
    command["kind"] = "subscribe".into();
    command["id"] = entry.wire_id.clone().into();
    command["limit"] = entry.slot.limit().into();
    socket.send(&command).await
}

async fn run(
    client: Client,
    socket: CollectionStream,
    mut commands: mpsc::Receiver<Add>,
    dropped: Arc<Notify>,
) {
    let mut socket = Some(socket);
    let mut entries: HashMap<u64, Entry> = HashMap::new();
    let mut routes = HashMap::new();
    let mut next_id = 0_u64;
    // A new wire ID on every resubscribe fences queued frames from older generations,
    // including servers whose protocol has no explicit generation field.
    let mut generation = 0_u64;
    let mut handles_closed = false;
    loop {
        let closed: Vec<_> = entries
            .iter()
            .filter_map(|(id, entry)| entry.slot.closed().then_some(*id))
            .collect();
        for id in closed {
            let entry = entries.remove(&id).unwrap();
            routes.remove(&entry.wire_id);
            if let Some(active) = &mut socket
                && active.unsubscribe(&entry.wire_id).await.is_err()
            {
                socket = None;
            }
        }
        if handles_closed && entries.is_empty() {
            break;
        }
        if socket.is_none() {
            for entry in entries.values_mut() {
                entry.slot.stale();
            }
        }
        tokio::select! {
            _ = dropped.notified() => {},
            add = commands.recv(), if !handles_closed => {
                let Some(add) = add else { handles_closed = true; continue; };
                if entries.len() == MAX_WATCHES {
                    let _ = add.accepted.send(Err(ClientError::Protocol("follow connection has 16 subscriptions".into())));
                    continue;
                }
                next_id += 1;
                generation += 1;
                let entry = Entry { slot: add.slot, selector: add.selector, wire_id: format!("follow-{next_id}-{generation}") };
                if let Some(active) = &mut socket
                    && subscribe(active, &entry).await.is_err() { socket = None; }
                routes.insert(entry.wire_id.clone(), next_id);
                entries.insert(next_id, entry);
                let _ = add.accepted.send(Ok(()));
            },
            frame = async { socket.as_mut().unwrap().next().await }, if socket.is_some() => {
                match frame {
                    Ok(Some(frame)) => {
                        let Some(id) = frame["id"].as_str().and_then(|id| routes.get(id)).copied() else { continue; };
                        let entry = entries.get_mut(&id).unwrap();
                        match entry.slot.frame(frame) {
                            Ok(true) => {
                                routes.remove(&entry.wire_id);
                                if socket.as_mut().unwrap().unsubscribe(&entry.wire_id).await.is_err() {
                                    socket = None;
                                    continue;
                                }
                                generation += 1;
                                entry.wire_id = format!("follow-{id}-{generation}");
                                routes.insert(entry.wire_id.clone(), id);
                                if subscribe(socket.as_mut().unwrap(), entry).await.is_err() { socket = None; }
                            }
                            Ok(false) => {},
                            Err(error) => entry.slot.protocol_error(error.to_string()),
                        }
                    }
                    Ok(None) | Err(_) => socket = None,
                }
            },
            connected = async {
                tokio::time::sleep(RECONNECT_DELAY).await;
                client.collection_stream().await
            }, if socket.is_none() => {
                if let Ok(mut active) = connected {
                    routes.clear();
                    let mut failed = false;
                    for (id, entry) in &mut entries {
                        generation += 1;
                        entry.wire_id = format!("follow-{id}-{generation}");
                        routes.insert(entry.wire_id.clone(), *id);
                        if subscribe(&mut active, entry).await.is_err() { failed = true; break; }
                    }
                    if !failed { socket = Some(active); }
                }
            },
        }
    }
    if let Some(active) = socket {
        active.close().await;
    }
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
}
