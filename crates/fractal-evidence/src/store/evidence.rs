//! The part of `store::evidence` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::SeatsStore;
use super::*;
pub use fractal_attention::store::evidence::*;

/// A roster head: its cut, every agent's refs, the first cards in order, and when published.
pub type PublishedRosterHead = (u64, Arc<Vec<Value>>, Vec<Value>, u128);
use crate::store::published_list::PublicationBuild;

pub struct AgentsPublicationSource {
    pub watermark: crate::store::agents_publication::AgentsStatusWatermark,
    pub rows: Arc<Vec<Value>>,
    pub valid_until_ms: Option<u64>,
}

pub trait EvidenceStore {
    fn agents_publication(&self) -> Option<Arc<crate::store::agents_publication::AgentsPublication>>;
    fn subscribe_agents_publications(&self) -> tokio::sync::watch::Receiver<u64>;
    fn agents_publication_build(&self) -> Result<PublicationBuild>;
    fn agents_publication_demand(&self) -> bool;
    fn agents_publication_source(&self, index: u64) -> Result<AgentsPublicationSource>;
    fn publish_presented_agents(
        &self,
        build: PublicationBuild,
        source: AgentsPublicationSource,
        at: u64,
        valid_until: Option<u64>,
        rows: Vec<Value>,
    ) -> Result<()>;
    /// Lazily attach the one Store publisher shared by receipt waits and collection
    /// consumers. Registration and journal installation happen when the Store opens.
    /// Attachment errors are returned on each call until attachment succeeds; failures
    /// are not cached. A successful attachment is shared by all later callers.
    fn ivm_publisher(&self) -> Result<Option<Arc<smallclaims::ivm::events::Publisher>>>;
    /// Whether the newest complete roster published at or before `index` already shows every
    /// card as it would at `index`: no claim since its cut changes a card's inputs (the same
    /// delta the incremental fold trusts), no agent timeline row arrived, and no queue deadline
    /// passed. A read that must see what was written before it need not wait for a refresh
    /// that would fold nothing. One range read over the claims since the cut; it never folds.
    fn published_agent_roster_unchanged_through(
            &self,
            index: u64,
            history: bool,
        ) -> Result<bool>;
    /// The newest complete current roster published at or before `index`, with its own graph
    /// index, while a refresher keeps advancing it. A reader serves these rows under that
    /// index's snapshot, never under its own newer one, and wakes the refresher. Without a
    /// refresher there is none: readers then fold exactly their own cut, as before.
    fn published_agent_roster(
            &self,
            index: u64,
            history: bool,
        ) -> Option<(u64, Arc<Vec<Value>>, u128)>;
    /// The complete roster published at exactly `cut`, for a page continuing from a first page
    /// served at that cut, with when it was published.
    fn published_agent_roster_at(
            &self,
            cut: u64,
            history: bool,
        ) -> Option<(Arc<Vec<Value>>, u128)>;
    /// When no complete roster is published yet: the newest current refs at or before `index`
    /// (membership, order and queue metadata of every agent) and the cards of their first
    /// `count` agents, all at that one cut. The refresher publishes this head first as the
    /// daemon starts, so a window or first page need not wait for every card to fold.
    fn published_agent_roster_head(
            &self,
            index: u64,
            count: usize,
        ) -> Option<PublishedRosterHead>;
    /// The actor that published the mission revision `run` works to: a broken gate's attention
    /// goes to it. A nested run works to its root run's revision.
    fn mission_run_publisher(&self, run: &str) -> Result<Option<String>>;
    /// The newest eligible local driver receipt after this exact check's request in writer
    /// sequence/position order. The same statement reads current SystemLocal starts and
    /// legacy admitted starts. A local row's after_store_index places it after that claim;
    /// this is local log ordering, not the driver's original writer/deadline proof.
    /// Multiple launches cannot bind a no-incarnation driver exit to the current process.
    fn mechanical_gate_exit_receipt(
            &self,
            requested: &ClaimRecord,
        ) -> Result<Option<(ClaimRecord, Option<String>)>>;
    /// Gate runners are launched directly rather than through desired declarations. Their
    /// owner is durable in the request; older mechanical/LLM requests encode it in the subject.
    fn mission_gate_runners(&self) -> Result<Vec<MissionGateRunner>>;
    fn gate_request_for_owner(&self, owner: &str) -> Result<Option<ClaimRecord>>;
    /// Every human gate request as (request, owner, reviewer): the episodes a human gate's
    /// attention card can name, open or long closed.
    fn human_gate_requests(&self) -> Result<Vec<(String, String, String)>>;
    /// Explicit credential evidence from the currently running native epoch only.
    fn harness_auth_evidence(
            &self,
            subject: &str,
            incarnation: &str,
        ) -> Result<Option<Value>>;
}

