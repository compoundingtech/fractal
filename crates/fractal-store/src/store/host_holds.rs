//! Host-owned durable admission predicates. These records never own lane capacity.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostHold {
    pub id: String,
    pub host: String,
    pub revision: u64,
    pub owner: String,
    pub reason: String,
    pub operations: Vec<String>,
    pub originating_run: Option<String>,
    pub receipt: Option<String>,
    pub created_at_unix_ms: u64,
    pub released_at_unix_ms: Option<u64>,
    pub resolution: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceHostHold {
    pub actor: String,
    pub host: String,
    pub reason: String,
    pub operations: Vec<String>,
    #[serde(default)]
    pub originating_run: Option<String>,
    #[serde(default)]
    pub receipt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseHostHold {
    pub actor: String,
    pub host: String,
    pub id: String,
    pub expected_revision: u64,
    pub resolution: String,
}

/// Operator recovery is distinct from an owner's ordinary resolution.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverHostHold {
    pub actor: String,
    pub host: String,
    pub id: String,
    pub expected_revision: u64,
    pub reason: String,
}

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-ha677d62abc9d owner=fractal_store::store::host_holds TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_host_holds (
        id TEXT PRIMARY KEY, host TEXT NOT NULL, owner TEXT NOT NULL,
        revision INTEGER NOT NULL, body TEXT NOT NULL, released INTEGER NOT NULL DEFAULT 0
    ); CREATE INDEX IF NOT EXISTS local_host_holds_active ON local_host_holds(host,released,id);")?;
    Ok(())
}

pub fn dependency(host: &str) -> String { format!("host-holds/{host}") }

fn list_tx(connection: &Connection, host: Option<&str>, include_released: bool) -> Result<Vec<HostHold>, FractalError> {
    smallclaims::touched::note_read(|| host.map_or_else(|| "kind:host-hold.changed".into(), dependency));
    let mut statement = connection.prepare_cached(
        "SELECT body FROM local_host_holds WHERE (?1 IS NULL OR host=?1) AND (?2 OR released=0) ORDER BY id"
    ).map_err(internal)?;
    statement.query_map(params![host, include_released], |row| row.get::<_, String>(0))
        .map_err(internal)?.map(|row| serde_json::from_str(&row.map_err(internal)?).map_err(internal)).collect()
}

/// Read only under the owning host writer transaction when deciding admission.
pub fn blocking_tx(connection: &Connection, host: &str, operation: &str) -> Result<Vec<HostHold>, FractalError> {
    Ok(list_tx(connection, Some(host), false)?.into_iter().filter(|hold|
        hold.operations.iter().any(|affected| affected == "*" || affected == operation)
    ).collect())
}

fn authenticate(actor: &str, requested: &str) -> Result<(), FractalError> {
    if actor != requested || !(actor.starts_with("person/") || actor.starts_with("agent/") || actor.starts_with("mission-run/")) {
        return Err(FractalError::new("host-hold-owner", "host hold mutations require the authenticated owner"));
    }
    Ok(())
}

/// Commit through the existing local observation feed, not diagnostic touched writes alone.
/// Incremental::observe maps this subject to the exact admission read dependency.
fn changed_tx(transaction: &Transaction<'_>, origin: &str, hold: &HostHold) -> Result<(), FractalError> {
    insert_local_observation_tx(transaction, origin, &ClaimInput {
        subject: dependency(&hold.host),
        kind: "host-hold.changed".into(),
        actor: Some(hold.owner.clone()),
        fields: BTreeMap::from([
            ("hold".into(), json!(hold.id)),
            ("revision".into(), json!(hold.revision)),
            ("released".into(), json!(hold.released_at_unix_ms.is_some())),
        ]),
        evidence: Vec::new(),
        idempotency_key: None,
        expected_subject: None,
    }, now_ms())?;
    Ok(())
}

impl Store {
    pub fn host_holds(&self, host: Option<&str>, include_released: bool) -> Result<Vec<HostHold>, FractalError> {
        list_tx(&self.readers.get(), host, include_released)
    }

