use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const SCHEMA: &str = "st.runtime.exec-generation.v1";

/// Positive producer evidence: this exact attempt returned before creating any child.
/// Absence of a generation record after a child was created is not this proof.
#[derive(Debug)]
pub struct StartRejected {
    pub operation: Option<String>,
}
impl std::fmt::Display for StartRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("launch rejected before producer creation")
    }
}
impl std::error::Error for StartRejected {}

/// Host admission identity persisted before any finite payload can leave its preparation gate.
/// This is recovery evidence, not an authority grant reconstructed from payload environment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceExecutionBinding {
    pub membership_id: String,
    pub generation: String,
    pub authority_epoch: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecGeneration {
    pub schema: String,
    pub pid: u32,
    pub created_at_unix_ms: u128,
    pub start_token: u64,
    pub generation_id: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub exit_signal: Option<i32>,
    #[serde(default)]
    pub group_exited: bool,
    #[serde(default)]
    pub isolation_mode: String,
    #[serde(default)]
    pub scope_unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_operation: Option<String>,
    /// Confirmed finite payload-gate release, not process creation or API-return time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_released_at_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_membership: Option<ResourceExecutionBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecObservation {
    Running(ExecGeneration),
    Exited(ExecGeneration),
    Indeterminate(String),
}

/// Coverage is explicit: detached sessions never promise hierarchical resource enforcement.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceCoverage {
    Enforced,
    LogicalOnly,
}

#[derive(Clone, Debug)]
pub struct FiniteLane {
    pub name: String,
    pub capacity: Option<crate::lane::Capacity>,
    pub require_enforced: bool,
}

/// The host authority binds a generation and commits its payload release together.
pub type PayloadAuthorization<'a> =
    dyn Fn(&ExecGeneration, &mut dyn FnMut() -> Result<u64>) -> Result<()> + 'a;

/// Borrowed host admission policy and callbacks for one finite launch.
pub struct FiniteExecutionAuthorization<'a> {
    pub lane: &'a FiniteLane,
    pub binding: &'a ResourceExecutionBinding,
    pub guard: &'a dyn Fn() -> Result<()>,
    pub authorize: &'a PayloadAuthorization<'a>,
}

#[derive(Default)]
struct ExecutionGuards<'a> {
    check: Option<&'a dyn Fn() -> Result<()>>,
    release: Option<&'a PayloadAuthorization<'a>>,
}

#[derive(Debug)]
pub struct UnsupportedCoverage;

impl std::fmt::Display for UnsupportedCoverage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("UnsupportedCoverage: finite execution requires Linux systemd lane enforcement")
    }
}
impl std::error::Error for UnsupportedCoverage {}

