//! Native source completeness: census immutable local run inputs, then sign a bounded frontier.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, ensure};
use rusqlite::{OptionalExtension as _, params};
use serde_json::Value;

use crate::roll_completion::{self, SourceConfig};
use crate::store::Store;

const HEARTBEAT: Duration = Duration::from_secs(60);

/// Serialize observation timestamp/append with native source capture/publication.
#[derive(Default)]
pub(crate) struct CaptureLock(pub Mutex<()>);

pub(crate) fn now_ms() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?)
}

#[derive(Clone)]
struct Run {
    id: String,
    created: String,
    inputs: String,
    creation: Option<String>,
}

#[cfg(test)]
fn runs(store: &Store, config: &SourceConfig) -> Result<Vec<Run>> {
    runs_since(store, config, None)
}

fn runs_since(store: &Store, config: &SourceConfig, after: Option<u64>) -> Result<Vec<Run>> {
    // Use the same canonical creation selection as mission_run_query HOST_SQL.
    // No phase/status predicate: terminal, pending and held runs are obligations.
    static SQL: LazyLock<String> = LazyLock::new(|| smallclaims::store::canonical_sql(
        "SELECT r.id,r.created_at_unix_ms,r.inputs,
            (SELECT created.id FROM claims created WHERE created.subject='mission-run/' || r.id
             AND created.kind='mission-run.created'
             AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=created.id AND state='repaired')
             ORDER BY CANONICAL_ASC(created) LIMIT 1)
         FROM mission_runs r WHERE r.mission_id>=?1 AND r.mission_id<?2
         ORDER BY r.created_at_unix_ms,r.id",
    ));
    static DELTA_SQL: LazyLock<String> = LazyLock::new(|| SQL.replace(
        "ORDER BY r.created_at_unix_ms,r.id",
        "AND r.id IN (SELECT substr(subject,13) FROM claims
          WHERE store_index>?3 AND kind='mission-run.created')
         ORDER BY r.created_at_unix_ms,r.id",
    ));
    let prefix = config.mission_prefix.strip_prefix("mission/").unwrap_or(&config.mission_prefix);
    ensure!(!prefix.is_empty(), "source mission prefix is empty");
    let connection = store.readers.get();
    let mut query = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h465d993a66cb owner=fractal_api_core::roll_source TODO=trace-statement-and-value-representation */ if after.is_some() { &DELTA_SQL } else { &SQL })?;
    let decode = |row: &rusqlite::Row<'_>| Ok(Run {
        id: row.get(0)?, created: row.get(1)?, inputs: row.get(2)?, creation: row.get(3)?,
    });
    let end = format!("{prefix}\u{10ffff}");
    Ok(if let Some(after) = after {
        query.query_map(params![prefix, end, after], decode)?.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        query.query_map(params![prefix, end], decode)?.collect::<rusqlite::Result<Vec<_>>>()?
    })
}

fn run_revision(store: &Store, config: &SourceConfig, inputs: &Value) -> Result<String> {
    let input = inputs.get(&config.resource_input).context("run has no configured immutable resource input")?;
    ensure!(input["kind"] == "resource", "source input is not a pinned resource");
    let claim_id = input["claim_id"].as_str().filter(|id| !id.is_empty()).context("resource input has no pinned claim id")?;
    let subject = input["subject"].as_str().context("resource input has no subject")?;
    let claim = store.claim_by_id(claim_id)?.context("pinned resource claim is unavailable")?;
    ensure!(claim.subject == subject && claim.kind == "resource.observed", "pinned resource input does not identify its observation");
    ensure!(store.claim_authorization(claim_id)?.verdict == smallclaims::principal::Verdict::Verified,
        "pinned revision observation is not authenticated");
    let revision = claim.body["fields"]["facts"][&config.revision_fact].as_str().context("pinned revision fact is absent")?;
    ensure!(matches!(revision.len(), 40 | 64) && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "pinned revision is not a full immutable revision");
    Ok(revision.to_owned())
}

static CREATION_SQL: LazyLock<String> = LazyLock::new(|| smallclaims::store::canonical_sql(
    "SELECT id FROM claims WHERE subject=?1 AND kind='mission-run.created'
     AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
     ORDER BY CANONICAL_ASC(claims) LIMIT 1",
));

