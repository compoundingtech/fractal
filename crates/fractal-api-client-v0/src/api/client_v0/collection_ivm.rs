//! The part of `api::client_v0::collection_ivm` that moved up from fractal-api-core.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::EvidenceStore;
use super::*;
use crate::graph_watch_ivm::{IvmViewBridge, ViewPage};
use smallclaims::ivm::{Readiness, events};
use std::cell::RefCell;
pub use fractal_api_core::api::client_v0::collection_ivm::*;
pub use fractal_api_core::api::client_v0::collection_ivm::{Adapter, Candidate, Delivered, Held, Output, RowReader, Sources, Window, read};

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use smallclaims::ivm::{Definition, LocalChange, View, Views};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio_tungstenite::tungstenite::Message;

    // Complete local-only transport fixture. No production source certificate is asserted.
    struct FixtureView;

    impl View for FixtureView {
        fn definition(&self) -> Definition {
            Definition {
                name: "fixture.socket",
                fingerprint: "fixture.socket.v1",
                kinds: &[],
                local_kinds: &["fixture.replace"],
                max_contributions: 1,
            }
        }
        fn contributions(
            &self,
            _: &smallclaims::ClaimRecord,
            _: &smallclaims::store::canonical::ClaimKey,
        ) -> anyhow::Result<Vec<smallclaims::ivm::Contribution>> {
            Ok(vec![])
        }
        fn create_schema(&self, connection: &rusqlite::Connection) -> anyhow::Result<()> {
            connection.execute_batch("CREATE TABLE IF NOT EXISTS fixture_rows(id TEXT PRIMARY KEY, owner TEXT NOT NULL, rank INTEGER NOT NULL, value TEXT NOT NULL); CREATE INDEX IF NOT EXISTS fixture_rows_rank ON fixture_rows(owner,rank,id); CREATE TABLE IF NOT EXISTS fixture_output(id TEXT PRIMARY KEY, value TEXT NOT NULL)")?;
            Ok(())
        }
        fn maintain_local_key(
            &self,
            tx: &rusqlite::Transaction<'_>,
            key: &str,
            _: &LocalChange,
        ) -> anyhow::Result<bool> {
            use rusqlite::OptionalExtension;
            let old: Option<String> = tx
                .query_row(
                    "SELECT value FROM fixture_output WHERE id=?1",
                    [key],
                    |row| row.get(0),
                )
                .optional()?;
            let new: Option<String> = tx
                .query_row(
                    "SELECT json_array(owner,rank,value) FROM fixture_rows WHERE id=?1",
                    [key],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(value) = &new {
                tx.execute("INSERT INTO fixture_output VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET value=excluded.value", rusqlite::params![key,value])?;
            } else {
                tx.execute("DELETE FROM fixture_output WHERE id=?1", [key])?;
            }
            Ok(old != new)
        }
    }

    fn replace(store: &Store, views: &Views, id: &str, rank: i64, value: Option<&str>) {
        store.connection.batched(|tx| {
            if let Some(value) = value {
                tx.execute("INSERT INTO fixture_rows VALUES(?1,'person/avery',?2,?3) ON CONFLICT(id) DO UPDATE SET rank=excluded.rank,value=excluded.value", rusqlite::params![id,rank,value])?;
            } else { tx.execute("DELETE FROM fixture_rows WHERE id=?1", [id])?; }
            let mut cut = smallclaims::ivm::source_cut(tx)?.unwrap();
            cut.local_generation += 1;
            views.local_change(tx, &LocalChange {kind:"fixture.replace".into(),old_keys:BTreeSet::from([id.into()]),new_keys:BTreeSet::from([id.into()]),evaluation_time_unix_ms:0}, cut)?;
            Ok::<_, anyhow::Error>(())
        }).unwrap().unwrap();
    }

    async fn frame(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Value {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
            {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(_) => socket.flush().await.unwrap(),
                other => panic!("unexpected frame {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn ivm_socket_upsert_remove_reorder_silence_resync_and_reconnect() {
        let root = tempfile::tempdir().unwrap();
        let views = Arc::new(Views::new(vec![Box::new(FixtureView)]).unwrap());
        let mut state = super::super::tests::test_state_named(root.path(), "alder");
        state.store = Arc::new(
            Store::open_with_ivm_views(&root.path().join("ivm.db"), "alder", views.clone())
                .unwrap(),
        );
        let store = state.store.clone();

        replace(&store, &views, "row/a", 1, Some("a1"));
        replace(&store, &views, "row/b", 2, Some("b1"));
        let rows = Arc::new(AtomicUsize::new(0));
        let observed = rows.clone();
        let adapter = Arc::new(Adapter {
            view: "fixture.socket",
            rows: Arc::new(move |_, session, request, conn, _, _, _| {
                rows.fetch_add(1, Ordering::SeqCst);
                let mut stmt=conn.prepare("SELECT id,value FROM fixture_rows WHERE owner=?1 ORDER BY rank,id LIMIT ?2").map_err(ApiError::internal)?;
                let items=stmt.query_map(rusqlite::params![session.authority_actor,request.limit.unwrap_or(2)+1],|row| Ok(json!({"id":row.get::<_,String>(0)?,"value":row.get::<_,String>(1)?}))).map_err(ApiError::internal)?.collect::<Result<Vec<_>,_>>().map_err(ApiError::internal)?;
                let has_more = items.len() > request.limit.unwrap_or(2);
                Ok((
                    items.into_iter().take(request.limit.unwrap_or(2)).collect(),
                    has_more,
                ))
            }),
        });
        let sources =
            Sources::from_store(store.clone(), BTreeMap::from([("work".into(), adapter)]))
                .unwrap()
                .unwrap();
        let app = axum::Router::new().route(
            "/stream",
            axum::routing::get(move |upgrade: WebSocketUpgrade| {
                let (state, sources) = (state.clone(), sources.clone());
                async move {
                    upgrade.on_upgrade(move |socket| {
                        collection_stream_socket_with_sources(
                            socket,
                            state,
                            ClientSession::local(Some("person/avery")).unwrap(),
                            None,
                            Some(sources),
                            |_, _, _, _| async {
                                panic!("legacy fold must never run for this source")
                            },
                        )
                    })
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("ws://{address}/stream");
        let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let subscribe =
            json!({"kind":"subscribe","id":"held","collection":"work","limit":2}).to_string();
        socket
            .send(Message::Text(subscribe.clone().into()))
            .await
            .unwrap();
        assert_eq!(frame(&mut socket).await["order"], json!(["row/a", "row/b"]));
        replace(&store, &views, "row/a", 3, Some("a2"));
        let change = frame(&mut socket).await;
        assert_eq!(change["kind"], "changes");
        assert_eq!(change["order"], json!(["row/b", "row/a"]));
        assert_eq!(change["upserts"], json!([{"id":"row/a","value":"a2"}]));
        replace(&store, &views, "row/b", 2, None);
        assert_eq!(frame(&mut socket).await["removes"], json!(["row/b"]));
        let before = observed.load(Ordering::SeqCst);
        replace(&store, &views, "row/a", 3, Some("a2"));
        assert!(
            tokio::time::timeout(Duration::from_millis(150), socket.next())
                .await
                .is_err()
        );
        assert_eq!(observed.load(Ordering::SeqCst), before);
        store
            .connection
            .batched(|tx| {
                let mut cut = smallclaims::ivm::source_cut(tx)?.unwrap();
                cut.local_generation += 1;
                views.local_change(
                    tx,
                    &LocalChange {
                        kind: "fixture.unrelated".into(),
                        old_keys: BTreeSet::new(),
                        new_keys: BTreeSet::new(),
                        evaluation_time_unix_ms: 0,
                    },
                    cut,
                )?;
                Ok::<_, anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(150), socket.next())
                .await
                .is_err()
        );
        assert_eq!(observed.load(Ordering::SeqCst), before);
        store
            .connection
            .batched(|tx| {
                tx.execute(
                    "UPDATE ivm_views SET ready=0 WHERE name='fixture.socket'",
                    [],
                )?;
                Ok::<_, anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(frame(&mut socket).await["kind"], "resync");
        let before_ready = observed.load(Ordering::SeqCst);
        store
            .connection
            .batched(|tx| {
                tx.execute(
                    "UPDATE ivm_views SET ready=1 WHERE name='fixture.socket'",
                    [],
                )?;
                Ok::<_, anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while observed.load(Ordering::SeqCst) == before_ready {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        // Ready with no changed key refreshes; identical rows still produce no delta.
        assert!(
            tokio::time::timeout(Duration::from_millis(150), socket.next())
                .await
                .is_err()
        );
        store
            .connection
            .batched(|tx| {
                smallclaims::ivm::events::rotate_identity(tx)?;
                Ok::<_, anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(frame(&mut socket).await["kind"], "resync");
        assert_eq!(frame(&mut socket).await["kind"], "snapshot");
        socket.close(None).await.unwrap();
        replace(&store, &views, "row/c", 0, Some("c1"));
        let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        socket.send(Message::Text(subscribe.into())).await.unwrap();
        let snapshot = frame(&mut socket).await;
        assert_eq!(snapshot["kind"], "snapshot");
        assert_eq!(snapshot["order"], json!(["row/c", "row/a"]));
        socket.close(None).await.unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn source_admission_gap_and_person_refusal_never_read_rows() {
        let root = tempfile::tempdir().unwrap();
        let views = Arc::new(Views::new(vec![Box::new(FixtureView)]).unwrap());
        let mut state = super::super::tests::test_state_named(root.path(), "alder");
        state.store = Arc::new(
            Store::open_with_ivm_views(&root.path().join("ivm.db"), "alder", views.clone())
                .unwrap(),
        );
        let count = Arc::new(AtomicUsize::new(0));
        let rows = count.clone();
        let adapter = Arc::new(Adapter {
            view: "fixture.socket",
            rows: Arc::new(move |_, _, _, _, _, _, _| {
                rows.fetch_add(1, Ordering::SeqCst);
                Ok((vec![], false))
            }),
        });
        let sources = Sources::from_store(
            state.store.clone(),
            BTreeMap::from([("attention".into(), adapter.clone())]),
        )
        .unwrap()
        .unwrap();
        let request:CollectionSubscribe=serde_json::from_value(json!({"kind":"subscribe","id":"held","collection":"attention","person":"person/intruder"})).unwrap();
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let result = read(
            state.clone(),
            ClientSession::local(Some("person/avery")).unwrap(),
            request,
            slots.clone().acquire_owned().await.unwrap(),
            sources.clone(),
            adapter.clone(),
            Held::default(),
        )
        .await;
        assert_eq!(result.err().unwrap().status, StatusCode::FORBIDDEN);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        // This deliberate bypass leaves a Ready registry at cut0 while the Store gains
        // an undispatched durable input. Registration is insufficient for a row read.
        state
            .store
            .connection
            .batched(|tx| {
                smallclaims::store::append_claim_record_tx(
                    tx,
                    "alder",
                    "source/uncovered",
                    "fixture.uncovered",
                    None,
                    &json!({}),
                    &[],
                    None,
                )?;
                Ok::<_, anyhow::Error>(())
            })
            .unwrap()
            .unwrap();
        let request = serde_json::from_value(
            json!({"kind":"subscribe","id":"held","collection":"attention"}),
        )
        .unwrap();
        let result = read(
            state,
            ClientSession::local(Some("person/avery")).unwrap(),
            request,
            slots.acquire_owned().await.unwrap(),
            sources,
            adapter,
            Held::default(),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
