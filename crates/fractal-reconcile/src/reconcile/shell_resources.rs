//! Shell requests are a durable transport into the existing finite host authority.
use super::*;
use crate::store::resource_membership::{Admission, Grant};
use crate::store::shell_resources::{Execution, PayloadResult, PARENT_ENV};

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn reconcile_shell_resources(&self) -> Result<()> {
        let executions = self.store.shell_resources(&self.host, true)?;
        if executions.is_empty() { return Ok(()); }
        self.ensure_resource_authority()?;
        for mut execution in executions {
            if let Err(error) = self.drive_shell_resource(&mut execution) {
                execution.reason = format!("{error:#}");
                // A failed preparation is not proof of absence. Retain custody until native
                // retirement proves every prepared root gone, including after daemon recovery.
                execution.state = match self.retire_shell_resource(&execution) {
                    Ok(true) => "refused",
                    Ok(false) => "retiring",
                    Err(retirement) => { execution.reason.push_str(&format!("; retirement: {retirement:#}")); "retiring" }
                }.into();
            }
            if matches!(execution.state.as_str(), "completed" | "refused" | "cancelled") {
                self.seat_admission.status.state().lane_waits.retain(|wait| wait.subject != execution.command.lane.id);
                self.shell_exec_watches.lock().unwrap_or_else(PoisonError::into_inner).remove(&execution.command.id);
                execution.command.environment.clear();
            }
            if self.store.update_shell_resource(&execution)? { self.signal_changed(); }
        }
        Ok(())
    }

    fn shell_result_path(&self, id: &str) -> PathBuf {
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
        let retired = self.runtime.retire_resource_lane(&self.store, request)?;
        if retired {
            self.incremental.touch(&crate::store::resource_membership::dependency(&request.host, &request.name));
            self.incremental.touch("seat-admission");
            self.seat_admission.status.state().lane_waits.retain(|wait| wait.subject != request.id);
        }
        Ok(retired)
    }

    fn drive_shell_resource(&self, execution: &mut Execution) -> Result<()> {
        let now = u64::try_from(now_ms()).unwrap_or(u64::MAX);
        if execution.state == "finishing" {
            if self.retire_shell_resource(execution)? {
                execution.state = "completed".into();
                execution.ended_at = Some(now);
            }
            return Ok(());
        }
        if execution.cancel || execution.state == "retiring" {
            if self.retire_shell_resource(execution)? {
                execution.state = if execution.cancel { "cancelled" } else { "refused" }.into();
                execution.ended_at = Some(now);
                if execution.cancel { execution.reason = "cancelled by the shell caller".into(); }
            }
            return Ok(());
        }
        let request = execution.command.lane.clone();
        self.store.shell_resource_owner_live(&request.owner)?;
        let grant: Grant = if let Some(parent) = &execution.command.parent {
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
                    execution.ended_at = Some(now);
                    return Ok(());
                }
                Admission::Admitted(grant) => grant,
            }
        };
        let record = self.store.resource_membership_for_grant(&grant)?;
        let result_path = self.shell_result_path(&execution.command.id);
        let newly_started = !record.roots.iter().any(|root| root.runtime_id == execution.command.id);
        if newly_started {
            execution.queue_wait_ms = self.store.shell_resource_queue_wait_ms(execution, now);
            let mut environment = execution.command.environment.clone();
            environment.insert(PARENT_ENV.into(), serde_json::to_string(&crate::store::shell_resources::ParentContext { grant: grant.clone(), request: request.clone() })?);
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
            self.runtime.start_resource_guarded(&self.store, &member, &|| {
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
            if !newly_started { execution.state = "running".into(); }
            if let Some(timeout) = execution.command.timeout_ms.filter(|_| !execution.timed_out) {
                let elapsed = self.store.resource_execution_elapsed(&request.id, &request.owner, &execution.command.id, now)?.context("shell release clock missing")?;
                if elapsed >= timeout {
                    self.runtime.retire_resource_exec(&self.store, &request, &execution.command.id)?;
                    execution.timed_out = true;
                    return self.drive_shell_resource(execution);
                }
                self.arm_restart(&format!("shell-timeout:{}", execution.command.id), now_ms().saturating_add(u128::from(timeout.saturating_sub(elapsed))));
            }
            return Ok(());
        }
        anyhow::ensure!(observation.status == "exited", "shell execution status is indeterminate; custody retained");
        let (stdout, stderr) = self.runtime.resource_exec_output(&execution.command.id, &generation.generation_id)?;
        execution.stdout = stdout; execution.stderr = stderr;
        execution.exit_code = observation.exit_code.map(i32::try_from).transpose()?;
        execution.exit_signal = self.runtime.resource_exec_signal(&execution.command.id, &generation.generation_id)?;
        match fs::read(&result_path) {
            Ok(bytes) => {
                let result: PayloadResult = serde_json::from_slice(&bytes)?;
                execution.exit_code = result.exit_code; execution.exit_signal = result.exit_signal;
                execution.head_start.clone_from(&result.head_start); execution.clean_start = result.clean_start;
                execution.result = Some(result);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
        if execution.timed_out { execution.exit_code = Some(124); execution.exit_signal = None; }
        self.store.end_resource_execution(&grant, &execution.command.id, now)?;
        execution.state = "finishing".into();
        if !self.retire_shell_resource(execution)? { return Ok(()); }
        execution.state = "completed".into();
        execution.reason = if execution.timed_out { "payload execution timeout; custody retired" } else { "payload exited and execution custody retired" }.into();
        execution.ended_at = Some(now);
        self.shell_exec_watches.lock().unwrap_or_else(PoisonError::into_inner).remove(&execution.command.id);
        Ok(())
    }
}