/// Validate causal onset from original signed native intent, never supplied clocks.
/// Read-only: callers inside receipt admission must not seal or append graph claims.
pub(crate) fn validate_intent_with_config(
    store: &Store, config: &SourceConfig, host: &str, intent_id: &str,
) -> Result<(String, u64)> {
    store.read_snapshot(|_| {
        let creation = store.claim_by_id(intent_id)?.context("native intent receipt is unavailable")?;
        ensure!(creation.kind == "mission-run.created" && creation.origin == host,
            "intent must name the original observing host's native creation");
        let selected: String = store.readers.get().query_row(/* sqlite-large-value: allow-for-now lv-h0a0a9ff62f14 owner=fractal_api_core::roll_source TODO=trace-statement-and-value-representation */ &CREATION_SQL, [&creation.subject], |row| row.get(0))
            .optional()?.context("canonical native creation is unavailable")?;
        ensure!(selected == intent_id && store.claim_has_authenticated_writer(intent_id, host)?,
            "intent receipt lacks canonical authenticated original writer provenance");
        let mission = creation.body["fields"]["mission"].as_str().context("native creation mission is absent")?;
        let mission = mission.strip_prefix("mission/").unwrap_or(mission);
        ensure!(mission.starts_with(config.mission_prefix.strip_prefix("mission/").unwrap_or(&config.mission_prefix)),
            "native intent does not match the signed source mission prefix");
        let revision = run_revision(store, config, &creation.body["fields"]["inputs"])?;
        let accepted = creation.accepted_at_unix_ms.try_into().context("native acceptance clock overflow")?;
        Ok((format!("{}{}", config.roll_id_prefix, revision), accepted))
    })
}

pub(crate) fn validate_intent_receipt(
    store: &Store, producer: &str, host: &str, intent_id: &str,
) -> Result<(String, u64)> {
    let creation = store.claim_by_id(intent_id)?.context("native intent receipt is unavailable")?;
    let mission = creation.body["fields"]["mission"].as_str().context("native creation mission is absent")?;
    let mission = mission.strip_prefix("mission/").unwrap_or(mission);
    let configs = roll_completion::source_configs(store)?;
    let mut matching = configs.iter().filter(|config| config.producer == producer && mission.starts_with(
        config.mission_prefix.strip_prefix("mission/").unwrap_or(&config.mission_prefix)));
    let config = matching.next().context("intent has no admitted matching source")?;
    ensure!(matching.next().is_none(), "intent matches multiple native sources");
    validate_intent_with_config(store, config, host, intent_id)
}

/// Resolve the immutable native intent used by a rollout declaration. No mutable
/// run-view clock, downloaded checkout, caller revision or wall-clock fallback.
pub fn intent_context(store: &Store, run_subject: &str) -> Result<Value> {
    ensure!(run_subject.strip_prefix("mission-run/").is_some_and(|id| !id.is_empty()),
        "roll intent requires a canonical mission-run subject");
    // Prepare proof for already accepted native intent, like replication inventory.
    store.seal_local_batches()?;
    store.judge_claims(true)?;
    store.read_snapshot(|_| {
        let creation_id: String = store.readers.get().query_row(/* sqlite-large-value: allow-for-now lv-hc76357e25b55 owner=fractal_api_core::roll_source TODO=trace-statement-and-value-representation */ &CREATION_SQL, [run_subject], |row| row.get(0))
            .optional()?.context("canonical run creation receipt is unavailable")?;
        let creation = store.claim_by_id(&creation_id)?.context("canonical run creation receipt disappeared")?;
        ensure!(store.claim_has_authenticated_writer(&creation_id, &creation.origin)?,
            "canonical run origin is not authenticated");
        let mission = creation.body["fields"]["mission"].as_str().context("run creation has no canonical mission")?;
        let mission = mission.strip_prefix("mission/").unwrap_or(mission);
        let configs = roll_completion::source_configs(store)?;
        let mut matching = configs.iter().filter(|config| {
            let prefix = config.mission_prefix.strip_prefix("mission/").unwrap_or(&config.mission_prefix);
            mission.starts_with(prefix)
        });
        let config = matching.next().context("run has no admitted native rollout source")?;
        ensure!(matching.next().is_none(), "run matches multiple native rollout sources");
        let inputs = &creation.body["fields"]["inputs"];
        let revision = run_revision(store, config, inputs)?;
        let resource_claim_id = inputs[&config.resource_input]["claim_id"].as_str()
            .context("immutable resource claim id is unavailable")?;
        let declared_at_ms: u64 = creation.accepted_at_unix_ms.try_into().context("native acceptance clock overflow")?;
        Ok(serde_json::json!({
            "roll_id": format!("{}{}", config.roll_id_prefix, revision),
            "declared_at_ms": declared_at_ms,
            "dotfiles_revision": revision,
            "resource_claim_id": resource_claim_id,
        }))
    })
}

