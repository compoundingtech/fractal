//! Read-only host envelope observation; request paths never wait for admission.
//! Startup admits until the blocking reconcile pass reads state. Cached observations
//! expire independently of periodic file reads, so stale pressure cannot hold work.
use super::*;
use fractal_runtime::seat_admission::{Config, Decision, local_host, read};
use fractal_runtime::seat_admission_status::Status;

#[derive(Default)]
pub struct Controller {
    config: Option<Config>,
    pub status: Arc<Status>,
    dirty: Arc<AtomicBool>,
    checked: Mutex<Option<u64>>,
    outside_observation: Mutex<Option<Option<fractal_runtime::host::HostMemoryObservation>>>,
    _watcher: Option<notify::RecommendedWatcher>,
    /// What admission by fit reads of this host; none where lanes are off.
    pub lanes: Option<LaneHost>,
    pub finite_policy: fractal_runtime::host::FinitePolicy,
    pub pressure: Mutex<Option<fractal_runtime::resource_pressure::Observer>>,
    pressure_dirty: AtomicBool,
    #[cfg(test)]
    pressure_samples: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<(
        tokio::time::Instant, fractal_runtime::resource_pressure::Decision,
    )>>,
}

/// The room a host gives lanes, and how to read unreclaimable host memory outside lanes
/// (docs/st3/seat-lanes.md, "Admission by fit").
#[derive(Clone, Copy)]
pub struct LaneHost {
    pub room: fractal_runtime::Capacity,
    pub outside: fn() -> Option<fractal_runtime::host::HostMemoryObservation>,
    /// The lane a burst raises a seat to: admission keeps room for one such raise from starts
    /// ([`BURST_LANE`]; none keeps none).
    pub burst_lane: Option<fractal_runtime::Capacity>,
}

/// The lane a run's burst raises its seat to: `build` in .st/lane-bursts.json, which
/// scripts/fractal-test asks for (docs/st3/seat-lanes.md, "Bursts").
pub const BURST_LANE: fractal_runtime::Capacity = fractal_runtime::Capacity { cores: 8, memory: 24 << 30 };

impl<R: RuntimeControl> Reconciler<R> {
    /// Scoped to the daemon's reconciler run, but independent of serialized passes and
    /// writer admission. Only decision transitions invalidate admission work.
    pub(super) async fn sample_resource_pressure(self: &Arc<Self>) {
        if !cfg!(target_os = "linux") {
            std::future::pending::<()>().await;
        }
        let period = self.seat_admission.finite_policy.psi_brake.sample_interval();
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut previous = None;
        loop {
            interval.tick().await;
            let this = self.clone();
            let decision = match tokio::task::spawn_blocking(move || {
                crate::task_cpu::blocking("resource.pressure", || this.seat_admission.sample_pressure())
            }).await {
                Ok(decision) => decision,
                Err(error) => {
                    tracing::error!(%error, "PSI sampler task failed");
                    continue;
                }
            };
            #[cfg(test)]
            if let (Ok(decision), Some(samples)) = (&decision, self.seat_admission.pressure_samples.get()) {
                let _ = samples.send((tokio::time::Instant::now(), *decision));
            }
            match decision {
                Ok(decision) if previous != Some(decision) => {
                    if previous.is_some() && decision.blocked {
                        tracing::warn!(reason = ?decision.reason, "host PSI brake blocked admission");
                    } else {
                        tracing::debug!(blocked = decision.blocked, reason = ?decision.reason, "host PSI brake decision");
                    }
                    previous = Some(decision);
                    // The pass owns incremental/store locks; the sampling clock never waits on them.
                    self.seat_admission.pressure_dirty.store(true, Ordering::Release);
                    // Sampling changes admission inputs, not the durable graph event stream.
                    self.notify.notify_one();
                }
                Ok(_) => {}
                Err(error) => tracing::error!(%error, "invalid native admission PSI policy"),
            }
        }
    }