impl EvidenceStore for Store {
    fn agents_publication(&self) -> Option<Arc<crate::store::agents_publication::AgentsPublication>> {
        match self.fractal.presented_agents.current() {
            crate::store::published_list::ListRead::Served(publication, _) => Some(publication),
            _ => None,
        }
    }

    fn agents_publication_demand(&self) -> bool {
        self.fractal.agents_publication_demand()
    }

    fn subscribe_agents_publications(&self) -> tokio::sync::watch::Receiver<u64> {
        let receiver = self.fractal.presented_agents.subscribe();
        // A subscription contributes only receiver demand: its wake is not an explicit
        // refresh request that must survive the last receiver leaving.
        if let Some(wake) = self.fractal.agent_roster_refresh.get() {
            wake.notify_one();
        }
        receiver
    }

    fn agents_publication_build(&self) -> Result<PublicationBuild> {
        self.fractal.presented_agents.begin_materialization()
            .context("the agents publication materialization counter is exhausted")
    }

    fn agents_publication_source(&self, index: u64) -> Result<AgentsPublicationSource> {
        let local = roster_local_frontier(&self.readers.get(), index)?;
        let cache = self.fractal.agent_resources_cache.lock()
            .map_err(|_| anyhow::anyhow!("agent resources cache poisoned"))?;
        let entry = cache.iter().rev().find(|entry| entry.index == index && entry.local == local
            && !entry.history && entry.covered.is_none())
            .context("the complete current agents source is unavailable")?;
        Ok(AgentsPublicationSource {
            watermark: crate::store::agents_publication::AgentsStatusWatermark {
                store_index: index, local_frontier: local,
            },
            rows: Arc::clone(&entry.items),
            valid_until_ms: entry.valid_until_unix_ms.map(|deadline| deadline.min(u128::from(u64::MAX)) as u64),
        })
    }

    fn publish_presented_agents(
        &self,
        build: PublicationBuild,
        source: AgentsPublicationSource,
        at: u64,
        valid_until: Option<u64>,
        rows: Vec<Value>,
    ) -> Result<()> {
        use crate::store::{agents_publication::PresentedAgents, published_list::Publication};
        if let Some(current) = self.agents_publication()
            && current.metadata().status_watermark == source.watermark && current.rows() == rows
            && self.fractal.presented_agents.retain_materialization(build, &current)
        {
            return Ok(());
        }
        let order = rows.iter().map(|row| row["id"].as_str()
            .context("a complete agent card has no ID").map(str::to_owned))
            .collect::<Result<Vec<_>>>()?;
        self.fractal.presented_agents.start();
        let epoch = self.fractal.presented_agents_epoch.get();
        if self.fractal.presented_agents.publish_identified(build, |identity| Publication {
            cut: source.watermark.store_index,
            published_at_unix_ms: u128::from(at),
            valid_until_unix_ms: valid_until.map(u128::from),
            rows: PresentedAgents::new(epoch, identity, source.watermark, at, rows, order),
        }).is_none() {
            anyhow::bail!("the agents source reset while materializing its publication");
        }
        // Serialize once outside the shared owner lock.
        if let Some(publication) = self.agents_publication() { let _ = publication.encoded(); }
        Ok(())
    }