impl FiniteLane {
    pub fn coverage(&self) -> Result<ResourceCoverage> {
        if crate::lane::enforced()
            && crate::isolation_mode() == crate::Isolation::Scope
            && crate::isolate::systemd_254()
        {
            Ok(ResourceCoverage::Enforced)
        } else if self.require_enforced {
            Err(UnsupportedCoverage.into())
        } else {
            Ok(ResourceCoverage::LogicalOnly)
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExecRuntime {
    state_dir: PathBuf,
    log_dir: PathBuf,
    owned_children: Arc<Mutex<BTreeMap<u32, OwnedChild>>>,
    unlaned_seat: Option<crate::UnlanedSeatPlacement>,
    scope_join_limit: Duration,
    finite_lane: Option<FiniteLane>,
    resource_membership: Option<ResourceExecutionBinding>,
}

/// A child this runtime started, until its exit is in its record.
#[derive(Debug)]
enum OwnedChild {
    Running(Child),
    /// Reaped, with its exit not yet written to its record. The record is written outside the
    /// map's lock, which every exec shares.
    Exited(std::process::ExitStatus),
}

#[derive(Debug, Clone, Copy)]
enum ExecTeardownCaller {
    EndIf,
    SignalIf,
    EndLeftovers,
    EndLeftoversLater,
    SpawnReplacement,
    Remove,
}

fn teardown_span(
    id: &str,
    caller: ExecTeardownCaller,
    expected_generation: Option<&str>,
    observation: Option<&ExecObservation>,
) -> tracing::Span {
    let (status, generation) = match observation {
        Some(ExecObservation::Running(generation)) => ("running", Some(generation)),
        Some(ExecObservation::Exited(generation)) => ("exited", Some(generation)),
        Some(ExecObservation::Indeterminate(_)) => ("indeterminate", None),
        None => ("missing", None),
    };
    let span = tracing::info_span!(
        "exec_teardown",
        ?caller,
        runtime_id = id,
        expected_incarnation = expected_generation.unwrap_or("not_applicable"),
        observed_incarnation = "unavailable",
        operation_id = "unavailable",
        start_operation_id = "unavailable",
        registry_status = status,
        registry_pid = tracing::field::Empty,
        registry_created_at = tracing::field::Empty,
        registry_exit_code = tracing::field::Empty,
        registry_exit_signal = tracing::field::Empty,
        registry_group_exited = tracing::field::Empty,
        recorded_scope = "unavailable",
        tagged_scope = "not_applicable",
        spawn_lock = "not_applicable",
        start_token = tracing::field::Empty,
    );
    if let Some(generation) = generation {
        span.record("observed_incarnation", generation.generation_id.as_str());
        span.record("start_operation_id", generation.start_operation.as_deref().unwrap_or("unavailable"));
        span.record("registry_pid", generation.pid);
        span.record("registry_created_at", tracing::field::display(generation.created_at_unix_ms));
        span.record("registry_exit_code", tracing::field::debug(generation.exit_code));
        span.record("registry_exit_signal", tracing::field::debug(generation.exit_signal));
        span.record("registry_group_exited", generation.group_exited);
        span.record("recorded_scope", generation.scope_unit.as_deref().unwrap_or("not_applicable"));
        span.record("start_token", generation.start_token);
    } else {
        for field in [
            "registry_pid",
            "registry_created_at",
            "registry_exit_code",
            "registry_exit_signal",
            "registry_group_exited",
            "start_token",
        ] {
            span.record(field, "unavailable");
        }
    }
    span
}

impl ExecRuntime {
    pub fn new(state_dir: PathBuf, log_dir: PathBuf) -> Self {
        crate::warn_if_degraded("st3");
        Self {
            state_dir,
            log_dir,
            owned_children: Arc::new(Mutex::new(BTreeMap::new())),
            unlaned_seat: None,
            scope_join_limit: crate::isolate::SCOPE_JOIN_LIMIT,
            finite_lane: None,
            resource_membership: None,
        }
    }

    #[cfg(test)]
    fn with_scope_join_limit(mut self, limit: Duration) -> Self {
        self.scope_join_limit = limit;
        self
    }

    /// Set only by the daemon's seat runtime, never inferred from ambient environment.
    pub fn with_unlaned_seat(mut self, policy: Option<crate::UnlanedSeatPlacement>) -> Self {
        self.unlaned_seat = policy;
        self
    }

    pub fn spawn(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<ExecGeneration> {
        self.spawn_guarded(id, launch, cwd, env, ExecutionGuards::default())
    }

    pub fn spawn_checked(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        guard: &dyn Fn() -> Result<()>,
    ) -> Result<ExecGeneration> {
        self.spawn_guarded(id, launch, cwd, env, ExecutionGuards { check: Some(guard), release: None })
    }

    /// Runtime-only checked launch. A guard check is not atomic with an external authority's
    /// cancellation transaction; host resource membership must use [`Self::spawn_in_lane_authorized`].
    pub fn spawn_in_lane_checked(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        lane: &FiniteLane,
        guard: &dyn Fn() -> Result<()>,
    ) -> Result<ExecGeneration> {
        lane.coverage()?;
        let mut runtime = self.clone();
        runtime.finite_lane = Some(lane.clone());
        runtime.spawn_checked(id, launch, cwd, env, guard)
    }

    /// Bind the generation and release its payload in the host authority's serialized transaction.
    /// `authorize` must durably bind `execution` before invoking `release`, and commit its returned
    /// timestamp before returning. The supplied closure performs the actual gate write and runtime
    /// receipt persistence; checking permission and releasing after the callback returns is unsafe.
    pub fn spawn_in_lane_authorized(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        authorization: FiniteExecutionAuthorization<'_>,
    ) -> Result<ExecGeneration> {
        authorization.lane.coverage()?;
        let mut runtime = self.clone();
        runtime.finite_lane = Some(authorization.lane.clone());
        runtime.resource_membership = Some(authorization.binding.clone());
        runtime.spawn_guarded(id, launch, cwd, env, ExecutionGuards {
            check: Some(authorization.guard), release: Some(authorization.authorize),
        })
    }

    fn spawn_guarded(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        guards: ExecutionGuards<'_>,
    ) -> Result<ExecGeneration> {
        let mut child_created = false;
        self.spawn_inner(id, launch, cwd, env, guards, &mut child_created)
            .map_err(|error| {
                if child_created {
                    error
                } else {
                    error.context(StartRejected {
                        operation: env.get("ST3_START_OPERATION").cloned(),
                    })
                }
            })
    }

    fn spawn_inner(
        &self,
        id: &str,
        launch: &crate::Launch,
        cwd: &Path,
        env: &BTreeMap<String, String>,
        guards: ExecutionGuards<'_>,
        child_created: &mut bool,
    ) -> Result<ExecGeneration> {
        let ExecutionGuards { check: guard, release: authorize } = guards;
        crate::procfs::require_matching_procfs()?;
        fs::create_dir_all(&self.state_dir)?;
        fs::create_dir_all(&self.log_dir)?;
        if self.finite_lane.is_some() {
            match self.observe(id)? {
                Some(ExecObservation::Running(generation) | ExecObservation::Exited(generation)) => {
                    anyhow::ensure!(self.retired_if(id, &generation.generation_id)?,
                        "finite execution predecessor retains containment custody");
                }
                Some(ExecObservation::Indeterminate(reason)) => {
                    anyhow::bail!("finite execution predecessor is indeterminate: {reason}");
                }
                None => {}
            }
        }
        // What the previous generation left running ends before its replacement starts. Each
        // launch has a scope of its own, so this never reaches the replacement.
        if let Err(error) = self.end_leftovers_for(id, ExecTeardownCaller::SpawnReplacement) {
            if self.finite_lane.is_some() {
                return Err(error.context("finite execution replacement retains previous containment custody"));
            }
            eprintln!("st3: WARN what the last generation of {id} left running did not end: {error:#}");
        }
        self.rotate_generation(id)?;
        let log_path = self.log_path(id, false);
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&log_path)
            .with_context(|| format!("open exec log {}", log_path.display()))?;

        let (program, arguments): (OsString, Vec<OsString>) = match launch {
            crate::Launch::Shell(source) => (
                OsString::from("sh"),
                vec![OsString::from("-c"), OsString::from(source)],
            ),
            crate::Launch::Argv(argv) => {
                let (program, arguments) = argv
                    .split_first()
                    .context("an argv launch must contain a program")?;
                (
                    OsString::from(program),
                    arguments.iter().map(OsString::from).collect(),
                )
            }
        };
        // Builds and tests run below the live path; see `crate::priority`.
        let prefix = if self.finite_lane.is_some() && cfg!(target_os = "macos") {
            Vec::new()
        } else {
            crate::work_prefix(env)
        };
        let (program, arguments) = match prefix.split_first() {
            Some((prefix, prefix_arguments)) => (
                OsString::from(prefix),
                prefix_arguments
                    .iter()
                    .map(OsString::from)
                    .chain(std::iter::once(program))
                    .chain(arguments)
                    .collect(),
            ),
            None => (program, arguments),
        };
        let unit = crate::scope_unit("st3", id);
        // Finite placement is host-supplied, never selected by payload environment.
        let mut env = env.clone();
        let capacity = crate::lane::Capacity::take(&mut env);
        let coverage = self.finite_lane.as_ref().map(FiniteLane::coverage).transpose()?;
        let lane = match (&self.finite_lane, coverage) {
            (Some(policy), Some(ResourceCoverage::Enforced)) => {
                Some(crate::lane::prepare_finite(&policy.name, policy.capacity)?)
            }
            (Some(_), _) => None,
            (None, _) => crate::lane::prepare(capacity, id)?,
        };
        // stdin is a private inherited pipe, not an environment token or filesystem sentinel.
        // EOF on daemon loss refuses execution; the wrapper execs the original argv unchanged.
        let (program, arguments) = if self.finite_lane.is_some() {
            let gated_arguments = std::iter::once(OsString::from("-c"))
                .chain(std::iter::once(OsString::from(
                    "IFS= read -r gate && [ \"$gate\" = release ] || exit 125; exec \"$@\"",
                )))
                .chain(std::iter::once(OsString::from("fractal-finite-gate")))
                .chain(std::iter::once(program))
                .chain(arguments)
                .collect::<Vec<_>>();
            (OsString::from("sh"), gated_arguments)
        } else {
            (program, arguments)
        };
        let argument_refs = arguments.iter().map(OsString::as_os_str).collect::<Vec<_>>();
        let mut command = crate::isolate::wrap_in(
            &unit,
            program.as_os_str(),
            &argument_refs,
            Some(&env),
            lane.as_ref(),
            self.unlaned_seat.as_ref(),
        );
        command
            .current_dir(cwd)
            .stdin(if self.finite_lane.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(log.try_clone()?)
            .stderr(log);
        if self.finite_lane.is_some() {
            let stream = |name| -> Result<fs::File> {
                fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600)
                    .open(self.stream_path(id, name)).with_context(|| format!("open finite {name}"))
            };
            command.stdout(stream("stdout")?).stderr(stream("stderr")?);
        }
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        if let Some(guard) = guard {
            guard().inspect_err(|error| {
                tracing::debug!(
                    caller = "spawn_inner",
                    runtime_id = id,
                    operation_id = env.get("ST3_START_OPERATION").map_or("unavailable", String::as_str),
                    guard_decision = "skip",
                    reason = "spawn_guard_rejected",
                    %error,
                    "exec spawn guard rejected"
                );
            })?;
        }
        let mut child = command.spawn().context("spawn detached exec")?;
        *child_created = true;
        let pid = child.id();
        let mut payload_gate = child.stdin.take();
        let start_token = process_start_token(pid)?;
        let created_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let generation_id = generation_id(id, pid, start_token, created_at_unix_ms);
        let mut generation = ExecGeneration {
            schema: SCHEMA.into(),
            pid,
            created_at_unix_ms,
            start_token,
            generation_id,
            exit_code: None,
            exit_signal: None,
            group_exited: false,
            isolation_mode: isolation_name(crate::isolation_mode()).into(),
            scope_unit: (crate::isolation_mode() == crate::Isolation::Scope).then_some(unit),
            start_operation: env.get("ST3_START_OPERATION").cloned(),
            payload_released_at_unix_ms: None,
            resource_membership: self.resource_membership.clone(),
        };
        self.write_generation(id, &generation)?;
        self.owned_children
            .lock()
            .expect("exec child mutex poisoned")
            .insert(pid, OwnedChild::Running(child));
        // `systemd-run` joins its scope only when the user manager starts it, and runs the task
        // after. Until then it is in this process's cgroup and ends with it, so the exec outlives
        // this process only once it has joined.
        if let Some(unit) = generation.scope_unit.as_deref() {
            let join = crate::isolate::await_join(unit, pid, self.scope_join_limit, || {
                process_generation_alive(pid, start_token)
            });
            if !matches!(join, Ok(crate::isolate::Join::Joined | crate::isolate::Join::Exited)) {
                // A failed spawn leaves nothing that could still die with this process. Unjoined,
                // the leader is still `systemd-run`, alone in the session it started.
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
                join.with_context(|| {
                    format!("the exec's join of its scope {unit} could not be read, so it was ended")
                })?;
                anyhow::bail!(
                    "the exec did not join its scope {unit} within {}ms, so it was ended",
                    self.scope_join_limit.as_millis()
                );
            }
        }
        if let Some(policy) = &self.finite_lane {
            let mut released_at = None;
            let release_result = (|| -> Result<()> {
                if let Some(lane) = &lane {
                    crate::lane::verify_finite(lane, policy.capacity,
                        generation.scope_unit.as_deref().context("finite execution lacks its scope")?, pid)?;
                }
                let mut attempted = false;
                let mut persisted = false;
                let mut release = || -> Result<u64> {
                    anyhow::ensure!(!attempted, "finite execution payload release was already attempted");
                    attempted = true;
                    payload_gate.as_mut().context("finite execution lacks its payload gate")?
                        .write_all(b"release\n").context("release finite execution payload")?;
                    let timestamp = u64::try_from(
                        SystemTime::now().duration_since(UNIX_EPOCH)
                            .context("execution release clock predates UNIX epoch")?.as_millis(),
                    )?;
                    released_at = Some(timestamp);
                    let mut receipt = generation.clone();
                    receipt.payload_released_at_unix_ms = released_at;
                    self.write_generation(id, &receipt)
                        .context("persist released payload receipt; execution custody retained on failure")?;
                    persisted = true;
                    Ok(timestamp)
                };
                if let Some(authorize) = authorize {
                    authorize(&generation, &mut release)?;
                } else {
                    if let Some(guard) = guard {
                        guard()?;
                    }
                    release()?;
                }
                anyhow::ensure!(persisted, "finite execution authorization omitted payload release receipt");
                Ok(())
            })();
            generation.payload_released_at_unix_ms = released_at;
            if let Err(error) = release_result {
                drop(payload_gate);
                let cleanup = self.kill_if(id, Some(&generation.generation_id));
                return match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(error.context(format!("finite execution cleanup retains custody: {cleanup:#}"))),
                };
            }
        }
        Ok(generation)
    }

    pub fn observe(&self, id: &str) -> Result<Option<ExecObservation>> {
        let path = self.record_path(id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut generation: ExecGeneration = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => return Ok(Some(ExecObservation::Indeterminate(error.to_string()))),
        };
        if generation.schema != SCHEMA {
            return Ok(Some(ExecObservation::Indeterminate(format!(
                "unsupported generation schema {}",
                generation.schema
            ))));
        }
        // A terminal group is durable evidence about this exact exec. In particular, do not
        // let an unrelated process that later reuses the PID make a completed generation look
        // live again on platforms whose best available start token is the PID itself.
        if generation.group_exited {
            return Ok(Some(ExecObservation::Exited(generation)));
        }
        if generation.exit_code.is_some() || generation.exit_signal.is_some() {
            return self.observe_exited_group(id, generation).map(Some);
        }
        let running = self.reap_owned(id, &mut generation)?;
        if generation.group_exited {
            return Ok(Some(ExecObservation::Exited(generation)));
        }
        if !running && (generation.exit_code.is_some() || generation.exit_signal.is_some()) {
            return self.observe_exited_group(id, generation).map(Some);
        }
        #[cfg(test)]
        BEFORE_IDENTITY.with(|hook| {
            if let Some(hook) = &*hook.borrow() {
                hook(generation.pid);
            }
        });
        match process_identity(generation.pid) {
            Ok((state, token)) if token == generation.start_token && state != 'Z' => {
                Ok(Some(ExecObservation::Running(generation)))
            }
            // An owned child that exited after the look above is a zombie until it is reaped,
            // so its status is still there to take.
            Ok((_state, token)) if token == generation.start_token => {
                self.observe_exited_group(id, generation).map(Some)
            }
            Ok(_) => Ok(Some(ExecObservation::Exited(generation))),
            Err(error) if process_is_absent(&error) => {
                self.observe_exited_group(id, generation).map(Some)
            }
            Err(error) if error.is::<crate::ProcessNamespaceMismatch>() => Err(error),
            Err(error) => Ok(Some(ExecObservation::Indeterminate(error.to_string()))),
        }
    }

    /// Wait for this generation's leader with a kernel exit subscription, not a reconciliation
    /// timer. The child map is only locked by the normal nonblocking reap after the notification.
    /// Descendants can still retain containment custody: completion is not retirement.
    pub fn wait_for_exit(&self, id: &str, expected_generation: &str) -> Result<ExecGeneration> {
        let generation = self.read_generation_path(&self.record_path(id))?
            .context("execution generation is missing")?;
        anyhow::ensure!(generation.generation_id == expected_generation, "execution generation changed");
        if generation.exit_code.is_none() && generation.exit_signal.is_none() && !generation.group_exited {
            wait_process_exit(generation.pid, generation.start_token)?;
        }
        let observation = self.observe(id)?.context("execution generation disappeared while waiting")?;
        let generation = match observation {
            ExecObservation::Running(generation) | ExecObservation::Exited(generation) => generation,
            ExecObservation::Indeterminate(reason) => anyhow::bail!("execution exit is indeterminate: {reason}"),
        };
        anyhow::ensure!(generation.generation_id == expected_generation, "execution generation changed while waiting");
        Ok(generation)
    }

    /// Separate finite payload streams, fenced against replacement before and after the read.
    /// A recovered non-child execution may have an unknown exit status; output does not invent it.
    pub fn read_output(&self, id: &str, expected_generation: &str) -> Result<(String, String)> {
        let check = || -> Result<()> {
            let generation = self.read_generation_path(&self.record_path(id))?
                .context("execution generation is missing")?;
            anyhow::ensure!(generation.generation_id == expected_generation, "execution generation changed while reading output");
            Ok(())
        };
        check()?;
        let stdout = fs::read(self.stream_path(id, "stdout"))?;
        let stderr = fs::read(self.stream_path(id, "stderr"))?;
        check()?;
        Ok((String::from_utf8_lossy(&stdout).into_owned(), String::from_utf8_lossy(&stderr).into_owned()))
    }

    /// Reaps `generation`'s leader if this runtime started it and it has exited, and returns
    /// whether it is a child of this runtime that is still running. The gate poll and a
    /// reconcile pass can observe one exec at once. Whichever reaps the child marks it exited
    /// with its status before it writes the record, and lets the child go only after; an
    /// observer that finds the mark takes the exit from it, and one that finds no child reads
    /// the record again. Otherwise an observer that read the record before the exit would find
    /// the process gone and record the exit without its code.
    fn reap_owned(&self, id: &str, generation: &mut ExecGeneration) -> Result<bool> {
        let exited = {
            let mut children = self
                .owned_children
                .lock()
                .expect("exec child mutex poisoned");
            match children.get_mut(&generation.pid) {
                Some(OwnedChild::Running(child)) => {
                    let Some(status) = child.try_wait()? else {
                        return Ok(true);
                    };
                    children.insert(generation.pid, OwnedChild::Exited(status));
                    Some(status)
                }
                Some(OwnedChild::Exited(status)) => Some(*status),
                None => None,
            }
        };
        if let Some(status) = exited {
            generation.exit_code = status.code();
            generation.exit_signal = exit_signal(&status);
            #[cfg(test)]
            BEFORE_EXIT_WRITE.with(|hook| {
                if let Some(hook) = &*hook.borrow() {
                    hook(generation.pid);
                }
            });
            // A failed write keeps the mark, so a later observer writes the exit again.
            self.write_generation(id, generation)?;
            let mut children = self
                .owned_children
                .lock()
                .expect("exec child mutex poisoned");
            if matches!(children.get(&generation.pid), Some(OwnedChild::Exited(_))) {
                children.remove(&generation.pid);
            }
            return Ok(false);
        }
        // The exit was written before its mark was removed, so this read sees it. A record that
        // no longer reads keeps the copy this observer has.
        if let Ok(Some(current)) = self.read_generation_path(&self.record_path(id))
            && current.schema == SCHEMA
            && current.generation_id == generation.generation_id
        {
            *generation = current;
        }
        Ok(false)
    }

    fn observe_exited_group(
        &self,
        id: &str,
        mut generation: ExecGeneration,
    ) -> Result<ExecObservation> {
        // The leader can exit after observe's first status probe. Reap a waitable
        // child before persisting group completion, including while descendants live.
        // A zombie leader thread can still have live sibling threads: never block
        // waiting for the whole process to exit while holding the child-map mutex.
        if generation.exit_code.is_none()
            && generation.exit_signal.is_none()
            && self.reap_owned(id, &mut generation)?
        {
            return Ok(ExecObservation::Running(generation));
        }
        if generation.group_exited {
            return Ok(ExecObservation::Exited(generation));
        }
        match observe_exited_group(generation)? {
            ExecObservation::Exited(mut generation) => {
                generation.group_exited = true;
                self.write_generation(id, &generation)?;
                Ok(ExecObservation::Exited(generation))
            }
            observation => Ok(observation),
        }
    }

    pub fn stop(&self, id: &str) -> Result<()> {
        self.stop_if(id, None)
    }

    pub fn stop_if(&self, id: &str, expected_generation: Option<&str>) -> Result<()> {
        self.end_if(id, expected_generation, libc::SIGTERM)
    }

    /// Sends `signal`, SIGTERM or SIGKILL, to everything the exec started. Its work scope holds
    /// its process group and every process that left that group, so the scope gets the signal
    /// where there is one. Once the group has ended, what it left in its scope ends.
    fn end_if(&self, id: &str, expected_generation: Option<&str>, signal: i32) -> Result<()> {
        let observation = self.observe(id)?;
        let span = teardown_span(id, ExecTeardownCaller::EndIf, expected_generation, observation.as_ref());
        let _entered = span.enter();
        let generation = match observation {
            Some(ExecObservation::Running(generation)) => generation,
            Some(ExecObservation::Exited(generation)) => {
                if expected_generation.is_some_and(|expected| expected != generation.generation_id) {
                    anyhow::bail!("exec `{id}` changed incarnation before signal {signal}");
                }
                if let Some(unit) = &generation.scope_unit {
                    let grace = if signal == libc::SIGKILL {
                        Duration::ZERO
                    } else {
                        crate::SCOPE_GRACE
                    };
                    tracing::info!(signal, guard_decision = "allow", reason = "exited_generation_leftovers", incarnation_guard = "not_applied_to_exited_generation", "ending exited exec work scope");
                    crate::end_scope(unit, grace)?;
                } else {
                    tracing::debug!(signal, guard_decision = "skip", reason = "exited_generation_without_scope", "exec teardown needs no signal");
                }
                return Ok(());
            }
            Some(ExecObservation::Indeterminate(error)) => {
                tracing::debug!(guard_decision = "skip", reason = "indeterminate_generation", %error, "exec teardown skipped");
                anyhow::bail!("cannot signal exec `{id}`: generation is indeterminate: {error}");
            }
            None => {
                tracing::debug!(guard_decision = "skip", reason = "missing_generation", "exec teardown skipped");
                return Ok(());
            }
        };
        if expected_generation.is_some_and(|expected| expected != generation.generation_id) {
            tracing::debug!(signal, guard_decision = "skip", reason = "incarnation_mismatch", "exec teardown rejected");
            anyhow::bail!("exec `{id}` changed incarnation before signal {signal}");
        }
        if let Some(unit) = &generation.scope_unit {
            // Read before the signal: a leader that has not joined its scope yet is systemd-run,
            // alone in its group, and the scope's signal reaches nobody, so the group gets it too.
            // Should the scope start in between, the task keeps the leader's PID and group.
            let joined = crate::isolate::joined(unit, generation.pid);
            tracing::info!(signal, joined, guard_decision = "allow", reason = "running_generation_scope", "signalling exec work scope");
            match crate::signal_scope(unit, signal) {
                Ok(true) if joined => return Ok(()),
                Ok(true) => {
                    tracing::debug!(signal, reason = "leader_not_in_scope", "signalling exec process group as well");
                }
                Ok(false) => {
                    tracing::debug!(signal, reason = "scope_signal_not_sent", "falling back to exec process group");
                }
                Err(error) => eprintln!(
                    "st3: WARN exec {id}'s scope missed signal {signal}, so only its group gets it: {error:#}"
                ),
            }
        }
        signal_group(id, generation.pid, signal)
    }

    pub fn signal_if(
        &self,
        id: &str,
        expected_generation: Option<&str>,
        signal: i32,
    ) -> Result<()> {
        let observation = self.observe(id)?;
        let span = teardown_span(id, ExecTeardownCaller::SignalIf, expected_generation, observation.as_ref());
        let _entered = span.enter();
        let Some(ExecObservation::Running(generation)) = observation else {
            tracing::debug!(signal, guard_decision = "skip", reason = "generation_not_running", "exec signal skipped");
            return Ok(());
        };
        if expected_generation.is_some_and(|expected| expected != generation.generation_id) {
            tracing::debug!(signal, guard_decision = "skip", reason = "incarnation_mismatch", "exec signal rejected");
            anyhow::bail!("exec `{id}` changed incarnation before signal {signal}");
        }
        signal_group(id, generation.pid, signal)
    }

    pub fn kill(&self, id: &str) -> Result<()> {
        self.kill_if(id, None)
    }

    pub fn kill_if(&self, id: &str, expected_generation: Option<&str>) -> Result<()> {
        self.end_if(id, expected_generation, libc::SIGKILL)
    }

    /// Stop this exact generation and await both leader exit and whole-scope emptiness with
    /// kernel notifications. This is the blocking retirement worker, never a host-writer call.
    /// A delivered signal alone, or a detached-session exit, does not authorize capacity release.
    pub fn retire_if(&self, id: &str, expected_generation: &str) -> Result<ExecGeneration> {
        self.kill_if(id, Some(expected_generation))?;
        let generation = self.wait_for_exit(id, expected_generation)?;
        let unit = generation.scope_unit.as_deref().context(
            "logical-only containment cannot prove whole-tree retirement; custody retained",
        )?;
        let guard = || -> Result<()> {
            let current = self.read_generation_path(&self.record_path(id))?
                .context("execution generation disappeared during retirement")?;
            anyhow::ensure!(current.generation_id == expected_generation
                && current.scope_unit.as_deref() == Some(unit),
                "execution generation changed during retirement");
            Ok(())
        };
        crate::isolate::end_scope_with_proof(unit, &guard)?
            .context("execution scope has no whole-tree retirement proof")?;
        anyhow::ensure!(self.retired_if(id, expected_generation)?,
            "execution retains whole-tree custody after scope retirement");
        let current = self.read_generation_path(&self.record_path(id))?
            .context("execution generation disappeared after retirement")?;
        anyhow::ensure!(current.generation_id == expected_generation,
            "execution generation changed after retirement");
        Ok(current)
    }

    /// Exact-generation whole-tree retirement. A missing/replaced record is not empty evidence.
    /// Detached sessions cannot prove escaped descendant quiescence and therefore retain custody.
    pub fn retired_if(&self, id: &str, expected_generation: &str) -> Result<bool> {
        self.retired_with_scope_proof(id, expected_generation, &crate::isolate::observe_closed_scope)
    }

    fn retired_with_scope_proof(&self, id: &str, expected_generation: &str,
        proof: &dyn Fn(&str) -> Result<Option<crate::isolate::ScopeRetirementProof>>) -> Result<bool> {
        let observation = self.observe(id)?.context("finite execution generation is missing")?;
        let mut generation = match observation {
            ExecObservation::Exited(generation) => generation,
            ExecObservation::Running(generation) => {
                anyhow::ensure!(generation.generation_id == expected_generation, "finite execution generation changed");
                return Ok(false);
            }
            ExecObservation::Indeterminate(reason) => anyhow::bail!("finite execution retirement is indeterminate: {reason}"),
        };
        anyhow::ensure!(generation.generation_id == expected_generation, "finite execution generation changed");
        let unit = generation.scope_unit.as_deref().context(
            "logical-only containment cannot prove whole-tree retirement; custody retained",
        )?;
        let Some(proof) = proof(unit)? else { return Ok(false); };
        proof.validate(unit)?;
        // PID reuse proves leader identity loss, not process-group emptiness. The exact scope
        // proof supplies that missing evidence without inspecting or signalling the reused PID.
        if !generation.group_exited {
            let current = self.read_generation_path(&self.record_path(id))?
                .context("finite execution generation disappeared during scope reconciliation")?;
            anyhow::ensure!(current.generation_id == expected_generation
                && current.scope_unit == generation.scope_unit,
                "finite execution generation changed during scope reconciliation");
            generation.group_exited = true;
            self.write_generation(id, &generation)?;
        }
        Ok(true)
    }

    /// The work scope of an exec whose process group has ended. It can still hold a process that
    /// left the group, such as a build started in a session of its own.
    pub fn leftover_scope(&self, id: &str) -> Result<Option<String>> {
        Ok(match self.observe(id)? {
            Some(ExecObservation::Exited(generation)) => generation.scope_unit,
            _ => None,
        })
    }

    /// Ends what an ended exec left running in its work scope. A running exec keeps everything:
    /// [`Self::stop_if`] ends it.
    pub fn end_leftovers(&self, id: &str) -> Result<()> {
        self.end_leftovers_for(id, ExecTeardownCaller::EndLeftovers)
    }

    fn end_leftovers_for(&self, id: &str, caller: ExecTeardownCaller) -> Result<()> {
        let observation = self.observe(id)?;
        let span = teardown_span(id, caller, None, observation.as_ref());
        let _entered = span.enter();
        if let Some(ExecObservation::Exited(generation)) = observation
            && let Some(unit) = &generation.scope_unit
        {
            tracing::info!(guard_decision = "allow", reason = "exited_generation_leftovers", "ending exec leftover scope");
            crate::end_scope(unit, crate::SCOPE_GRACE)?;
        } else {
            tracing::debug!(guard_decision = "skip", reason = "no_exited_generation_scope", "exec leftover teardown skipped");
        }
        Ok(())
    }

    /// [`Self::end_leftovers`] on a background worker. A scope already seen empty costs one read
    /// of the exec's record.
    pub fn end_leftovers_later(&self, id: &str) {
        let observation = self.observe(id);
        let span = teardown_span(id, ExecTeardownCaller::EndLeftoversLater, None, observation.as_ref().ok().and_then(Option::as_ref));
        if observation.is_err() {
            span.record("registry_status", "unavailable");
        }
        let _entered = span.enter();
        if let Err(error) = &observation {
            tracing::debug!(guard_decision = "skip", reason = "generation_observation_failed", %error, "deferred exec leftover observation failed");
        }
        let Ok(Some(ExecObservation::Exited(generation))) = observation else {
            tracing::debug!(guard_decision = "skip", reason = "no_verified_exited_generation", "deferred exec leftover teardown skipped");
            return;
        };
        let Some(unit) = generation.scope_unit else {
            tracing::debug!(guard_decision = "skip", reason = "no_recorded_scope", "deferred exec leftover teardown skipped");
            return;
        };
        if crate::isolate::has_ended(&unit) {
            tracing::debug!(guard_decision = "skip", reason = "scope_already_ended", "deferred exec leftover teardown skipped");
            return;
        }
        let id = id.to_owned();
        let worker_span = span.clone();
        crate::isolate::end_later(unit.clone(), move || {
            worker_span.in_scope(|| {
                tracing::info!(guard_decision = "allow", reason = "deferred_exited_generation_leftovers", "ending exec leftover scope");
                crate::end_scope(&unit, crate::SCOPE_GRACE)
                    .map(|_| ())
                    .with_context(|| format!("end what exec `{id}` left running"))
            })
        });
    }

    pub fn read_log(&self, id: &str) -> Result<Option<String>> {
        let path = self.log_path(id, false);
        match fs::read_to_string(&path) {
            Ok(log) => Ok(Some(log)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("read exec log {}", path.display())),
        }
    }

    pub fn read_log_bytes(&self, id: &str, previous: bool) -> Result<Option<Vec<u8>>> {
        let path = self.log_path(id, previous);
        match fs::read(&path) {
            Ok(log) => Ok(Some(log)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("read exec log {}", path.display())),
        }
    }

    pub fn previous_generation(&self, id: &str) -> Result<Option<ExecGeneration>> {
        self.read_generation_path(&self.previous_record_path(id))
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        // The record is the last place that names the exec's work scope.
        self.end_leftovers_for(id, ExecTeardownCaller::Remove)?;
        for path in [
            self.record_path(id),
            self.previous_record_path(id),
            self.log_path(id, false),
            self.log_path(id, true),
            self.stream_path(id, "stdout"),
            self.stream_path(id, "stderr"),
            self.log_dir.join(format!("{id}.stdout.1")),
            self.log_dir.join(format!("{id}.stderr.1")),
        ] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.state_dir.join(format!("{id}.json"))
    }

    fn previous_record_path(&self, id: &str) -> PathBuf {
        self.state_dir.join(format!("{id}.json.1"))
    }

    fn log_path(&self, id: &str, previous: bool) -> PathBuf {
        self.log_dir.join(if previous {
            format!("{id}.log.1")
        } else {
            format!("{id}.log")
        })
    }

    fn stream_path(&self, id: &str, stream: &str) -> PathBuf {
        self.log_dir.join(format!("{id}.{stream}"))
    }

    fn rotate_generation(&self, id: &str) -> Result<()> {
        rotate_file(&self.record_path(id), &self.previous_record_path(id))?;
        rotate_file(&self.log_path(id, false), &self.log_path(id, true))?;
        for stream in ["stdout", "stderr"] {
            let path = self.stream_path(id, stream);
            rotate_file(&path, &self.log_dir.join(format!("{id}.{stream}.1")))?;
        }
        Ok(())
    }

    fn read_generation_path(&self, path: &Path) -> Result<Option<ExecGeneration>> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(Into::into),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write_generation(&self, id: &str, generation: &ExecGeneration) -> Result<()> {
        // Release receipts and exit evidence are monotonic facts. A concurrent observer may have
        // read the pre-release record; serialize read/merge/write so it cannot erase the receipt.
        static RECORD_WRITES: Mutex<()> = Mutex::new(());
        let _writer = RECORD_WRITES.lock().expect("exec generation writer mutex poisoned");
        let current = self.read_generation_path(&self.record_path(id))?;
        let mut generation = std::borrow::Cow::Borrowed(generation);
        if let Some(current) = current.filter(|current| current.generation_id == generation.generation_id) {
            if generation.resource_membership.is_none() && current.resource_membership.is_some() {
                generation.to_mut().resource_membership = current.resource_membership.clone();
            }
            anyhow::ensure!(generation.resource_membership == current.resource_membership,
                "execution resource membership binding changed");
            if generation.payload_released_at_unix_ms.is_none() && current.payload_released_at_unix_ms.is_some() {
                generation.to_mut().payload_released_at_unix_ms = current.payload_released_at_unix_ms;
            }
            if generation.exit_code.is_none() && current.exit_code.is_some() {
                generation.to_mut().exit_code = current.exit_code;
            }
            if generation.exit_signal.is_none() && current.exit_signal.is_some() {
                generation.to_mut().exit_signal = current.exit_signal;
            }
            if current.group_exited && !generation.group_exited {
                generation.to_mut().group_exited = true;
            }
        }
        let path = self.record_path(id);
        // Two observers in this process can write one record at once.
        static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let write = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = path.with_extension(format!("json.tmp.{}.{write}", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        serde_json::to_writer(&mut file, generation.as_ref())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        Ok(())
    }
}

/// The wrapper can exit on SIGTERM while its children ignore that signal. Keep its group
/// running until those children stop, so reconciliation still escalates to SIGKILL after
/// a daemon restart. A reused leader PID never grants authority over its new group.
fn observe_exited_group(generation: ExecGeneration) -> Result<ExecObservation> {
    match process_identity(generation.pid) {
        Ok((state, token)) if token != generation.start_token || state != 'Z' => {
            return Ok(ExecObservation::Exited(generation));
        }
        Ok(_) => {}
        Err(error) if process_is_absent(&error) => {}
        Err(error) if error.is::<crate::ProcessNamespaceMismatch>() => return Err(error),
        Err(error) => return Ok(ExecObservation::Indeterminate(error.to_string())),
    }
    match process_group_is_live(generation.pid) {
        Ok(true) => Ok(ExecObservation::Running(generation)),
        Ok(false) => Ok(ExecObservation::Exited(generation)),
        Err(error) if error.is::<crate::ProcessNamespaceMismatch>() => Err(error),
        Err(error) => Ok(ExecObservation::Indeterminate(error.to_string())),
    }
}

fn signal_group(id: &str, group: u32, signal: i32) -> Result<()> {
    tracing::info!(
        runtime_id = id,
        process_group = group,
        signal,
        guard_decision = "allow",
        reason = "signal_process_group",
        "signalling exec process group"
    );
    let result = unsafe { libc::kill(-(group as i32), signal) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            tracing::debug!(runtime_id = id, process_group = group, signal, %error, "exec process group signal failed");
            return Err(error)
                .with_context(|| format!("signal {signal} to exec `{id}`'s process group"));
        }
        tracing::debug!(runtime_id = id, process_group = group, signal, guard_decision = "skip", reason = "process_group_absent", "exec process group already absent");
    }
    Ok(())
}

fn process_group_is_live(group: u32) -> Result<bool> {
    crate::procfs::require_matching_procfs()?;
    if unsafe { libc::kill(-(group as i32), 0) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        return Err(error.into());
    }
    #[cfg(target_os = "linux")]
    {
        // kill(0) includes zombies. Orphaned descendants may await reaping by the host's
        // init process; they can no longer execute work and must not hold cleanup open.
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            let Some(stat) = read_group_process_stat(&entry.path().join("stat"))? else {
                continue;
            };
            let Some((_, tail)) = stat.rsplit_once(") ") else {
                continue;
            };
            let mut fields = tail.split_whitespace();
            let state = fields.next();
            let process_group = fields.nth(1).and_then(|field| field.parse::<u32>().ok());
            if process_group == Some(group) {
                if state != Some("Z") {
                    return Ok(true);
                }
                // A zombie main thread may leave live worker threads behind. Field 20
                // counts those threads even though /proc lists only group leaders.
                let thread_count = tail
                    .split_whitespace()
                    .nth(17)
                    .context("process stat lacks a thread count")?
                    .parse::<u32>()?;
                if thread_count > 1 {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    #[cfg(not(target_os = "linux"))]
    Ok(true)
}

#[cfg(target_os = "linux")]
fn read_group_process_stat(path: &Path) -> std::io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(stat) => Ok(Some(stat)),
        // A process can be reaped after open succeeds but before the proc inode
        // is read. Linux reports ESRCH in that window, rather than ENOENT.
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ESRCH) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(None),
        Err(error) => Err(error),
    }
}

fn process_is_absent(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
    })
}

fn rotate_file(current: &Path, previous: &Path) -> Result<()> {
    if !current.exists() {
        return Ok(());
    }
    match fs::remove_file(previous) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match fs::rename(current, previous) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn isolation_name(mode: crate::Isolation) -> &'static str {
    match mode {
        crate::Isolation::Scope => "scope",
        crate::Isolation::Detached => "detached",
        crate::Isolation::DegradedDetached => "degraded-detached",
    }
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt as _;
    status.signal()
}

#[cfg(target_os = "linux")]
pub fn process_start_token(pid: u32) -> Result<u64> {
    process_identity(pid).map(|(_, token)| token)
}

/// Whether this exact captured generation can still execute. Missing/reused/zombie
/// processes are definitive loss; inaccessible or malformed process evidence is an error.
pub fn process_generation_alive(pid: u32, token: u64) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        // An unavailable procfs mount is not evidence that a particular process died.
        crate::procfs::require_matching_procfs()?;
        match process_identity(pid) {
            Ok((state, current)) => Ok(current == token && !matches!(state, 'Z' | 'X' | 'x')),
            Err(error) if error.downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, token);
        anyhow::bail!("exact process generation liveness is unavailable on this platform")
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn process_identity(pid: u32) -> Result<(char, u64)> {
    crate::procfs::require_matching_procfs()?;
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let end = stat.rfind(')').context("malformed process stat")?;
    let tail = stat.get(end + 2..).context("malformed process stat tail")?;
    let mut fields = tail.split_whitespace();
    let state = fields
        .next()
        .and_then(|value| value.chars().next())
        .context("process stat lacks a state")?;
    let token = fields
        .nth(18)
        .context("process stat lacks a start token")?
        .parse()?;
    Ok((state, token))
}

#[cfg(not(target_os = "linux"))]
pub fn process_start_token(pid: u32) -> Result<u64> {
    // This platform has no portable process start token. The runtime uses the
    // PID as its best available token, so a fast exit must not make spawn fail.
    Ok(pid as u64)
}

#[cfg(not(target_os = "linux"))]
fn process_identity(pid: u32) -> Result<(char, u64)> {
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        Ok(('R', pid as u64))
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[cfg(test)]
type IdentityHook = Box<dyn Fn(u32)>;

#[cfg(test)]
thread_local! {
    /// Runs in `observe` between taking an owned child's status and reading its process, so a
    /// test can make the child exit in that window.
    static BEFORE_IDENTITY: std::cell::RefCell<Option<IdentityHook>> =
        const { std::cell::RefCell::new(None) };
    /// Runs in `observe` after an owned child is marked exited and before its record is
    /// written, so a test can observe it in that window.
    static BEFORE_EXIT_WRITE: std::cell::RefCell<Option<IdentityHook>> =
        const { std::cell::RefCell::new(None) };
}

/// Subscribe before rechecking identity so an exit between the snapshot and wait is not lost.
#[cfg(target_os = "linux")]
fn wait_process_exit(pid: u32, start_token: u64) -> Result<()> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if descriptor < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(error).context("subscribe to execution pidfd");
    }
    // SAFETY: pidfd_open returned a fresh owned descriptor.
    let descriptor = unsafe { OwnedFd::from_raw_fd(i32::try_from(descriptor)?) };
    if !process_generation_alive(pid, start_token)? {
        return Ok(());
    }
    let mut event = libc::pollfd {
        fd: descriptor.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut event, 1, -1) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error).context("wait for execution pidfd");
        }
        anyhow::ensure!(event.revents & libc::POLLIN != 0,
            "execution pidfd produced an unreadable notification");
        return Ok(());
    }
}