    /// Configure observation without reading the state file on the async startup path.
    pub fn with_seat_admission(mut self, config: Option<Config>, status: Arc<Status>) -> Self {
        let dirty = Arc::new(AtomicBool::new(true));
        let watcher = config.as_ref().and_then(|config| {
            let target = config.state_file.clone();
            let directory = target.parent()?.to_path_buf();
            let changed = dirty.clone();
            let notify = self.notify.clone();
            let events = self.event_notify.clone();
            let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.as_ref().map_or(true, |event| FileWatch::relevant(event, &target, &directory, None)) {
                    changed.store(true, Ordering::Release);
                    signal_changed(&notify, &events);
                }
            }).ok()?;
            // Watch the directory: the controller atomically replaces the file's inode.
            watcher.watch(config.state_file.parent()?, notify::RecursiveMode::NonRecursive).ok()?;
            Some(watcher)
        });
        if config.is_some() && watcher.is_none() {
            tracing::warn!("seat admission file watcher unavailable; using periodic observation");
        }
        if config.is_some() {
            status.state().decision = Decision {
                mode: "fail-open".into(), reason: "seat admission initializing; admitting until first observation".into(),
                queue: false, observed_at: None,
            };
        }
        self.seat_admission = Controller {
            config, status, dirty, checked: Mutex::new(None), outside_observation: Mutex::new(None),
            _watcher: watcher, lanes: self.seat_admission.lanes,
            finite_policy: self.seat_admission.finite_policy,
            pressure: Mutex::new(None),
            pressure_dirty: AtomicBool::new(false),
            #[cfg(test)]
            pressure_samples: Default::default(),
        };
        self
    }

    pub(super) fn refresh_seat_admission(&self) {
        self.refresh_seat_admission_at(u64::try_from(now_ms() / 1000).unwrap_or(u64::MAX));
    }

    fn record_pressure_layout_diagnostic(&self, placement: &str) -> Result<()> {
        self.record_once(&format!("daemon/{}", self.host), "daemon.diagnostic", BTreeMap::from([
            ("severity".into(), Value::String("warning".into())),
            ("code".into(), Value::String("lane-psi-not-applicable".into())),
            ("reason".into(), Value::String(format!(
                "finite lane PSI is not applicable to cgroup placement {placement}; qualified root PSI and the memory floor remain active"
            ))),
        ]))
    }

    fn refresh_seat_admission_at(&self, now: u64) {
        let admission = &self.seat_admission;
        if admission.pressure_dirty.swap(false, Ordering::AcqRel) {
            self.incremental.touch("host-lane-pressure");
            // Copy under the pressure lock, then release it before touching the store.
            let placement = admission.pressure.lock().unwrap_or_else(PoisonError::into_inner)
                .as_ref().and_then(|observer| observer.lane_psi_not_applicable_placement())
                .map(str::to_owned);
            if let Some(placement) = placement
                && let Err(error) = self.record_pressure_layout_diagnostic(&placement)
            {
                tracing::warn!(%error, "could not publish lane PSI coverage diagnostic");
                admission.pressure_dirty.store(true, Ordering::Release);
            }
        }
        if let Some(host) = admission.lanes {
            let observation = (host.outside)();
            let mut previous = admission.outside_observation.lock().unwrap_or_else(PoisonError::into_inner);
            if *previous != Some(observation) {
                *previous = Some(observation);
                self.incremental.touch("host-lane-memory");
                self.signal_changed();
            }
            // Reuse the admission observation deadline, including hosts without an envelope file.
            // A deadline observes memory; only a measured fit can grant capacity.
            if admission.config.is_none() {
                self.arm_deadline_at("seat-admission", u128::from(now.saturating_add(30)) * 1000,
                    crate::reconcile_telemetry::WakeCause::Other);
            }
        }
        let Some(config) = &admission.config else { return };
        let mut checked = admission.checked.lock().unwrap_or_else(PoisonError::into_inner);
        let due = checked.is_none_or(|at| now.saturating_sub(at) >= 30 || now < at);
        let decision = if admission.dirty.swap(false, Ordering::AcqRel) || due {
            let decision = local_host().map_or_else(
                |error| Decision { mode: "fail-open".into(), reason: format!("local hostname unavailable: {error}"), queue: false, observed_at: None },
                |host| read(config, &host, now),
            );
            *checked = Some(now);
            Some(decision)
        } else {
            // Expiry is a cached-state transition, not a reason to open the file.
            admission.status.state().decision.observed_at
                .filter(|at| now.saturating_sub(*at) > 120)
                .map(|_| Decision {
                    mode: "fail-open".into(), reason: "admission state is stale".into(),
                    queue: false, observed_at: None,
                })
        };
        if let Some(decision) = decision {
            let mut state = admission.status.state();
            if state.decision != decision {
                if state.decision.mode != decision.mode || state.decision.reason != decision.reason {
                    tracing::info!(mode = decision.mode, reason = decision.reason, "seat admission decision");
                }
                state.decision = decision;
                self.incremental.touch("seat-admission");
                self.signal_changed();
            }
        }
        let state = admission.status.state();
        let expiry = state.decision.observed_at.map(|at| at.saturating_add(121));
        let next = checked.unwrap_or(now).saturating_add(30);
        let deadline = expiry.filter(|expiry| *expiry > now).map_or(next, |expiry| expiry.min(next));
        self.arm_deadline_at("seat-admission", u128::from(deadline) * 1000,
            crate::reconcile_telemetry::WakeCause::Other);
    }
}

impl Controller {
    /// Admission reads the last sampler result, never kernel files or the brake clock.
    pub fn pressure_decision(&self) -> fractal_runtime::resource_pressure::Decision {
        use fractal_runtime::resource_pressure::{Decision, ObservationCause, Reason};
        self.pressure.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
            .map_or(Decision { blocked: true,
                reason: Some(Reason::UnknownObservation { cause: ObservationCause::NotYetSampled }) },
                fractal_runtime::resource_pressure::Observer::decision)
    }

    pub(super) fn sample_pressure(&self) -> Result<fractal_runtime::resource_pressure::Decision> {
        // This guard covers only the Observer's kernel PSI read and pure Brake update.
        // Never acquire store, incremental or other reconciliation locks inside it.
        let mut pressure = self.pressure.lock().unwrap_or_else(PoisonError::into_inner);
        if pressure.is_none() {
            *pressure = Some(fractal_runtime::resource_pressure::Observer::new(self.finite_policy.psi_brake.clone())?);
        }
        pressure.as_mut().context("pressure observer initialization failed")?.observe(&self.finite_policy.psi_brake)
    }
}

#[cfg(test)]
mod tests;