    fn ivm_publisher(&self) -> Result<Option<Arc<smallclaims::ivm::events::Publisher>>> {
        if self.fractal.ivm_views.is_none() {
            return Ok(None);
        }
        let mut publisher = self.fractal
            .ivm_publisher
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(publisher) = &*publisher {
            return Ok(Some(publisher.clone()));
        }
        let attached = Arc::new(smallclaims::ivm::events::Publisher::attach(&self.graph, 128)?);
        *publisher = Some(attached.clone());
        Ok(Some(attached))
    }

    fn published_agent_roster_unchanged_through(
            &self,
            index: u64,
            history: bool,
        ) -> Result<bool> {
        if self.fractal.agent_roster_refresh.get().is_none()
            || self.agent_roster_faults_pending(history) {
            return Ok(false);
        }
        let Some((cut, local, valid_until, items)) = self.fractal.agent_resources_cache.lock()
            .expect("agent resources cache poisoned").iter()
            .filter(|entry| entry.history == history && entry.covered.is_none() && entry.index <= index)
            .max_by_key(|entry| (entry.index, entry.local))
            .map(|entry| (entry.index, entry.local, entry.valid_until_unix_ms, Arc::clone(&entry.items)))
        else {
            return Ok(false);
        };
        if valid_until.is_some_and(|expiry| now_ms() >= expiry)
            || roster_local_frontier(&self.readers.get(), index)? != local
        {
            return Ok(false);
        }
        if cut == index {
            return Ok(true);
        }
        Ok(match self.changed_agent_resources(cut, index, &items)? {
            AgentResourcesChange::Delta(delta) => delta.subjects.is_empty() && !delta.queues && !delta.membership,
            AgentResourcesChange::Invalidated { .. } => false,
        })
    }

    fn published_agent_roster(
            &self,
            index: u64,
            history: bool,
        ) -> Option<(u64, Arc<Vec<Value>>, u128)> {
        self.fractal.agent_roster_refresh.get()?;
        self.warn_if_agent_roster_overdue();
        let newest = |entries: &mut dyn Iterator<Item = &crate::store::runtime::AgentResourcesEntry>| entries
            .filter(|entry| entry.history == history && entry.covered.is_none() && entry.index <= index)
            .max_by_key(|entry| (entry.index, entry.local))
            .map(|entry| (entry.index, Arc::clone(&entry.items), entry.published_at_unix_ms));
        // While a projection catching up refolds the roster, readers keep the one they had.
        let roster = newest(&mut self.fractal.agent_resources_cache.lock()
            .expect("agent resources cache poisoned").iter())
            .or_else(|| newest(&mut self.fractal.agent_roster_previous.lock()
                .expect("previous agent roster poisoned").resources.iter()))?;
        crate::otel::record_roster("hit", roster.1.len());
        Some(roster)
    }

    fn published_agent_roster_at(
            &self,
            cut: u64,
            history: bool,
        ) -> Option<(Arc<Vec<Value>>, u128)> {
        self.fractal.agent_roster_refresh.get()?;
        let at = |entries: &mut dyn Iterator<Item = &crate::store::runtime::AgentResourcesEntry>| entries
            .filter(|entry| entry.history == history && entry.covered.is_none() && entry.index == cut)
            .max_by_key(|entry| entry.local)
            .map(|entry| (Arc::clone(&entry.items), entry.published_at_unix_ms));
        let roster = at(&mut self.fractal.agent_resources_cache.lock()
            .expect("agent resources cache poisoned").iter())
            .or_else(|| at(&mut self.fractal.agent_roster_previous.lock()
                .expect("previous agent roster poisoned").resources.iter()))?;
        crate::otel::record_roster("hit", roster.0.len());
        Some(roster)
    }

