//! Data DSL lanes lower to the host's native finite-execution membership boundary.
//! Inherited exec attempts use distinct roots while the parent scope retains one membership and
//! custody of every root through retirement.
use super::*;
use crate::store::resource_membership::{Admission, Coverage, Fit, LaneRequest, Owner};

use crate::model::RESOURCE_LANE_REQUEST_TAG as REQUEST_TAG;

pub(super) fn request(member: &MemberSpec) -> Result<Option<LaneRequest>> {
    member.tags.get(REQUEST_TAG).map(|value| serde_json::from_str(value).map_err(Into::into)).transpose()
}
pub(super) fn set_request(member: &mut MemberSpec, request: &LaneRequest) -> Result<()> {
    member.tags.insert(REQUEST_TAG.into(), serde_json::to_string(request)?);
    Ok(())
}

fn lower_request(lane: &crate::model::ResourceLaneSpec, id: String, owner: Owner) -> LaneRequest {
    LaneRequest {
        id, owner, name: lane.name.clone(), host: lane.host.clone(), max_members: lane.max_members,
        capacity: lane.capacity, coverage: match lane.coverage {
            crate::model::ResourceLaneCoverage::Logical => Coverage::Logical,
            crate::model::ResourceLaneCoverage::Enforced => Coverage::Enforced,
        },
        admission_timeout_ms: lane.admission_timeout_ms,
    }
}

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn resource_step_has_lane(&self, view: &crate::model::StepRunView) -> Result<bool> {
        for desired in self.store.desired_subjects_for_owner_step(&view.subject)? {
            if let Some(member) = desired.member.as_ref() && request(member)?.is_some() { return Ok(true); }
        }
        Ok(false)
    }

    pub(super) fn observe_resource_retirement(&self, subject: &DesiredSubject, member: &MemberSpec, observation: &RuntimeObservation) -> Result<bool> {
        if !matches!(observation.status.as_str(), "exited" | "vanished" | "stopped" | "absent" | "failed") { return Ok(true); }
        let Some(request) = request(member)? else { return Ok(true); };
        let Some(record) = self.store.resource_memberships_for_host(&self.host)?.into_iter()
            .find(|record| record.request == request) else { return Ok(true); };
        if record.state == "retired" { return Ok(true); }
        let Some(grant) = record.grant.as_ref() else { return Ok(true); };
        let Some(root) = record.roots.iter().find(|root| root.runtime_id == member.runtime_id) else { return Ok(true); };
        let Some(execution) = root.execution.as_ref() else { return Ok(true); };
        if observation.incarnation_id.as_deref() != Some(execution.generation_id.as_str()) { return Ok(true); }
        if root.execution_ended_at.is_none() {
            self.store.end_resource_execution(grant, &member.runtime_id, u64::try_from(now_ms()).unwrap_or(u64::MAX))?;
        }
        if request.owner.invocation.as_deref() != Some(subject.subject.as_str()) {
            // An inherited root ends here; its owning program scope retains membership custody.
            return Ok(true);
        }
        if !self.runtime.retire_resource_lane(&self.store, &request)? {
            // Unknown retirement retains custody; native events or explicit recovery wake it.
            return Ok(false);
        }
        self.incremental.touch(&crate::store::resource_membership::dependency(&request.host, &request.name));
        self.incremental.touch("seat-admission");
        self.signal_changed();
        Ok(true)
    }

    pub(super) fn resource_step_elapsed(&self, view: &crate::model::StepRunView) -> Result<Option<u128>> {
        let records = self.store.resource_memberships_for_host(&self.host)?;
        let mut elapsed = None::<u64>;
        for desired in self.store.desired_subjects_for_owner_step(&view.subject)? {
            let Some(member) = desired.member.as_ref() else { continue; };
            let Some(request) = request(member)? else { continue; };
            let Some(record) = records.iter().find(|record| record.request == request) else { continue; };
            let Some(root) = record.roots.iter().find(|root| root.runtime_id == member.runtime_id) else { continue; };
            if request.id.starts_with("dsl:") && request.owner.attempt != view.attempt { continue; }
            if let Some(root_elapsed) = self.store.resource_execution_elapsed(
                &record.request.id, &record.request.owner, &root.runtime_id,
                u64::try_from(now_ms()).unwrap_or(u64::MAX),
            )? {
                elapsed = Some(elapsed.map_or(root_elapsed, |previous| previous.max(root_elapsed)));
            }
        }
        Ok(elapsed.map(u128::from))
    }

    pub(super) fn cancel_invalid_resource_lanes(&self, desired: &[DesiredSubject]) -> Result<()> {
        if !self.store.resource_memberships_for_host(&self.host)?.is_empty() {
            self.ensure_resource_authority()?;
        }
        self.cancel_invalid_program_resources()?;
        for record in self.store.resource_memberships_for_host(&self.host)? {
            if record.request.owner.invocation.as_deref().is_none_or(|invocation| !invocation.starts_with("exec/"))
                || matches!(record.state.as_str(), "retired" | "refused" | "withdrawn")
            { continue; }
            let retained = desired.iter().filter_map(|subject| subject.member.as_ref())
                .map(request).collect::<Result<Vec<_>>>()?.into_iter().flatten()
                .any(|request| request == record.request);
            let active = self.store.mission_run(&record.request.owner.run)?.is_some_and(|run| {
                run.steps.iter().any(|step| step.subject == record.request.owner.step
                    && step.attempt == record.request.owner.attempt
                    && !matches!(step.status.as_str(), "completed" | "failed" | "cancelled" | "skipped"))
            });
            if retained && active { continue; }
            if self.runtime.retire_resource_lane(&self.store, &record.request)? {
                self.incremental.touch(&crate::store::resource_membership::dependency(&record.request.host, &record.request.name));
                self.incremental.touch("seat-admission");
                self.signal_changed();
            }
        }
        Ok(())
    }

    pub(super) fn resource_lane_fit(&self, candidate: &str, capacity: Option<fractal_runtime::Capacity>) -> Result<Fit> {
        let (Some(host), Some(capacity)) = (self.seat_admission.lanes, capacity) else { return Ok(Fit::default()); };
        if self.store.resource_memberships_for_host(&self.host)?.iter()
            .any(|record| record.request.id == candidate && record.state != "queued")
        {
            return Ok(Fit::default());
        }
        smallclaims::touched::note_read(|| "seat-admission".into());
        let now = u64::try_from(now_ms() / 1000).unwrap_or(u64::MAX);
        // Finite custody is summed atomically by the store writer, not from this snapshot.
        let held_bytes = self.lanes_held(candidate, now, true, false)?;
        let reserve = self.burst_reserve(candidate, capacity)?;
        let outside_bytes = (host.outside)().context("UnsupportedCoverage: outside-lane host observation unavailable")?;
        let impossible = capacity.cores > host.room.cores || capacity.memory > host.room.memory;
        let reserve_wait = capacity.memory > host.room.memory.saturating_sub(reserve.full);
        let head = self.seat_admission.status.lane_head(now).filter(|head| head != candidate);
        if self.seat_admission.status.lane_wait(candidate,
            "finite execution waits for native admission".into(), !impossible && !reserve_wait, now)
        {
            self.incremental.touch("seat-admission");
            self.signal_changed();
        }
        Ok(Fit {
            room: Some(host.room), held_bytes, outside_bytes, reserve_bytes: reserve.kept,
            fifo_predecessor: if reserve_wait && !impossible { Some("burst reserve".into()) } else { head },
        })
    }

    pub(super) fn admit_resource_lane(&self, subject: &DesiredSubject, member: &MemberSpec) -> Result<bool> {
        let Some(request) = request(member)? else { return Ok(true); };
        self.ensure_resource_authority()?;
        anyhow::ensure!(subject.kind == "exec" && member.host == request.host && request.host == self.host,
            "resource lane must bind its actual exec host");
        let direct = subject.owner_run.as_deref() == Some(request.owner.run.as_str())
            && subject.owner_step.as_deref() == Some(request.owner.step.as_str())
            && request.owner.invocation.as_deref() == Some(subject.subject.as_str());
        let inherited = if let Some(run) = subject.owner_run.as_deref() {
            self.store.resource_context_for_run(run)?.is_some_and(|context| context.request == request)
        } else { false };
        anyhow::ensure!(direct || inherited, "resource context must belong to the actual execution owner");
        smallclaims::touched::note_read(|| crate::store::resource_membership::dependency(&request.host, &request.name));
        let now = u64::try_from(now_ms()).unwrap_or(u64::MAX);
        let fit = self.resource_lane_fit(&request.id, request.capacity)?;
        let prior = self.store.resource_memberships_for_host(&self.host)?.into_iter()
            .find(|record| record.request.id == request.id).map(|record| record.state);
        let admission = self.store.request_resource_membership(&request, fit, now)?;
        let current = self.store.resource_membership_record(&request.id, &request.owner)?;
        if prior.as_deref() != Some(current.state.as_str()) {
            self.incremental.touch(&crate::store::resource_membership::dependency(&request.host, &request.name));
            self.incremental.touch("seat-admission");
            self.signal_changed();
        }
        match admission {
            Admission::Admitted(_) => Ok(true),
            Admission::Queued { reason } => {
                if let Some(remaining) = self.store.resource_admission_remaining(&request.id, &request.owner, now)? {
                    self.arm_restart(&format!("resource-admission:{}", request.id), now_ms().saturating_add(u128::from(remaining)));
                }
                self.record_once(&subject.subject, "runtime.reconcile-decision", BTreeMap::from([
                    ("key".into(), Value::from("resource-admission")), ("decision".into(), Value::from("resource-wait")),
                    ("reason".into(), Value::from(reason)), ("membership_id".into(), Value::from(request.id)),
                ]))?;
                Ok(false)
            }
            Admission::Refused { code, reason } => {
                self.store.set_step_state_unless_terminal(&request.owner.step, "failed", Some(&reason))?;
                self.record_once(&subject.subject, "runtime.reconcile-decision", BTreeMap::from([
                    ("key".into(), Value::from("resource-admission")), ("decision".into(), Value::from("resource-refused")),
                    ("code".into(), Value::from(code.as_str())), ("reason".into(), Value::from(reason)),
                ]))?;
                Ok(false)
            }
        }
    }

    pub(super) fn bind_resource_lane(&self, run: &MissionRunView, step: &RuntimeStep<'_>, view: &crate::model::StepRunView, member: &mut MemberSpec, subject: &str) -> Result<()> {
        if member.kind != MemberKind::Exec { return Ok(()); }
        let Some(lane) = &step.spec.resource_lane else {
            if let Some(context) = self.store.resource_context_for_run(&run.subject)? {
                anyhow::ensure!(context.request.host == member.host, "child execution must run on its resource authority host");
                // Retries share the parent's membership, but each execution needs its own root.
                member.runtime_id = crate::graph::runtime_id(&format!(
                    "{}/{}/attempt/{}/{}", run.generation, view.subject, view.attempt, member.runtime_id,
                ));
                set_request(member, &context.request)?;
            }
            return Ok(());
        };
        anyhow::ensure!(member.host == lane.host, "step lane host must match its exec execution host");
        let request = lower_request(lane,
            format!("dsl:{}:{}:{}:{}", run.subject, run.generation, view.attempt, subject),
            Owner { run: run.subject.clone(), step: view.subject.clone(), attempt: view.attempt, invocation: Some(subject.to_owned()) },
        );
        set_request(member, &request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_exec_retries_keep_distinct_roots_in_parent_retirement_custody() {
        use super::super::tests::{FakeRuntime, apply_source};

        let source = r#"version 2
        mission "child" state="ready" {
            goal "Retry inherited execution."
            step "verify" {
                agentless
                exec "verify" { command "true"; }
            }
        }"#;
        let store = Arc::new(Store::open_memory("node").unwrap());
        apply_source(&store, source, "inherited-retry");
        let parent = LaneRequest {
            id: "program-parent".into(), name: "verification".into(), host: "node".into(),
            max_members: 1, capacity: None, coverage: Coverage::Logical, admission_timeout_ms: None,
            owner: Owner {
                run: "mission-run/parent".into(), step: "mission-step/parent/work".into(),
                attempt: 1, invocation: None,
            },
        };
        store.start_resource_authority("node", "authority").unwrap();
        let Admission::Admitted(grant) = store.request_resource_membership(&parent, Fit::default(), 100).unwrap()
            else { panic!("parent membership must admit"); };
        let child = store.mission_run_subject_for_idempotency_key("inherited-retry-run");
        store.attach_resource_child(&grant, &parent.owner, &child).unwrap();
        let created = store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "child".into(), revision: None, workspace: "/tmp".into(),
            requester: Some("person/test".into()), mode: Some("run".into()),
            inputs: BTreeMap::new(), idempotency_key: "inherited-retry-run".into(),
        }).unwrap();
        let run = store.mission_run(&created.id).unwrap().unwrap();
        let mut view = run.steps[0].clone();
        let intent = crate::graph::parse_intent(source, "node").unwrap();
        let spec = &intent.missions["child"].steps["verify"];
        let step = RuntimeStep {
            spec, dependency_prefix: String::new(), parent: None, mission_capacity: None,
        };
        let declarations = crate::graph::parse_execution_intent(
            spec.declarations_kdl.as_deref().unwrap(), "node", &run.id,
        ).unwrap();
        let mut subject = declarations.subjects.values()
            .find(|subject| subject.member.as_ref().is_some_and(|member| member.kind == MemberKind::Exec))
            .unwrap().clone();
        subject.owner_run = Some(run.subject.clone());
        subject.owner_step = Some(view.subject.clone());
        let original = subject.member.as_ref().unwrap().clone();
        let reconciler = Reconciler::new(
            store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()),
        );
        let mut roots = Vec::new();
        for attempt in [1, 2] {
            view.attempt = attempt;
            let mut member = original.clone();
            reconciler.bind_resource_lane(&run, &step, &view, &mut member, &subject.subject).unwrap();
            assert_eq!(request(&member).unwrap(), Some(parent.clone()));
            let mut replay = original.clone();
            reconciler.bind_resource_lane(&run, &step, &view, &mut replay, &subject.subject).unwrap();
            assert_eq!(replay.runtime_id, member.runtime_id, "same attempt materializes the same root");
            assert!(!roots.contains(&member.runtime_id), "retry must not reuse an already-prepared root");
            store.prepare_resource_membership(&grant, &member.runtime_id).unwrap();
            let execution: fractal_runtime::ExecGeneration = serde_json::from_value(serde_json::json!({
                "schema": "st.runtime.exec-generation.v1", "pid": 1, "createdAtUnixMs": 100,
                "startToken": 1, "generationId": format!("generation-{attempt}"),
                "payloadReleasedAtUnixMs": 200 + u64::from(attempt)
            })).unwrap();
            store.launch_resource_membership(&grant, &member.runtime_id, &execution, 300).unwrap();
            let mut observation = RuntimeObservation {
                runtime_id: member.runtime_id.clone(), terminal: false, status: "exited".into(),
                exit_code: Some(1), incarnation_id: Some("stale-generation".into()),
            };
            assert!(reconciler.observe_resource_retirement(&subject, &member, &observation).unwrap());
            let record = store.resource_membership_for_grant(&grant).unwrap();
            assert!(record.roots.last().unwrap().execution_ended_at.is_none());
            observation.incarnation_id = Some(execution.generation_id);
            // The fake runtime has no membership-retirement implementation: ending an inherited
            // root must only record its exit, never attempt to retire the parent's scope.
            assert!(reconciler.observe_resource_retirement(&subject, &member, &observation).unwrap());
            roots.push(member.runtime_id);
            assert_eq!(store.resource_memberships_for_host("node").unwrap().len(), 1);
            let record = store.resource_context_for_run(&run.subject).unwrap().unwrap();
            assert_eq!(record.request, parent);
            assert_eq!(record.state, "launched");
            assert_eq!(record.child_runs, vec![run.subject.clone()]);
            assert!(record.roots.iter().all(|root| root.execution_ended_at.is_some()));
        }
        let mut successor = run.clone();
        successor.generation.push_str("-successor");
        let mut member = original;
        reconciler.bind_resource_lane(&successor, &step, &view, &mut member, &subject.subject).unwrap();
        assert!(!roots.contains(&member.runtime_id), "new generation must not reuse an earlier root");
        assert_eq!(request(&member).unwrap(), Some(parent.clone()));
        store.cancel_resource_membership(&grant, &parent.owner, 400).unwrap();
        let retiring = store.resource_membership_for_grant(&grant).unwrap();
        assert_eq!(retiring.request, parent);
        assert_eq!(retiring.state, "retiring");
        assert_eq!(retiring.roots.iter().map(|root| root.runtime_id.clone()).collect::<Vec<_>>(), roots);
        assert!(retiring.roots.iter().all(|root| root.execution_ended_at.is_some()));
        assert!(store.prepare_resource_membership(&grant, &member.runtime_id).is_err());
    }


    #[test]
    fn data_and_program_requests_share_native_admission_and_queue_clocks() {
        let source = r#"version 2
        mission "test" state="ready" {
            goal "Verify."
            step "verify" {
                agentless
                lane "verification" host="node" max-members=1 coverage="logical" admission-timeout="2s"
                exec "verify" { command "true"; }
            }
        }"#;
        let intent = crate::graph::parse_intent(source, "node").unwrap();
        let lane = intent.missions["test"].steps["verify"].resource_lane.as_ref().unwrap();
        let owner = Owner { run: "mission-run/test".into(), step: "mission-step/test/verify".into(), attempt: 1, invocation: Some("exec/test/verify".into()) };
        let data = lower_request(lane, "data".into(), owner.clone());
        let exec = crate::graph::parse_execution_intent(
            intent.missions["test"].steps["verify"].declarations_kdl.as_deref().unwrap(),
            "node",
            "test",
        ).unwrap();
        let original = exec.subjects.values().find_map(|subject| subject.member.as_ref()).unwrap();
        let mut bound = original.clone();
        set_request(&mut bound, &data).unwrap();
        assert_eq!(request(&bound).unwrap(), Some(data.clone()));
        assert_eq!(bound.launch_changes(original), vec!["resource-lane"]);
        assert!(bound.launch_changes(&bound).is_empty());
        let program: LaneRequest = serde_json::from_value(serde_json::json!({
            "id": "program", "name": "verification", "host": "node", "maxMembers": 1,
            "capacity": null, "coverage": "logical", "admissionTimeoutMs": 2000, "owner": owner,
        })).unwrap();
        let store = Store::open_memory("node").unwrap();
        store.start_resource_authority("node", "authority").unwrap();
        let Admission::Admitted(first) = store.request_resource_membership(&data, Fit::default(), 100).unwrap() else { panic!("data request must admit"); };
        assert!(matches!(store.request_resource_membership(&program, Fit::default(), 200).unwrap(), Admission::Queued { .. }));
        let queued = store.resource_membership_record("program", &program.owner).unwrap();
        assert_eq!(queued.queued_at, 200);
        assert!(queued.payload_released_at.is_none(), "queueing must not open execution time");
        store.cancel_resource_membership(&first, &data.owner, 300).unwrap();
        let runtime = fractal_runtime::ExecRuntime::new(std::path::PathBuf::new(), std::path::PathBuf::new());
        assert!(store.retire_resource_membership(&first, &data.owner, &runtime, 300).unwrap());
        assert!(matches!(store.request_resource_membership(&program, Fit::default(), 400).unwrap(), Admission::Admitted(_)));
        let admitted = store.resource_membership_record("program", &program.owner).unwrap();
        assert_eq!(admitted.admitted_at, Some(400));
        assert!(admitted.payload_released_at.is_none());
    }

    #[test]
    fn inherited_lane_context_does_not_stop_agentless_execution_clocks() {
        use super::super::tests::{FakeRuntime, apply_source};

        struct Clock;
        impl Drop for Clock {
            fn drop(&mut self) { smallclaims::store::set_thread_clock(None); }
        }
        let _clock = Clock;
        const START: u128 = 1_800_000_000_000;
        for kind in ["gate", "after-run", "program", "inherited-exec", "queued-exec"] {
            smallclaims::store::set_thread_clock(Some(START));
            let store = Arc::new(Store::open_memory("node").unwrap());
            store.set_write_clock_at(START).unwrap();
            let bundle_hash = store.put_blob(b"export default {}").unwrap();
            let body = match kind {
                "gate" => r#"agentless
                    gate "wait" { field "state" "resource/never" is "ready"; }"#.to_owned(),
                "after-run" => r#"after-run "mission-run/upstream""#.to_owned(),
                "program" => format!("program \"/unused.mjs\" sha256=\"{bundle_hash}\""),
                "inherited-exec" => r#"agentless
                    exec "payload" { command "true"; }"#.to_owned(),
                "queued-exec" => r#"agentless
                    lane "checks" host="node" max-members=1 coverage="logical"
                    exec "payload" { command "true"; }"#.to_owned(),
                _ => unreachable!(),
            };
            apply_source(&store, &format!(r#"version 2
                mission "child" state="ready" {{
                    goal "Bound agentless work inside an inherited lane."
                    step "work" timeout="1s" {{ {body} }}
                }}
                mission "upstream" state="ready" {{
                    goal "Keep the awaited run open."
                    step "hold" {{
                        agentless
                        gate "wait" {{ field "state" "resource/never" is "ready"; }}
                    }}
                }}"#), "clock-definitions");
            let start = |mission: &str| store.create_mission_run(&crate::model::MissionRunRequest {
                mission: mission.into(), revision: None, workspace: "/tmp".into(),
                requester: None, mode: None, inputs: BTreeMap::new(),
                idempotency_key: mission.into(),
            }).unwrap();
            let upstream = start("upstream");
            // Resource context must exist before the child becomes runnable.
            let child_subject = store.mission_run_subject_for_idempotency_key("child");
            store.attach_resource_child(&grant, &owner, &child_subject).unwrap();
            let child = start("child");
            assert_eq!(child.subject, child_subject);
            // Use the real awaited subject, independent of the store's run-id allocation.
            let mut mission = store.mission_spec("child", Some(&child.revision)).unwrap().unwrap();
            if kind == "after-run" {
                mission.steps.get_mut("work").unwrap().after_run = Some(upstream.subject.clone());
            }
            let owner = Owner {
                run: upstream.subject.clone(), step: upstream.steps[0].subject.clone(),
                attempt: upstream.steps[0].attempt, invocation: None,
            };
            let parent = LaneRequest {
                id: "parent".into(), name: "checks".into(), host: "node".into(),
                max_members: 1, capacity: None, coverage: Coverage::Logical,
                admission_timeout_ms: None, owner: owner.clone(),
            };
            store.start_resource_authority("node", "clock-authority").unwrap();
            let Admission::Admitted(grant) = store.request_resource_membership(
                &parent, Fit::default(), START as u64,
            ).unwrap() else { panic!("parent must be admitted"); };
            assert!(store.resource_context_for_run(&child.subject).unwrap().is_some());
            let reconciler = Reconciler::new(
                store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()),
            );
            // As in the program-host fault regression, prevent a JavaScript host from starting
            // without depending on installed runtimes, process scheduling, or wall-clock sleeps.
            let dir = tempfile::tempdir().unwrap();
            if kind == "program" {
                let programs = dir.path().join("programs");
                std::fs::create_dir_all(&programs).unwrap();
                std::fs::write(programs.join("groups"), b"").unwrap();
                let engine = program::ProgramEngine::new(
                    store.clone(), Arc::new(|_: &str| {}), programs, program::Liveness::default(),
                );
                assert!(reconciler.program_engine.set(engine).is_ok());
            }
            // Evaluate only mission transitions: payload launch and runtime observations are
            // deliberately absent, so admission can never be mistaken for payload release.
            for _ in 0..4 {
                let run = store.mission_run(&child.id).unwrap().unwrap();
                reconciler.evaluate_mission_run(&run, &mission).unwrap();
            }
            let active = store.mission_run(&child.id).unwrap().unwrap();
            let view = active.steps.iter().find(|step| step.step == "work").unwrap();
            let payload = matches!(kind, "inherited-exec" | "queued-exec");
            assert_eq!(reconciler.resource_step_has_lane(view).unwrap(), payload, "{kind}");
            assert_eq!(view.status, if payload { "ready" } else { "working" }, "{kind}");
            assert_eq!(view.execution_started_at_unix_ms, (!payload).then_some(START), "{kind}");
            if kind == "queued-exec" {
                let desired = store.desired_subjects_for_owner_step(&view.subject).unwrap();
                let request = desired.iter().filter_map(|subject| subject.member.as_ref())
                    .find_map(|member| request(member).unwrap()).unwrap();
                assert!(matches!(store.request_resource_membership(
                    &request, Fit::default(), START as u64,
                ).unwrap(), Admission::Queued { .. }));
            }
            smallclaims::store::set_thread_clock(Some(START + 1_001));
            store.set_write_clock_at(START + 1_001).unwrap();
            let run = store.mission_run(&child.id).unwrap().unwrap();
            let view = run.steps.iter().find(|step| step.step == "work").unwrap();
            assert_eq!(view.execution_elapsed_ms, if payload { 0 } else { 1_001 }, "{kind}");
            let steps = flatten_mission_steps(&mission);
            let step = steps.iter().find(|step| step.spec.path == "work").unwrap();
            assert_eq!(reconciler.step_timed_out(view, step).unwrap(), !payload, "{kind}");
            reconciler.evaluate_mission_run(&run, &mission).unwrap();
            let terminal = store.mission_run(&child.id).unwrap().unwrap();
            let view = terminal.steps.iter().find(|step| step.step == "work").unwrap();
            assert_eq!(view.status, if payload { "ready" } else { "failed" }, "{kind}");
            if payload {
                assert_eq!(reconciler.resource_step_elapsed(view).unwrap(), None, "{kind}");
                assert_eq!(view.execution_started_at_unix_ms, None, "{kind}");
            }
        }
    }
}