#[cfg(target_os = "macos")]
fn wait_process_exit(pid: u32, _start_token: u64) -> Result<()> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    let descriptor = unsafe { libc::kqueue() };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error()).context("create execution exit kqueue");
    }
    // SAFETY: kqueue returned a fresh owned descriptor.
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    // SAFETY: kevent is a plain C event record, initialized before passing it to the kernel.
    let mut subscription: libc::kevent = unsafe { std::mem::zeroed() };
    subscription.ident = pid as libc::uintptr_t;
    subscription.filter = libc::EVFILT_PROC;
    subscription.flags = libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT;
    subscription.fflags = libc::NOTE_EXIT;
    let result = unsafe {
        libc::kevent(descriptor.as_raw_fd(), &subscription, 1, std::ptr::null_mut(), 0, std::ptr::null())
    };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(error).context("subscribe to execution exit kqueue");
    }
    loop {
        // SAFETY: the kernel initializes this single event record on successful return.
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        let ready = unsafe {
            libc::kevent(descriptor.as_raw_fd(), std::ptr::null(), 0, &mut event, 1, std::ptr::null())
        };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error).context("wait for execution exit kqueue");
        }
        anyhow::ensure!(ready == 1 && event.flags & libc::EV_ERROR == 0
            && event.ident == pid as libc::uintptr_t && event.fflags & libc::NOTE_EXIT != 0,
            "execution kqueue produced an unreadable exit notification");
        return Ok(());
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn wait_process_exit(_pid: u32, _start_token: u64) -> Result<()> {
    anyhow::bail!("kernel execution exit subscriptions are unavailable on this platform")
}