    fn published_agent_roster_head(
            &self,
            index: u64,
            count: usize,
        ) -> Option<PublishedRosterHead> {
        self.fractal.agent_roster_refresh.get()?;
        self.warn_if_agent_roster_overdue();
        let current = self.fractal.agent_page_refs_cache.lock()
            .expect("agent page refs cache poisoned").iter()
            .filter(|entry| !entry.history && entry.index <= index)
            .max_by_key(|entry| entry.index)
            .map(|entry| (entry.index, Arc::clone(&entry.items)));
        let head_at = |cut: u64, refs: &Arc<Vec<Value>>, entries: &mut dyn Iterator<Item = &crate::store::runtime::AgentResourcesEntry>| entries
            .filter(|entry| entry.index == cut && !entry.history)
            .find_map(|entry| {
                let cards = entry.items.iter()
                    .filter_map(|card| Some((card["id"].as_str()?, card)))
                    .collect::<HashMap<_, _>>();
                refs.iter().take(count)
                    .map(|reference| cards.get(reference["id"].as_str()?).map(|card| (*card).clone()))
                    .collect::<Option<Vec<_>>>()
                    .map(|head| (head, entry.published_at_unix_ms))
            });
        let (cut, refs, head) = match current {
            Some((cut, refs)) => {
                let head = head_at(cut, &refs, &mut self.fractal.agent_resources_cache.lock()
                    .expect("agent resources cache poisoned").iter())?;
                (cut, refs, head)
            }
            // While a projection catching up refolds the roster, readers keep the head they had:
            // its page order and the cards at the same cut.
            None => {
                let previous = self.fractal.agent_roster_previous.lock()
                    .expect("previous agent roster poisoned");
                let refs = previous.page_refs.as_ref().filter(|entry| entry.index <= index)?;
                let (cut, refs) = (refs.index, Arc::clone(&refs.items));
                let head = head_at(cut, &refs, &mut previous.resources.iter())?;
                (cut, refs, head)
            }
        };
        crate::otel::record_roster("hit", head.0.len());
        Some((cut, refs, head.0, head.1))
    }