    /// Shared attention/now source; origin runs are links, never lifecycle owners.
    pub fn host_hold_attention_items(&self, person: Option<&str>) -> Result<Vec<AttentionItemView>> {
        self.host_holds(None, false)?.into_iter()
            .filter(|hold| person.is_none_or(|person| person == hold.owner))
            .map(|hold| {
                let mut targets = vec![format!("host/{}", hold.host)];
                targets.extend(hold.originating_run.clone());
                targets.extend(hold.receipt.clone());
                Ok(AttentionItemView {
                    deadline_unix_ms: None, default_answer: None,
                    episode: format!("{}@{}", hold.id, hold.revision), priority: "high".into(),
                    kind: "host-hold".into(), review_mode: None, subject: hold.id.clone(),
                    person: hold.owner.clone(), requester_id: Some(hold.owner.clone()),
                    launch_id: None, variant_id: None, message_id: None, prompt_call: None,
                    answers: Vec::new(), conversation: None,
                    title: format!("{} admission held", hold.host), detail: hold.reason.clone(),
                    mission: None, mission_run: None, step: None, targets,
                    requested_at_unix_ms: u128::from(hold.created_at_unix_ms),
                    actions: vec![attention_action("release", &["fractal", "hosts", "release",
                        &hold.host, &hold.id, "--expected-revision", &hold.revision.to_string(),
                        "--resolution", "owner-confirmed", "--as", &hold.owner])],
                    request: None,
                })
            }).collect()
    }

    fn host_holds_changed(&self) {
        self.fractal.attention_list.forget();
        if let Some(wake) = self.fractal.attention_list.refresher.get() { wake.notify_one(); }
    }