fn generation_id(id: &str, pid: u32, start_token: u64, created_at_unix_ms: u128) -> String {
    let mut hash = Sha256::new();
    hash.update(id.as_bytes());
    hash.update(pid.to_be_bytes());
    hash.update(start_token.to_be_bytes());
    hash.update(created_at_unix_ms.to_be_bytes());
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_resource_binding() -> ResourceExecutionBinding {
        ResourceExecutionBinding {
            membership_id: "fixture-membership".into(),
            generation: "fixture-membership-generation".into(),
            authority_epoch: "fixture-host-epoch".into(),
        }
    }

    #[test]
    fn finite_authorization_wraps_actual_release_and_observes_durable_receipt() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let policy = FiniteLane {
            name: "authorization_fixture".into(), capacity: None, require_enforced: false,
        };
        let authorized = std::cell::Cell::new(false);
        let generation = runtime.spawn_in_lane_authorized(
            "authorized", &crate::Launch::Shell("touch payload-started; exit 9".into()),
            root.path(), &std::env::vars().collect(), FiniteExecutionAuthorization {
                lane: &policy, binding: &fixture_resource_binding(), guard: &|| Ok(()),
                authorize: &|execution, release| {
                let bound = runtime.read_generation_path(&runtime.record_path("authorized"))?.unwrap();
                assert_eq!(bound.generation_id, execution.generation_id);
                assert_eq!(bound.payload_released_at_unix_ms, None);
                assert_eq!(bound.resource_membership, Some(fixture_resource_binding()));
                assert!(!root.path().join("payload-started").exists());
                let timestamp = release()?;
                let receipt = runtime.read_generation_path(&runtime.record_path("authorized"))?.unwrap();
                assert_eq!(receipt.payload_released_at_unix_ms, Some(timestamp));
                assert!(release().is_err(), "authorization cannot release a generation twice");
                authorized.set(true);
                Ok(())
                },
            },
        ).unwrap();
        assert!(authorized.get());
        assert!(generation.payload_released_at_unix_ms.is_some());
        assert_eq!(runtime.wait_for_exit("authorized", &generation.generation_id).unwrap().exit_code, Some(9));
        assert!(root.path().join("payload-started").exists());
    }

    #[test]
    fn finite_authorization_cannot_succeed_without_invoking_release() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let policy = FiniteLane {
            name: "omitted_release_fixture".into(), capacity: None, require_enforced: false,
        };
        let error = runtime.spawn_in_lane_authorized(
            "omitted", &crate::Launch::Shell("touch unauthorized-payload".into()),
            root.path(), &std::env::vars().collect(), FiniteExecutionAuthorization {
                lane: &policy, binding: &fixture_resource_binding(), guard: &|| Ok(()),
                authorize: &|_, _| Ok(()),
            },
        ).unwrap_err();
        assert!(format!("{error:#}").contains("omitted payload release receipt"));
        assert!(!root.path().join("unauthorized-payload").exists());
        let record = runtime.read_generation_path(&runtime.record_path("omitted")).unwrap().unwrap();
        assert_eq!(record.payload_released_at_unix_ms, None);
        assert_eq!(record.resource_membership, Some(fixture_resource_binding()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reused_leader_pid_requires_exact_empty_scope_before_retirement() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        fs::create_dir_all(&runtime.state_dir).unwrap();
        // A restarted daemon has no owned child; the persisted PID names another process.
        let generation = ExecGeneration {
            scope_unit: Some("st3-reused-leader.scope".into()),
            ..serde_json::from_value(serde_json::json!({
                "schema": SCHEMA, "pid": std::process::id(), "createdAtUnixMs": 1,
                "startToken": process_start_token(std::process::id()).unwrap().wrapping_add(1),
                "generationId": "old-generation"
            })).unwrap()
        };
        runtime.write_generation("reused", &generation).unwrap();
        assert!(!runtime.retired_with_scope_proof("reused", "old-generation", &|_| Ok(None)).unwrap());
        assert!(!runtime.read_generation_path(&runtime.record_path("reused")).unwrap().unwrap().group_exited);
        let proof = crate::isolate::ScopeRetirementProof {
            version: 1, unit: generation.scope_unit.clone().unwrap(),
            load_state: "not-found".into(), active_state: "inactive".into(),
            control_group: String::new(), cgroup: None,
        };
        let mut wrong = proof.clone();
        wrong.unit = "st3-other.scope".into();
        assert!(runtime.retired_with_scope_proof("reused", "old-generation", &|_| Ok(Some(wrong.clone()))).is_err());
        assert!(runtime.retired_with_scope_proof("reused", "old-generation", &|unit| {
            assert_eq!(unit, proof.unit);
            Ok(Some(proof.clone()))
        }).unwrap());
        assert!(runtime.read_generation_path(&runtime.record_path("reused")).unwrap().unwrap().group_exited);
        assert!(matches!(runtime.observe("reused").unwrap(), Some(ExecObservation::Exited(_))));
        assert!(runtime.retired_with_scope_proof("reused", "replacement", &|_| Ok(Some(proof.clone()))).is_err());
    }

    #[test]
    fn finite_kernel_wait_preserves_status_and_separate_output() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let policy = FiniteLane {
            name: "output_fixture".into(), capacity: None, require_enforced: false,
        };
        let generation = runtime.spawn_in_lane_checked(
            "output", &crate::Launch::Shell("printf stdout-value; printf stderr-value >&2; exit 7".into()),
            root.path(), &std::env::vars().collect(), &policy, &|| Ok(()),
        ).unwrap();
        let ended = runtime.wait_for_exit("output", &generation.generation_id).unwrap();
        assert_eq!(ended.exit_code, Some(7));
        assert!(generation.payload_released_at_unix_ms.is_some());
        assert_eq!(ended.payload_released_at_unix_ms, generation.payload_released_at_unix_ms);
        let mut stale = generation.clone();
        stale.payload_released_at_unix_ms = None;
        stale.exit_code = None;
        stale.exit_signal = None;
        stale.group_exited = false;
        runtime.write_generation("output", &stale).unwrap();
        let persisted = runtime.wait_for_exit("output", &generation.generation_id).unwrap();
        assert_eq!(persisted.exit_code, Some(7));
        assert_eq!(persisted.payload_released_at_unix_ms, generation.payload_released_at_unix_ms);
        assert_eq!(runtime.read_output("output", &generation.generation_id).unwrap(),
            ("stdout-value".to_owned(), "stderr-value".to_owned()));
        assert!(runtime.wait_for_exit("output", "stale-generation").is_err());
        assert!(runtime.read_output("output", "stale-generation").is_err());
        assert_eq!(runtime.wait_for_exit("output", &generation.generation_id).unwrap().exit_code, Some(7));
    }

    #[test]
    fn finite_kernel_wait_does_not_block_observation_or_exact_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let policy = FiniteLane {
            name: "wait_fixture".into(), capacity: None, require_enforced: false,
        };
        let generation = runtime.spawn_in_lane_checked(
            "wait", &crate::Launch::Argv(vec!["sleep".into(), "60".into()]),
            root.path(), &std::env::vars().collect(), &policy, &|| Ok(()),
        ).unwrap();
        let waiting = runtime.clone();
        let expected = generation.generation_id.clone();
        let waiter = thread::spawn(move || waiting.wait_for_exit("wait", &expected));
        assert!(matches!(runtime.observe("wait").unwrap(), Some(ExecObservation::Running(_))));
        runtime.kill_if("wait", Some(&generation.generation_id)).unwrap();
        assert_eq!(waiter.join().unwrap().unwrap().exit_signal, Some(libc::SIGKILL));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finite_scope_keeps_custody_after_leader_exit_until_escaped_tree_empties() {
        if crate::isolation_mode() != crate::Isolation::Scope || !crate::isolate::systemd_254() {
            return;
        }
        let _switch = crate::lane::TEST_SWITCH.lock().unwrap();
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) { crate::lane::set_enforced(self.0); }
        }
        let _restore = Restore(crate::lane::enforced());
        crate::lane::set_enforced(true);
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let policy = FiniteLane {
            name: format!("fixture_{}", std::process::id()),
            capacity: Some(crate::lane::Capacity { cores: 1, memory: 256 * 1024 * 1024 }),
            require_enforced: true,
        };
        let generation = runtime.spawn_in_lane_checked(
            "finite-tree",
            &crate::Launch::Shell("setsid sh -c 'touch descendant-started; exec sleep 60' </dev/null >/dev/null 2>&1 & while [ ! -e descendant-started ]; do sleep 0.01; done; exit 0".into()),
            root.path(), &std::env::vars().collect(), &policy,
            &|| {
                anyhow::ensure!(!root.path().join("descendant-started").exists(),
                    "payload escaped its preparation gate");
                Ok(())
            },
        ).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if matches!(runtime.observe("finite-tree").unwrap(),
                Some(ExecObservation::Exited(_))) { break; }
            assert!(std::time::Instant::now() < deadline, "fixture leader did not exit");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!runtime.retired_if("finite-tree", &generation.generation_id).unwrap());
        assert!(runtime.kill_if("finite-tree", Some("stale-generation")).is_err());
        assert!(!runtime.retired_if("finite-tree", &generation.generation_id).unwrap());
        runtime.kill_if("finite-tree", Some(&generation.generation_id)).unwrap();
        runtime.retire_if("finite-tree", &generation.generation_id).unwrap();
        assert!(runtime.retired_if("finite-tree", &generation.generation_id).unwrap());
        crate::lane::end(&format!("finite:{}", policy.name));
    }
    #[test]
    fn finite_final_guard_failure_never_releases_payload() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let calls = std::cell::Cell::new(0);
        let policy = FiniteLane {
            name: "fixture".into(),
            capacity: None,
            require_enforced: false,
        };
        let error = runtime.spawn_in_lane_checked(
            "finite-guard", &crate::Launch::Shell("touch payload-released".into()),
            root.path(), &std::env::vars().collect(), &policy, &|| {
                calls.set(calls.get() + 1);
                anyhow::ensure!(calls.get() == 1, "fixture admission revoked");
                Ok(())
            },
        ).unwrap_err();
        assert!(format!("{error:#}").contains("fixture admission revoked"));
        assert_eq!(calls.get(), 2);
        assert!(!root.path().join("payload-released").exists());
        let record = runtime.read_generation_path(&runtime.record_path("finite-guard")).unwrap().unwrap();
        assert_eq!(record.payload_released_at_unix_ms, None);
    }

    #[test]
    fn finite_launch_generation_fences_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let policy = FiniteLane {
            name: "fixture".into(),
            capacity: None,
            require_enforced: false,
        };
        let generation = runtime.spawn_in_lane_checked(
            "finite-argv", &crate::Launch::Argv(vec![
                "sh".into(), "-c".into(), "printf '%s' \"$1\"; sleep 60".into(),
                "fixture".into(), "literal $HOME % value".into(),
            ]), root.path(), &std::env::vars().collect(), &policy, &|| Ok(()),
        ).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !runtime.read_output("finite-argv", &generation.generation_id).unwrap().0.contains("literal $HOME % value") {
            assert!(std::time::Instant::now() < deadline, "finite gate changed payload argv");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(runtime.kill_if("finite-argv", Some("stale-generation")).is_err());
        assert!(runtime.retired_if("finite-argv", "stale-generation").is_err());
        runtime.kill_if("finite-argv", Some(&generation.generation_id)).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_finite_coverage_is_logical_and_enforcement_refuses_before_creation() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let mut policy = FiniteLane {
            name: "fixture".into(),
            capacity: Some(crate::lane::Capacity::DEFAULT),
            require_enforced: false,
        };
        assert_eq!(policy.coverage().unwrap(), ResourceCoverage::LogicalOnly);
        policy.require_enforced = true;
        let error = runtime.spawn_in_lane_checked(
            "unsupported", &crate::Launch::Shell("touch payload".into()),
            root.path(), &BTreeMap::new(), &policy, &|| Ok(()),
        ).unwrap_err();
        assert!(error.is::<UnsupportedCoverage>());
        assert!(runtime.observe("unsupported").unwrap().is_none());
        assert!(!root.path().join("payload").exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_finite_exit_retains_uncertain_whole_tree_custody() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let policy = FiniteLane {
            name: "fixture".into(), capacity: None,
            require_enforced: false,
        };
        let generation = runtime.spawn_in_lane_checked(
            "logical", &crate::Launch::Shell("exit 0".into()),
            root.path(), &std::env::vars().collect(), &policy, &|| Ok(()),
        ).unwrap();
        assert!(runtime.retired_if("logical", &generation.generation_id).is_err());
        assert!(runtime.spawn_in_lane_checked(
            "logical", &crate::Launch::Shell("touch replacement".into()),
            root.path(), &std::env::vars().collect(), &policy, &|| Ok(()),
        ).is_err());
        assert!(!root.path().join("replacement").exists());
    }
    #[test]
    fn direct_exec_runtime_has_no_unlaned_seat_policy() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        assert!(runtime.unlaned_seat.is_none());
        let configured = runtime.with_unlaned_seat(Some(crate::seat_placement::tests::policy()));
        assert!(configured.unlaned_seat.is_some());
    }
    use std::thread;
    use std::time::Duration;

    #[cfg(target_os = "linux")]
    #[test]
    fn current_process_token_matches_proc_self() {
        let status = fs::read_to_string("/proc/self/status").unwrap();
        eprintln!(
            "process namespace probe: getpid={} proc_self={:?} {}",
            std::process::id(),
            fs::read_link("/proc/self").unwrap(),
            status
                .lines()
                .find(|line| line.starts_with("NSpid:"))
                .unwrap_or("NSpid: unavailable")
        );
        let stat = fs::read_to_string("/proc/self/stat").unwrap();
        let (_, tail) = stat.rsplit_once(") ").unwrap();
        let token = tail
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse::<u64>()
            .unwrap();

        assert_eq!(process_start_token(std::process::id()).unwrap(), token);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_outer_procfs_refuses_launch_and_control_before_mutation() {
        const ROOT: &str = "FRACTAL_EXEC_INHERITED_PROCFS_TEST_ROOT";
        if let Some(root) = std::env::var_os(ROOT) {
            let root = PathBuf::from(root);
            let runtime = ExecRuntime::new(root.join("exec"), root.join("logs"));
            let error = runtime
                .spawn(
                    "refused",
                    &crate::Launch::Shell("touch child-created".into()),
                    &root,
                    &BTreeMap::new(),
                )
                .unwrap_err();
            assert!(error.downcast_ref::<StartRejected>().is_some());
            assert!(error.downcast_ref::<crate::ProcessNamespaceMismatch>().is_some());
            assert!(
                process_start_token(std::process::id())
                    .unwrap_err()
                    .downcast_ref::<crate::ProcessNamespaceMismatch>()
                    .is_some()
            );
            assert!(
                process_group_is_live(std::process::id())
                    .unwrap_err()
                    .downcast_ref::<crate::ProcessNamespaceMismatch>()
                    .is_some()
            );
            let pty = crate::PtyRuntime::new(root.join("pty"));
            let error = pty
                .spawn(
                    "refused",
                    &crate::Launch::Shell("touch pty-child-created".into()),
                    &root,
                    &BTreeMap::new(),
                    None,
                    &BTreeMap::new(),
                    None,
                )
                .unwrap_err();
            assert!(error.downcast_ref::<crate::ProcessNamespaceMismatch>().is_some());
            assert!(!root.join("pty").exists());
            assert!(
                pty.snapshot()
                    .unwrap_err()
                    .downcast_ref::<crate::ProcessNamespaceMismatch>()
                    .is_some()
            );
            assert!(!root.join("pty").exists());
            assert!(!root.join("pty-child-created").exists());
            assert!(!root.join("exec").exists());
            assert!(!root.join("logs").exists());
            assert!(!root.join("child-created").exists());
            let control = ExecRuntime::new(root.join("control"), root.join("control-logs"));
            fs::create_dir_all(&control.state_dir).unwrap();
            for exit_code in [None, Some(0)] {
                let generation = ExecGeneration {
                    schema: SCHEMA.into(),
                    pid: i32::MAX as u32,
                    created_at_unix_ms: 1,
                    start_token: 0,
                    generation_id: "namespace-control-refusal".into(),
                    exit_code,
                    exit_signal: None,
                    group_exited: false,
                    isolation_mode: "detached".into(),
                    scope_unit: None,
                    start_operation: None,
                    payload_released_at_unix_ms: None,
                    resource_membership: None,
                };
                control.write_generation("control", &generation).unwrap();
                let before = fs::read(control.record_path("control")).unwrap();
                let error = control.observe("control").unwrap_err();
                assert!(error.is::<crate::ProcessNamespaceMismatch>());
                for (operation, result) in [
                    ("stop", control.stop("control")),
                    ("kill", control.kill("control")),
                    (
                        "signal",
                        control.signal_if("control", None, libc::SIGTERM),
                    ),
                ] {
                    let error = result.unwrap_err();
                    assert!(
                        error.is::<crate::ProcessNamespaceMismatch>(),
                        "{operation} lost the typed refusal: {error:#}"
                    );
                    eprintln!(
                        "namespace control refusal: {operation}, exit_code={exit_code:?}, typed={error}"
                    );
                }
                assert_eq!(fs::read(control.record_path("control")).unwrap(), before);
            }
            assert!(!root.join("control-logs").exists());
            fs::write(root.join("refusal-confirmed"), b"before-child").unwrap();
            return;
        }

        let root = tempfile::tempdir().unwrap();
        // Deliberately inherit procfs rather than mounting one for the new PID
        // namespace. Re-exec the real test binary so spawn runs in that namespace.
        let output = std::process::Command::new("unshare")
            .args(["--user", "--map-root-user", "--pid", "--fork"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::inherited_outer_procfs_refuses_launch_and_control_before_mutation",
                "--nocapture",
            ])
            .env(ROOT, root.path())
            .output()
            .expect("launch inherited-procfs namespace regression");
        // A dev box can forbid unprivileged user namespaces (Ubuntu's AppArmor sets
        // kernel.apparmor_restrict_unprivileged_userns=1), and then unshare fails before the
        // regression starts. Outside CI only, say so and record the skip in CI_TEST_ENV_SKIPS,
        // which st-test signs into the note. CI always runs it.
        let in_ci = std::env::var_os("CI_RUN_ID").is_some() || std::env::var_os("GITHUB_ACTIONS").is_some();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !in_ci
            && output.status.code() == Some(1)
            && stderr
                .lines()
                .any(|line| line.starts_with("unshare: ") && line.ends_with("Operation not permitted"))
        {
            eprintln!("skipped: no unprivileged userns on this host ({})", stderr.trim());
            if let Some(skips) = std::env::var_os("CI_TEST_ENV_SKIPS") {
                use std::io::Write;
                writeln!(
                    fs::OpenOptions::new().create(true).append(true).open(skips).unwrap(),
                    "fractal-runtime process::tests::inherited_outer_procfs_refuses_launch_and_control_before_mutation"
                )
                .unwrap();
            }
            return;
        }
        assert!(
            output.status.success(),
            "namespace regression failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!root.path().join("exec").exists());
        assert!(!root.path().join("logs").exists());
        assert!(!root.path().join("child-created").exists());
        assert!(!root.path().join("pty").exists());
        assert!(!root.path().join("pty-child-created").exists());
        assert_eq!(
            fs::read(root.path().join("refusal-confirmed")).unwrap(),
            b"before-child"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_start_token_survives_a_fast_process_exit() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();

        assert_eq!(process_start_token(pid).unwrap(), pid as u64);
    }

    #[test]
    fn an_absent_process_is_a_normal_exit_on_every_unix_platform() {
        let error = anyhow::Error::from(std::io::Error::from_raw_os_error(libc::ESRCH));

        assert!(process_is_absent(&error));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_process_reaped_after_its_stat_is_opened_does_not_abort_a_group_scan() {
        use std::os::fd::AsRawFd as _;

        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let stat = fs::File::open(format!("/proc/{}/stat", child.id())).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        // Keep the opened proc inode after reaping, rather than relying on load to
        // exit between read_to_string's open and read. Reopening it has the same ESRCH.
        let path = PathBuf::from(format!("/proc/self/fd/{}", stat.as_raw_fd()));
        assert_eq!(
            fs::read_to_string(&path).unwrap_err().raw_os_error(),
            Some(libc::ESRCH)
        );
        assert_eq!(read_group_process_stat(&path).unwrap(), None);
        assert!(read_group_process_stat(Path::new("/proc/self")).is_err());
    }

    #[test]
    fn an_indeterminate_generation_cannot_certify_a_stop_or_kill() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        fs::create_dir_all(&runtime.state_dir).unwrap();
        fs::write(runtime.record_path("work"), b"unreadable generation").unwrap();

        assert!(matches!(
            runtime.observe("work").unwrap(),
            Some(ExecObservation::Indeterminate(_))
        ));
        assert!(runtime.stop_if("work", Some("expected-generation")).is_err());
        assert!(runtime.kill_if("work", Some("expected-generation")).is_err());
    }

    #[test]
    fn a_terminal_record_stays_exited_when_its_pid_is_live() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let pid = std::process::id();
        let generation = ExecGeneration {
            schema: SCHEMA.into(),
            pid,
            created_at_unix_ms: 1,
            start_token: process_start_token(pid).unwrap(),
            generation_id: "completed-generation".into(),
            exit_code: Some(0),
            exit_signal: None,
            group_exited: true,
            isolation_mode: "detached".into(),
            scope_unit: None,
            start_operation: None,
            payload_released_at_unix_ms: None,
            resource_membership: None,
        };
        fs::create_dir_all(&runtime.state_dir).unwrap();
        runtime.write_generation("completed", &generation).unwrap();

        assert_eq!(
            runtime.observe("completed").unwrap(),
            Some(ExecObservation::Exited(generation))
        );
    }

    #[test]
    fn launch_operation_is_bound_to_the_persisted_exec_generation() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let mut environment = std::env::vars().collect::<BTreeMap<_, _>>();
        environment.insert("ST3_START_OPERATION".into(), "operation-one".into());
        let launched = runtime
            .spawn(
                "operation-proof",
                &crate::Launch::Argv(vec!["sh".into(), "-c".into(), "exit 0".into()]),
                root.path(),
                &environment,
            )
            .unwrap();
        assert_eq!(launched.start_operation.as_deref(), Some("operation-one"));
        let exited = wait_for_exit(&runtime, "operation-proof");
        assert_eq!(exited.generation_id, launched.generation_id);
        assert_eq!(exited.start_operation, launched.start_operation);
        let reopened = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        assert_eq!(
            reopened.observe("operation-proof").unwrap(),
            Some(ExecObservation::Exited(exited))
        );
    }

    fn wait_for_exit(runtime: &ExecRuntime, id: &str) -> ExecGeneration {
        for _ in 0..200 {
            match runtime.observe(id).unwrap() {
                Some(ExecObservation::Exited(generation)) => return generation,
                Some(ExecObservation::Running(_)) => thread::sleep(Duration::from_millis(10)),
                other => panic!("unexpected exec observation: {other:?}"),
            }
        }
        panic!("the exec did not exit");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_owned_exec_that_exits_while_it_is_observed_keeps_its_exit_code() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let go = root.path().join("go");
        runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec![
                    "sh".into(),
                    "-c".into(),
                    "while [ ! -e go ]; do sleep 0.01; done; exit 3".into(),
                ]),
                root.path(),
                &std::env::vars().collect(),
            )
            .unwrap();
        // The child is still running when observe takes its status, and has exited, unreaped,
        // when observe reads its process.
        BEFORE_IDENTITY.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |pid| {
                fs::write(&go, "").unwrap();
                for _ in 0..3000 {
                    if process_identity(pid).unwrap().0 == 'Z' {
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                panic!("the exec never exited");
            }))
        });
        let observed = runtime.observe("work").unwrap();
        BEFORE_IDENTITY.with(|hook| *hook.borrow_mut() = None);
        let Some(ExecObservation::Exited(generation)) = observed else {
            panic!("unexpected exec observation: {observed:?}");
        };
        assert_eq!(generation.exit_code, Some(3));
        assert_eq!(wait_for_exit(&runtime, "work").exit_code, Some(3));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_exit_reaped_by_a_concurrent_observer_keeps_its_exit_code() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let go = root.path().join("go");
        runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec![
                    "sh".into(),
                    "-c".into(),
                    "while [ ! -e go ]; do sleep 0.01; done; exit 3".into(),
                ]),
                root.path(),
                &std::env::vars().collect(),
            )
            .unwrap();
        // The daemon's gate poll and its reconcile pass observe one exec at once. This observer
        // read the record and found its child still running; another reaps it before this one
        // reads its process, which is then gone.
        let other = runtime.clone();
        BEFORE_IDENTITY.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |pid| {
                fs::write(&go, "").unwrap();
                for _ in 0..3000 {
                    if process_identity(pid).unwrap().0 == 'Z' {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                let other = other.clone();
                let reaped = thread::spawn(move || other.observe("work").unwrap())
                    .join()
                    .unwrap();
                assert!(
                    matches!(&reaped, Some(ExecObservation::Exited(generation)) if generation.exit_code == Some(3)),
                    "{reaped:?}"
                );
            }))
        });
        let observed = runtime.observe("work").unwrap();
        BEFORE_IDENTITY.with(|hook| *hook.borrow_mut() = None);
        let Some(ExecObservation::Exited(generation)) = observed else {
            panic!("unexpected exec observation: {observed:?}");
        };
        assert_eq!(generation.exit_code, Some(3));
        assert_eq!(wait_for_exit(&runtime, "work").exit_code, Some(3));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_observer_between_a_reap_and_its_record_takes_the_exit_code() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let launched = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec!["sh".into(), "-c".into(), "exit 3".into()]),
                root.path(),
                &std::env::vars().collect(),
            )
            .unwrap();
        for _ in 0..3000 {
            if process_identity(launched.pid).unwrap().0 == 'Z' {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // This observer has reaped the child and not yet written its record when another
        // observes it.
        let other = runtime.clone();
        BEFORE_EXIT_WRITE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_pid| {
                let other = other.clone();
                let between = thread::spawn(move || other.observe("work").unwrap())
                    .join()
                    .unwrap();
                assert!(
                    matches!(&between, Some(ExecObservation::Exited(generation)) if generation.exit_code == Some(3)),
                    "{between:?}"
                );
            }))
        });
        let observed = runtime.observe("work").unwrap();
        BEFORE_EXIT_WRITE.with(|hook| *hook.borrow_mut() = None);
        let Some(ExecObservation::Exited(generation)) = observed else {
            panic!("unexpected exec observation: {observed:?}");
        };
        assert_eq!(generation.exit_code, Some(3));
        assert_eq!(wait_for_exit(&runtime, "work").exit_code, Some(3));
        assert!(runtime.owned_children.lock().unwrap().is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reaps_an_owned_child_before_committing_its_terminal_record() {
        use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
        use std::os::unix::ffi::OsStrExt as _;

        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let release_path = root.path().join("release");
        let release_name = std::ffi::CString::new(release_path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(release_name.as_ptr(), 0o600) }, 0);
        let generation = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec![
                    "sh".into(),
                    "-c".into(),
                    "read release < \"$1\"; exit 7".into(),
                    "sh".into(),
                    release_path.to_str().unwrap().into(),
                ]),
                root.path(),
                &std::env::vars().collect(),
            )
            .unwrap();
        let mut release = fs::OpenOptions::new()
            .write(true)
            .open(release_path)
            .unwrap();
        // Hold the child alive through observe's first status probe, then make it exit
        // before the process-identity probe. The FIFO and pidfd make the race deterministic.
        assert!(
            runtime
                .owned_children
                .lock()
                .unwrap()
                .get_mut(&generation.pid)
                .map(|child| match child {
                    OwnedChild::Running(child) => child.try_wait().unwrap().is_none(),
                    OwnedChild::Exited(_) => false,
                })
                .unwrap()
        );
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, generation.pid, 0) };
        assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
        let exited = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        writeln!(release, "exit").unwrap();
        drop(release);
        let mut event = libc::pollfd {
            fd: exited.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut event, 1, -1) }, 1);
        assert_ne!(event.revents & libc::POLLIN, 0);
        assert_eq!(process_identity(generation.pid).unwrap().0, 'Z');

        let observed = runtime.observe_exited_group("work", generation).unwrap();
        let ExecObservation::Exited(ended) = observed else {
            panic!("unexpected exec observation: {observed:?}");
        };
        assert_eq!(ended.exit_code, Some(7));
        assert_eq!(ended.exit_signal, None);
        assert!(ended.group_exited);
        assert!(runtime.owned_children.lock().unwrap().is_empty());
        let reopened = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        assert_eq!(
            reopened.observe("work").unwrap(),
            Some(ExecObservation::Exited(ended))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_owned_leader_exit_keeps_its_status_while_a_descendant_is_running() {
        use std::io::BufRead as _;
        use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
        use std::os::unix::ffi::OsStrExt as _;

        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        let descendant_release = root.path().join("descendant-release");
        let descendant_pid = root.path().join("descendant-pid");
        let leader_release = root.path().join("leader-release");
        for path in [&descendant_release, &descendant_pid, &leader_release] {
            let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        }
        let generation = runtime
                .spawn(
                    "work",
                    &crate::Launch::Argv(vec![
                        "sh".into(),
                        "-c".into(),
                        "sh -c 'read release < \"$1\"' sh \"$1\" & printf '%s\\n' \"$!\" > \"$2\"; read release < \"$3\"; exit 7".into(),
                        "sh".into(),
                        descendant_release.to_str().unwrap().into(),
                        descendant_pid.to_str().unwrap().into(),
                        leader_release.to_str().unwrap().into(),
                    ]),
                    root.path(),
                    &std::env::vars().collect(),
                )
                .unwrap();
        let mut pid = String::new();
        std::io::BufReader::new(fs::File::open(descendant_pid).unwrap())
            .read_line(&mut pid)
            .unwrap();
        let descendant = pid.trim().parse::<u32>().unwrap();
        let pidfd = |pid: u32| {
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
            assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
            unsafe { OwnedFd::from_raw_fd(fd as i32) }
        };
        let wait = |fd: &OwnedFd| {
            let mut event = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            assert_eq!(unsafe { libc::poll(&mut event, 1, -1) }, 1);
            assert_ne!(event.revents & libc::POLLIN, 0);
        };
        let leader_exited = pidfd(generation.pid);
        let descendant_exited = pidfd(descendant);
        // As in the no-descendant regression, the child exits between the first status
        // probe and the process-identity probe. The descendant stays blocked on its FIFO.
        assert!(
            runtime
                .owned_children
                .lock()
                .unwrap()
                .get_mut(&generation.pid)
                .map(|child| match child {
                    OwnedChild::Running(child) => child.try_wait().unwrap().is_none(),
                    OwnedChild::Exited(_) => false,
                })
                .unwrap()
        );
        let mut release = fs::OpenOptions::new()
            .write(true)
            .open(leader_release)
            .unwrap();
        writeln!(release, "exit").unwrap();
        drop(release);
        wait(&leader_exited);
        assert_eq!(process_identity(generation.pid).unwrap().0, 'Z');

        let observed = runtime.observe_exited_group("work", generation).unwrap();
        let ExecObservation::Running(running) = observed else {
            panic!("the live descendant must keep its group running: {observed:?}");
        };
        assert_eq!(running.exit_code, Some(7));
        assert!(!running.group_exited);
        assert!(runtime.owned_children.lock().unwrap().is_empty());
        assert_eq!(
            runtime.observe("work").unwrap(),
            Some(ExecObservation::Running(running))
        );

        let mut release = fs::OpenOptions::new()
            .write(true)
            .open(descendant_release)
            .unwrap();
        writeln!(release, "exit").unwrap();
        drop(release);
        wait(&descendant_exited);
        let Some(ExecObservation::Exited(ended)) = runtime.observe("work").unwrap() else {
            panic!("the group must stop running once its descendant exits");
        };
        assert_eq!(ended.exit_code, Some(7));
        assert!(ended.group_exited);
        let reopened = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        assert_eq!(
            reopened.observe("work").unwrap(),
            Some(ExecObservation::Exited(ended))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_owned_exec_with_a_zombie_leader_thread_is_running() {
        assert_zombie_leader_thread_is_running(false);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_adopted_exec_with_a_zombie_leader_thread_is_running() {
        assert_zombie_leader_thread_is_running(true);
    }

    #[cfg(target_os = "linux")]
    fn assert_zombie_leader_thread_is_running(adopted: bool) {
        use std::io::BufRead as _;
        use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
        use std::os::unix::ffi::OsStrExt as _;

        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let ready_path = root.path().join("ready");
        let release_path = root.path().join("release");
        for path in [&ready_path, &release_path] {
            let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        }
        let ready = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&ready_path)
            .unwrap();
        // pthread_exit terminates only the main thread. The worker reports readiness
        // after the kernel exposes that thread as Z, then lives until explicitly released.
        let fixture = r#"
import ctypes, os, sys, threading
pid = os.getpid()
def worker():
    while open(f'/proc/{pid}/stat').read().rsplit(') ', 1)[1].split()[0] != 'Z':
        os.sched_yield()
    with open(sys.argv[1], 'w') as ready:
        ready.write('ready\n')
    with open(sys.argv[2]) as release:
        release.readline()
    os._exit(7)
threading.Thread(target=worker).start()
libc = ctypes.CDLL(None)
libc.pthread_exit.argtypes = [ctypes.c_void_p]
libc.pthread_exit.restype = None
libc.pthread_exit(None)
"#;
        let generation = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec![
                    "python3".into(),
                    "-c".into(),
                    fixture.into(),
                    ready_path.to_str().unwrap().into(),
                    release_path.to_str().unwrap().into(),
                ]),
                root.path(),
                &std::env::vars().collect(),
            )
            .unwrap();
        // A pidfd confines cleanup to this process even after its PID is reaped/reused.
        // It must also bypass observe, which blocks in the broken implementation.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, generation.pid, 0) };
        assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
        struct Cleanup(ExecRuntime, u32, OwnedFd);
        impl Cleanup {
            fn kill(&self) -> std::io::Result<()> {
                let result = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        self.2.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                };
                if result == 0 {
                    return Ok(());
                }
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.kill();
                if let Some(OwnedChild::Running(mut child)) =
                    self.0.owned_children.lock().unwrap().remove(&self.1)
                {
                    let _ = child.wait();
                }
            }
        }
        let cleanup = Cleanup(runtime.clone(), generation.pid, unsafe {
            OwnedFd::from_raw_fd(fd as i32)
        });
        let mut event = libc::pollfd {
            fd: ready.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(
            unsafe { libc::poll(&mut event, 1, 10_000) },
            1,
            "worker never became ready"
        );
        let mut line = String::new();
        std::io::BufReader::new(ready).read_line(&mut line).unwrap();
        assert_eq!(line, "ready\n");
        assert_eq!(process_identity(generation.pid).unwrap().0, 'Z');
        assert!(
            runtime
                .owned_children
                .lock()
                .unwrap()
                .get_mut(&generation.pid)
                .map(|child| match child {
                    OwnedChild::Running(child) => child.try_wait().unwrap().is_none(),
                    OwnedChild::Exited(_) => false,
                })
                .unwrap(),
            "the process must not yet be waitable"
        );
        let observing = if adopted {
            ExecRuntime::new(root.path().join("exec"), root.path().join("logs"))
        } else {
            runtime.clone()
        };
        let (send, receive) = std::sync::mpsc::channel();
        let observer = thread::spawn(move || {
            send.send(observing.observe("work")).unwrap();
        });
        let result = receive.recv_timeout(Duration::from_secs(10));
        if result.is_err() {
            cleanup.kill().unwrap();
            observer.join().unwrap();
            panic!("observe blocked on a zombie leader thread with a live worker");
        }
        observer.join().unwrap();
        assert_eq!(
            result.unwrap().unwrap(),
            Some(ExecObservation::Running(generation.clone()))
        );
        assert_eq!(
            runtime.observe("work").unwrap(),
            Some(ExecObservation::Running(generation))
        );
        let mut release = fs::OpenOptions::new()
            .write(true)
            .open(release_path)
            .unwrap();
        writeln!(release, "exit").unwrap();
        drop(release);
        let ended = wait_for_exit(&runtime, "work");
        assert_eq!(ended.exit_code, Some(7));
        assert!(ended.group_exited);
    }

    #[test]
    fn retains_the_current_and_previous_log_generations() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let environment = std::env::vars().collect::<BTreeMap<_, _>>();
        let cwd = root.path();
        let first = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec![
                    "sh".into(),
                    "-c".into(),
                    "printf first; exit 3".into(),
                ]),
                cwd,
                &environment,
            )
            .unwrap();
        let first_exit = wait_for_exit(&runtime, "work");
        assert_eq!(first_exit.exit_code, Some(3));

        let second = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec!["sh".into(), "-c".into(), "printf second".into()]),
                cwd,
                &environment,
            )
            .unwrap();
        let second_exit = wait_for_exit(&runtime, "work");

        assert_ne!(first.generation_id, second.generation_id);
        assert_eq!(second_exit.exit_code, Some(0));
        assert_eq!(
            runtime.previous_generation("work").unwrap().unwrap(),
            first_exit
        );
        assert_eq!(
            runtime.read_log_bytes("work", true).unwrap().unwrap(),
            b"first"
        );
        assert_eq!(
            runtime.read_log_bytes("work", false).unwrap().unwrap(),
            b"second"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_exited_wrapper_keeps_its_resistant_children_owned_after_adoption() {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let generation = runtime.spawn(
            "work",
            &crate::Launch::Shell("sh -c 'trap \"\" TERM; echo $$ > child.pid; while :; do sleep 60; done' & wait".into()),
            root.path(), &std::env::vars().collect(),
        ).unwrap();
        struct Cleanup(i32);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe {
                    libc::kill(-self.0, libc::SIGKILL);
                }
            }
        }
        let _cleanup = Cleanup(generation.pid as i32);
        // The shell creates child.pid before it writes the pid: wait for the pid, not the file.
        let mut child = None;
        for _ in 0..200 {
            child = fs::read_to_string(root.path().join("child.pid"))
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            if child.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let child = child.expect("the child never wrote its pid");
        runtime
            .stop_if("work", Some(&generation.generation_id))
            .unwrap();
        // Reap the wrapper and retain its exit status while the child still runs.
        for _ in 0..200 {
            if let Some(ExecObservation::Running(observed)) = runtime.observe("work").unwrap()
                && observed.exit_signal.is_some()
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let adopted = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        assert!(matches!(
            adopted.observe("work").unwrap(),
            Some(ExecObservation::Running(_))
        ));
        assert!(adopted.kill_if("work", Some("another-generation")).is_err());
        adopted
            .kill_if("work", Some(&generation.generation_id))
            .unwrap();
        let exited = wait_for_exit(&adopted, "work");
        assert_eq!(exited.exit_signal, Some(libc::SIGTERM));
        assert!(process_identity(child).map_or(true, |(state, _)| state == 'Z'));
    }

    /// A launch that starts `orphan.pid`'s writer in a session of its own whose parent exits at
    /// once, waits for the pid, then runs `then`. The writer has left the exec's process group.
    #[cfg(target_os = "linux")]
    fn orphaning_launch(then: &str) -> crate::Launch {
        crate::Launch::Shell(format!(
            "(setsid sh -c 'echo $$ > orphan.pid; exec sleep 600' &); \
             while [ ! -s orphan.pid ]; do sleep 0.02; done; {then}"
        ))
    }

    #[cfg(target_os = "linux")]
    fn read_orphan(root: &Path) -> u32 {
        for _ in 0..500 {
            if let Some(pid) = fs::read_to_string(root.join("orphan.pid"))
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the exec never started its background process");
    }

    #[cfg(target_os = "linux")]
    fn assert_gone_soon(pid: u32) {
        for _ in 0..250 {
            if process_identity(pid).map_or(true, |(state, _)| state == 'Z') {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        panic!("background process {pid} outlived its exec");
    }

    #[cfg(target_os = "linux")]
    fn wait_for_file(root: &Path, name: &str) -> String {
        for _ in 0..3000 {
            if let Ok(text) = fs::read_to_string(root.join(name))
                && text.ends_with('\n')
            {
                return text.trim().to_owned();
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the exec never wrote {name}");
    }

    #[cfg(target_os = "linux")]
    fn cgroup_of(pid: u32) -> PathBuf {
        let text = fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap();
        let path = text.lines().find_map(|line| line.strip_prefix("0::")).unwrap();
        Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'))
    }

    /// The cgroup of the scope `generation` runs in, once `systemd-run` has moved it there.
    #[cfg(target_os = "linux")]
    fn scope_cgroup(generation: &ExecGeneration) -> PathBuf {
        let unit = generation.scope_unit.as_deref().unwrap();
        for _ in 0..500 {
            let cgroup = cgroup_of(generation.pid);
            if cgroup.ends_with(unit) {
                return cgroup;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the exec never entered its scope {unit}");
    }

    #[cfg(target_os = "linux")]
    fn cpu_usage(cgroup: &Path) -> Duration {
        let stat = fs::read_to_string(cgroup.join("cpu.stat")).unwrap();
        let usec = stat
            .lines()
            .find_map(|line| line.strip_prefix("usage_usec "))
            .unwrap();
        Duration::from_micros(usec.trim().parse().unwrap())
    }

    /// A seat id no other test run on this host uses, so concurrent runs never share a lane.
    #[cfg(target_os = "linux")]
    fn lane_test_id(name: &str) -> String {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        format!("lane-test.{name}.{}.{nanos}", std::process::id())
    }

    #[cfg(target_os = "linux")]
    fn systemctl_show(unit: &str, property: &str) -> String {
        let output = std::process::Command::new("systemctl")
            .args(["--user", "show", &format!("--property={property}"), "--value", unit])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// Stops a test's exec and ends its lane when dropped, so a failing test leaves nothing behind.
    #[cfg(target_os = "linux")]
    struct LaneOnDrop(ExecRuntime, String);

    #[cfg(target_os = "linux")]
    impl Drop for LaneOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill(&self.1);
            let slice = crate::lane::seat_slice(&self.1);
            for verb in ["stop", "revert"] {
                let _ = std::process::Command::new("systemctl").args(["--user", verb, &slice]).output();
            }
        }
    }

    /// Starts `launch` as seat `id` in a lane of one core and 256 MiB; returns its generation and
    /// the cgroup of the scope it runs in.
    #[cfg(target_os = "linux")]
    fn spawn_laned(runtime: &ExecRuntime, id: &str, root: &Path, launch: &str) -> (ExecGeneration, PathBuf) {
        let lane = crate::lane::Capacity { cores: 1, memory: 256 << 20 };
        let mut environment = scope_environment();
        environment.insert(crate::lane::CAPACITY_ENV.into(), lane.to_string());
        let _switch = crate::lane::TEST_SWITCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::lane::set_enforced(true);
        let generation = runtime.spawn(id, &crate::Launch::Shell(launch.into()), root, &environment);
        crate::lane::set_enforced(false);
        let generation = generation.unwrap();
        let cgroup = scope_cgroup(&generation);
        (generation, cgroup)
    }

    #[cfg(target_os = "linux")]
    fn wait_until(what: &str, done: impl Fn() -> bool) {
        for _ in 0..500 {
            if done() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("{what}");
    }

    /// A seat lane (docs/st3/seat-lanes.md) is the seat's own slice, capped, and each launch of its
    /// task is a scope inside it: the task and every child share its cores, a child that left its
    /// session included; the task never sees the lane's launch key and its builds are told to stay
    /// in place. A job the seat starts as its own unit in the slice outlives a restart of the task,
    /// and ending the lane (a seat stopped for good) ends the job and the slice's limits. A host
    /// with lanes off starts the same declaration without one.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_lane_caps_the_seat_keeps_its_jobs_across_a_restart_and_ends_for_good() {
        if crate::isolation_mode() != crate::Isolation::Scope || !crate::isolate::systemd_254() {
            eprintln!("skipped: no systemd 254 user scopes");
            return;
        }
        // Off (the default): the declaration is ignored and the task starts as it does today.
        let off = tempfile::tempdir().unwrap();
        let unlaned = ExecRuntime::new(off.path().join("exec"), off.path().join("logs"));
        let _kill_unlaned = KillOnDrop(unlaned.clone());
        let mut environment = scope_environment();
        environment.insert(
            crate::lane::CAPACITY_ENV.into(),
            crate::lane::Capacity { cores: 1, memory: 256 << 20 }.to_string(),
        );
        let switch = crate::lane::TEST_SWITCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = unlaned
            .spawn("work", &crate::Launch::Shell("exec sleep 600".into()), off.path(), &environment)
            .unwrap();
        drop(switch);
        let cgroup = scope_cgroup(&generation);
        assert!(cgroup.parent().unwrap().ends_with("app.slice"), "{}", cgroup.display());
        assert_eq!(fs::read_to_string(cgroup.join("memory.max")).unwrap().trim(), "max");
        unlaned.stop_if("work", Some(&generation.generation_id)).unwrap();
        wait_for_exit(&unlaned, "work");
        assert!(!crate::lane::prepared_for_test("work"), "a seat without a lane holds no slot");

        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let id = lane_test_id("restart");
        let _lane = LaneOnDrop(runtime.clone(), id.clone());
        let job = format!("{}-job.service", crate::isolate::sanitize(&id));
        let (generation, cgroup) = spawn_laned(
            &runtime,
            &id,
            root.path(),
            &format!(
                "echo \"${{BUILDS_SLICE_ACTIVE:-unset}} ${{FRACTAL_SEAT_CAPACITY:-unset}} $FRACTAL_LANE_SLICE\" > lane.env; \
                 (setsid sh -c 'echo $$ > orphan.pid; exec sleep 600' &); \
                 systemd-run --user --quiet --unit={job} --slice=\"$FRACTAL_LANE_SLICE\" sleep 600; \
                 echo spin > spin.start; \
                 for n in 1 2; do timeout 1 sh -c 'while :; do :; done' & done; wait; \
                 echo done > spin.done; exec sleep 600"
            ),
        );
        let slice_name = crate::lane::seat_slice(&id);
        let slice = cgroup.parent().unwrap().to_path_buf();
        assert!(slice.ends_with(format!("app.slice/app-lanes.slice/{slice_name}")), "{}", slice.display());
        assert_eq!(fs::read_to_string(slice.join("cpu.max")).unwrap().trim(), "100000 100000");
        assert_eq!(fs::read_to_string(slice.join("memory.max")).unwrap().trim(), (256u64 << 20).to_string());
        assert_eq!(fs::read_to_string(slice.join("memory.swap.max")).unwrap().trim(), (64u64 << 20).to_string());
        assert_eq!(systemctl_show(generation.scope_unit.as_deref().unwrap(), "OOMPolicy"), "continue");

        assert_eq!(wait_for_file(root.path(), "lane.env"), format!("1 unset {slice_name}"));
        let orphan = read_orphan(root.path());
        assert_eq!(cgroup_of(orphan), cgroup, "a child that left its session is still in the lane");

        // Two spinning children share one core.
        wait_for_file(root.path(), "spin.start");
        let (started, used) = (std::time::Instant::now(), cpu_usage(&slice));
        wait_for_file(root.path(), "spin.done");
        let (elapsed, used) = (started.elapsed(), cpu_usage(&slice) - used);
        assert!(
            used.as_secs_f64() <= elapsed.as_secs_f64() * 1.25,
            "two spinners used {used:?} of CPU in {elapsed:?} under a one-core lane"
        );
        assert_eq!(systemctl_show(&job, "ActiveState"), "active");

        // A restart: the task's scope ends with what it started, the lane and its job stay.
        runtime.stop_if(&id, Some(&generation.generation_id)).unwrap();
        wait_for_exit(&runtime, &id);
        assert_gone_soon(orphan);
        wait_until("the restarted task's scope outlived its stop", || !cgroup.exists());
        let (restarted, again) = spawn_laned(&runtime, &id, root.path(), "exec sleep 600");
        assert_eq!(again.parent().unwrap(), slice, "a restart starts in the seat's own lane");
        assert_eq!(systemctl_show(&job, "ActiveState"), "active", "the job outlived the restart");

        // An end asked before a launch does not end the lane that launch runs in: a seat stopped
        // for good and started again at once keeps its new lane.
        let stale_end = crate::lane::request_end_for_test(&id);
        runtime.stop_if(&id, Some(&restarted.generation_id)).unwrap();
        wait_for_exit(&runtime, &id);
        let (relaunched, _) = spawn_laned(&runtime, &id, root.path(), "exec sleep 600");
        stale_end().unwrap();
        assert!(matches!(runtime.observe(&id).unwrap(), Some(ExecObservation::Running(_))));
        assert_eq!(systemctl_show(&job, "ActiveState"), "active", "a stale end left the lane's job");

        // Stopped for good after a daemon restart, which forgets what the old process prepared:
        // the lane still ends, its job with it, and its limits are gone.
        runtime.stop_if(&id, Some(&relaunched.generation_id)).unwrap();
        wait_for_exit(&runtime, &id);
        crate::lane::forget_for_test(&id);
        crate::lane::end(&id);
        wait_until("the lane's job outlived the lane", || systemctl_show(&job, "ActiveState") != "active");
        wait_until("the lane outlived its end", || !slice.exists());
        wait_until("the lane's limits outlived its end", || {
            systemctl_show(&slice_name, "MemoryMax") == "infinity"
        });
        wait_until("the lane's slot outlived its end", || !crate::lane::prepared_for_test(&id));
    }

    /// A child that asks for more memory than its lane holds is killed inside the lane, and only
    /// that child ends: the task that started it keeps running.
    ///
    /// Ignored by default: it makes an OOM kill on purpose. Run it where the host guard subtracts
    /// `app-lanes.slice` (docs/st3/seat-lanes.md), after telling whoever runs the guard.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "kills a process on purpose; run where the host guard subtracts app-lanes.slice"]
    fn a_child_over_its_lane_memory_is_killed_and_the_task_keeps_running() {
        assert!(crate::isolate::systemd_254(), "needs systemd 254 user scopes");
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let id = lane_test_id("oom");
        let _lane = LaneOnDrop(runtime.clone(), id.clone());
        // 384 MiB against a lane of 256 MiB and 64 MiB of swap.
        let (_, cgroup) = spawn_laned(
            &runtime,
            &id,
            root.path(),
            "setsid sh -c 'head -c 402653184 /dev/zero | tail -c 402653184 > /dev/null; echo $? > hog.status'; \
             echo alive > after.hog; exec sleep 600",
        );
        assert_eq!(wait_for_file(root.path(), "hog.status"), "137");
        assert_eq!(wait_for_file(root.path(), "after.hog"), "alive");
        let events = fs::read_to_string(cgroup.parent().unwrap().join("memory.events")).unwrap();
        let kills: u64 = events
            .lines()
            .find_map(|line| line.strip_prefix("oom_kill "))
            .unwrap()
            .parse()
            .unwrap();
        assert!(kills >= 1, "{events}");
        assert!(matches!(runtime.observe(&id).unwrap(), Some(ExecObservation::Running(_))));
    }

    /// Kills the test's exec when dropped, so a failing test leaves nothing behind.
    #[cfg(target_os = "linux")]
    struct KillOnDrop(ExecRuntime);

    #[cfg(target_os = "linux")]
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill("work");
        }
    }

    /// The environment a member needs to start in a systemd user scope, and no more.
    #[cfg(target_os = "linux")]
    fn scope_environment() -> BTreeMap<String, String> {
        ["PATH", "HOME", "XDG_RUNTIME_DIR"]
            .into_iter()
            .filter_map(|key| Some((key.to_owned(), std::env::var(key).ok()?)))
            .collect()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stopping_an_exec_ends_what_it_left_running() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("skipped: no systemd user scopes");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        let generation = runtime
            .spawn(
                "work",
                &orphaning_launch("exec sleep 600"),
                root.path(),
                &scope_environment(),
            )
            .unwrap();
        let orphan = read_orphan(root.path());

        runtime
            .stop_if("work", Some(&generation.generation_id))
            .unwrap();

        wait_for_exit(&runtime, "work");
        assert_gone_soon(orphan);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_ended_execs_leftovers_end_and_its_replacement_keeps_running() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("skipped: no systemd user scopes");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        let ended = runtime
            .spawn(
                "work",
                &orphaning_launch("exit 0"),
                root.path(),
                &scope_environment(),
            )
            .unwrap();
        let orphan = read_orphan(root.path());
        wait_for_exit(&runtime, "work");

        let replacement = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec!["sleep".into(), "600".into()]),
                root.path(),
                &scope_environment(),
            )
            .unwrap();

        assert_gone_soon(orphan);
        // Each launch has a scope of its own. Ending the ended one again, as a reconcile pass
        // that read the record before the replacement started does, leaves the replacement.
        let ended_unit = ended.scope_unit.unwrap();
        assert_ne!(Some(&ended_unit), replacement.scope_unit.as_ref());
        assert!(crate::end_scope(&ended_unit, crate::SCOPE_GRACE).unwrap());
        runtime.end_leftovers("work").unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(matches!(
            runtime.observe("work").unwrap(),
            Some(ExecObservation::Running(_))
        ));
        runtime
            .kill_if("work", Some(&replacement.generation_id))
            .unwrap();
        wait_for_exit(&runtime, "work");
    }

    /// A oneshot unit that is starting until something is written to `release`, so a scope ordered
    /// after it waits, loaded and empty. Stopped when dropped.
    #[cfg(target_os = "linux")]
    struct StartGate(String);

    #[cfg(target_os = "linux")]
    impl StartGate {
        fn new(release: &Path) -> Self {
            use std::os::unix::ffi::OsStrExt as _;
            let name = std::ffi::CString::new(release.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
            let unit = format!("st3-test-start-gate-{}-{nanos}.service", std::process::id());
            let status = std::process::Command::new("systemd-run")
                .args(["--user", "--quiet", "--no-block", "--collect", "--property=Type=oneshot"])
                .arg(format!("--unit={unit}"))
                .args(["sh", "-c", "read line < \"$0\""])
                .arg(release)
                .status()
                .unwrap();
            assert!(status.success());
            Self(unit)
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for StartGate {
        fn drop(&mut self) {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "stop", &self.0])
                .status();
        }
    }

    /// The spawn's own PATH finds a systemd-run that orders the exec's scope after `gate`, so the
    /// scope starts only when the test releases it.
    #[cfg(target_os = "linux")]
    fn gated_environment(root: &Path, gate: &StartGate) -> BTreeMap<String, String> {
        let mut environment = scope_environment();
        let systemd_run = crate::resolve_executable("systemd-run", &environment).unwrap();
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        crate::test_executable::fake_executable(
            &bin,
            "systemd-run",
            &format!("#!/bin/sh\nexec '{}' --property=After={} \"$@\"\n", systemd_run.display(), gate.0),
        );
        let path = format!("{}:{}", bin.display(), environment["PATH"]);
        environment.insert("PATH".into(), path);
        environment
    }

    /// Spawns `sleep 600` as `work` on another thread, behind a scope start that waits for `gate`,
    /// and returns once its record is written and its scope is loaded.
    #[cfg(target_os = "linux")]
    fn spawn_behind(
        runtime: &ExecRuntime,
        root: &Path,
        gate: &StartGate,
    ) -> (thread::JoinHandle<Result<ExecGeneration>>, ExecGeneration) {
        let environment = gated_environment(root, gate);
        let spawning = {
            let runtime = runtime.clone();
            let root = root.to_owned();
            thread::spawn(move || {
                runtime.spawn(
                    "work",
                    &crate::Launch::Argv(vec!["sleep".into(), "600".into()]),
                    &root,
                    &environment,
                )
            })
        };
        let record = || runtime.read_generation_path(&runtime.record_path("work")).ok().flatten();
        wait_until("the exec was never recorded", || record().is_some());
        let recorded = record().unwrap();
        let unit = recorded.scope_unit.clone().unwrap();
        wait_until("the exec's scope was never loaded", || {
            systemctl_show(&unit, "LoadState") == "loaded"
        });
        (spawning, recorded)
    }

    /// The spawn returns only once `systemd-run` has joined the exec's scope. Before that it is in
    /// the spawning service's cgroup, and a kill of that cgroup would end it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_spawn_returns_once_the_exec_has_joined_its_scope() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("skipped: no systemd user scopes");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        let release = root.path().join("release");
        let gate = StartGate::new(&release);
        let (spawning, recorded) = spawn_behind(&runtime, root.path(), &gate);
        let unit = recorded.scope_unit.clone().unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(!spawning.is_finished(), "the spawn returned before its exec joined {unit}");

        fs::write(&release, "\n").unwrap();
        let generation = spawning.join().unwrap().unwrap();
        assert_eq!(generation, recorded);
        assert!(crate::isolate::joined(&unit, generation.pid));
    }

    /// A kill sent before the exec's scope has started ends the exec. `systemd-run` moves into its
    /// scope when the user manager starts it and only then runs the task, and until then the
    /// scope is loaded and empty. A kill can come while the spawn waits, or from a daemon that
    /// restarted while it did.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_kill_before_the_exec_has_joined_its_scope_ends_it() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("skipped: no systemd user scopes");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        let release = root.path().join("release");
        let gate = StartGate::new(&release);
        let (spawning, generation) = spawn_behind(&runtime, root.path(), &gate);
        let unit = generation.scope_unit.clone().unwrap();
        assert!(!crate::isolate::joined(&unit, generation.pid));

        runtime
            .kill_if("work", Some(&generation.generation_id))
            .unwrap();
        fs::write(&release, "\n").unwrap();

        // The spawn sees its exec end rather than join.
        spawning.join().unwrap().unwrap();
        wait_for_exit(&runtime, "work");
    }

    /// A user manager that never starts the scope does not hold the spawn: the exec, still
    /// `systemd-run` in the spawning service's cgroup, ends, and the spawn fails.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_exec_that_never_joins_its_scope_ends_and_fails_its_spawn() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("skipped: no systemd user scopes");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"))
            .with_scope_join_limit(Duration::from_millis(300));
        let _kill = KillOnDrop(runtime.clone());
        let release = root.path().join("release");
        let gate = StartGate::new(&release);
        let environment = gated_environment(root.path(), &gate);
        let error = runtime
            .spawn(
                "work",
                &crate::Launch::Argv(vec!["sleep".into(), "600".into()]),
                root.path(),
                &environment,
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("did not join its scope"), "{error:#}");
        // The leader ended as `systemd-run`, so the task never ran.
        let exited = wait_for_exit(&runtime, "work");
        assert_eq!(exited.exit_signal, Some(libc::SIGKILL));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_ended_exec_ends_what_it_left_running() {
        if crate::isolation_mode() != crate::Isolation::Scope {
            eprintln!("skipped: no systemd user scopes");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let _kill = KillOnDrop(runtime.clone());
        runtime
            .spawn(
                "work",
                &orphaning_launch("exit 0"),
                root.path(),
                &scope_environment(),
            )
            .unwrap();
        let orphan = read_orphan(root.path());
        wait_for_exit(&runtime, "work");

        // The reconciler asks this on each pass in which a stopped runtime is not running.
        runtime.end_leftovers("work").unwrap();

        assert_gone_soon(orphan);
    }

    /// What an exec's grandchild reports about its own priority: `sh -c nice` run by `sh`, so the
    /// value is the one a build the seat starts would inherit.
    #[cfg(target_os = "linux")]
    fn inherited_nice(id: &str, priority: Option<&str>) -> String {
        let root = tempfile::tempdir().unwrap();
        let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
        let mut environment = scope_environment();
        if let Some(priority) = priority {
            environment.insert(crate::PRIORITY_ENV.into(), priority.into());
        }
        runtime
            .spawn(
                id,
                &crate::Launch::Shell("sh -c nice".into()),
                root.path(),
                &environment,
            )
            .unwrap();
        wait_for_exit(&runtime, id);
        runtime
            .read_log(id)
            .unwrap()
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_exec_starts_below_the_daemon_unless_its_seat_asks_for_normal() {
        let base = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
        assert_eq!(
            inherited_nice("default", None),
            (base + 10).min(19).to_string()
        );
        assert_eq!(
            inherited_nice("explicit", Some("background")),
            (base + 10).min(19).to_string()
        );
        assert_eq!(inherited_nice("normal", Some("normal")), base.to_string());
    }
}