#[derive(Debug, PartialEq, Eq)]
struct Census {
    from: u64,
    through: u64,
    pending_intents: Vec<(String, String)>,
    covered: Vec<String>,
}

#[cfg(test)]
fn census(store: &Store, config: &SourceConfig, now: u64) -> Result<Option<Census>> {
    census_pending(store, config, now, runs(store, config)?)
}

fn census_pending(store: &Store, config: &SourceConfig, now: u64, pending: Vec<Run>) -> Result<Option<Census>> {
    ensure!(config.enabled_at_ms <= now, "source enablement is in the future");
    let declarations = roll_completion::source_declarations(store, &config.producer)?;
    let connection = store.readers.get();
    let mut visible = connection.prepare_cached("SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)")?;
    let mut declared = BTreeMap::new();
    for (roll, id, _) in declarations {
        if visible.query_row(/* sqlite-large-value: allow-for-now lv-hc9ae27e75a8c owner=fractal_api_core::roll_source TODO=trace-statement-and-value-representation */ [&id], |row| row.get::<_, bool>(0))? {
            declared.insert(roll, id);
        }
    }
    drop(visible);
    drop(connection);
    // New writes after the serialized cut may share this millisecond.
    let Some(mut through) = now.checked_sub(1) else { return Ok(None); };
    let mut pending_intents = Vec::new();
    let mut covered = Vec::new();
    for run in pending {
        let creation_id = run.creation.context("run has no canonical creation receipt")?;
        let creation = store.claim_by_id(&creation_id)?.context("canonical creation receipt disappeared")?;
        ensure!(store.claim_has_authenticated_writer(&creation_id, &creation.origin)?,
            "canonical run origin is not authenticated");
        // Skip only proven foreign-native runs. Their own daemon must census
        // them; an arbitrary origin string cannot hide a local obligation.
        if creation.origin != store.origin() { covered.push(run.id); continue; }
        let projected = run.created.parse::<u64>().context("native run creation time is invalid")?;
        ensure!(projected <= now, "native run creation time is in the future");
        // Projection dates can derive deadline-timeout. Source ordering uses
        // the canonical receipt's transaction-assigned native acceptance time.
        let created: u64 = creation.accepted_at_unix_ms.try_into().context("native acceptance clock overflow")?;
        ensure!(created <= now, "native acceptance clock is in the future");
        let inputs: Value = serde_json::from_str(&run.inputs).context("immutable native run inputs are invalid")?;
        ensure!(creation.body["fields"]["inputs"] == inputs, "native run inputs differ from canonical immutable creation");
        let revision = match run_revision(store, config, &inputs) {
            Ok(revision) => revision,
            Err(_) if created < config.enabled_at_ms => continue,
            Err(error) => return Err(error).with_context(|| format!("census run {}", run.id)),
        };
        let roll_id = format!("{}{}", config.roll_id_prefix, revision);
        match declared.get(roll_id.as_str()) {
            Some(declaration) if roll_completion::intent_is_admitted(store, &creation_id, declaration) => covered.push(run.id),
            declaration => {
                if created >= config.enabled_at_ms { through = through.min(created.saturating_sub(1)); }
                if let Some(declaration) = declaration {
                    pending_intents.push((declaration.clone(), creation_id));
                }
            }
        }
    }
    if through < config.enabled_at_ms && pending_intents.is_empty() { return Ok(None); }
    Ok(Some(Census { from: config.enabled_at_ms, through, pending_intents, covered }))
}

