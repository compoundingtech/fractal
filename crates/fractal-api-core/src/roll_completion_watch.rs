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
}

fn scan(connection: &Connection, after: u64, through: u64) -> Result<Vec<Candidate>> {
    let mut query = connection.prepare_cached(
        "SELECT id, store_index,
            CASE json_extract(body,'$.fields.facts.kind')
                WHEN 'roll-declared' THEN 0 WHEN 'host-target-resolved' THEN 1
                WHEN 'host-observed' THEN 2 ELSE 3 END
         FROM claims WHERE store_index>?1 AND store_index<=?2
           AND subject>=?3 AND subject<?4 AND kind='resource.observed'
         ORDER BY store_index",
    )?;
    Ok(query.query_map(params![after, through, PREFIX, PREFIX_END], |row| {
        Ok(Candidate { id: row.get(0)?, index: row.get(1)?, rank: row.get(2)? })
    })?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn order(candidates: &mut [Candidate]) {
    // Causal classes first, then stable graph order within a class. This handles
    // reverse arrival of declaration/resolution/observation in the initial cut.
    candidates.sort_by(|left, right| (left.rank, left.index, &left.id).cmp(&(right.rank, right.index, &right.id)));
}

#[derive(Default)]
struct Frontier {
    through: u64,
    pending: Vec<Candidate>,
    observed_resolutions: BTreeSet<String>,
}

impl Frontier {
    fn refresh(&mut self, store: &Store) -> Result<()> {
        // Revalidate retained signed ledger references on every coalesced commit:
        // signature/verdict/trust changes can be SQL-only (no new claim index).
        // Cost is O(retained admitted receipt refs), not O(all graph history).
        // Revoked authority must remove cached good outcomes without a restart.
        roll_completion::rebuild(store)?;
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
                        match crate::roll_observer::observe(store, &candidate.id, now) {
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
        loop {
            let Some(store) = store.upgrade() else { return; };
            match tokio::task::spawn_blocking(move || {
                let result = frontier.refresh(&store);
                (frontier, result)
            }).await {
                Ok((next, result)) => {
                    frontier = next;
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
            // No timer polling: only commits can satisfy a missing dependency.
            wake.notified().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE claims (id TEXT, store_index INTEGER PRIMARY KEY, subject TEXT, kind TEXT, body TEXT);").unwrap();
        connection
    }

    fn insert(connection: &Connection, index: u64, subject: &str, kind: &str, receipt_kind: &str) {
        connection.execute("INSERT INTO claims VALUES (?1,?2,?3,?4,?5)", params![
            format!("claim-{index}"), index, subject, kind,
            serde_json::json!({"fields":{"facts":{"kind":receipt_kind}}}).to_string(),
        ]).unwrap();
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
        let mut pending = vec![Candidate { id: "observation".into(), index: 1, rank: 2 }];
        pending.extend([
            Candidate { id: "resolution".into(), index: 3, rank: 1 },
            Candidate { id: "declaration".into(), index: 4, rank: 0 },
        ]);
        order(&mut pending);
        assert_eq!(pending.iter().map(|candidate| candidate.id.as_str()).collect::<Vec<_>>(), vec!["declaration", "resolution", "observation"]);
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
        let target = Identity { dotfiles_revision: "a".repeat(40), factory_revision: None,
            daemon_revision: "b".repeat(40), system_path: None, home_path: None };
        // This simulates normal graph arrival, not an explicit SLO ingestion call.
        let observation = publish_fixture(&store, Receipt::HostObserved {
            version: 1, roll_id: "reactive".into(), declared_at_ms: now - 100,
            host: "host-a".into(), observed_at_ms: now, target: target.clone(),
            adoption: Adoption::Verified, evidence: Some(Box::new(Verification {
                system_path: "/nix/store/system".into(), home_path: "/nix/store/home".into(),
                expected_system_path: "/nix/store/system".into(), expected_home_path: "/nix/store/home".into(),
                daemon_revision: target.daemon_revision.clone(), factory_revision: None, daemon_ready: true,
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
        }, Some("agent/roll-authority"));
        publish_fixture(&store, Receipt::HostTargetResolved {
            version: 1, roll_id: "reactive".into(), declared_at_ms: now - 100,
            host: "host-a".into(), target, expected_system_path: "/nix/store/system".into(),
            expected_home_path: "/nix/store/home".into(), declaration_claim_id: declaration,
        }, Some("agent/roll-authority"));
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
        let source = "version 2\nagent \"roll-authority\" {\n command \"true\"\n host \"host-a\"\n}\n".to_owned();
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
        let target = Identity { dotfiles_revision: "a".repeat(40), factory_revision: None,
            daemon_revision: "b".repeat(40), system_path: None, home_path: None };
        let declaration = publish_fixture(&store, Receipt::RollDeclared {
            version: 1, roll_id: "automatic".into(), declared_at_ms: now - 100,
            targets: [("host-a".into(), target.clone())].into(),
        }, Some("agent/roll-authority"));
        publish_fixture(&store, Receipt::HostTargetResolved {
            version: 1, roll_id: "automatic".into(), declared_at_ms: now - 100,
            host: "host-a".into(), target, expected_system_path: "/nix/store/system".into(),
            expected_home_path: "/nix/store/home".into(), declaration_claim_id: declaration,
        }, Some("agent/roll-authority"));
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
