//! Read-only host envelope observation; request paths never wait for admission.
use super::*;
use fractal_runtime::seat_admission::{Config, Decision, local_host, read};
use fractal_runtime::seat_admission_status::Status;

#[derive(Default)]
pub struct Controller {
    config: Option<Config>,
    pub status: Arc<Status>,
    dirty: Arc<AtomicBool>,
    checked: Mutex<Option<u64>>,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl<R: RuntimeControl> Reconciler<R> {
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
        self.seat_admission = Controller { config, status, dirty, checked: Mutex::new(None), _watcher: watcher };
        self.refresh_seat_admission();
        self
    }

    pub(super) fn refresh_seat_admission(&self) {
        let admission = &self.seat_admission;
        let Some(config) = &admission.config else { return };
        let now = u64::try_from(now_ms() / 1000).unwrap_or(u64::MAX);
        let mut checked = admission.checked.lock().unwrap_or_else(PoisonError::into_inner);
        let due = checked.is_none_or(|at| now.saturating_sub(at) >= 30 || now < at);
        if admission.dirty.swap(false, Ordering::AcqRel) || due {
            let decision = local_host().map_or_else(
                |error| Decision { mode: "fail-open".into(), reason: format!("local hostname unavailable: {error}"), queue: false, observed_at: None },
                |host| read(config, &host, now),
            );
            let mut state = admission.status.state();
            if state.decision != decision {
                if state.decision.mode != decision.mode || state.decision.reason != decision.reason {
                    tracing::info!(mode = decision.mode, reason = decision.reason, "seat admission decision");
                }
                state.decision = decision;
                self.incremental.touch("seat-admission");
                self.signal_changed();
            }
            *checked = Some(now);
        }
        let state = admission.status.state();
        let expiry = state.decision.observed_at.map(|at| at.saturating_add(121));
        let next = checked.unwrap_or(now).saturating_add(30);
        let deadline = expiry.filter(|expiry| *expiry > now).map_or(next, |expiry| expiry.min(next));
        self.arm_deadline_at("seat-admission", u128::from(deadline) * 1000,
            crate::reconcile_telemetry::WakeCause::Other);
    }
}