/// Canonical native lifecycle metadata is initialized explicitly by native source
/// startup, independently of optional shadow views. Ordinary reopen preserves it;
/// replacement/restore cannot inherit coverage by reusing a numerical claim index.
/// Unsupported/unavailable lifecycle state is an error, never a synthesized identity.
pub fn source_lifetime(store: &Store) -> Result<String> {
    let (database, epoch): (String, String) = store.readers.get().query_row(
        "SELECT e.database_id, CAST(s.epoch AS TEXT)
         FROM ivm_event_state e CROSS JOIN ivm_source s
         WHERE e.singleton=1 AND e.available=1 AND s.singleton=1",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).context("native source lifetime is unavailable")?;
    ensure!(!database.is_empty() && !epoch.is_empty(), "native source lifetime is invalid");
    Ok(format!("{database}:{epoch}"))
}

/// Startup/authority changes replay once. Ordinary heartbeats scan only newly
/// indexed native creations and retained unresolved obligations, never history.
#[derive(Default)]
struct CensusCache(Mutex<BTreeMap<String, PendingRuns>>);

struct PendingRuns {
    authority: Vec<String>,
    lifetime: String,
    index: u64,
    runs: BTreeMap<String, Run>,
}

struct Capture {
    census: Census,
    source_index: u64,
    lifetime: String,
    now: u64,
}

fn capture(store: &Store, config: &SourceConfig) -> Result<Option<Capture>> {
    // Quiet local writers still need envelope provenance for their accepted
    // intents; do not depend on an unrelated replication exchange to seal it.
    store.seal_local_batches()?;
    store.judge_claims(true)?;
    // Pin the committed reader cut before releasing the native writer barrier.
    // The writer is held only for this O(1) barrier, never for the census scan.
    let writer = store.connection.write();
    store.read_snapshot(|source_index| {
        let now = now_ms()?;
        drop(writer);
        let lifetime = source_lifetime(store)?;
        let authority = roll_completion::authority_cut(store)?;
        let cache = store.extension::<CensusCache>();
        let mut cache = cache.0.lock().map_err(|_| anyhow::anyhow!("native census cache poisoned"))?;
        let pending = cache.entry(config.source_claim_id.clone()).or_insert_with(|| PendingRuns {
            authority: authority.clone(), lifetime: lifetime.clone(), index: 0, runs: BTreeMap::new(),
        });
        let reset = pending.index == 0 || pending.lifetime != lifetime || pending.authority != authority
            || source_index < pending.index;
        if reset { pending.runs.clear(); }
        for run in runs_since(store, config, (!reset).then_some(pending.index))? {
            pending.runs.insert(run.id.clone(), run);
        }
        // Publish cursor advancement only after every newly seen obligation has
        // been classified successfully; errors preserve pending work for retry.
        let census = census_pending(store, config, now, pending.runs.values().cloned().collect())?;
        if let Some(census) = &census {
            for id in &census.covered { pending.runs.remove(id); }
        }
        pending.index = source_index;
        pending.authority = authority;
        pending.lifetime.clone_from(&lifetime);
        Ok(census.map(|census| Capture { census, source_index, lifetime, now }))
    })
}

/// A monotonic quiet-host heartbeat, with immediate capture for newly enabled
/// source IDs. Bounded receipt chunks continue on publication commits; completed
/// sources wait for the next deadline, so quiet heartbeats cannot self-loop.
#[derive(Default)]
pub struct Heartbeat {
    seen: BTreeSet<String>,
    draining: BTreeSet<String>,
    next: Option<Instant>,
}

impl Heartbeat {
    pub fn deadline(&self) -> Option<Instant> { self.next }

    pub fn refresh(&mut self, store: &Store) -> Result<()> {
        let clock = Instant::now();
        let due = self.next.is_none_or(|deadline| deadline <= clock);
        // A failed read must not leave an already expired deadline spinning.
        if self.next.is_some() && due { self.next = Some(clock + HEARTBEAT); }
        let configs = roll_completion::source_configs(store)?;
        if configs.is_empty() {
            self.seen.clear();
            self.draining.clear();
            self.next = None;
            return Ok(());
        }
        let current: BTreeSet<_> = configs.iter().map(|config| config.source_claim_id.clone()).collect();
        self.seen.retain(|id| current.contains(id));
        self.draining.retain(|id| current.contains(id));
        self.next.get_or_insert(clock + HEARTBEAT);
        for config in configs {
            if !due && self.seen.contains(&config.source_claim_id) && !self.draining.contains(&config.source_claim_id) { continue; }
            self.seen.insert(config.source_claim_id.clone());
            // A failed census/publication waits for the ordinary heartbeat;
            // only a successful bounded chunk can request immediate continuation.
            self.draining.remove(&config.source_claim_id);
            let source = store.extension::<CaptureLock>();
            let _guard = source.0.lock().map_err(|_| anyhow::anyhow!("source capture lock poisoned"))?;
            match capture(store, &config) {
                Ok(Some(snapshot)) => {
                    if !snapshot.census.pending_intents.is_empty() {
                        let mut published = false;
                        for (declaration, intent) in &snapshot.census.pending_intents {
                            match roll_completion::publish_intent_observed(
                                store, &config.producer, declaration, intent, snapshot.now,
                            ) {
                                Ok(_) => published |= roll_completion::intent_is_admitted(store, intent, declaration),
                                Err(error) => tracing::warn!(producer = %config.producer, %error, "native rollout onset publication failed"),
                            }
                        }
                        if published { self.draining.insert(config.source_claim_id.clone()); }
                        continue;
                    }
                    match roll_completion::publish_frontier(store, &config,
                        snapshot.census.from, snapshot.census.through, snapshot.source_index, &snapshot.lifetime, snapshot.now) {
                        Ok(result) => {
                            if result["more_receipts"] == true {
                                self.draining.insert(config.source_claim_id.clone());
                            }
                        }
                        Err(error) => tracing::warn!(producer = %config.producer, %error, "native rollout source frontier publication failed"),
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(producer = %config.producer, %error, "native rollout source census unavailable"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MissionsStore;

    #[test]
    fn heartbeat_starts_without_a_wall_clock_frontier_or_timer() {
        let heartbeat = Heartbeat::default();
        assert!(heartbeat.deadline().is_none());
        assert!(heartbeat.seen.is_empty());
    }

    #[test]
    fn intent_context_never_substitutes_wall_time_for_absent_native_creation() {
        let store = Store::open_memory("host-a").unwrap();
        assert!(intent_context(&store, "missing").is_err());
        assert!(intent_context(&store, "mission-run/missing").is_err());
    }

    #[test]
    fn plain_native_store_lifetime_survives_reopen_but_not_replacement_or_unavailability() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("native.db");
        let original = {
            let store = Store::open(&path, "host-a").unwrap();
            assert!(source_lifetime(&store).is_err(), "a source not initialized by its owner is unknown");
            store.initialize_native_source_lifecycle().unwrap();
            assert!(store.ivm_views().is_none(), "native lifetime must not require shadow views");
            source_lifetime(&store).unwrap()
        };
        let store = Store::open(&path, "host-a").unwrap();
        store.initialize_native_source_lifecycle().unwrap();
        assert_eq!(source_lifetime(&store).unwrap(), original);
        let replacement = Store::open(&root.path().join("replacement.db"), "host-a").unwrap();
        replacement.initialize_native_source_lifecycle().unwrap();
        assert_ne!(source_lifetime(&replacement).unwrap(), original);
        store.connection.write().execute(
            "UPDATE ivm_event_state SET available=0 WHERE singleton=1", [],
        ).unwrap();
        assert!(source_lifetime(&store).is_err(), "unavailable native source must remain unknown");
        drop(store);
        let store = Store::open(&path, "host-a").unwrap();
        store.initialize_native_source_lifecycle().unwrap();
        assert!(source_lifetime(&store).is_err(), "reopen cannot silently repair unavailable authority");
    }

    #[test]
    fn native_lifecycle_initialization_preserves_existing_source_and_event_contract() {
        let store = Store::open_memory("host-a").unwrap();
        let views = smallclaims::ivm::Views::new(Vec::new()).unwrap();
        let cut = smallclaims::ivm::SourceCut {
            epoch: 7, admitted: 0, projected: 0, local_generation: 3,
        };
        {
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            views.create_schema(&transaction).unwrap();
            views.initialize_empty(&transaction, cut).unwrap();
            smallclaims::ivm::events::install(&transaction, 1024).unwrap();
            transaction.commit().unwrap();
        }
        let lifetime = source_lifetime(&store).unwrap();
        store.initialize_native_source_lifecycle().unwrap();
        assert_eq!(source_lifetime(&store).unwrap(), lifetime);
        assert_eq!(smallclaims::ivm::source_cut(&store.readers.get()).unwrap(), Some(cut));
        // Reinstall through the owning API proves retention was not replaced.
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        smallclaims::ivm::events::install(&transaction, 1024).unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn immediate_native_intent_uses_pinned_revision_and_acceptance_not_deadline_derived_view_clock() {
        use crate::roll_completion::{Receipt, SignedReceipt};
        use crate::roll_completion::tests::{publish_fixture, setup_signer};
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("host-a").unwrap();
        setup_signer(&store);
        roll_completion::install(&store, root.path()).unwrap();
        let enabled = publish_fixture(&store, Receipt::SourceEnabled {
            version: 1, producer: "agent/example/roll-authority".into(), enabled_at_ms: now_ms().unwrap(),
            mission_prefix: "fleet/roll/".into(), resource_input: "main".into(),
            revision_fact: "head".into(), roll_id_prefix: "dotfiles/".into(),
        }, Some("agent/example/roll-authority"));
        roll_completion::ingest(&store, SignedReceipt { claim_id: enabled }, now_ms().unwrap()).unwrap();
        let source = r#"version 2
mission "fleet/roll/intent-proof" state="ready" timeout="20m" {
    input "main" kind="resource"
    goal "Read a pinned native rollout intent."
}
"#;
        let intent = crate::graph::parse_intent(source, store.origin()).unwrap();
        let preview = store.mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None }).unwrap();
        store.apply(&intent, &preview.subject_tokens, &intent.source_hash).unwrap();
        roll_completion::rebuild(&store).unwrap();
        let revision = "a".repeat(40);
        let resource = store.append_claim(&crate::model::ClaimInput {
            subject: "resource/roll-source-test".into(), kind: "resource.observed".into(), actor: None,
            fields: BTreeMap::from([
                ("kind".into(), serde_json::json!("custom.fractal.roll-source-test")),
                ("state".into(), serde_json::json!("ready")),
                ("facts".into(), serde_json::json!({"head":revision})),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        store.set_write_clock_at(u128::from(now_ms().unwrap() + 10_000)).unwrap();
        let run = store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "fleet/roll/intent-proof".into(), revision: None, workspace: "/tmp".into(),
            requester: Some("agent/example/roll-authority".into()), mode: Some("run".into()),
            inputs: BTreeMap::from([("main".into(), format!("{}@{}", resource.subject, resource.id))]),
            idempotency_key: "source-intent-proof".into(),
        }).unwrap();
        // No unrelated replication exchange or explicit sealing intervenes.
        let context = intent_context(&store, &run.subject).unwrap();
        let creation_id: String = store.readers.get().query_row(
            "SELECT id FROM claims WHERE subject=?1 AND kind='mission-run.created' ORDER BY store_index LIMIT 1",
            [&run.subject], |row| row.get(0),
        ).unwrap();
        let creation = store.claim_by_id(&creation_id).unwrap().unwrap();
        let captured = runs(&store, &config()).unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].creation.as_deref(), Some(creation_id.as_str()),
            "full census must select the canonical creation through a supported ordering alias");
        let creation_index: u64 = store.readers.get().query_row(
            "SELECT store_index FROM claims WHERE id=?1", [&creation_id], |row| row.get(0),
        ).unwrap();
        assert!(runs_since(&store, &config(), Some(creation_index)).unwrap().is_empty(),
            "quiet census must not rescan already indexed native runs");
        assert_eq!(runs_since(&store, &config(), Some(creation_index - 1)).unwrap().len(), 1);
        let projected: String = store.readers.get().query_row(
            "SELECT created_at_unix_ms FROM mission_runs WHERE id=?1",
            [run.subject.trim_start_matches("mission-run/")], |row| row.get(0),
        ).unwrap();
        assert!(projected.parse::<u128>().unwrap() < creation.accepted_at_unix_ms);
        assert_eq!(context["declared_at_ms"], u64::try_from(creation.accepted_at_unix_ms).unwrap());
        assert_eq!(context["roll_id"], format!("dotfiles/{revision}"));
        assert_eq!(context["dotfiles_revision"], revision);
        assert_eq!(context["resource_claim_id"], resource.id);
        let config = roll_completion::source_configs(&store).unwrap().remove(0);
        let accepted = u64::try_from(creation.accepted_at_unix_ms).unwrap();
        assert_eq!(validate_intent_with_config(&store, &config, store.origin(), &creation_id).unwrap(),
            (format!("dotfiles/{revision}"), accepted));
        assert!(validate_intent_with_config(&store, &config, "another-host", &creation_id).is_err());
        let declaration = publish_fixture(&store, Receipt::RollDeclared {
            version: 1, roll_id: format!("dotfiles/{revision}"), declared_at_ms: accepted + 100,
            targets: BTreeMap::from([("host-a".into(), crate::roll_completion::Identity {
                dotfiles_revision: revision.clone(), daemon_revision: "b".repeat(40),
                system_path: None, home_path: None,
            })]),
        }, Some("agent/example/roll-authority"));
        roll_completion::ingest(&store, SignedReceipt { claim_id: declaration.clone() }, accepted + 200).unwrap();
        let blocked = census(&store, &config, accepted + 200).unwrap().unwrap();
        assert_eq!(blocked.through, accepted - 1);
        assert_eq!(blocked.pending_intents, vec![(declaration.clone(), creation_id.clone())]);
        roll_completion::publish_intent_observed(&store, &config.producer, &declaration, &creation_id, accepted + 200).unwrap();
        let recovered = census(&store, &config, accepted + 200).unwrap().unwrap();
        assert!(recovered.pending_intents.is_empty());
        assert_eq!(recovered.through, accepted + 199, "later declaration cannot permanently hide earlier same-revision intent");
        // A payload-origin edit is not envelope writer authority.
        store.connection.write().execute("UPDATE claims SET origin='forged-host' WHERE id=?1", [&creation_id]).unwrap();
        assert!(intent_context(&store, &run.subject).is_err());
    }

    fn config() -> SourceConfig {
        SourceConfig {
            producer: "agent/example/roll-authority".into(), source_claim_id: "source-enabled".into(),
            enabled_at_ms: 100, mission_prefix: "mission/fleet/roll/".into(),
            resource_input: "main".into(), revision_fact: "head".into(), roll_id_prefix: "dotfiles/".into(),
        }
    }

    #[test]
    fn quiet_census_reports_only_the_actual_capture_cut_and_rejects_future_enablement() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("host-a").unwrap();
        roll_completion::install(&store, root.path()).unwrap();
        assert_eq!(census(&store, &config(), 1_000).unwrap(),
            Some(Census { from: 100, through: 999, pending_intents: Vec::new(), covered: Vec::new() }));
        assert!(census(&store, &config(), 99).is_err());
        let mut heartbeat = Heartbeat::default();
        heartbeat.refresh(&store).unwrap();
        assert!(heartbeat.deadline().is_none(), "an unenrolled source cannot gain a heartbeat from wall time");
    }

    #[test]
    fn native_run_scan_includes_terminal_pending_and_held_rows_without_prefix_leakage() {
        let store = Store::open_memory("host-a").unwrap();
        {
            let connection = store.connection.write();
            for (id, mission, status, phase) in [
                ("terminal", "fleet/roll/example", "completed", "terminal"),
                ("pending", "fleet/roll/example", "running", "normal"),
                ("held", "fleet/roll/example", "running", "held"),
                ("unrelated", "fleet/rollback", "running", "normal"),
            ] {
                connection.execute(
                    "INSERT INTO mission_runs(id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms)
                     VALUES (?1,?2,'rev','gen','rev',?1,'/tmp','person/test','{}','run',?3,?4,'500','500')",
                    params![id, mission, status, phase],
                ).unwrap();
            }
        }
        let captured = runs(&store, &config()).unwrap();
        assert_eq!(captured.iter().map(|run| run.id.as_str()).collect::<Vec<_>>(), vec!["held", "pending", "terminal"]);
        assert!(captured.iter().all(|run| run.creation.is_none()));
        let root = tempfile::tempdir().unwrap();
        roll_completion::install(&store, root.path()).unwrap();
        assert!(census(&store, &config(), 1_000).is_err(), "unproven creation origin cannot disappear from coverage");
    }
}
