//! Shell requests are a durable transport into the existing finite host authority.
use super::*;
use crate::store::resource_membership::{Admission, Grant};
use crate::store::shell_resources::{Execution, PayloadResult, PARENT_ENV};

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn reconcile_shell_resources(&self) -> Result<()> {
        if self.store.shell_resources(&self.host, true)?.is_empty() { return Ok(()); }
        self.ensure_resource_authority()?;
        for mut execution in self.store.shell_resources(&self.host, true)? {
            if let Err(error) = execution.command.lane.class.ensure_supported() {
                tracing::warn!(execution = %execution.command.id, %error, "retaining newer-class execution until the execution host is upgraded");
                continue;
            }
            if let Err(error) = self.drive_shell_resource(&mut execution) {
                let launched = execution.started_at.is_some() || self.store.resource_memberships_for_host(&self.host)?.into_iter()
                    .flat_map(|member| member.roots).any(|root| root.runtime_id == execution.command.id && root.payload_released_at.is_some());
                let already_failing = matches!(execution.state.as_str(), "failing" | "finishing-refused");
                if !already_failing { execution.reason = format!("{error:#}"); }
                if launched {
                    // Observation failures never authorize destructive teardown. Re-enter the
                    // same bounded cancellation path, retaining the original refusal reason.
                    execution.state = "failing".into();
                    if !already_failing { self.signal_changed(); }
                } else {
                    execution.state = match self.retire_shell_resource(&execution) {
                        Ok(true) => "refused",
                        Ok(false) => "retiring",
                        Err(retirement) => { execution.reason.push_str(&format!("; retirement: {retirement:#}")); "retiring" }
                    }.into();
                }
            }
            if matches!(execution.state.as_str(), "completed" | "refused" | "cancelled") {
                #[cfg(target_os = "macos")]
                if let Some(workspace) = darwin_job_workspace(&execution.command.cwd, &execution.command.id) {
                    match fs::remove_dir_all(&workspace) {
                        Ok(()) => {},
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                        Err(error) => {
                            execution.state = "refused".into();
                            execution.reason = format!("darwin-workspace-leftover: {}: {error}", workspace.display());
                        }
                    }
                }
                let now = u64::try_from(now_ms()).unwrap_or(u64::MAX);
                execution.ended_at.get_or_insert(now);
                if execution.started_at.is_none() {
                    execution.queue_wait_ms = self.store.shell_resource_queue_wait_ms(&execution, now);
                }
                self.seat_admission.status.state().lane_waits.retain(|wait| wait.subject != execution.command.lane.id);
                self.shell_exec_watches.lock().unwrap_or_else(PoisonError::into_inner).remove(&execution.command.id);
                execution.command.environment.clear();
            }
            if self.store.update_shell_resource(&execution)? { self.signal_changed(); }
        }
        Ok(())
    }

    pub(super) fn shell_result_path(&self, id: &str) -> PathBuf {
        #[cfg(target_os = "macos")]
        if let Ok(execution) = self.store.shell_resource(id)
            && let Some(workspace) = darwin_job_workspace(&execution.command.cwd, id) {
            return workspace.join(".result.json");
        }
        self.driver_state_dir.parent().unwrap_or(&self.driver_state_dir).join("resource-exec").join(format!("{id}.json"))
    }

    fn retire_shell_resource(&self, execution: &Execution) -> Result<bool> {
        let request = &execution.command.lane;
        let member = self.store.resource_memberships_for_host(&self.host)?.into_iter().find(|member| member.request.id == request.id);
        let Some(member) = member else { return Ok(true); };
        if execution.command.parent.is_some() {
            if member.roots.iter().any(|root| root.runtime_id == execution.command.id) {
                self.runtime.retire_resource_exec(&self.store, request, &execution.command.id)?;
            }
            return Ok(true);
        }
        // Nested executions share this charged membership. Let each helper publish and
        // commit its outcome before removing the scope that also contains its cgroup.
        let children: Vec<_> = self.store.shell_resources(&self.host, true)?.into_iter()
            .filter(|child| child.command.parent.is_some() && child.command.lane.id == request.id).collect();
        if !children.is_empty() {
            for child in children {
                if !child.cancel {
                    self.store.cancel_shell_resource(&child.command.id)?;
                    self.signal_changed();
                }
            }
            return Ok(false);
        }
        let retired = self.runtime.retire_resource_lane(&self.store, request)?;
        if retired {
            self.incremental.touch(&crate::store::resource_membership::dependency(&request.host, &request.name));
            self.incremental.touch("seat-admission");
            self.seat_admission.status.state().lane_waits.retain(|wait| wait.subject != request.id);
        }
        Ok(retired)
    }

    fn capture_retirement_evidence(&self, execution: &mut Execution) -> Result<()> {
        use crate::store::resource_profile_learner::{Coverage, EndReason};
        if execution.result.as_ref().is_some_and(|result| result.evidence.retirement_generation.is_some()) {
            return Ok(());
        }
        let member = self.store.resource_membership_record(&execution.command.lane.id, &execution.command.lane.owner)?;
        let Some(root) = member.roots.iter().find(|root| root.runtime_id == execution.command.id) else { return Ok(()); };
        let Some(generation) = &root.execution else { return Ok(()); };
        let enforced = execution.command.lane.coverage == crate::store::resource_membership::Coverage::Enforced;
        let counters = if enforced {
            self.runtime.resource_exec_cgroup(&execution.command.id, &generation.generation_id)?
        } else { Default::default() };
        let declared_full = execution.command.lane.profile_estimate.as_ref().is_some_and(|estimate|
            estimate.learning_coverage == fractal_runtime::ResourceLearningCoverage::Full);
        let full = declared_full && enforced && member.roots.len() == 1 && member.child_runs.is_empty()
            && counters.memory_peak_bytes.is_some() && counters.memory_events.is_some()
            && counters.io_stat.is_some() && counters.populated.is_some();
        let result = execution.result.get_or_insert_with(PayloadResult::default);
        result.memory_peak_bytes = counters.memory_peak_bytes.or(result.memory_peak_bytes);
        result.evidence = crate::store::resource_profile_learner::evidence::Evidence {
            pre_retirement_populated: counters.populated,
            learning_coverage: if full { Coverage::Full } else { Coverage::ClientOnly },
            memory_events: counters.memory_events, io_stat: counters.io_stat,
            retirement_generation: Some(generation.generation_id.clone()),
            retirement_confirmed: false,
            end_reason: if execution.cancel { EndReason::Cancelled }
                else if execution.timed_out { EndReason::TimedOut }
                else if result.exit_signal.is_some() { EndReason::Signalled }
                else if result.exit_code.is_some() { EndReason::Natural }
                else { EndReason::Unknown },
        };
        Ok(())
    }

    fn shell_payload_observation(&self, execution: &Execution, generation: &str, now: u64) -> Result<PayloadResult> {
        let cwd = &execution.command.cwd;
        let head_end = std::process::Command::new("git").current_dir(cwd).args(["rev-parse", "HEAD"]).output()
            .ok().filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok()).map(|head| head.trim().to_owned());
        let clean_end = std::process::Command::new("git").current_dir(cwd)
            .args(["status", "--porcelain", "--untracked-files=normal"]).output()
            .ok().filter(|output| output.status.success()).map(|output| output.stdout.is_empty());
        let memory_peak_bytes = if execution.command.lane.coverage == crate::store::resource_membership::Coverage::Enforced {
            self.runtime.resource_exec_cgroup(&execution.command.id, generation)?.memory_peak_bytes
        } else { None };
        Ok(PayloadResult {
            head_start: execution.head_start.clone(), clean_start: execution.clean_start,
            head_end, clean_end, exit_code: None, exit_signal: None,
            wall_duration_ms: self.store.resource_execution_elapsed(&execution.command.lane.id,
                &execution.command.lane.owner, &execution.command.id, now)?.context("shell release clock missing")?,
            memory_peak_bytes,
            ..PayloadResult::default()
        })
    }

    fn read_shell_payload_result(&self, execution: &mut Execution) -> Result<bool> {
        match fs::read(self.shell_result_path(&execution.command.id)) {
            Ok(bytes) => {
                let result: PayloadResult = serde_json::from_slice(&bytes)?;
                execution.exit_code = result.exit_code; execution.exit_signal = result.exit_signal;
                execution.head_start.clone_from(&result.head_start); execution.clean_start = result.clean_start;
                execution.result = Some(result);
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Snapshot predecessor shells before rotating the epoch or destroying their cgroups.
    pub(super) fn prepare_shell_resource_recovery(&self) -> Result<()> {
        let members = self.store.resource_memberships_for_host(&self.host)?;
        let (exited, receiver) = std::sync::mpsc::channel();
        let mut running = Vec::new();
        for mut execution in self.store.shell_resources(&self.host, true)? {
            if let Err(error) = execution.command.lane.class.ensure_supported() {
                tracing::warn!(execution = %execution.command.id, %error, "retaining newer-class execution during authority recovery");
                continue;
            }
            let Some(root) = members.iter().flat_map(|member| &member.roots)
                .find(|root| root.runtime_id == execution.command.id && root.payload_released_at.is_some()) else { continue; };
            let generation = root.execution.as_ref().context("released shell has no generation")?.generation_id.clone();
            execution.started_at = root.payload_released_at;
            if !self.read_shell_payload_result(&mut execution)? && execution.result.is_none() {
                if let Ok(bytes) = fs::read(self.shell_result_path(&execution.command.id).with_extension("started")) {
                    let (head, clean) = serde_json::from_slice(&bytes)?;
                    execution.head_start = head; execution.clean_start = clean;
                }
                execution.result = Some(self.shell_payload_observation(&execution, &generation,
                    u64::try_from(now_ms()).unwrap_or(u64::MAX))?);
            }
            execution.state = if matches!(execution.state.as_str(), "failing" | "finishing-refused") {
                "finishing-refused"
            } else { "finishing" }.into();
            if self.store.update_shell_resource(&execution)? { self.signal_changed(); }
            if self.runtime.observe_exec(&execution.command.id)?
                .is_some_and(|observation| observation.status == "running"
                    && observation.incarnation_id.as_deref() == Some(generation.as_str())) {
                // Install every generation-fenced exit watch before sending any signal.
                let sender = exited.clone();
                let id = execution.command.id.clone();
                self.runtime.watch_resource_exec(&id, &generation, Arc::new({
                    let id = id.clone();
                    move || { let _ = sender.send(id.clone()); }
                }))?;
                running.push((id, generation));
            }
        }
        drop(exited);
        // One startup grace for the whole recovered set, not one timeout per root.
        // Native exit notifications wake this wait; no polling or sleep races publication.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for (id, generation) in &running { self.runtime.cancel_resource_exec(id, generation)?; }
        let mut pending: BTreeSet<_> = running.iter().map(|(id, _)| id.clone()).collect();
        while !pending.is_empty() {
            match receiver.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
                Ok(id) => { pending.remove(&id); }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout | std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        for (id, generation) in running {
            let mut execution = self.store.shell_resource(&id)?;
            if !self.read_shell_payload_result(&mut execution)? {
                // A helper that missed grace is about to be forcibly retired. Sample its
                // still-live cgroup now, but never attribute the helper's exit to its payload.
                execution.exit_code = None; execution.exit_signal = None;
                let mut result = self.shell_payload_observation(&execution, &generation,
                    u64::try_from(now_ms()).unwrap_or(u64::MAX))?;
                if let Some(previous) = &execution.result {
                    result.memory_peak_bytes = result.memory_peak_bytes.or(previous.memory_peak_bytes);
                    result.wall_duration_ms = result.wall_duration_ms.max(previous.wall_duration_ms);
                }
                execution.result = Some(result);
            }
            if self.store.update_shell_resource(&execution)? { self.signal_changed(); }
        }
        Ok(())
    }

    /// Retirement may finish asynchronously. Hydrate final evidence only after native
    /// absence is proved, preferring any helper publication made during shutdown.
    fn finalize_retired_shell_result(&self, execution: &mut Execution) -> Result<()> {
        let member = self.store.resource_membership_record(&execution.command.lane.id, &execution.command.lane.owner)?;
        let root = member.roots.iter().find(|root| root.runtime_id == execution.command.id).context("retired shell root missing")?;
        let generation = &root.execution.as_ref().context("retired shell generation missing")?.generation_id;
        let observation = self.runtime.observe_exec(&execution.command.id)?.context("retired shell record missing")?;
        anyhow::ensure!(observation.incarnation_id.as_deref() == Some(generation.as_str()) && observation.status == "exited",
            "retired shell lacks generation-fenced exit evidence");
        let (stdout, stderr) = self.runtime.resource_exec_output(&execution.command.id, generation)?;
        execution.stdout = stdout; execution.stderr = stderr;
        // The native root is the observer, not the payload. Only its result file can
        // identify the payload's exit code/signal; missing publication means unknown.
        execution.exit_code = None; execution.exit_signal = None;
        let evidence = execution.result.as_ref().map(|result| result.evidence.clone());
        if !self.read_shell_payload_result(execution)? {
            let mut result = self.shell_payload_observation(execution, generation, u64::try_from(now_ms()).unwrap_or(u64::MAX))?;
            if let Some(previous) = &execution.result {
                result.memory_peak_bytes = result.memory_peak_bytes.or(previous.memory_peak_bytes);
                result.wall_duration_ms = result.wall_duration_ms.max(previous.wall_duration_ms);
            }
            execution.result = Some(result);
        }
        if let (Some(result), Some(evidence)) = (execution.result.as_mut(), evidence) {
            result.evidence = evidence;
        }
        Ok(())
    }

    fn drive_shell_resource(&self, execution: &mut Execution) -> Result<()> {
        let now = u64::try_from(now_ms()).unwrap_or(u64::MAX);
        if matches!(execution.state.as_str(), "finishing" | "finishing-refused") {
            let refused = execution.state == "finishing-refused";
            self.capture_retirement_evidence(execution)?;
            if self.store.update_shell_resource(execution)? { self.signal_changed(); }
            if self.retire_shell_resource(execution)? {
                self.finalize_retired_shell_result(execution)?;
                if let Some(result) = execution.result.as_mut() { result.evidence.retirement_confirmed = true; }
                execution.state = if refused { "refused" } else if execution.cancel { "cancelled" } else { "completed" }.into();
                execution.ended_at = Some(now);
            }
            return Ok(());
        }
        if execution.command.parent.is_none() {
            if execution.command.lane.disk_path.is_none() && execution.command.lane.profile.is_some()
                && execution.command.lane.profile.as_ref().and_then(|key| self.seat_admission.finite_policy.profiles.get(key))
                    .is_some_and(|seed| seed.disk.is_some())
            {
                execution.command.lane.disk_path = Some(execution.command.cwd.clone());
            }
            self.resolve_resource_profile(&mut execution.command.lane)?;
            // Preserve the host-resolved snapshot before a payload can be prepared.
            if self.store.update_shell_resource(execution)? { self.signal_changed(); }
        }
        let request = execution.command.lane.clone();
        let cancelling_grant = if execution.cancel {
            let member = self.store.resource_memberships_for_host(&self.host)?.into_iter()
                .find(|member| member.request.id == request.id);
            match member.filter(|member| member.roots.iter().any(|root| root.runtime_id == execution.command.id)) {
                Some(member) => Some(member.grant.context("cancelled execution has no grant")?),
                None => None,
            }
        } else { None };
        if (execution.cancel && cancelling_grant.is_none()) || execution.state == "retiring" {
            if self.retire_shell_resource(execution)? {
                execution.state = if execution.cancel { "cancelled" } else { "refused" }.into();
                execution.ended_at = Some(now);
                if execution.cancel { execution.reason = "cancelled by the shell caller".into(); }
            }
            return Ok(());
        }
        let live_member = self.store.resource_memberships_for_host(&self.host)?.into_iter()
            .find(|member| member.request.id == request.id
                && member.roots.iter().any(|root| root.runtime_id == execution.command.id && root.payload_released_at.is_some()));
        if !execution.cancel && execution.state != "failing"
            && let Err(error) = self.store.shell_resource_owner_live(&request.owner)
        {
            if live_member.is_none() { return Err(error); }
            execution.state = "failing".into();
            execution.reason = format!("{error:#}");
        }
        let grant: Grant = if let Some(member) = live_member {
            member.grant.context("launched shell has no grant")?
        } else if let Some(grant) = cancelling_grant {
            grant
        } else if let Some(parent) = &execution.command.parent {
            let record = self.store.resource_membership_for_grant(parent)?;
            anyhow::ensure!(record.request == request && matches!(record.state.as_str(), "admitted" | "launched"), "nested parent membership is no longer live");
            parent.clone()
        } else {
            let fit = self.resource_lane_fit(&request)?;
            match self.store.request_resource_membership(&request, fit, now)? {
                Admission::Queued { reason } => {
                    let record = self.store.resource_membership_record(&request.id, &request.owner)?;
                    execution.state = "waiting".into(); execution.reason = reason; execution.blocked_by = record.blocked_by;
                    if let Some(remaining) = self.store.resource_admission_remaining(&request.id, &request.owner, now)? {
                        self.arm_restart(&format!("shell-admission:{}", execution.command.id), now_ms().saturating_add(u128::from(remaining)));
                    }
                    return Ok(());
                }
                Admission::Refused { code, reason } => {
                    let record = self.store.resource_membership_record(&request.id, &request.owner)?;
                    execution.state = "refused".into(); execution.reason = format!("{}: {reason}", code.as_str()); execution.blocked_by = record.blocked_by;
                    execution.refusal_code = Some(code);
                    execution.ended_at = Some(now);
                    return Ok(());
                }
                Admission::Admitted(grant) => grant,
            }
        };
        let refusing = execution.state == "failing";
        let record = self.store.resource_membership_for_grant(&grant)?;
        #[cfg(target_os = "macos")]
        if !record.roots.iter().any(|root| root.runtime_id == execution.command.id) {
            anyhow::ensure!(execution.command.parent.is_none(),
                "darwin-custody-nested: nested shell roots cannot acquire a second slot");
            let helper = fractal_runtime::darwin_slot::Helper::detect_in(&std::env::vars().collect())
                .context("helperUnavailable: requested Darwin custody lane has no authorized helper")?;
            let slot = helper.select_slot(&request.name, Path::new(&execution.command.cwd))?;
            let workspace = slot.root().join(&execution.command.id);
            fs::create_dir_all(&workspace).context("create Darwin job workspace")?;
            anyhow::ensure!(!fs::symlink_metadata(&workspace)?.file_type().is_symlink()
                && workspace.canonicalize()?.starts_with(slot.root().canonicalize()?),
                "darwin-custody-workspace: job workspace escaped its admitted slot root");
            // The slot UID has primary group nobody, not the root's staff group.
            // Retain the inherited caller ACE and grant only this job's slot UID access.
            let ace = format!("_fractal-{} allow list,add_file,search,add_subdirectory,delete_child,readattr,writeattr,readextattr,writeextattr,readsecurity,file_inherit,directory_inherit", slot.name());
            let permission = std::process::Command::new("/bin/chmod").args(["+a", &ace]).arg(&workspace).output()
                .context("grant admitted UID access to its job workspace")?;
            anyhow::ensure!(permission.status.success(), "darwin-custody-workspace: cannot grant slot access: {}",
                String::from_utf8_lossy(&permission.stderr));
            if execution.command.cwd.is_empty() {
                execution.command.cwd = workspace.to_string_lossy().into_owned();
            }
            execution.command.environment.insert("FRACTAL_JOB_WORKSPACE".into(), workspace.to_string_lossy().into_owned());
            execution.command.environment.insert(fractal_runtime::darwin_slot::HELPER_ENV.into(),
                helper.path().to_string_lossy().into_owned());
            if self.store.update_shell_resource(execution)? { self.signal_changed(); }
        }
        let result_path = self.shell_result_path(&execution.command.id);
        let newly_started = !record.roots.iter().any(|root| root.runtime_id == execution.command.id);
        if newly_started {
            execution.queue_wait_ms = self.store.shell_resource_queue_wait_ms(execution, now);
            let mut environment = self.store.take_shell_environment(&execution.command.id)
                .context("environment-unavailable: shell launch environment lost; resubmit the command")?;
            let root_exec_id = if execution.root_exec_id.is_empty() {
                request.id.strip_prefix("shell:").unwrap_or(&execution.command.id).to_owned()
            } else { execution.root_exec_id.clone() };
            environment.insert(PARENT_ENV.into(), serde_json::to_string(&crate::store::shell_resources::ParentContext {
                grant: grant.clone(), request: request.clone(), root_exec_id: Some(root_exec_id.clone()),
            })?);
            environment.insert("FRACTAL_RESOURCE_EXEC_ID".into(), root_exec_id);
            environment.insert("FRACTAL_RESOURCE_MEMBER_EXEC_ID".into(), execution.command.id.clone());
            // Nested CLIs must reconnect to the exact authority which owns the inherited grant.
            environment.insert("ST3_ENDPOINT".into(), self.endpoint.clone());
            environment.insert("FRACTAL_ENDPOINT".into(), self.endpoint.clone());
            fs::create_dir_all(result_path.parent().context("shell result path has no parent")?)?;
            let executable = launch_executable()?.to_string_lossy().into_owned();
            let mut argv = vec![executable, "resource-payload".into(), result_path.to_string_lossy().into_owned(), "--".into()];
            argv.extend(execution.command.argv.clone());
            let mut member = MemberSpec {
                kind: MemberKind::Exec, host: self.host.clone(), runtime_id: execution.command.id.clone(),
                workspace: execution.command.cwd.clone(), workspace_create: false, cwd: execution.command.cwd.clone(), terminal: false,
                launch: LaunchSpec::Argv(argv), environment, tags: BTreeMap::new(), display_name: None,
                lifecycle: MemberLifecycle::Service, terminal_binding: None, one_shot: false,
                restart: RestartType::Never, restart_intensity: RestartIntensity::default(), shutdown_timeout_ms: 5_000,
                driver: None, terminal_size: None, capacity: None, declared_priority: None, declared_lane: None, declared_budgets: None,
            };
            resource_lanes::set_request(&mut member, &request)?;
            self.runtime.start_resource_guarded(&self.store, &member, None, &|| {
                anyhow::ensure!(!self.store.shell_resource(&execution.command.id)?.cancel, "shell request cancelled before payload release");
                self.store.shell_resource_owner_live(&request.owner)?;
                Ok(())
            })?;
            self.seat_admission.status.state().lane_waits.retain(|wait| wait.subject != request.id);
            execution.state = "admitted".into(); execution.reason = "host membership admitted; payload released".into(); execution.blocked_by = None;
        }
        let record = self.store.resource_membership_for_grant(&grant)?;
        let root = record.roots.iter().find(|root| root.runtime_id == execution.command.id).context("shell execution root missing")?;
        let generation = root.execution.as_ref().context("shell preparation is uncertain; custody retained")?;
        execution.started_at = Some(root.payload_released_at.context("shell payload release has no durable timestamp")?);
        execution.command.environment.clear();
        match fs::read(result_path.with_extension("started")) {
            Ok(bytes) => {
                let (head, clean): (Option<String>, Option<bool>) = serde_json::from_slice(&bytes)?;
                execution.head_start = head; execution.clean_start = clean;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
        let mut watches = self.shell_exec_watches.lock().unwrap_or_else(PoisonError::into_inner);
        if !watches.contains(&execution.command.id) {
            let notify = self.notify.clone(); let event_notify = self.event_notify.clone();
            self.runtime.watch_resource_exec(&execution.command.id, &generation.generation_id, Arc::new(move || signal_changed(&notify, &event_notify)))?;
            watches.insert(execution.command.id.clone());
        }
        drop(watches);
        let observation = self.runtime.observe_exec(&execution.command.id)?.context("shell execution record disappeared")?;
        anyhow::ensure!(observation.incarnation_id.as_deref() == Some(generation.generation_id.as_str()), "shell execution generation changed");
        if observation.status == "running" {
            // Keep the admitted transition visible until the next native wake.
            if !newly_started && !refusing { execution.state = "running".into(); }
            let elapsed = self.store.resource_execution_elapsed(&request.id, &request.owner, &execution.command.id, now)?.context("shell release clock missing")?;
            if let Some(timeout) = execution.command.timeout_ms.filter(|_| !execution.timed_out) {
                if elapsed >= timeout { execution.timed_out = true; }
                else {
                    self.arm_restart(&format!("shell-timeout:{}", execution.command.id), now_ms().saturating_add(u128::from(timeout - elapsed)));
                }
            }
            if execution.cancel || execution.timed_out || refusing {
                // This is an elapsed execution-clock offset, not wall time; the host clock
                // remains monotonic while live and is reconstructed conservatively on recovery.
                let began = *execution.cancellation_started_at.get_or_insert(elapsed);
                let remaining = 5_000_u64.saturating_sub(elapsed.saturating_sub(began));
                if remaining == 0 {
                    // Preserve observations while the cgroup still exists, even if the helper
                    // is stuck or never installed its SIGTERM handler.
                    execution.result = Some(self.shell_payload_observation(execution, &generation.generation_id, now)?);
                    if self.store.update_shell_resource(execution)? { self.signal_changed(); }
                    let killing = std::time::Instant::now();
                    self.runtime.retire_resource_exec(&self.store, &request, &execution.command.id)?;
                    if let Some(result) = &mut execution.result {
                        result.wall_duration_ms = result.wall_duration_ms.saturating_add(
                            u64::try_from(killing.elapsed().as_millis()).unwrap_or(u64::MAX));
                    }
                    return self.drive_shell_resource(execution);
                }
                self.arm_restart(&format!("shell-cancel:{}", execution.command.id), now_ms().saturating_add(u128::from(remaining)));
                if execution.cancellation_started_at == Some(elapsed) {
                    // Commit the deadline before signalling so recovery cannot restart grace.
                    if self.store.update_shell_resource(execution)? { self.signal_changed(); }
                    self.runtime.cancel_resource_exec(&execution.command.id, &generation.generation_id)?;
                }
            }
            return Ok(());
        }
        anyhow::ensure!(observation.status == "exited", "shell execution status is indeterminate; custody retained");
        let (stdout, stderr) = self.runtime.resource_exec_output(&execution.command.id, &generation.generation_id)?;
        execution.stdout = stdout; execution.stderr = stderr;
        execution.exit_code = None; execution.exit_signal = None;
        match fs::read(&result_path) {
            Ok(bytes) => {
                let result: PayloadResult = serde_json::from_slice(&bytes)?;
                execution.exit_code = result.exit_code; execution.exit_signal = result.exit_signal;
                execution.head_start.clone_from(&result.head_start); execution.clean_start = result.clean_start;
                execution.result = Some(result);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut result = self.shell_payload_observation(execution, &generation.generation_id, now)?;
                if let Some(previous) = &execution.result {
                    result.memory_peak_bytes = result.memory_peak_bytes.or(previous.memory_peak_bytes);
                    result.wall_duration_ms = previous.wall_duration_ms;
                }
                execution.result = Some(result);
            },
            Err(error) => return Err(error.into()),
        }
        self.capture_retirement_evidence(execution)?;
        self.store.end_resource_execution(&grant, &execution.command.id, now)?;
        execution.state = if refusing { "finishing-refused" } else { "finishing" }.into();
        if !refusing {
            execution.reason = if execution.cancel { "cancelled by the shell caller" }
                else if execution.timed_out { "payload execution timeout; custody retired" }
                else { "payload exited and execution custody retired" }.into();
        }
        // Commit the result before retirement can destroy the cgroup or a daemon crash
        // can turn a completed payload into an observation-less cancellation.
        if self.store.update_shell_resource(execution)? { self.signal_changed(); }
        if !self.retire_shell_resource(execution)? { return Ok(()); }
        if let Some(result) = execution.result.as_mut() { result.evidence.retirement_confirmed = true; }
        execution.state = if refusing { "refused" } else if execution.cancel { "cancelled" } else { "completed" }.into();
        execution.ended_at = Some(now);
        self.shell_exec_watches.lock().unwrap_or_else(PoisonError::into_inner).remove(&execution.command.id);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn darwin_job_workspace(cwd: &str, id: &str) -> Option<PathBuf> {
    use fractal_runtime::darwin_slot::Slot;
    [Slot::Heavy1, Slot::Gate1, Slot::Gate2, Slot::Gate3, Slot::Eval1].into_iter()
        .map(|slot| slot.root().to_path_buf())
        .find(|root| Path::new(cwd).starts_with(root))
        .map(|root| root.join(id))
}

#[cfg(all(test, target_os = "macos"))]
mod darwin_tests {
    use super::*;

    #[test]
    fn results_and_cleanup_stay_in_the_admitted_job_workspace() {
        use fractal_runtime::darwin_slot::Slot;
        for slot in [Slot::Heavy1, Slot::Gate1, Slot::Gate2, Slot::Gate3, Slot::Eval1] {
            let cwd = slot.root().join("sources");
            assert_eq!(darwin_job_workspace(cwd.to_str().unwrap(), "job"),
                Some(slot.root().join("job")));
        }
        assert!(darwin_job_workspace("/workspace/project", "job").is_none());
        assert!(darwin_job_workspace("/Users/Shared/fractal-jobs/heavy10/job", "job").is_none());
        assert!(darwin_job_workspace("", "job").is_none());
    }
}