    pub fn place_host_hold(&self, authenticated_actor: &str, request: &PlaceHostHold) -> Result<HostHold, FractalError> {
        authenticate(authenticated_actor, &request.actor)?;
        if request.host != self.origin {
            return Err(FractalError::new("host-hold-authority", "only the target host may place its holds"));
        }
        if request.reason.trim().is_empty() || request.operations.is_empty() || request.operations.iter().any(|op| op.trim().is_empty()) {
            return Err(FractalError::new("invalid-host-hold", "a hold needs a reason and affected operations"));
        }
        let hold = HostHold {
            id: format!("host-hold/{}", uuid::Uuid::new_v4()), host: request.host.clone(), revision: 1,
            owner: authenticated_actor.into(), reason: request.reason.clone(), operations: request.operations.clone(),
            originating_run: request.originating_run.clone(), receipt: request.receipt.clone(),
            created_at_unix_ms: u64::try_from(now_ms()).unwrap_or(u64::MAX), released_at_unix_ms: None, resolution: None,
        };
        let mut writer = self.connection.write();
        let transaction = writer.transaction().map_err(internal)?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h93ce54913c6d owner=fractal_store::store::host_holds TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_host_holds(id,host,owner,revision,body) VALUES(?1,?2,?3,?4,?5)",
            params![hold.id, hold.host, hold.owner, hold.revision, serde_json::to_string(&hold).map_err(internal)?]).map_err(internal)?;
        smallclaims::touched::note_wrote(|| dependency(&hold.host));
        smallclaims::touched::note_wrote(|| "host-holds".into());
        changed_tx(&transaction, &self.origin, &hold)?;
        transaction.commit().map_err(internal)?;
        drop(writer);
        self.host_holds_changed();
        self.observe_resource_holds(&hold.host, &hold.operations, None);
        Ok(hold)
    }

    pub fn release_host_hold(&self, authenticated_actor: &str, request: &ReleaseHostHold) -> Result<HostHold, FractalError> {
        self.resolve_host_hold(authenticated_actor, request, false)
    }

    pub fn recover_host_hold(&self, authenticated_actor: &str, configured_operator: &str, request: &RecoverHostHold) -> Result<HostHold, FractalError> {
        authenticate(authenticated_actor, &request.actor)?;
        if authenticated_actor != configured_operator || !configured_operator.starts_with("person/") {
            return Err(FractalError::new("host-hold-recovery-authority", "only the configured host operator may recover a hold"));
        }
        self.resolve_host_hold(authenticated_actor, &ReleaseHostHold {
            actor: request.actor.clone(), host: request.host.clone(), id: request.id.clone(),
            expected_revision: request.expected_revision, resolution: request.reason.clone(),
        }, true)
    }

    fn resolve_host_hold(&self, authenticated_actor: &str, request: &ReleaseHostHold, recovery: bool) -> Result<HostHold, FractalError> {
        authenticate(authenticated_actor, &request.actor)?;
        if request.host != self.origin {
            return Err(FractalError::new("host-hold-authority", "only the target host may release its holds"));
        }
        if request.resolution.trim().is_empty() {
            return Err(FractalError::new("invalid-host-hold", "release needs a resolution"));
        }
        let mut writer = self.connection.write();
        let transaction = writer.transaction().map_err(internal)?;
        let body: String = transaction.query_row("SELECT body FROM local_host_holds WHERE id=?1 AND host=?2",
            params![request.id, request.host], |row| row.get(0)).optional().map_err(internal)?
            .ok_or_else(|| FractalError::new("host-hold-not-found", "no such hold on this host"))?;
        let mut hold: HostHold = serde_json::from_str(&body).map_err(internal)?;
        if !recovery && hold.owner != authenticated_actor {
            return Err(FractalError::new("host-hold-owner", "only the authenticated hold owner may release it"));
        }
        if hold.revision != request.expected_revision || hold.released_at_unix_ms.is_some() {
            return Err(FractalError::new("host-hold-revision", "hold revision changed; read the hold before resolving it"));
        }
        if recovery {
            let owner_run = if hold.owner.starts_with("mission-run/") {
                Some(hold.owner.clone())
            } else {
                transaction.query_row("SELECT owner_run FROM desired WHERE subject=?1", [&hold.owner],
                    |row| row.get::<_, Option<String>>(0)).optional().map_err(internal)?.flatten()
            };
            let run = owner_run.ok_or_else(|| FractalError::new(
                "host-hold-recovery-owner", "only a run-owned hold can be recovered; other owners must release their holds"))?;
            let phase: Option<String> = transaction.query_row(
                "SELECT phase FROM mission_runs WHERE id=?1",
                [run.strip_prefix("mission-run/").unwrap_or(&run)], |row| row.get(0)).optional().map_err(internal)?;
            if phase.as_deref() != Some("terminal") {
                return Err(FractalError::new("host-hold-owner-live", "the owning run is not proven terminal; its owner must release the hold"));
            }
        }
        hold.revision += 1;
        hold.released_at_unix_ms = Some(u64::try_from(now_ms()).unwrap_or(u64::MAX));
        hold.resolution = Some(request.resolution.clone());
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h16424da0718d owner=fractal_store::store::host_holds TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_host_holds SET revision=?2,body=?3,released=1 WHERE id=?1",
            params![hold.id, hold.revision, serde_json::to_string(&hold).map_err(internal)?]).map_err(internal)?;
        smallclaims::touched::note_wrote(|| dependency(&hold.host));
        smallclaims::touched::note_wrote(|| "host-holds".into());
        changed_tx(&transaction, &self.origin, &hold)?;
        insert_local_observation_tx(&transaction, &self.origin, &ClaimInput {
            subject: hold.id.clone(),
            kind: if recovery { "host-hold.recovered" } else { "host-hold.released" }.into(),
            actor: Some(authenticated_actor.into()),
            fields: BTreeMap::from([
                ("original_owner".into(), json!(hold.owner)),
                ("host".into(), json!(hold.host)),
                ("expected_revision".into(), json!(request.expected_revision)),
                ("revision".into(), json!(hold.revision)),
                ("reason".into(), json!(request.resolution)),
            ]),
            evidence: Vec::new(), idempotency_key: None, expected_subject: None,
        }, now_ms())?;
        transaction.commit().map_err(internal)?;
        drop(writer);
        self.host_holds_changed();
        self.observe_resource_holds(&hold.host, &hold.operations, Some(std::time::Duration::from_millis(
            hold.released_at_unix_ms.unwrap_or(hold.created_at_unix_ms).saturating_sub(hold.created_at_unix_ms))));
        Ok(hold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> PlaceHostHold { PlaceHostHold { actor: "person/operator".into(), host: "host-a".into(), reason: "Investigate activation".into(), operations: vec!["activation".into()], originating_run: Some("mission-run/terminal".into()), receipt: Some("receipt/failure".into()) } }
    #[test]
    fn recovery_retains_unknown_run_and_non_run_owner_custody() {
        let store = Store::open_memory("host-a").unwrap();
        for actor in ["person/operator", "mission-run/missing"] {
            let mut placement = request(); placement.actor = actor.into();
            let hold = store.place_host_hold(actor, &placement).unwrap();
            assert!(store.recover_host_hold("person/operator", "person/operator", &RecoverHostHold {
                actor: "person/operator".into(), host: hold.host.clone(), id: hold.id.clone(),
                expected_revision: hold.revision, reason: "operator investigated".into(),
            }).is_err());
            assert!(store.host_holds(None, false).unwrap().contains(&hold));
        }
    }
    #[test]
    fn live_owner_release_and_terminal_recovery_are_revision_fenced_and_audited() {
        let store = Store::open_memory("host-a").unwrap();
        store.connection.write().execute(
            "INSERT INTO mission_runs(id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms) VALUES ('RUN','example','rev','gen','rev','RUN','/workspace','person/operator','{}','normal','running','running','10','10')", []).unwrap();
        let mut placement = request(); placement.actor = "mission-run/RUN".into();
        let hold = store.place_host_hold(&placement.actor, &placement).unwrap();
        let recovery = RecoverHostHold { actor: "person/operator".into(), host: hold.host.clone(),
            id: hold.id.clone(), expected_revision: hold.revision, reason: "owner finished; host repaired".into() };
        assert!(store.recover_host_hold("person/operator", "person/other", &recovery).is_err());
        assert!(store.recover_host_hold("agent/worker", "person/operator", &recovery).is_err());
        assert!(store.recover_host_hold("person/operator", "person/operator", &recovery).is_err());
        let other = store.place_host_hold(&placement.actor, &placement).unwrap();
        let released = store.release_host_hold(&other.owner, &ReleaseHostHold {
            actor: other.owner.clone(), host: other.host, id: other.id, expected_revision: other.revision,
            resolution: "owner repaired while live".into(),
        }).unwrap();
        assert_eq!(released.revision, 2);
        store.connection.write().execute("UPDATE mission_runs SET status='succeeded',phase='terminal' WHERE id='RUN'", []).unwrap();
        assert_eq!(store.host_holds(None, false).unwrap(), vec![hold.clone()], "terminal runs must never release holds");
        let mut stale = recovery.clone(); stale.expected_revision += 1;
        assert!(store.recover_host_hold("person/operator", "person/operator", &stale).is_err());
        let recovered = store.recover_host_hold("person/operator", "person/operator", &recovery).unwrap();
        assert_eq!(recovered.owner, hold.owner);
        assert_eq!(recovered.revision, hold.revision + 1);
        assert!(store.recover_host_hold("person/operator", "person/operator", &recovery).is_err());
        let connection = store.readers.get();
        let (actor, body): (String, String) = connection.query_row(
            "SELECT actor,body FROM local_observations WHERE subject=?1 AND kind='host-hold.recovered'",
            [&hold.id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!(actor, "person/operator");
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["fields"]["original_owner"], hold.owner);
        assert_eq!(body["fields"]["reason"], recovery.reason);
        let owner_releases: i64 = connection.query_row(
            "SELECT count(*) FROM local_observations WHERE kind='host-hold.released' AND actor='mission-run/RUN'",
            [], |row| row.get(0)).unwrap();
        assert_eq!(owner_releases, 1);
    }
    #[test]
    fn alternate_store_constructors_initialize_host_hold_schema() {
        let root = tempfile::tempdir().unwrap();
        let views = std::sync::Arc::new(smallclaims::ivm::Views::new(vec![]).unwrap());
        let stores = [
            Store::open_shared_memory("host-a").unwrap(),
            Store::open_with_ivm_views(&root.path().join("graph.db"), "host-a", views).unwrap(),
        ];
        for store in stores {
            assert!(store.host_holds(None, false).unwrap().is_empty());
            let hold = store.place_host_hold("person/operator", &request()).unwrap();
            assert_eq!(store.host_holds(None, false).unwrap(), vec![hold]);
        }
    }
    #[test]
    fn owner_and_revision_fence_release_without_occupying_capacity() {
        let store = Store::open_memory("host-a").unwrap();
        let hold = store.place_host_hold("person/operator", &request()).unwrap();
        assert_eq!(blocking_tx(&store.readers.get(), "host-a", "activation").unwrap(), vec![hold.clone()]);
        assert!(blocking_tx(&store.readers.get(), "host-a", "check").unwrap().is_empty());
        let mut release = ReleaseHostHold { actor: "person/other".into(), host: hold.host.clone(), id: hold.id.clone(), expected_revision: 1, resolution: "repaired".into() };
        assert!(store.release_host_hold("person/other", &release).is_err());
        release.actor = hold.owner.clone(); release.expected_revision = 2;
        assert!(store.release_host_hold(&hold.owner, &release).is_err());
        release.expected_revision = 1;
        assert_eq!(store.release_host_hold(&hold.owner, &release).unwrap().revision, 2);
        assert!(store.host_holds(None, false).unwrap().is_empty());
        assert_eq!(store.host_holds(None, true).unwrap().len(), 1);
        assert!(store.release_host_hold(&hold.owner, &release).is_err());
    }
    #[test]
    fn holds_survive_restart_and_do_not_depend_on_originating_run() {
        let root = tempfile::tempdir().unwrap(); let path = root.path().join("graph.db");
        let id = { let store = Store::open(&path, "host-a").unwrap(); store.place_host_hold("person/operator", &request()).unwrap().id };
        let reopened = Store::open(&path, "host-a").unwrap();
        assert_eq!(reopened.host_holds(None, false).unwrap()[0].id, id);
        assert!(reopened.place_host_hold("person/other", &request()).is_err());
        let mut remote = request(); remote.host = "host-b".into();
        assert!(reopened.place_host_hold("person/operator", &remote).is_err());
    }
    #[test]
    fn committed_hold_transitions_invalidate_incremental_admission_read_sets() {
        let store = Store::open_memory("host-a").unwrap();
        let incremental = crate::incremental::Incremental::default();
        incremental.observe(&store).unwrap();
        let hold = store.place_host_hold("person/operator", &request()).unwrap();
        incremental.observe(&store).unwrap();
        let (blocked, reads) = smallclaims::touched::record(|| blocking_tx(&store.readers.get(), "host-a", "activation"));
        assert_eq!(blocked.unwrap().len(), 1);
        assert!(reads.contains(&dependency("host-a")));
        incremental.evaluated("queued/first", reads.clone(), None);
        incremental.evaluated("queued/second", reads.clone(), None);
        incremental.evaluated("unrelated", BTreeSet::from([dependency("host-b")]), None);
        assert!(!incremental.needs("queued/first", now_ms()));
        store.release_host_hold(&hold.owner, &ReleaseHostHold {
            actor: hold.owner.clone(), host: hold.host, id: hold.id,
            expected_revision: hold.revision, resolution: "repaired".into(),
        }).unwrap();
        incremental.observe(&store).unwrap();
        assert!(incremental.needs("queued/first", now_ms()));
        assert!(incremental.needs("queued/second", now_ms()));
        assert!(!incremental.needs("unrelated", now_ms()));
        assert!(blocking_tx(&store.readers.get(), "host-a", "activation").unwrap().is_empty());
        incremental.evaluated("queued/first", reads, None);
        store.place_host_hold("person/operator", &request()).unwrap();
        incremental.observe(&store).unwrap();
        assert!(incremental.needs("queued/first", now_ms()), "placing a hold also invalidates admission");
    }
}
