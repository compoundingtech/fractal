use super::*;

impl Store {
    /// Incoming named edges, using the target index rather than scanning declaration bodies.
    /// While a preexisting store's edge index is still being folded in bounded background
    /// steps, the same answer comes straight from the selected declarations.
    pub fn declared_resource_referrers(&self, subject: &str) -> Result<Vec<Value>> {
        let connection = self.readers.get();
        if resource_references::backfilling(&connection)? {
            return resource_references::referrers_from_declarations(&connection, subject);
        }
        let mut statement = connection.prepare_cached(
            "SELECT owner, name, reason FROM declared_resource_edges WHERE target=?1 AND relation='declared-resource' ORDER BY owner, name",
        )?;
        let rows = statement.query_map([subject], |row| {
            Ok(json!({
                "owner": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "reason": row.get::<_, Option<String>>(2)?,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn mission_spec(
        &self,
        mission_id: &str,
        revision: Option<&str>,
    ) -> Result<Option<MissionSpec>> {
        smallclaims::touched::note_read(|| format!("mission/{mission_id}"));
        let connection = self.readers.get();
        let body = if let Some(revision) = revision {
            connection
                .query_row(
                    "SELECT body FROM mission_revisions WHERE mission_id=?1 AND revision=?2",
                    params![mission_id, revision],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
        } else {
            connection
                .query_row(
                    "SELECT r.body FROM mission_definitions d JOIN mission_revisions r ON r.mission_id=d.mission_id AND r.revision=d.revision WHERE d.mission_id=?1",
                    [mission_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
        };
        body.map(|body| serde_json::from_str(&body))
            .transpose()
            .map_err(Into::into)
    }

    /// Name steps for display in one read: each step's mission, run, path, title, first goal,
    /// status and last change. Unknown subjects are left out.
    pub fn step_labels(&self, subjects: &[String]) -> Result<BTreeMap<String, StepLabel>> {
        if subjects.is_empty() {
            return Ok(BTreeMap::new());
        }
        let connection = self.readers.get();
        step_labels::read(&connection, subjects)
    }

    pub fn mission_definitions_for_ids(
        &self,
        ids: &[String],
    ) -> Result<Vec<MissionDefinitionView>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.readers.get();
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT r.body, c.accepted_at_unix_ms FROM mission_definitions d
             JOIN mission_revisions r ON r.mission_id=d.mission_id AND r.revision=d.revision
             JOIN claims c ON c.id=d.claim_id WHERE d.mission_id IN ({placeholders})"
        );
        let bare = ids.iter().map(|id| id.trim_start_matches("mission/"));
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-he8c8a96f10dc owner=st3_store::store::missions TODO=trace-statement-and-value-representation */ &sql)?;
        statement
            .query_map(rusqlite::params_from_iter(bare), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .map(|row| {
                let (body, accepted) = row?;
                Ok(MissionDefinitionView {
                    mission: serde_json::from_str(&body)?,
                    updated_at_unix_ms: accepted.parse()?,
                })
            })
            .collect()
    }

    pub fn mission_run_subject_for_idempotency_key(&self, idempotency_key: &str) -> String {
        format!(
            "mission-run/{}",
            &hex::encode(Sha256::digest(
                format!("{}:{idempotency_key}", self.origin).as_bytes()
            ))[..32]
        )
    }

    pub fn create_mission_run_inner(
        &self,
        request: &MissionRunRequest,
        child: Option<ChildMissionContext>,
        occurrence_subject: Option<&str>,
        latest_ref: Option<(&str, &str)>,
    ) -> Result<MissionRunView, St3Error> {
        let mission_id = request
            .mission
            .strip_prefix("mission/")
            .unwrap_or(&request.mission);
        if child.is_none() && mission_id.starts_with("__st3/") {
            return Err(St3Error::new(
                "internal-mission",
                "an embedded loop mission can start only through its parent loop",
            ));
        }
        owned_sets::guard_mission_start(&self.readers.get(), mission_id)?;
        let mission = self
            .mission_spec(mission_id, request.revision.as_deref())
            .map_err(internal)?
            .ok_or_else(|| {
                St3Error::new(
                    "missing-mission",
                    format!("mission `{mission_id}` does not exist"),
                )
            })?;
        if mission.state == MissionState::Retired {
            return Err(St3Error::new(
                "mission-retired",
                format!(
                    "mission `{mission_id}` is retired; publish a ready revision to start it again"
                ),
            ));
        }
        if mission.state != MissionState::Ready {
            return Err(St3Error::new(
                "mission-not-ready",
                format!("mission `{mission_id}` is not ready"),
            ));
        }
        let request_hash = hex::encode(Sha256::digest(
            serde_json::to_vec(&json!({
                "request": request,
                "parent": child.as_ref().map(|child| json!({
                    "root_revision": child.root_revision,
                    "root_run_id": child.root_run_id,
                    "parent_step_run": child.parent_step_run,
                    "default_selector": child.default_selector,
                })),
            }))
            .map_err(internal)?,
        ));
        let mut connection = self.connection.write();
        if let Some(subject) = occurrence_subject
            && let Some(run) = connection
                .query_row(
                    "SELECT id FROM mission_runs WHERE id=?1",
                    [subject.trim_start_matches("mission-run/")],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(internal)?
        {
            return mission_run_view_tx(&connection, &run).map_err(internal);
        }
        smallclaims::store::idempotency::cached_response(&connection, &request.idempotency_key)?;
        if let Some((response, stored_hash)) = connection
            .query_row(
                "SELECT i.response, r.request_hash FROM idempotency i JOIN mission_run_requests r ON r.operation_id=i.operation_id WHERE i.operation_id=?1",
                [opaque_cache_key(&request.idempotency_key)],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(internal)?
        {
            if stored_hash != request_hash {
                return Err(St3Error::new(
                    "idempotency-mismatch",
                    "the mission-run idempotency key was used with different input",
                ));
            }
            return serde_json::from_str(&response).map_err(internal);
        }
        let transaction = connection.transaction().map_err(internal)?;
        owned_sets::guard_mission_start(&transaction, mission_id)?;
        if let Some((resource, discovery)) = latest_ref
            && let Some(reason) = stale_ref_request_tx(&transaction, resource, discovery).map_err(internal)? {
            return Err(St3Error::new("stale-ref-head", reason));
        }
        let inputs = resolve_mission_run_inputs(&transaction, &mission, &request.inputs)?;
        if let Some((resource, discovery)) = latest_ref {
            if let Some(reason) = stale_pull_request_request_tx(&transaction, resource, discovery)
                .map_err(internal)?
            {
                return Err(St3Error::new("stale-pull-request", reason));
            }
            let is_pull_request = claim_by_id_tx(&transaction, discovery)
                .map_err(internal)?
                .is_some_and(|claim| {
                    claim.subject == resource
                        && claim.body.pointer("/fields/kind").and_then(Value::as_str)
                            == Some("vcs.pull-request")
                });
            if is_pull_request {
                let completed: Option<String> = transaction
                    .query_row(
                        "SELECT id FROM mission_runs WHERE mission_id=?1 AND status='completed'
                     AND EXISTS(SELECT 1 FROM json_each(mission_runs.inputs) AS input
                         WHERE json_extract(input.value, '$.subject')=?2
                         AND json_extract(input.value, '$.claim_id')=?3)
                     ORDER BY id LIMIT 1",
                        params![mission_id, resource, discovery],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(internal)?;
                if let Some(run) = completed {
                    return Err(St3Error::new(
                        "completed-subscription-snapshot",
                        format!("mission-run/{run} already completed for {resource}@{discovery}"),
                    ));
                }
            }
        }
        enforce_mission_run_capacity(&transaction, &mission)?;
        let subject = occurrence_subject.map(str::to_owned).unwrap_or_else(|| {
            self.mission_run_subject_for_idempotency_key(&request.idempotency_key)
        });
        let run_id = subject.trim_start_matches("mission-run/").to_owned();
        let generation_key = occurrence_subject.map_or_else(
            || format!("{}:{}:generation:1", self.origin, request.idempotency_key),
            |subject| format!("{subject}:generation:1"),
        );
        let generation_id = hex::encode(Sha256::digest(generation_key.as_bytes()))[..32].to_owned();
        let generation_subject = format!("run-generation/{generation_id}");
        let root_revision = child
            .as_ref()
            .map(|child| child.root_revision.clone())
            .unwrap_or_else(|| mission.revision.clone());
        let root_run_id = child
            .as_ref()
            .map(|child| child.root_run_id.clone())
            .unwrap_or_else(|| run_id.clone());
        let root_mission_run = format!("mission-run/{root_run_id}");
        let parent_step_run = child.as_ref().map(|child| child.parent_step_run.clone());
        let default_selector = child
            .as_ref()
            .and_then(|child| child.default_selector.clone());
        let requester = normalize_actor(
            request.requester.as_deref().unwrap_or("person/requester"),
            "person",
        );
        let mode = request.mode.as_deref().unwrap_or("run");
        if !matches!(mode, "run" | "eval") {
            return Err(St3Error::new(
                "invalid-run-mode",
                format!("run mode `{mode}` is not registered"),
            ));
        }
        validate_mission_run_timeout(&mission, mode)?;
        let mut variables = BTreeMap::from([
            ("ST_MISSION".into(), mission.id.clone()),
            ("ST_MISSION_REVISION".into(), mission.revision.clone()),
            ("ST_MISSION_RUN".into(), run_id.clone()),
            ("ST_RUN_GENERATION".into(), generation_id.clone()),
            ("ST_REQUESTER".into(), requester.clone()),
            ("ST_ROOT_MISSION_RUN".into(), root_mission_run.clone()),
            (
                "ST_ROOT_MISSION_RUN_ID".into(),
                root_mission_run
                    .strip_prefix("mission-run/")
                    .unwrap_or(&root_mission_run)
                    .into(),
            ),
            ("ST_WORKSPACE".into(), request.workspace.clone()),
            (
                "ST_PARENT_STEP_RUN".into(),
                parent_step_run.clone().unwrap_or_default(),
            ),
        ]);
        for (name, input) in &inputs {
            variables.insert(format!("input.{name}"), input.value.clone());
        }
        extend_loop_variables(&mut variables, &inputs);
        let now = now_ms();
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-hf473e6b602b5 owner=st3_store::store::missions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs(id, mission_id, initial_revision, current_generation_id, root_revision, root_run_id, parent_step_run, workspace, requester, inputs, mode, status, phase, created_at_unix_ms, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'running', 'normal', ?12, ?12)",
                params![run_id, mission.id, mission.revision, generation_id, root_revision, root_run_id, parent_step_run, request.workspace, requester, serde_json::to_string(&inputs).map_err(internal)?, mode, now.to_string()],
            )
            .map_err(internal)?;
        let deadline_at_unix_ms =
            insert_mission_deadline_tx(&transaction, &run_id, mission.timeout_ms, Some(now))?;
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h034c707c5f4c owner=st3_store::store::missions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO run_generations(id, run_id, revision, predecessor_id, status, actor, reason, created_at_unix_ms, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, NULL, 'running', ?4, 'initial mission run', ?5, ?5)",
                params![generation_id, run_id, mission.revision, requester, now.to_string()],
            )
            .map_err(internal)?;
        let mut flat = Vec::new();
        flatten_steps(&mission, default_selector.clone(), &[], &mut flat);
        let parents = crate::mission::parent_step_index(&mission);
        for (step, selector, constraints) in flat {
            let (assignee, available_to, agentless) = interpolate_selector(&selector, &variables)?;
            let step_subject = format!("step-run/{generation_id}/{}", step.path);
            let mut step_variables = variables.clone();
            step_variables.insert("ST_STEP".into(), step.path.clone());
            step_variables.insert("ST_STEP_RUN".into(), step_subject.clone());
            step_variables.insert("ST_ATTEMPT".into(), "1".into());
            step_variables.insert("ST_ASSIGNEE".into(), assignee.clone().unwrap_or_default());
            step_variables.insert(
                "ST_PARENT_STEP_RUN".into(),
                parents
                    .get(step.path.as_str())
                    .copied()
                    .flatten()
                    .map(|path| format!("step-run/{generation_id}/{path}"))
                    .or_else(|| parent_step_run.clone())
                    .unwrap_or_default(),
            );
            let title = step
                .title
                .as_deref()
                .map(|value| crate::mission::interpolate(value, &step_variables))
                .transpose()?;
            let goals = interpolate_goals(&step.goals, &step_variables)?;
            let constraints = interpolate_goals(&constraints, &step_variables)?;
            transaction
                .execute(
                    /* sqlite-large-value: allow-for-now lv-ha40aca118e21 owner=st3_store::store::missions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs(subject, run_id, generation_id, step_path, definition_hash, status, attempt, assignee, available_to, agentless, title, goals, created_at_unix_ms, updated_at_unix_ms, constraints)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 1, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12)",
                    params![step_subject, run_id, generation_id, step.path, step.definition_hash, assignee, serde_json::to_string(&available_to).map_err(internal)?, agentless, title, goals, now.to_string(), constraints],
                )
                .map_err(internal)?;
        }
        let body = json!({
            "fields": {
                "status": "running",
                "mission": mission.subject,
                "revision": mission.revision,
                "initial_revision": mission.revision,
                "current_generation": generation_subject,
                "root_revision": root_revision,
                "root_mission_run": root_mission_run,
                "parent_step_run": parent_step_run,
                "default_selector": default_selector,
                "workspace": request.workspace,
                "requester": requester,
                "inputs": inputs,
                "mode": mode,
                "timeout_ms": mission.timeout_ms,
                "deadline_at_unix_ms": deadline_at_unix_ms,
            }
        });
        let mut body = body;
        add_run_report_fields(&mut body, &mission, None, false);
        append_receipt_claim_tx(
            &transaction,
            &self.origin,
            &subject,
            "mission-run.created",
            Some(&requester),
            &body,
            &[],
            None,
            &request.idempotency_key,
        )
        .map_err(internal)?;
        append_claim_tx(
            &transaction,
            &self.origin,
            &generation_subject,
            "run-generation.created",
            Some(&requester),
            &json!({"fields": {
                "run": subject,
                "revision": mission.revision,
                "status": "running",
                "reason": "initial mission run"
            }}),
            &[],
            None,
        )
        .map_err(internal)?;
        let view = mission_run_view_tx(&transaction, &run_id).map_err(internal)?;
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-hc53874ccccbd owner=st3_store::store::missions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO idempotency(operation_id, response) VALUES (?1, ?2)",
                params![
                    opaque_cache_key(&request.idempotency_key),
                    serde_json::to_string(&view).map_err(internal)?
                ],
            )
            .map_err(internal)?;
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h81dad04cbe21 owner=st3_store::store::missions TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_run_requests(operation_id, request_hash) VALUES (?1, ?2)",
                params![opaque_cache_key(&request.idempotency_key), request_hash],
            )
            .map_err(internal)?;
        transaction.commit().map_err(internal)?;
        Ok(view)
    }

    pub fn mission_run(&self, run: &str) -> Result<Option<MissionRunView>> {
        let run = run.strip_prefix("mission-run/").unwrap_or(run);
        smallclaims::touched::note_read(|| format!("mission-run/{run}"));
        let connection = self.readers.get();
        let view = mission_run_view_tx(&connection, run).optional()?;
        if let Some(view) = &view {
            note_run_view_reads(view);
        }
        Ok(view)
    }

    /// Lightweight run headers for collection projections that do not render steps.
    /// Unlike `mission_runs`, this avoids per-step queue and loop history scans.
    pub fn mission_run_headers(&self) -> Result<Vec<MissionRunView>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h982087d6b28e owner=st3_store::store::missions TODO=trace-statement-and-value-representation */ MISSION_RUN_HEADER_SELECT)?;
        statement
            .query_map([], mission_run_header_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// One run's view with the effective step state and definitions the reconciler needs.
    ///
    /// The presentation view scans message history for each step's wake badge. Doing that for
    /// every active run on every graph change is quadratic in fleet history and can saturate an
    /// otherwise idle daemon.
    pub fn mission_run_for_reconcile(&self, id: &str) -> Result<MissionRunView> {
        let connection = self.readers.get();
        let view = mission_run_view_for_reconcile_tx(&connection, id)?;
        note_run_view_reads(&view);
        Ok(view)
    }

    /// Return the work fields needed by the reconciler without computing CLI-only
    /// timing and wake annotations. Those annotations scan immutable history and
    /// are intentionally too expensive for the daemon's inner control loop.
    pub fn work_for_reconcile(&self, actor: &str) -> Result<Vec<StepRunView>> {
        let work = reconcile_work_context::read(self, actor, now_ms())?;
        // A generation can add steps this seat has not read yet. Existing steps already
        // register their own subjects below; their state changes must not invalidate every
        // seat. Handoffs mark the new recipient's actor key in incremental::change_keys.
        smallclaims::touched::note_read(|| format!("actor:{actor}"));
        smallclaims::touched::note_read(|| "kind:run-generation.created".to_owned());
        // Proposals enter and leave revision-draining on the proposal subject,
        // not the run subject. Approval and cancellation receipts do not carry
        // the run, so retain these collection dependencies without another query.
        for kind in [
            "revision-proposal.created",
            "revision-proposal.approved",
            "revision-proposal.cancelled",
        ] {
            smallclaims::touched::note_read(|| format!("kind:{kind}"));
        }
        for step in &work {
            smallclaims::touched::note_read(|| step.subject.clone());
            smallclaims::touched::note_read(|| step.run.clone());
        }
        Ok(work)
    }

    /// One current-step scan for the whole roster. This avoids replaying wake
    /// history or querying the step table separately for every agent card.
    pub fn agent_work_queues(&self) -> Result<BTreeMap<String, AgentWorkQueue>> {
        self.agent_work_queues_for(None)
    }

    /// Build queue previews from the same selector, reading only one agent's inputs when named.
    pub fn agent_work_queues_for(
        &self,
        selected_agent: Option<&str>,
    ) -> Result<BTreeMap<String, AgentWorkQueue>> {
        crate::performance::task("agent_work_queues", || -> Result<BTreeMap<String, AgentWorkQueue>> {
        let connection = self.readers.get();
        let orders = seat_run_orders_tx(&connection, selected_agent)?;
        let rows = seat_step_rows_tx(&connection, selected_agent)?;
        let mut seats = BTreeMap::<&str, Vec<SeatStep<'_>>>::new();
        for row in &rows {
            let step = row.seat_step();
            for agent in [row.assignee.as_deref(), row.claimant.as_deref()]
                .into_iter()
                .flatten()
                .collect::<BTreeSet<_>>()
            {
                if selected_agent.is_some_and(|selected| selected != agent) {
                    continue;
                }
                seats.entry(agent).or_default().push(step);
            }
        }
        let mut queues = BTreeMap::<String, AgentWorkQueue>::new();
        for (agent, steps) in seats {
            let order = orders.get(agent).map(Vec::as_slice).unwrap_or_default();
            let selection = seat_queue::select(agent, &steps, order);
            if selection.held.is_empty() && selection.ready.is_empty() {
                continue;
            }
            queues.insert(
                agent.to_owned(),
                AgentWorkQueue {
                    current_work_ids: selection
                        .held
                        .iter()
                        .take(AGENT_WORK_PREVIEW_LIMIT)
                        .map(|subject| (*subject).to_owned())
                        .collect(),
                    active_work_count: selection.held.len() as u64,
                    next_work_id: selection.next().map(str::to_owned),
                    upcoming_work_ids: selection
                        .ready
                        .iter()
                        .take(AGENT_WORK_PREVIEW_LIMIT)
                        .map(|subject| (*subject).to_owned())
                        .collect(),
                    queued_work_count: selection.ready.len() as u64,
                },
            );
        }
        Ok(queues)
        })
    }

    pub fn step_run(&self, subject: &str) -> Result<Option<StepRunView>> {
        let subject = normalize_step_run(subject);
        let connection = self.readers.get();
        let mut view = connection
            .query_row(
                "SELECT subject, run_id, step_path, definition_hash, status, attempt, assignee, available_to, agentless, title, goals, worker_reported,
                        lease_owner, lease_incarnation, lease_expires_at_unix_ms, blocked_reason, not_before_unix_ms, created_at_unix_ms, updated_at_unix_ms, readiness_epoch, constraints
                 FROM step_runs WHERE subject=?1",
                [subject],
                step_run_from_row,
            )
            .optional()
            .map_err(anyhow::Error::from)?;
        if let Some(view) = &mut view {
            enrich_step_queue(&connection, view)?;
        }
        Ok(view)
    }

    pub fn mission(
        &self,
        intent: &NormalizedIntent,
        resolved_intent: IntentInput,
    ) -> Result<MissionResponse, St3Error> {
        self.mission_at(intent, resolved_intent, None)
    }

    pub fn mission_at(
        &self,
        intent: &NormalizedIntent,
        resolved_intent: IntentInput,
        at_index: Option<u64>,
    ) -> Result<MissionResponse, St3Error> {
        self.mission_at_with_declared_agents(intent, resolved_intent, at_index, &BTreeSet::new())
    }

    pub fn mission_at_with_declared_agents(
        &self,
        intent: &NormalizedIntent,
        resolved_intent: IntentInput,
        at_index: Option<u64>,
        declared_agents: &BTreeSet<String>,
    ) -> Result<MissionResponse, St3Error> {
        let connection = self.readers.get();
        let current_index = current_index(&connection).map_err(internal)?;
        let store_index = selected_index(current_index, at_index)?;
        let mut changes = Vec::new();
        let mut tokens = BTreeMap::new();
        let mut actions = Vec::new();
        let mut blockers = Vec::new();
        let mut warnings = Vec::new();
        if let Err(error) = message_subscriptions::validate(&connection, intent, store_index) {
            blockers.push(format!("{}: {}", error.code, error.message));
        }
        if let Err(error) = crate::provenance::validate_publication(&connection, intent) {
            blockers.push(format!("{}: {}", error.code, error.message));
        }

        if intent.deprecated_syntax.contains("pty") {
            warnings.push(
                "deprecated-kdl-node: `pty {}` is temporarily accepted; use canonical `terminal {}` before the friend-ready v0"
                    .into(),
            );
        }

        for reference in &intent.document_refs {
            let Some((name, hash)) = reference.rsplit_once('@') else {
                blockers.push(format!(
                    "document `{reference}` has no selected binding; run `st documents put` first"
                ));
                continue;
            };
            let exists = connection
                .query_row(
                    "SELECT 1 FROM documents WHERE name = ?1 AND hash = ?2 AND created_index<=?3",
                    params![name, hash, store_index],
                    |_| Ok(()),
                )
                .optional()
                .map_err(internal)?
                .is_some();
            if !exists {
                blockers.push(format!(
                    "missing document `{reference}`; run `st documents put` first"
                ));
                continue;
            }
            let latest: Option<String> = connection
                .query_row(
                    /* sqlite-large-value: allow-for-now lv-h92a5309dc47b owner=st3_store::store::missions TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT documents.hash FROM documents WHERE documents.name = ?1 AND documents.created_index<=?2 ORDER BY binding_key DESC LIMIT 1"),
                    params![name, store_index],
                    |row| row.get(0),
                )
                .optional()
                .map_err(internal)?;
            if latest.as_deref() != Some(hash) {
                warnings.push(format!(
                    "`{reference}` is not the latest version of `{name}`; the requested version remains valid"
                ));
            }
        }

        let known = |subject: &str| -> Result<bool, St3Error> {
            if intent.subjects.contains_key(subject) || declared_agents.contains(subject) {
                return Ok(true);
            }
            connection
                .query_row(
                    "SELECT 1 FROM claims WHERE subject=?1 AND store_index<=?2 LIMIT 1",
                    params![subject, store_index],
                    |_| Ok(()),
                )
                .optional()
                .map(|value| value.is_some())
                .map_err(internal)
        };
        for desired in intent.subjects.values() {
            if desired.kind == "agent" {
                for grouping in crate::graph::agent_under(&desired.desired) {
                    if grouping.agent == desired.subject {
                        warnings.push(format!(
                            "agent `{}` is grouped under itself",
                            desired.subject
                        ));
                    } else if !known(&grouping.agent)? {
                        warnings.push(format!(
                            "agent `{}` is grouped under missing agent `{}`",
                            desired.subject, grouping.agent
                        ));
                    }
                }
            }
            if desired.kind == "message"
                && let Some(to) = canonical_child_string(&desired.desired, "to")
            {
                let recipient = if to == "requester" || to.contains('/') {
                    to
                } else {
                    format!("agent/{to}")
                };
                if recipient != "requester"
                    && !recipient.starts_with("person/")
                    && !known(&recipient)?
                {
                    blockers.push(format!(
                        "message `{}` references undeclared recipient `{recipient}`",
                        desired.subject
                    ));
                }
            }
            if desired.kind == "observer"
                && let Some(observer) = crate::graph::observer_spec(&desired.desired)
                && !observer.stopped
                && !known(&observer.resource)?
            {
                warnings.push(format!(
                    "observer `{}` references missing resource `{}`",
                    desired.subject, observer.resource
                ));
            }
            if desired.kind == "subscription"
                && let Some(subscription) = crate::graph::subscription_spec(&desired.desired)
                && !subscription.stopped
            {
                if crate::github_watch::standing_locator(&subscription.observer).is_none()
                    && !known(&subscription.observer)?
                {
                    warnings.push(format!(
                        "subscription `{}` references missing observer `{}`",
                        desired.subject, subscription.observer
                    ));
                }
                if subscription.delivery == "message" && !known(&subscription.to)? {
                    warnings.push(format!(
                        "subscription `{}` has missing delivery target `{}`",
                        desired.subject, subscription.to
                    ));
                }
                if subscription.delivery == "mission"
                    && let (Some(mission_id), Some(revision), Some(input)) = (
                        subscription.mission.as_deref(),
                        subscription.revision.as_deref(),
                        subscription.resource_input.as_deref(),
                    )
                {
                    let local = connection
                        .query_row(
                            "SELECT body FROM mission_revisions
                             WHERE mission_id=?1 AND revision=?2 AND created_index<=?3",
                            params![mission_id, revision, store_index],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()
                        .map_err(internal)?
                        .map(|body| serde_json::from_str::<MissionSpec>(&body).map_err(internal))
                        .transpose()?;
                    let published = intent
                        .missions
                        .get(mission_id)
                        .filter(|mission| mission.revision == revision)
                        .cloned()
                        .or(local);
                    match published {
                        Some(mission)
                            if mission
                                .inputs
                                .get(input)
                                .is_none_or(|declaration| declaration.kind != MissionInputKind::Resource) => blockers.push(
                            format!(
                                "subscription `{}` uses undeclared resource input `{input}` in mission/{}@{revision}",
                                desired.subject, mission_id
                            ),
                        ),
                        None => blockers.push(format!(
                            "subscription `{}` references unpublished mission/{}@{revision}",
                            desired.subject, mission_id
                        )),
                        _ => {}
                    }
                }
            }
        }
        for mission in intent.missions.values() {
            append_review_remediation_warnings(mission, &mut warnings);
            let mut selectors = mission.work_selector.iter().cloned().collect::<Vec<_>>();
            selectors.extend(
                flatten_mission_step_specs(mission)
                    .into_iter()
                    .filter_map(|step| step.work_selector.clone()),
            );
            let mut flat = Vec::new();
            flatten_steps(mission, None, &[], &mut flat);
            let selected = flat
                .iter()
                .flat_map(|(_, selector, _)| selector_agents(selector))
                .collect::<BTreeSet<_>>();
            for agent in run_agent_constraints(mission).keys() {
                if !selected.contains(agent) {
                    warnings.push(format!(
                        "mission `{}` gives agent `{agent}` constraints, but no step selects that agent",
                        mission.subject
                    ));
                }
            }
            for selector in &selectors {
                for agent in selector_agents(selector) {
                    let owned_runtime =
                        agent
                            .strip_prefix("agent/${ST_MISSION_RUN}/")
                            .is_some_and(|local| {
                                mission.revision_owners.iter().any(|owner| {
                                    owner == &format!("agent/{local}")
                                        || owner
                                            .rsplit_once('.')
                                            .is_some_and(|(_, name)| name == local)
                                })
                            });
                    if !owned_runtime && !mission.revision_owners.contains(agent) && !known(agent)?
                    {
                        warnings.push(format!(
                            "mission `{}` references missing eligible agent `{agent}`",
                            mission.subject
                        ));
                    }
                }
            }
        }
        let grouping_edges = intent
            .subjects
            .values()
            .filter(|desired| desired.kind == "agent")
            .map(|desired| {
                (
                    desired.subject.clone(),
                    crate::graph::agent_under(&desired.desired)
                        .into_iter()
                        .map(|under| under.agent)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for start in grouping_edges.keys() {
            let mut frontier = vec![start.as_str()];
            let mut visited = BTreeSet::new();
            while let Some(agent) = frontier.pop() {
                if !visited.insert(agent) {
                    continue;
                }
                for parent in grouping_edges.get(agent).into_iter().flatten() {
                    if parent == start && agent != start {
                        warnings.push(format!("agent grouping for `{start}` contains a cycle"));
                    } else if grouping_edges.contains_key(parent) {
                        frontier.push(parent);
                    }
                }
            }
        }
        blockers.sort();
        blockers.dedup();
        warnings.sort();
        warnings.dedup();

        for (subject, desired) in &intent.subjects {
            let current = desired_row_at(&connection, subject, at_index).map_err(internal)?;
            let revision = desired_revision(desired);
            let changed = !desired_matches(current.as_ref(), desired, &revision)?;
            tokens.insert(
                subject.clone(),
                intent_leaves_at(&connection, subject, at_index).map_err(internal)?,
            );
            if changed {
                changes.push(SubjectChange {
                    subject: subject.clone(),
                    change: if current.is_some() {
                        "update"
                    } else {
                        "create"
                    }
                    .into(),
                    old_revision: current.as_ref().map(|row| row.revision.clone()),
                    new_revision: revision,
                });
            }
            if !changed && desired.member.is_none() {
                continue;
            }
            if let Some(action) = crate::suspension::publication_action(self, desired, false, || {
                Ok(current.as_ref().and_then(|row| row.member.as_deref())
                    .map(serde_json::from_str::<crate::model::MemberSpec>)
                    .transpose()?)
            })
                .map_err(internal)?
            {
                if changed || action.action == "restart" {
                    actions.push(action);
                }
            } else if desired.kind == "stop" {
                actions.push(PlannedAction {
                    subject: subject.clone(),
                    action: "stop".into(),
                    reason: "the desired state explicitly stops this member".into(),
                });
            } else if desired.member.is_some() && changed {
                actions.push(PlannedAction {
                    subject: subject.clone(),
                    action: "observe-or-start".into(),
                    reason: "the desired member is active".into(),
                });
            }
        }
        for mission in intent.missions.values() {
            let subject = mission.subject.clone();
            let current: Option<(String, String)> = connection
                .query_row(
                    "SELECT revision, claim_id FROM mission_definitions WHERE mission_id=?1",
                    [&mission.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(internal)?;
            tokens.insert(
                subject.clone(),
                current
                    .as_ref()
                    .map(|(_, claim)| vec![claim.clone()])
                    .unwrap_or_default(),
            );
            if current
                .as_ref()
                .is_some_and(|(revision, _)| revision == &mission.revision)
            {
                continue;
            }
            changes.push(SubjectChange {
                subject: subject.clone(),
                change: if current.is_some() {
                    "update"
                } else {
                    "create"
                }
                .into(),
                old_revision: current.map(|(revision, _)| revision),
                new_revision: mission.revision.clone(),
            });
            actions.push(PlannedAction {
                subject,
                action: "publish-mission".into(),
                reason: "the immutable mission revision is not published".into(),
            });
        }
        for declaration in intent.mission_runs.values() {
            prepare_mission_run_declaration(
                &connection,
                declaration,
                store_index,
                &mut tokens,
                &mut actions,
                &mut blockers,
                &mut warnings,
            )?;
        }
        for declaration in intent.planning_sessions.values() {
            prepare_planning_session_declaration(
                &connection,
                declaration,
                store_index,
                &mut tokens,
                &mut actions,
                &mut blockers,
            )?;
        }
        for refresh in &intent.resource_refreshes {
            tokens.entry(refresh.resource.clone()).or_insert(
                claim_ids_at(&connection, &refresh.resource, Some(store_index))
                    .map_err(internal)?
                    .into_iter()
                    .last()
                    .into_iter()
                    .collect(),
            );
            let exists = connection
                .query_row(
                    "SELECT 1 FROM desired WHERE subject=?1 AND kind='resource'",
                    [&refresh.resource],
                    |_| Ok(()),
                )
                .optional()
                .map_err(internal)?
                .is_some();
            if exists {
                match publication_operation_is_new(
                    &connection,
                    &refresh.resource,
                    "refresh",
                    &refresh.id,
                    refresh,
                ) {
                    Ok(true) => actions.push(PlannedAction {
                        subject: refresh.resource.clone(),
                        action: format!("refresh:{}", refresh.id),
                        reason: "the publication requests a fresh observation".into(),
                    }),
                    Ok(false) => {}
                    Err(error) => blockers.push(error.message),
                }
            } else {
                blockers.push(format!("resource `{}` does not exist", refresh.resource));
            }
        }
        for repair in &intent.replica_repairs {
            let repair_subject = format!(
                "repair/{}",
                repair
                    .record_ref
                    .strip_prefix("record/")
                    .unwrap_or(&repair.record_ref)
            );
            tokens.insert(
                repair_subject.clone(),
                claim_ids_at(&connection, &repair_subject, Some(store_index)).map_err(internal)?,
            );
            match validate_replica_repair(&connection, repair) {
                Ok(true) => actions.push(PlannedAction {
                    subject: repair_subject,
                    action: "repair-record".into(),
                    reason: "the invalid record has no matching repair".into(),
                }),
                Ok(false) => {}
                Err(error) => blockers.push(error.message),
            }
        }

        blockers.sort();
        blockers.dedup();
        warnings.sort();
        warnings.dedup();

        let declaration_diffs = declarations::diffs(
            &connection,
            intent,
            changes.iter().map(|change| change.subject.as_str()),
            Some(store_index),
        )?;
        let mut mission_provenance = intent.mission_provenance.clone();
        for (id, mission) in &intent.missions {
            if !mission_provenance.contains_key(id)
                && let Some(provenance) =
                    crate::provenance::read(&connection, id, &mission.revision).map_err(internal)?
            {
                mission_provenance.insert(id.clone(), provenance);
            }
        }
        Ok(MissionResponse {
            declaration_diffs,
            store_index,
            source_hash: intent.source_hash.clone(),
            normalized: intent.normalized.clone(),
            resolved_intent,
            changes,
            predicted_actions: actions,
            blockers,
            warnings,
            subject_tokens: tokens,
            mission_provenance,
            mission_revisions: intent
                .missions
                .values()
                .map(|mission| (mission.subject.clone(), mission.revision.clone()))
                .collect(),
        })
    }

}

const AGENT_WORK_PREVIEW_LIMIT: usize = 5;

#[allow(clippy::too_many_arguments)]
fn prepare_mission_run_declaration(
    connection: &Connection,
    declaration: &MissionRunDeclaration,
    store_index: u64,
    tokens: &mut BTreeMap<String, Vec<String>>,
    actions: &mut Vec<PlannedAction>,
    blockers: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(), St3Error> {
    let run_id = declaration
        .subject
        .strip_prefix("mission-run/")
        .unwrap_or(&declaration.subject);
    tokens.insert(
        declaration.subject.clone(),
        claim_ids_at(connection, &declaration.subject, Some(store_index))
            .map_err(internal)?
            .into_iter()
            .last()
            .into_iter()
            .collect(),
    );
    let current = connection
        .query_row(
            "SELECT mission_id, initial_revision, workspace, requester, status, current_generation_id,
                    inputs, mode
             FROM mission_runs WHERE id=?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(internal)?;
    if let Some(creation) = &declaration.creation {
        if let Some((mission, revision, workspace, requester, _, _, inputs, mode)) = &current {
            let stored_inputs = serde_json::from_str::<BTreeMap<String, MissionRunInput>>(inputs)
                .map_err(internal)?;
            let stored_values = stored_inputs
                .into_iter()
                .map(|(name, input)| (name, input.value))
                .collect::<BTreeMap<_, _>>();
            if mission != &creation.mission
                || revision != &creation.revision
                || workspace != &creation.workspace
                || requester != &creation.requester
                || stored_values != creation.inputs
                || mode != &creation.mode
                || mission_run_after_tx(connection, run_id)? != creation.after
                || mission_run_report_tx(connection, run_id)?
                    != requested_run_report(connection, creation)?
            {
                blockers.push(format!(
                    "mission run `{}` already exists with different creation fields",
                    declaration.subject
                ));
            }
        } else {
            let mission = connection
                .query_row(
                    "SELECT body FROM mission_revisions WHERE mission_id=?1 AND revision=?2",
                    params![creation.mission, creation.revision],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(internal)?
                .map(|body| serde_json::from_str::<MissionSpec>(&body))
                .transpose()
                .map_err(internal)?;
            match mission {
                None => blockers.push(missing_mission_revision(
                    connection,
                    &creation.mission,
                    &creation.revision,
                )?),
                Some(mission) if mission.state != MissionState::Ready => blockers.push(format!(
                    "mission `mission/{}` revision `{}` is not ready",
                    creation.mission, creation.revision
                )),
                Some(mission) => {
                    if let Err(error) =
                        resolve_mission_run_inputs(connection, &mission, &creation.inputs)
                    {
                        blockers.push(error.message);
                    }
                    if let Err(error) = enforce_mission_run_capacity(connection, &mission) {
                        blockers.push(error.message);
                    }
                    if let Some(after) = &creation.after
                        && let Err(error) =
                            ensure_awaited_mission_run_tx(connection, &declaration.subject, after)
                                .and_then(|()| crate::mission::mission_after_run(mission, after))
                    {
                        blockers.push(error.message);
                    }
                    actions.push(PlannedAction {
                        subject: declaration.subject.clone(),
                        action: "start-mission-run".into(),
                        reason: "the named mission run does not exist".into(),
                    });
                }
            }
        }
    }
    if current.is_none() && declaration.creation.is_none() {
        blockers.push(format!(
            "mission run `{}` does not exist",
            declaration.subject
        ));
        return Ok(());
    }
    let status = current.as_ref().map(|value| value.4.as_str());
    let current_generation = current.as_ref().map(|value| value.5.as_str());
    for revision in declaration.revisions.values() {
        if current_generation.is_some_and(|current| {
            revision
                .from_generation
                .strip_prefix("run-generation/")
                .unwrap_or(&revision.from_generation)
                != current
        }) {
            blockers.push(format!(
                "revision `{}` names a stale run generation",
                revision.id
            ));
        }
        let exists = connection
            .query_row(
                "SELECT 1 FROM mission_revisions WHERE mission_id=?1 AND revision=?2",
                params![revision.mission, revision.revision],
                |_| Ok(()),
            )
            .optional()
            .map_err(internal)?
            .is_some();
        if !exists {
            blockers.push(format!(
                "revision `{}` names a mission that is not stored: {}",
                revision.id,
                missing_mission_revision(connection, &revision.mission, &revision.revision)?
            ));
        }
        match publication_operation_is_new(
            connection,
            &declaration.subject,
            "revision",
            &revision.id,
            revision,
        ) {
            Ok(true) => actions.push(PlannedAction {
                subject: declaration.subject.clone(),
                action: format!("revise:{}", revision.id),
                reason: revision.reason.clone(),
            }),
            Ok(false) => {}
            Err(error) => blockers.push(error.message),
        }
    }
    for reset in declaration.resets.values() {
        if current_generation.is_some_and(|current| {
            reset
                .from_generation
                .strip_prefix("run-generation/")
                .unwrap_or(&reset.from_generation)
                != current
        }) {
            blockers.push(format!("reset `{}` names a stale run generation", reset.id));
        }
        match publication_operation_is_new(
            connection,
            &declaration.subject,
            "reset",
            &reset.id,
            reset,
        ) {
            Ok(true) => actions.push(PlannedAction {
                subject: runtime_subject_for_run(run_id, &reset.runtime),
                action: format!("reset:{}", reset.id),
                reason: reset.reason.clone(),
            }),
            Ok(false) => {}
            Err(error) => blockers.push(error.message),
        }
    }
    for cancellation in declaration.cancellations.values() {
        let is_new = match publication_operation_is_new(
            connection,
            &declaration.subject,
            "cancellation",
            &cancellation.id,
            cancellation,
        ) {
            Ok(value) => value,
            Err(error) => {
                blockers.push(error.message);
                continue;
            }
        };
        if !is_new {
            continue;
        } else if matches!(status, Some("completed" | "failed" | "cancelled")) {
            warnings.push(format!(
                "mission run `{}` is already {}",
                declaration.subject,
                status.unwrap_or_default()
            ));
        } else {
            actions.push(PlannedAction {
                subject: declaration.subject.clone(),
                action: format!("cancel:{}", cancellation.id),
                reason: cancellation.reason.clone(),
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prepare_planning_session_declaration(
    connection: &Connection,
    declaration: &PlanningSessionDeclaration,
    store_index: u64,
    tokens: &mut BTreeMap<String, Vec<String>>,
    actions: &mut Vec<PlannedAction>,
    blockers: &mut Vec<String>,
) -> Result<(), St3Error> {
    let id = declaration
        .subject
        .strip_prefix("planning-session/")
        .unwrap_or(&declaration.subject);
    tokens.insert(
        declaration.subject.clone(),
        claim_ids_at(connection, &declaration.subject, Some(store_index))
            .map_err(internal)?
            .into_iter()
            .last()
            .into_iter()
            .collect(),
    );
    let current = connection
        .query_row(
            "SELECT mission_id, request_ref, workspace, requester, planner, status,
                    target_run_id, source_generation_id
             FROM planning_sessions WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()
        .map_err(internal)?;
    if let Some(creation) = &declaration.creation {
        if let Some((mission, request, workspace, requester, planner, _, target, generation)) =
            &current
        {
            if mission != &creation.mission
                || request != &creation.request
                || workspace != &creation.workspace
                || requester != &creation.requester
                || planner != &crate::graph::planning_planner_subject(&declaration.subject)
                || target.as_deref()
                    != creation
                        .target_run
                        .as_deref()
                        .map(|value| value.strip_prefix("mission-run/").unwrap_or(value))
                || generation.as_deref()
                    != creation
                        .target_generation
                        .as_deref()
                        .map(|value| value.strip_prefix("run-generation/").unwrap_or(value))
            {
                blockers.push(format!(
                    "launch `{}` already exists with different creation fields",
                    declaration.subject
                ));
            }
        } else {
            if let (Some(run), Some(generation)) =
                (&creation.target_run, &creation.target_generation)
            {
                let run_id = run.strip_prefix("mission-run/").unwrap_or(run);
                let expected_generation = generation
                    .strip_prefix("run-generation/")
                    .unwrap_or(generation);
                let actual = connection
                    .query_row(
                        "SELECT current_generation_id FROM mission_runs WHERE id=?1",
                        [run_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(internal)?;
                if actual.as_deref() != Some(expected_generation) {
                    blockers.push(format!(
                        "launch `{}` names a stale target generation",
                        declaration.subject
                    ));
                }
            }
            actions.push(PlannedAction {
                subject: declaration.subject.clone(),
                action: "start-launch".into(),
                reason: "the named launch does not exist".into(),
            });
        }
    }
    if current.is_none() && declaration.creation.is_none() {
        blockers.push(format!("launch `{}` does not exist", declaration.subject));
    }
    for feedback in declaration.feedback.values() {
        match publication_operation_is_new(
            connection,
            &declaration.subject,
            "feedback",
            &feedback.id,
            feedback,
        ) {
            Ok(true) => actions.push(PlannedAction {
                subject: declaration.subject.clone(),
                action: format!("feedback:{}", feedback.id),
                reason: "the publication supplies launch feedback".into(),
            }),
            Ok(false) => {}
            Err(error) => blockers.push(error.message),
        }
    }
    for cancellation in declaration.cancellations.values() {
        match publication_operation_is_new(
            connection,
            &declaration.subject,
            "cancellation",
            &cancellation.id,
            cancellation,
        ) {
            Ok(true) => actions.push(PlannedAction {
                subject: declaration.subject.clone(),
                action: format!("cancel:{}", cancellation.id),
                reason: cancellation.reason.clone(),
            }),
            Ok(false) => {}
            Err(error) => blockers.push(error.message),
        }
    }
    Ok(())
}

fn intent_leaves_at(
    connection: &Connection,
    subject: &str,
    at_index: Option<u64>,
) -> Result<Vec<String>> {
    let through = at_index.unwrap_or(i64::MAX as u64);
    let mut statement = connection.prepare_cached(
        "SELECT id, predecessors FROM claims
         WHERE subject=?1 AND kind='intent.desired' AND store_index<=?2
           AND NOT EXISTS (
               SELECT 1 FROM replica_records
               WHERE replica_records.claim_id=claims.id
                 AND replica_records.state='repaired'
           )
         ORDER BY id",
    )?;
    let rows = statement
        .query_map(params![subject, through], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let referenced = rows
        .iter()
        .flat_map(|(_, predecessors)| {
            serde_json::from_str::<Vec<String>>(predecessors).unwrap_or_default()
        })
        .collect::<BTreeSet<_>>();
    Ok(rows
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| !referenced.contains(id))
        .collect())
}

fn append_review_remediation_warnings(mission: &MissionSpec, warnings: &mut Vec<String>) {
    fn describes(step: &crate::model::StepSpec, terms: &[&str]) -> bool {
        let description = std::iter::once(step.id.as_str())
            .chain(step.title.as_deref())
            .chain(step.goals.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        terms.iter().any(|term| description.contains(term))
    }

    for id in &mission.display_order {
        let step = &mission.steps[id];
        let remediation = describes(step, &["fix", "remediat", "repair", "resolve finding"]);
        if remediation {
            for dependency in &step.dependencies {
                let DependencySpec::Step {
                    step: target,
                    state,
                } = dependency
                else {
                    continue;
                };
                let Some(predecessor) = mission.steps.get(target) else {
                    continue;
                };
                if state == "completed"
                    && describes(predecessor, &["review", "validation", "audit"])
                    && !describes(predecessor, &["report-only"])
                {
                    warnings.push(format!(
                        "remediation step `{}` depends on review step `{}` completing; if findings make the review fail, failure propagation makes remediation unreachable. Make the review report findings and complete, or use an explicit bounded review/fix loop",
                        step.path, predecessor.path
                    ));
                }
            }
        }
        if let Some(nested) = &step.nested_mission {
            append_review_remediation_warnings(nested, warnings);
        }
    }
}

/// Who a new declaration of an existing run asks it to report to, resolved against the mission
/// revision the run started from, so that it compares with what the creation claim stored.
fn requested_run_report(
    connection: &Connection,
    creation: &crate::model::MissionRunCreation,
) -> Result<Option<(String, bool, bool)>, St3Error> {
    let mission = mission_revision_spec_tx(connection, &creation.mission, &creation.revision)?;
    Ok(mission.and_then(|mission| {
        resolved_run_report(
            &mission,
            creation.report_to.as_deref(),
            creation.report_completed,
        )
    }))
}

/// Who a run reports to and whether it also reports completion, as its creation claim recorded
/// them.
fn mission_run_report_tx(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<(String, bool, bool)>, St3Error> {
    connection
        .query_row(
            "SELECT json_extract(body, '$.fields.report_to'),
                    COALESCE(json_extract(body, '$.fields.report_completed'), 0),
                    COALESCE(json_extract(body, '$.fields.report_stalls'), 1)
             FROM claims WHERE subject=?1 AND kind='mission-run.created' LIMIT 1",
            [format!("mission-run/{run_id}")],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()
        .map(|row| row.and_then(|(report_to, completed, stalls)| Some((report_to?, completed, stalls))))
        .map_err(internal)
}

fn mission_run_after_tx(connection: &Connection, run_id: &str) -> Result<Option<String>, St3Error> {
    connection
        .query_row(
            "SELECT after_run FROM mission_run_after WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(internal)
}

/// A run view reads its run, generation and step rows, which change only with claims on those
/// subjects.
pub fn note_run_view_reads(view: &MissionRunView) {
    smallclaims::touched::note_read(|| view.subject.clone());
    smallclaims::touched::note_read(|| view.generation.clone());
    smallclaims::touched::note_read(|| format!("generations:{}", view.subject));
    for step in &view.steps {
        smallclaims::touched::note_read(|| step.subject.clone());
    }
}

/// Current held and ready agent steps, for every seat or for one seat.
pub fn seat_step_rows_tx(connection: &Connection, agent: Option<&str>) -> Result<Vec<RosterStepRow>> {
    let snapshot_unix_ms = now_ms();
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hfacf322d08e0 owner=st3_store::store::missions TODO=trace-statement-and-value-representation */ SEAT_STEP_ROWS)?;
    let mut rows = statement
        .query_map(params![agent, snapshot_unix_ms.to_string()], |row| {
            let mut status: String = row.get(3)?;
            let mut claimant = row.get(5)?;
            let expires: Option<String> = row.get(8)?;
            if matches!(status.as_str(), "claimed" | "working" | "verifying")
                && expires
                    .and_then(|value| value.parse::<u128>().ok())
                    .is_some_and(|expiry| expiry <= snapshot_unix_ms)
            {
                status = "ready".into();
                claimant = None;
            }
            Ok(RosterStepRow {
                subject: row.get(0)?,
                run: format!("mission-run/{}", row.get::<_, String>(1)?),
                step: row.get(2)?,
                status,
                assignee: row.get(4)?,
                claimant,
                available_to: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
                created_at_unix_ms: row.get::<_, String>(7)?.parse().unwrap_or_default(),
                carried_claimant: None,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let carried_claimants = carried_claimants_tx(
        connection,
        rows.iter()
            .filter(|row| row.status == "ready" && row.assignee.is_some())
            .map(|row| row.subject.as_str()),
    )?;
    for row in &mut rows {
        row.carried_claimant = carried_claimants.get(&row.subject).cloned();
    }
    Ok(rows)
}

pub fn seat_run_orders_tx(
    connection: &Connection,
    agent: Option<&str>,
) -> Result<BTreeMap<String, Vec<String>>> {
    Ok(seat_queue_inputs_tx(connection, agent)?
        .into_iter()
        .map(|(agent, inputs)| {
            let order = inputs.live_order();
            (agent, order)
        })
        .collect())
}

/// One published revision of a mission, if this store holds it.
pub fn mission_revision_spec_tx(
    connection: &Connection,
    mission: &str,
    revision: &str,
) -> Result<Option<MissionSpec>, St3Error> {
    connection
        .query_row(
            "SELECT body FROM mission_revisions WHERE mission_id=?1 AND revision=?2",
            params![mission, revision],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(internal)?
        .map(|body| serde_json::from_str::<MissionSpec>(&body))
        .transpose()
        .map_err(internal)
}