    fn mission_run_publisher(&self, run: &str) -> Result<Option<String>> {
        let connection = self.readers.get();
        let id = run.strip_prefix("mission-run/").unwrap_or(run);
        let Some(header) = mission_run_header_tx(&connection, id).optional()? else {
            return Ok(None);
        };
        let root_id = header
            .root_mission_run
            .strip_prefix("mission-run/")
            .unwrap_or(&header.root_mission_run);
        let root = if root_id == header.id {
            header
        } else {
            let Some(root) = mission_run_header_tx(&connection, root_id).optional()? else {
                return Ok(None);
            };
            root
        };
        let mission = root
            .mission
            .strip_prefix("mission/")
            .unwrap_or(&root.mission);
        Ok(connection
            .query_row(
                "SELECT claims.actor FROM mission_revisions
                 JOIN claims ON claims.id=mission_revisions.claim_id
                 WHERE mission_revisions.mission_id=?1 AND mission_revisions.revision=?2",
                params![mission, root.revision],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    fn mechanical_gate_exit_receipt(
            &self,
            requested: &ClaimRecord,
        ) -> Result<Option<(ClaimRecord, Option<String>)>> {
        smallclaims::touched::note_read(|| requested.subject.clone());
        smallclaims::touched::note_read(|| "kind:record.repaired".into());
        if requested.origin != self.origin() {
            return Ok(None);
        }
        let request_position = smallclaims::store::canonical::position_sql("request");
        let launch_position = smallclaims::store::canonical::position_sql("created");
        let exit_position = smallclaims::store::canonical::position_sql("claims");
        let connection = self.readers.get();
        let found = connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h6706f1613b0b owner=fractal_evidence::store::evidence TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
                "WITH current_request AS MATERIALIZED (
                   SELECT request.subject, request.batch_id, request.store_index,
                          request_batch.replica_sequence AS sequence,
                          {request_position} AS position
                   FROM claims request JOIN batches request_batch ON request_batch.id=request.batch_id
                   WHERE request.id=?1 AND request.subject=?2 AND request.kind='gate.requested'
                     AND request.origin=?3 AND request_batch.origin=?3
                     AND NOT EXISTS (SELECT 1 FROM replica_records repaired
                       WHERE repaired.claim_id=request.id AND repaired.state='repaired')
                 ), legacy_launches AS MATERIALIZED (
                   SELECT json_extract(created.body, '$.fields.incarnation_id') AS incarnation,
                          json_extract(created.body, '$.fields.runtime_id') AS runtime_id
                   FROM claims created JOIN batches launch_batch ON launch_batch.id=created.batch_id
                     JOIN current_request
                   WHERE created.subject=?2 AND created.kind='runtime.action.succeeded'
                     AND created.actor IS NULL
                     AND created.origin=?3 AND launch_batch.origin=?3
                     AND json_extract(created.body, '$.fields.action')='start'
                     AND (launch_batch.replica_sequence, {launch_position})
                         > (current_request.sequence, current_request.position)
                     AND (launch_batch.replica_sequence<>current_request.sequence
                          OR created.batch_id=current_request.batch_id)
                     AND NOT EXISTS (SELECT 1 FROM replica_records repaired
                       WHERE repaired.claim_id=created.id AND repaired.state='repaired')
                   ORDER BY CANONICAL_DESC(created) LIMIT 2
                 ), local_launches AS MATERIALIZED (
                   SELECT json_extract(local.body, '$.fields.incarnation_id') AS incarnation,
                          json_extract(local.body, '$.fields.runtime_id') AS runtime_id
                   FROM local_observations local JOIN current_request
                   WHERE local.subject=?2 AND local.kind='runtime.action.succeeded'
                     AND local.actor IS NULL
                     AND json_extract(local.body, '$.fields.action')='start'
                     AND local.after_store_index>=current_request.store_index
                   ORDER BY local.id DESC LIMIT 2
                 ), launches AS MATERIALIZED (
                   SELECT incarnation, runtime_id FROM legacy_launches
                   UNION ALL SELECT incarnation, runtime_id FROM local_launches
                 )
                 SELECT {CLAIM_COLUMNS},
                        (SELECT incarnation FROM launches LIMIT 1),
                        (SELECT COUNT(*) FROM launches),
                        (SELECT runtime_id FROM launches LIMIT 1)
                 FROM claims JOIN batches ON batches.id=claims.batch_id JOIN current_request
                 WHERE claims.subject=?2 AND claims.kind='runtime.observed' AND claims.actor=?2
                   AND claims.origin=?3 AND batches.origin=?3
                   AND (batches.replica_sequence, {exit_position})
                       > (current_request.sequence, current_request.position)
                   AND (batches.replica_sequence<>current_request.sequence
                        OR claims.batch_id=current_request.batch_id)
                   AND NOT EXISTS (SELECT 1 FROM replica_records repaired
                     WHERE repaired.claim_id=claims.id AND repaired.state='repaired')
                 ORDER BY CANONICAL_DESC(claims) LIMIT 1"
            )))?
            .query_row(
                /* sqlite-large-value: allow-for-now lv-h0b479aa9741b owner=fractal_evidence::store::evidence TODO=trace-statement-and-value-representation */ params![requested.id, requested.subject, self.origin()],
                |row| {
                    Ok((
                        claim_from_row(row)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, u64>(11)?,
                        row.get::<_, Option<String>>(12)?,
                    ))
                },
            )
            .optional()?;
        Ok(found
            .filter(|(_, _, launches, runtime_id)| {
                *launches <= 1 && runtime_id.as_deref()
                    .is_none_or(|runtime_id| runtime_id == requested.subject.replace('/', "."))
            })
            .map(|(receipt, incarnation, _, _)| (receipt, incarnation)))
    }

    fn mission_gate_runners(&self) -> Result<Vec<MissionGateRunner>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare(
            "SELECT DISTINCT subject, origin,
                    COALESCE(json_extract(body, '$.fields.owner'),
                      'legacy:' || substr(subject, 16, instr(substr(subject, 16), '/')-1))
             FROM claims request WHERE kind='gate.requested'
               AND (json_extract(body, '$.fields.runner') IN ('exec','loop-metric')
                    OR json_extract(body, '$.fields.model') IS NOT NULL)
               AND NOT EXISTS (
                 SELECT 1 FROM claims stopped
                 WHERE stopped.subject=request.subject AND stopped.kind='runtime.observed'
                   AND json_extract(stopped.body, '$.fields.status')='stopped'
               )",
        )?;
        let mut requests = BTreeMap::<String, Vec<(String, String)>>::new();
        for row in statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })? {
            let (subject, host, owner) = row?;
            // A runner leaves this list once its stop is observed on it.
            smallclaims::touched::note_read(|| subject.clone());
            requests.entry(owner).or_default().push((subject, host));
        }
        // New requests, and owners that finish or move on.
        for kind in [
            "gate.requested",
            "step-run.state",
            "mission-run.state",
            "run-generation.state",
        ] {
            smallclaims::touched::note_read(|| format!("kind:{kind}"));
        }
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        // Match by a map rather than an OR/prefix SQL join. That join visits every step
        // for every historical gate request and stalls reconciliation as runs accumulate.
        let mut owners = connection.prepare(
            "SELECT step.subject, 'mission-run/' || run.id,
                    step.status IN ('completed','failed','cancelled')
                    OR generation.status IN ('completed','failed','cancelled','superseded')
                    OR run.phase='terminal' OR run.phase LIKE 'cleanup-%'
                    OR root.phase='terminal' OR root.phase LIKE 'cleanup-%'
             FROM step_runs step
             JOIN mission_runs run ON run.id=step.run_id
             JOIN mission_runs root ON root.id=run.root_run_id
             JOIN run_generations generation ON generation.id=step.generation_id
             UNION ALL
             SELECT 'mission-run/' || run.id, 'mission-run/' || run.id,
                    run.phase IN ('final-cancelled','terminal') OR run.phase LIKE 'cleanup-%'
                    OR run.status IN ('completed','failed','cancelled')
                    OR root.phase='terminal' OR root.phase LIKE 'cleanup-%'
             FROM mission_runs run
             JOIN mission_runs root ON root.id=run.root_run_id",
        )?;
        let mut runners = Vec::new();
        for row in owners.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })? {
            let (owner, owner_run, retired) = row?;
            for key in [owner.clone(), format!("legacy:{}", owner.replace('/', "."))] {
                if let Some(requests) = requests.get(&key) {
                    runners.extend(requests.iter().map(|(subject, host)| MissionGateRunner {
                        subject: subject.clone(),
                        host: host.clone(),
                        owner_run: owner_run.clone(),
                        owner_step: owner.starts_with("step-run/").then(|| owner.clone()),
                        retired,
                    }));
                }
            }
        }
        runners.sort_by(|left, right| left.subject.cmp(&right.subject));
        Ok(runners)
    }

    fn gate_request_for_owner(&self, owner: &str) -> Result<Option<ClaimRecord>> {
        let connection = self.readers.get();
        connection
            .query_row(
                /* sqlite-large-value: allow-for-now lv-hb6ab7e0bb7a5 owner=fractal_evidence::store::evidence TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id, store_index, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms
                 FROM claims WHERE kind='gate.requested'
                 AND json_extract(body, '$.fields.owner')=?1
                 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
                [owner],
                claim_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn human_gate_requests(&self) -> Result<Vec<(String, String, String)>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h9b347ba24aa6 owner=fractal_evidence::store::evidence TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
            "SELECT request.id, json_extract(request.body, '$.fields.owner'), {}
             FROM claims request INDEXED BY claims_human_gate_request_index
             WHERE request.kind='gate.requested' AND json_extract(request.body, '$.fields.reviewer') IS NOT NULL
               AND json_extract(request.body, '$.fields.owner') IS NOT NULL
             ORDER BY CANONICAL_ASC(request)",
            human_review_reassignment::reviewer_sql("request", None),
        )))?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn harness_auth_evidence(
            &self,
            subject: &str,
            incarnation: &str,
        ) -> Result<Option<Value>> {
        let connection = self.readers.get();
        connection
            .query_row(
                /* sqlite-large-value: allow-for-now lv-he1c4046f31f1 owner=fractal_evidence::store::evidence TODO=trace-statement-and-value-representation */ &canonical_sql(
                    "SELECT body FROM claims INDEXED BY claims_harness_auth_incarnation_index WHERE subject=?1 AND kind='harness.observed'
             AND json_extract(body, '$.fields.incarnation_id')=?2
             AND json_type(body, '$.fields.provider_auth') IN ('true','false')
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
                ),
                params![subject, incarnation],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|body| Ok(serde_json::from_str::<Value>(&body)?["fields"].clone()))
            .transpose()
    }
}

