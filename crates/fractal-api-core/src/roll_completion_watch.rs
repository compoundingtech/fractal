//! Admit replicated signed roll receipts reactively, never from a synchronous SLO read.
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{Connection, params};
use tokio::sync::Notify;

use crate::roll_completion::{self, SignedReceipt};
use crate::store::Store;

const PREFIX: &str = "resource/fleet/roll-completion/";
// The next byte after the trailing slash bounds the reserved subject prefix.
const PREFIX_END: &str = "resource/fleet/roll-completion0";

#[derive(Default)]
struct Started(OnceLock<()>);

#[cfg(test)]
#[derive(Default)]
struct Refreshed(Notify);

#[derive(Debug, PartialEq, Eq)]
struct Candidate {
    id: String,
    index: u64,
    rank: u8,
    source_index: Option<u64>,
    accepted_at: u128,
    writer_sequence: Option<u64>,
}

fn scan(connection: &Connection, after: u64, through: u64) -> Result<Vec<Candidate>> {
    let mut query = connection.prepare_cached(
        "SELECT id, store_index,
            CASE json_extract(body,'$.fields.facts.kind')
                WHEN 'source-enabled' THEN 0 WHEN 'roll-declared' THEN 0 WHEN 'host-target-resolved' THEN 1
                WHEN 'host-observed' THEN 2 WHEN 'intent-observed' THEN 2 WHEN 'source-frontier' THEN 3 ELSE 4 END,
            CASE WHEN json_extract(body,'$.fields.facts.kind')='source-frontier'
                THEN CAST(json_extract(body,'$.fields.facts.source_index') AS TEXT) END,
            accepted_at_unix_ms,
            CASE WHEN json_extract(body,'$.fields.facts.kind')='source-frontier' THEN
                (SELECT MIN(sequence) FROM replica_records
                 WHERE claim_id=claims.id AND writer=claims.origin AND state='valid') END
         FROM claims WHERE store_index>?1 AND store_index<=?2
           AND subject>=?3 AND subject<?4 AND kind='resource.observed'
         ORDER BY store_index",
    )?;
    Ok(query.query_map(params![after, through, PREFIX, PREFIX_END], |row| {
        Ok(Candidate { id: row.get(0)?, index: row.get(1)?, rank: row.get(2)?,
            source_index: row.get::<_, Option<String>>(3)?.and_then(|value| value.parse().ok()),
            accepted_at: row.get::<_, String>(4)?.parse().unwrap_or(u128::MAX),
            writer_sequence: row.get(5)? })
    })?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn order(candidates: &mut [Candidate]) {
    // Canonical original-writer sequence orders epochs even with tied clocks
    // and reset local cursors. Core independently verifies this provenance and
    // refuses any late obsolete epoch; this ordering grants no authority.
    candidates.sort_by(|left, right|
        (left.rank, left.writer_sequence.unwrap_or(u64::MAX), if left.rank == 3 { left.accepted_at } else { 0 }, left.source_index.unwrap_or(left.index), left.index, &left.id)
            .cmp(&(right.rank, right.writer_sequence.unwrap_or(u64::MAX), if right.rank == 3 { right.accepted_at } else { 0 }, right.source_index.unwrap_or(right.index), right.index, &right.id)));
}

#[derive(Default)]
struct Frontier {
    through: u64,
    pending: Vec<Candidate>,
    observed_resolutions: BTreeSet<String>,
    authority: Option<Vec<String>>,
}

impl Frontier {
    fn refresh(&mut self, store: &Store) -> Result<()> {
        // Ordinary receipt/heartbeat commits are incremental. Only canonical
        // authority/membership/config changes invalidate retained signed proofs.
        let authority = roll_completion::authority_cut(store)?;
        if self.authority.as_ref() != Some(&authority) {
            roll_completion::rebuild(store)?;
            self.authority = Some(authority);
        }
        let through = store.index()?;
        let fresh = scan(&store.readers.get(), self.through, through)?;
        // A failed publication can itself append an unsigned observation.
        // Retry on later config/signature/dependency commits, not that append.
        let retry_collection = fresh.is_empty() || fresh.iter().any(|candidate| candidate.rank != 2);
        self.pending.extend(fresh);
        self.through = through;
        order(&mut self.pending);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().min(u128::from(u64::MAX)) as u64;
        let mut pending = Vec::new();
        for candidate in self.pending.drain(..) {
            match roll_completion::ingest(store, SignedReceipt { claim_id: candidate.id.clone() }, now) {
                Ok(_) => {
                    if candidate.rank == 1
                        && let Ok(resolved) = roll_completion::resolved_target(store, &candidate.id)
                        && resolved.host == store.origin()
                        && !self.observed_resolutions.contains(&candidate.id)
                    {
                        if !retry_collection {
                            pending.push(candidate);
                            continue;
                        }
                        match crate::roll_observer::observe(store, &candidate.id) {
                            Ok(_) => { self.observed_resolutions.insert(candidate.id); }
                            Err(error) => {
                                tracing::warn!(claim_id = %candidate.id, %error, "local roll observation failed");
                                pending.push(candidate);
                            }
                        }
                    }
                }
                Err(error) => match roll_completion::dependencies_pending(store, &candidate.id) {
                    Ok(true) => pending.push(candidate),
                    Ok(false) => tracing::warn!(claim_id = %candidate.id, error = %error, "rejecting replicated roll receipt"),
                    Err(dependency_error) => {
                        tracing::warn!(claim_id = %candidate.id, error = %dependency_error, "cannot inspect roll receipt dependencies");
                        pending.push(candidate);
                    }
                },
            }
        }
        self.pending = pending;
        Ok(())
    }
}

/// Start once, after the signed receipt registry is installed. Subscribe before
/// replay so a commit between the initial index and the wait cannot be lost.
/// The callback only signals; all graph reads and admission happen off writer.
pub fn start(store: &Arc<Store>) {
    if store.extension::<Started>().0.set(()).is_err() { return; }
    let wake = Arc::new(Notify::new());
    let committed = Arc::clone(&wake);
    let observer = store.observe_commits(move |_| committed.notify_one());
    #[cfg(test)]
    let refreshed = store.extension::<Refreshed>();
    let store = Arc::downgrade(store);
    tokio::spawn(async move {
        let _observer = observer;
        let mut frontier = Frontier::default();
        let mut heartbeat = crate::roll_source::Heartbeat::default();
        loop {
            let Some(store) = store.upgrade() else { return; };
            match tokio::task::spawn_blocking(move || {
                let result = frontier.refresh(&store).and_then(|()| heartbeat.refresh(&store));
                (frontier, heartbeat, result)
            }).await {
                Ok((next, next_heartbeat, result)) => {
                    frontier = next;
                    heartbeat = next_heartbeat;
                    if let Err(error) = result {
                        tracing::warn!(%error, "roll receipt frontier refresh failed");
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "roll receipt frontier worker failed");
                    return;
                }
            }
            #[cfg(test)]
            refreshed.0.notify_one();
            // Commits resolve dependencies; a quiet enabled source also needs
            // a real native census heartbeat, never a wall-now read fallback.
            match heartbeat.deadline() {
                Some(deadline) => tokio::select! {
                    _ = wake.notified() => {}
                    _ = tokio::time::sleep_until(deadline.into()) => {}
                },
                None => wake.notified().await,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE claims (id TEXT, store_index INTEGER PRIMARY KEY, subject TEXT, kind TEXT, body TEXT, accepted_at_unix_ms TEXT NOT NULL DEFAULT '1', origin TEXT NOT NULL DEFAULT 'host');
            CREATE TABLE replica_records(claim_id TEXT,writer TEXT,state TEXT,sequence INTEGER);").unwrap();
        connection
    }

    fn insert(connection: &Connection, index: u64, subject: &str, kind: &str, receipt_kind: &str) {
        connection.execute("INSERT INTO claims(id,store_index,subject,kind,body) VALUES (?1,?2,?3,?4,?5)", params![
            format!("claim-{index}"), index, subject, kind,
            serde_json::json!({"fields":{"facts":{"kind":receipt_kind}}}).to_string(),
        ]).unwrap();
    }

    #[test]
    fn locally_published_heartbeat_chain_never_accumulates_pending_frontiers() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("host-a").unwrap();
        roll_completion::install(&store, root.path()).unwrap();
        roll_completion::tests::native_source_fixture(&store);
        let mut frontier = Frontier::default();
        let mut previous: Option<String> = None;
        for _ in 0..4 {
            // A fresh scheduler makes the heartbeat immediately due without
            // sleeping or changing the native envelope clock.
            let before = store.index().unwrap();
            crate::roll_source::Heartbeat::default().refresh(&store).unwrap();
            let candidates = scan(&store.readers.get(), before, store.index().unwrap()).unwrap();
            let published = candidates.iter().find(|candidate| candidate.rank == 3)
                .expect("quiet enrolled source must publish a native frontier");
            if let Some(previous) = previous {
                assert!(!roll_completion::dependencies_pending(&store, &previous).unwrap());
            }
            previous = Some(published.id.clone());
            frontier.refresh(&store).unwrap();
            assert!(frontier.pending.is_empty(), "local publication retries and superseded frontiers are terminal");
            frontier.refresh(&store).unwrap();
            assert!(frontier.pending.is_empty());
        }
    }

    #[test]
    fn initial_cut_is_topological_and_incremental_cut_excludes_old_and_unrelated_claims() {
        let connection = database();
        insert(&connection, 1, &format!("{PREFIX}roll/host/1"), "resource.observed", "host-observed");
        insert(&connection, 2, &format!("{PREFIX}roll/host/resolution"), "resource.observed", "host-target-resolved");
        insert(&connection, 3, &format!("{PREFIX}roll/declaration"), "resource.observed", "roll-declared");
        insert(&connection, 4, "resource/other/roll/declaration", "resource.observed", "roll-declared");
        insert(&connection, 5, &format!("{PREFIX}roll/other"), "note", "roll-declared");
        let mut first = scan(&connection, 0, 5).unwrap();
        order(&mut first);
        assert_eq!(first.iter().map(|candidate| candidate.index).collect::<Vec<_>>(), vec![3, 2, 1]);
        insert(&connection, 6, &format!("{PREFIX}roll/host/2"), "resource.observed", "host-observed");
        assert_eq!(scan(&connection, 5, 6).unwrap().iter().map(|candidate| candidate.index).collect::<Vec<_>>(), vec![6]);
        assert!(scan(&connection, 6, 6).unwrap().is_empty());
    }

    #[test]
    fn newly_arrived_dependencies_sort_before_pending_observations() {
        let mut pending = vec![Candidate { id: "observation".into(), index: 1, rank: 2, source_index: None, accepted_at: 1, writer_sequence: None }];
        pending.extend([
            Candidate { id: "resolution".into(), index: 3, rank: 1, source_index: None, accepted_at: 1, writer_sequence: None },
            Candidate { id: "declaration".into(), index: 4, rank: 0, source_index: None, accepted_at: 1, writer_sequence: None },
        ]);
        order(&mut pending);
        assert_eq!(pending.iter().map(|candidate| candidate.id.as_str()).collect::<Vec<_>>(), vec!["declaration", "resolution", "observation"]);
    }

    #[test]
    fn source_frontier_waits_for_declarations_and_native_intent_onsets_even_when_it_arrives_first() {
        let connection = database();
        insert(&connection, 1, &format!("{PREFIX}producer/frontier"), "resource.observed", "source-frontier");
        insert(&connection, 2, &format!("{PREFIX}roll/host/1"), "resource.observed", "host-observed");
        insert(&connection, 3, &format!("{PREFIX}roll/host/resolution"), "resource.observed", "host-target-resolved");
        insert(&connection, 4, &format!("{PREFIX}roll/declaration"), "resource.observed", "roll-declared");
        insert(&connection, 5, &format!("{PREFIX}roll/intents/host/native-run"), "resource.observed", "intent-observed");
        let mut candidates = scan(&connection, 0, 5).unwrap();
        order(&mut candidates);
        assert_eq!(candidates.iter().map(|candidate| candidate.index).collect::<Vec<_>>(), vec![4, 3, 2, 5, 1]);
        assert_eq!(candidates.last().unwrap().rank, 3);
    }

    #[test]
    fn reverse_arriving_frontier_chain_is_topological_in_one_capture() {
        let connection = database();
        // Store arrival order is the exact reverse of this native source chain.
        for (arrival, source_index, previous) in [(1, 300, Some("claim-2")), (2, 200, Some("claim-3")), (3, 100, None)] {
            connection.execute("INSERT INTO claims(id,store_index,subject,kind,body) VALUES (?1,?2,?3,'resource.observed',?4)", params![
                format!("claim-{arrival}"), arrival, format!("{PREFIX}sources/producer/host/frontier"),
                serde_json::json!({"fields":{"facts":{"kind":"source-frontier",
                    "source_index":source_index,"previous_frontier_claim_id":previous}}}).to_string(),
            ]).unwrap();
        }
        let mut candidates = scan(&connection, 0, 3).unwrap();
        order(&mut candidates);
        assert_eq!(candidates.iter().map(|candidate| candidate.source_index.unwrap()).collect::<Vec<_>>(), vec![100, 200, 300]);
        // One admission pass can find every predecessor already accepted.
        let mut accepted = BTreeSet::new();
        for candidate in candidates {
            let previous: Option<String> = connection.query_row(
                "SELECT json_extract(body,'$.fields.facts.previous_frontier_claim_id') FROM claims WHERE id=?1",
                [&candidate.id], |row| row.get(0),
            ).unwrap();
            assert!(previous.is_none_or(|id| accepted.contains(&id)));
            accepted.insert(candidate.id);
        }
        assert_eq!(accepted.len(), 3);
    }

    #[test]
    fn restore_low_cursor_follows_old_high_cursor_by_native_writer_sequence_with_tied_clocks() {
        let mut candidates = vec![
            Candidate { id: "restored".into(), index: 1, rank: 3, source_index: Some(10), accepted_at: 100, writer_sequence: Some(20) },
            Candidate { id: "old".into(), index: 2, rank: 3, source_index: Some(100), accepted_at: 100, writer_sequence: Some(10) },
        ];
        order(&mut candidates);
        assert_eq!(candidates.iter().map(|candidate| candidate.id.as_str()).collect::<Vec<_>>(), vec!["old", "restored"]);
    }

    #[tokio::test]
    async fn signed_observation_is_admitted_reactively_after_its_dependencies_arrive() {
        use crate::roll_completion::{Adoption, Identity, Receipt, Verification};
        use crate::roll_completion::tests::{publish_fixture, setup_signer};
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("host-a").unwrap());
        setup_signer(&store);
        roll_completion::install(&store, root.path()).unwrap();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let target = Identity { dotfiles_revision: "a".repeat(40),
            daemon_revision: "b".repeat(40), system_path: None, home_path: None };
        // This simulates normal graph arrival, not an explicit SLO ingestion call.
        let observation = publish_fixture(&store, Receipt::HostObserved {
            version: 1, roll_id: "reactive".into(), declared_at_ms: now - 100,
            host: "host-a".into(), observed_at_ms: now, target: target.clone(),
            adoption: Adoption::Verified, evidence: Some(Box::new(Verification {
                system_path: "/nix/store/system".into(), home_path: "/nix/store/home".into(),
                expected_system_path: "/nix/store/system".into(), expected_home_path: "/nix/store/home".into(),
                daemon_revision: target.daemon_revision.clone(), daemon_ready: true,
            })),
        }, None);
        assert!(roll_completion::dependencies_pending(&store, &observation).unwrap());
        let refreshed = store.extension::<Refreshed>();
        start(&store);
        tokio::time::timeout(std::time::Duration::from_secs(10), refreshed.0.notified()).await.unwrap();
        assert!(roll_completion::dependencies_pending(&store, &observation).unwrap());
        let declaration = publish_fixture(&store, Receipt::RollDeclared {
            version: 1, roll_id: "reactive".into(), declared_at_ms: now - 100,
            targets: [("host-a".into(), target.clone())].into(),
        }, Some("agent/example/roll-authority"));
        publish_fixture(&store, Receipt::HostTargetResolved {
            version: 1, roll_id: "reactive".into(), declared_at_ms: now - 100,
            host: "host-a".into(), target, expected_system_path: "/nix/store/system".into(),
            expected_home_path: "/nix/store/home".into(), declaration_claim_id: declaration,
        }, Some("agent/example/roll-authority"));
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let row = roll_completion::row(&store, now + 1);
                if row["measurement"]["windows"][0]["eligible"] == 1.0 {
                    assert_eq!(row["measurement"]["windows"][0]["bad"], 0.0);
                    break;
                }
                refreshed.0.notified().await;
            }
        }).await.unwrap();
        // Remove the source authority after a valid result was cached. A
        // subsequent commit must remove good coverage without daemon restart.
        let source = "version 2\nagent \"example/roll-authority\" {\n command \"true\"\n host \"host-a\"\n}\n".to_owned();
        let intent = crate::graph::parse_intent(&source, store.origin()).unwrap();
        let preview = store.mission(&intent, crate::model::IntentInput { kdl: source, source_name: None }).unwrap();
        store.apply(&intent, &preview.subject_tokens, &intent.source_hash).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if roll_completion::row(&store, now + 1)["measurement"]["windows"][0]["eligible"] == 0.0 { break; }
                refreshed.0.notified().await;
            }
        }).await.unwrap();
        setup_signer(&store);
        assert!(store.declared_resource_referrers("resource/fleet/roll-completion").unwrap().iter().any(|binding|
            binding["owner"] == "agent/example/roll-authority" && binding["name"] == "roll-authority"),
            "restoring authority must publish a new desired token, not replay the first apply");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if roll_completion::row(&store, now + 1)["measurement"]["windows"][0]["eligible"] == 1.0 { break; }
                refreshed.0.notified().await;
            }
        }).await.unwrap();
    }

    #[test]
    fn newly_admitted_local_resolution_collects_once_without_an_observation_claim_storm() {
        use crate::roll_completion::{Identity, Receipt};
        use crate::roll_completion::tests::publish_fixture;
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("host-a").unwrap();
        roll_completion::install(&store, root.path()).unwrap();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let target = Identity { dotfiles_revision: "a".repeat(40),
            daemon_revision: "b".repeat(40), system_path: None, home_path: None };
        let declaration = publish_fixture(&store, Receipt::RollDeclared {
            version: 1, roll_id: "automatic".into(), declared_at_ms: now - 100,
            targets: [("host-a".into(), target.clone())].into(),
        }, Some("agent/example/roll-authority"));
        publish_fixture(&store, Receipt::HostTargetResolved {
            version: 1, roll_id: "automatic".into(), declared_at_ms: now - 100,
            host: "host-a".into(), target, expected_system_path: "/nix/store/system".into(),
            expected_home_path: "/nix/store/home".into(), declaration_claim_id: declaration,
        }, Some("agent/example/roll-authority"));
        let mut frontier = Frontier::default();
        frontier.refresh(&store).unwrap();
        let count = || store.readers.get().query_row(
            "SELECT COUNT(*) FROM claims WHERE subject>=?1 AND subject<?2 AND json_extract(body,'$.fields.facts.kind')='host-observed'",
            params![PREFIX, PREFIX_END], |row| row.get::<_, u64>(0),
        ).unwrap();
        assert_eq!(count(), 1);
        frontier.refresh(&store).unwrap();
        frontier.refresh(&store).unwrap();
        assert_eq!(count(), 1);
        // A daemon restart's initial replay measures once again, even though
        // the original resolution was already admitted to the signed ledger.
        Frontier::default().refresh(&store).unwrap();
        assert_eq!(count(), 2);
    }
}
