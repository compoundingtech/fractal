//! Host-local change cursors. Resets requeue the caller's complete host candidate set;
//! ordinary pages read indexed event keys, never rebuild message answers.
use super::*;
use crate::peer::MessageMarkEvents;
use crate::store::{message_marks, messages_shard};

#[derive(Default)]
struct Feed {
    state: OnceLock<FeedState>,
}

struct FeedState {
    epoch: String,
    changed: watch::Sender<u64>,
    _observers: Vec<smallclaims::sqlite::CommitObserver>,
    #[cfg(test)]
    evaluations: watch::Sender<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    epoch: String,
    marks: u64,
    scope: u64,
    admitted: u64,
    lifecycle: i64,
}

impl Feed {
    fn get(&self, store: &Store) -> &FeedState {
        self.state.get_or_init(|| {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let epoch = format!("{}-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            let (changed, _) = watch::channel(0);
            let source = changed.clone();
            let mut observers = vec![store.connection.observe_tables(&["local_message_mark_events", "local_message_mark_scope_events"], move |_| {
                source.send_modify(|state| *state += 1);
            })];
            if let Some(shard) = messages_shard::shard(store) {
                let source = changed.clone();
                observers.push(shard.connection.observe_tables(&["stream_lifecycle", "stream_messages"], move |_| {
                    source.send_modify(|state| *state += 1);
                }));
            }
            FeedState {
                epoch, changed, _observers: observers,
                #[cfg(test)]
                evaluations: watch::channel(0).0,
            }
        })
    }
}

fn retained(after: u64, oldest: u64, newest: u64) -> bool {
    after <= newest && if oldest == 0 { after == 0 } else { after >= oldest.saturating_sub(1) }
}

fn page(store: &Store, feed: &FeedState, cursor: Option<&str>, max: usize) -> anyhow::Result<MessageMarkEvents> {
    #[cfg(test)]
    feed.evaluations.send_modify(|count| *count += 1);
    let connection = store.readers.get();
    // Retention validation and page keys must share one snapshot: a concurrent trim
    // must not turn a validated cursor into a silently truncated page.
    let connection = connection.unchecked_transaction()?;
    let newest = message_marks::event_position(&connection)?;
    let oldest: u64 = connection.prepare_cached("SELECT COALESCE(MIN(id),0) FROM local_message_mark_events")?.query_row(/* sqlite-large-value: allow-for-now lv-h834172c1c620 owner=fractal_api_core::api::seat_freshness::message_mark_events TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?;
    let (scope_oldest, scope_newest): (u64, u64) = connection.prepare_cached("SELECT (SELECT COALESCE(MIN(id),0) FROM local_message_mark_scope_events), (SELECT COALESCE(MAX(id),0) FROM local_message_mark_scope_events)")?.query_row(/* sqlite-large-value: allow-for-now lv-hae39a164406d owner=fractal_api_core::api::seat_freshness::message_mark_events TODO=trace-statement-and-value-representation */ [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let stream = messages_shard::mailbox_reads::position(store)?;
    let previous = cursor.and_then(|cursor| serde_json::from_str::<Cursor>(cursor).ok());
    let valid = previous.as_ref().is_some_and(|cursor| cursor.epoch == feed.epoch
        && retained(cursor.marks, oldest, newest) && retained(cursor.scope, scope_oldest, scope_newest)
        && cursor.admitted <= stream.admitted && cursor.lifecycle <= stream.lifecycle);
    let mut messages = BTreeSet::new();
    let mut consumers = BTreeSet::new();
    let (next, has_more) = if valid {
        let mut next = previous.expect("validated cursor");
        let mut remaining = max;
        let events = message_marks::events_after(&connection, next.marks, remaining)?;
        remaining -= events.len();
        for (id, message, _) in events { next.marks = id; messages.insert(message); }
        if remaining > 0 {
            let events = connection.prepare_cached("SELECT id,message,consumer FROM local_message_mark_scope_events WHERE id>?1 ORDER BY id LIMIT ?2")?
                .query_map(rusqlite::params![next.scope, remaining], |row| Ok((row.get::<_, u64>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            remaining -= events.len();
            for (id, message, consumer) in events {
                next.scope = id;
                if let Some(message) = message { messages.insert(message); }
                if let Some(consumer) = consumer { consumers.insert(consumer); }
            }
        }
        if let Some(shard) = messages_shard::shard(store) {
            let reader = shard.readers.get();
            if remaining > 0 {
                let events = reader.prepare_cached("SELECT store_index,subject FROM stream_messages WHERE store_index>?1 ORDER BY store_index LIMIT ?2")?
                    .query_map(rusqlite::params![next.admitted, remaining], |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                remaining -= events.len();
                for (id, message) in events { next.admitted = id; messages.insert(message); }
            }
            if remaining > 0 {
                let events = reader.prepare_cached("SELECT rowid,subject FROM stream_lifecycle WHERE rowid>?1 ORDER BY rowid LIMIT ?2")?
                    .query_map(rusqlite::params![next.lifecycle, remaining], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for (id, message) in events { next.lifecycle = id; messages.insert(message); }
            }
        }
        let more = next.marks < newest || next.scope < scope_newest || next.admitted < stream.admitted || next.lifecycle < stream.lifecycle;
        (next, more)
    } else {
        (Cursor { epoch: feed.epoch.clone(), marks: newest, scope: scope_newest, admitted: stream.admitted, lifecycle: stream.lifecycle }, false)
    };
    Ok(MessageMarkEvents { epoch: feed.epoch.clone(), cursor: serde_json::to_string(&next)?, messages: messages.into_iter().collect(), consumers: consumers.into_iter().collect(), reset: !valid, has_more })
}

pub(super) async fn read(store: &Arc<Store>, cursor: Option<&str>, max: usize) -> Result<Value, ApiError> {
    let feed = store.extension::<Feed>();
    let feed_state = feed.get(store);
    let mut changed = feed_state.changed.subscribe();
    let deadline = tokio::time::sleep(Duration::from_secs(25));
    tokio::pin!(deadline);
    loop {
        changed.borrow_and_update();
        let current = {
            let store = Arc::clone(store);
            let feed = Arc::clone(&feed);
            let cursor = cursor.map(str::to_owned);
            blocking_store(move || page(&store, feed.get(&store), cursor.as_deref(), max)).await?
        };
        if current.reset || !current.messages.is_empty() || !current.consumers.is_empty() || current.has_more {
            return serde_json::to_value(current).map_err(|error| validation(error.to_string()));
        }
        tokio::select! {
            _ = changed.changed() => {},
            _ = &mut deadline => return serde_json::to_value(current).map_err(|error| validation(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shard_local_close_emits_only_its_message_key() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "owner").unwrap());
        let shard = messages_shard::open(&store, root.path()).unwrap();
        shard.append_claim(&fractal_wire::ClaimInput {
            subject: "message/stream-event".into(),
            kind: "message.sent".into(),
            actor: Some("agent/sender".into()),
            fields: serde_json::from_value(json!({
                "from":"agent/sender", "to":"agent/reader", "content":"stream note", "status":"sent"
            })).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
        let feed = Feed::default();
        let feed = feed.get(&store);
        let initial = page(&store, feed, None, 10).unwrap();
        let mut changed = feed.changed.subscribe();
        changed.borrow_and_update();
        messages_shard::record_admitted_lifecycle(
            &store, "message/stream-event", "message.closed", Some("agent/reader"),
            &BTreeMap::from([("status".into(), json!("closed"))]),
        ).unwrap();
        assert!(changed.has_changed().unwrap());
        let closed = page(&store, feed, Some(&initial.cursor), 10).unwrap();
        assert!(!closed.reset);
        assert_eq!(closed.messages, ["message/stream-event"]);
        assert!(closed.consumers.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn idle_keepalive_performs_one_key_read_and_no_timeout_rescan() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&root.path().join("idle.sqlite3"), "owner").unwrap());
        let feed = store.extension::<Feed>();
        let initial = page(&store, feed.get(&store), None, 10).unwrap();
        let mut evaluations = feed.get(&store).evaluations.subscribe();
        let before = *evaluations.borrow_and_update();
        let cursor = initial.cursor.clone();
        let request_store = Arc::clone(&store);
        let waiting = tokio::spawn(async move { read(&request_store, Some(&cursor), 10).await.unwrap() });
        evaluations.changed().await.unwrap();
        tokio::time::advance(Duration::from_secs(25)).await;
        let response: MessageMarkEvents = serde_json::from_value(waiting.await.unwrap()).unwrap();
        assert_eq!(*evaluations.borrow(), before + 1);
        assert!(!response.reset);
        assert!(response.messages.is_empty());
        assert!(response.consumers.is_empty());
        assert_eq!(response.cursor, initial.cursor);
    }

    use super::*;

    #[test]
    fn indexed_pages_are_bounded_and_trim_resets() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("events.sqlite3"), "owner").unwrap();
        let feed = Feed::default();
        let feed = feed.get(&store);
        let initial = page(&store, feed, None, 1).unwrap();
        assert!(initial.reset);
        assert!(initial.messages.is_empty());
        {
            let writer = store.connection.write();
            writer.execute("INSERT INTO local_message_mark_events(message,consumer) VALUES('message/a','agent/reader'),('message/b','agent/reader')", []).unwrap();
        }
        let first = page(&store, feed, Some(&initial.cursor), 1).unwrap();
        assert_eq!(first.messages, ["message/a"]);
        assert!(first.has_more);
        let second = page(&store, feed, Some(&first.cursor), 1).unwrap();
        assert_eq!(second.messages, ["message/b"]);
        assert!(!second.has_more);
        let unchanged = page(&store, feed, Some(&second.cursor), 1).unwrap();
        assert!(!unchanged.reset);
        assert!(unchanged.messages.is_empty());
        assert_eq!(unchanged.cursor, second.cursor);
        store.connection.write().execute("DELETE FROM local_message_mark_events WHERE id=1", []).unwrap();
        assert!(page(&store, feed, Some(&initial.cursor), 1).unwrap().reset);
        assert!(page(&store, feed, Some("invalid"), 1).unwrap().reset);
    }

    #[test]
    fn scope_event_ids_remain_monotonic_after_retention_and_reopen_without_a_sequence_row() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("scope-retention.sqlite3");
        let newest = {
            let store = Store::open(&path, "owner").unwrap();
            store.connection.write().execute_batch(
                "WITH RECURSIVE events(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM events WHERE id<16390)
                 INSERT INTO local_message_mark_scope_events(consumer) SELECT 'agent/changed' FROM events"
            ).unwrap();
            let reader = store.readers.get();
            let (count, oldest, newest): (u64, u64, u64) = reader.query_row(
                "SELECT COUNT(*),MIN(id),MAX(id) FROM local_message_mark_scope_events", [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).unwrap();
            assert_eq!((count, oldest, newest), (16384, 7, 16390));
            let sequences: u64 = reader.query_row(
                "SELECT COUNT(*) FROM sqlite_sequence WHERE name='local_message_mark_scope_events'",
                [], |row| row.get(0),
            ).unwrap();
            assert_eq!(sequences, 0, "scope events must not add shared sequence-scan work");
            newest
        };
        let store = Store::open(&path, "owner").unwrap();
        store.connection.write().execute(
            "INSERT INTO local_message_mark_scope_events(consumer) VALUES('agent/next')", [],
        ).unwrap();
        let next: u64 = store.readers.get().query_row(
            "SELECT MAX(id) FROM local_message_mark_scope_events", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(next, newest + 1);
    }

    #[test]
    fn unrelated_commits_do_not_wake_and_scope_keys_stay_precise() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("scope.sqlite3"), "owner").unwrap();
        // Schema changes invalidate all dependencies; create the fixture before observing DML.
        store.connection.write().execute_batch("CREATE TABLE unrelated(value TEXT)").unwrap();
        let feed = Feed::default();
        let feed = feed.get(&store);
        let initial = page(&store, feed, None, 10).unwrap();
        let mut changed = feed.changed.subscribe();
        changed.borrow_and_update();
        store.connection.write().execute("INSERT INTO unrelated VALUES('quiet')", []).unwrap();
        assert!(!changed.has_changed().unwrap());
        store.connection.write().execute("INSERT INTO local_message_mark_scope_events(consumer) VALUES('agent/changed')", []).unwrap();
        assert!(changed.has_changed().unwrap());
        let result = page(&store, feed, Some(&initial.cursor), 10).unwrap();
        assert!(!result.reset);
        assert!(result.messages.is_empty());
        assert_eq!(result.consumers, ["agent/changed"]);
    }
}

