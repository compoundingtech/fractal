//! One client-process conversation-open measurement, opt-in and content-free.
//!
//! The selector observes the ordinary UI; it never opens a pane or changes a request.
//! This is separate from STUI_TIMING_LOG's all-request/first-wire-frame observer.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    time::Instant,
};

const MAX_TARGET_BYTES: usize = 256;
const MAX_LOG_BYTES: usize = 2048;

pub(crate) struct Trace {
    started: Instant,
    target: String,
    file: Option<File>,
    bytes: usize,
    intent: bool,
    queued: bool,
    update: bool,
    nonempty: bool,
}

impl Trace {
    /// Both variables are required. Missing/invalid configuration never changes the UI.
    pub(crate) fn from_env(started: Instant) -> Option<Self> {
        let path = std::env::var_os("STUI_OPEN_TRACE_FILE")?;
        let target = std::env::var("STUI_OPEN_TRACE_CONVERSATION").ok()?;
        match Self::create(started, Path::new(&path), target) {
            Ok(trace) => Some(trace),
            Err(_) => {
                eprintln!("Fractal TUI: conversation-open timing unavailable; continuing without it");
                None
            }
        }
    }

    fn create(started: Instant, path: &Path, target: String) -> io::Result<Self> {
        if target.len() > MAX_TARGET_BYTES
            || !(target.starts_with("agent/") || target.starts_with("session/"))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid selector",
            ));
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        let mut trace = Self {
            started,
            target,
            file: Some(file),
            bytes: 0,
            intent: false,
            queued: false,
            update: false,
            nonempty: false,
        };
        trace.record("process_started", None);
        Ok(trace)
    }

    fn record(&mut self, stage: &'static str, entries: Option<usize>) {
        let Some(file) = &mut self.file else { return };
        let line = serde_json::json!({
            "stage": stage,
            "at_ms": self.started.elapsed().as_secs_f64() * 1000.0,
            "entries": entries,
            "live_update_seen": self.update,
            "live_nonempty_update_seen": self.nonempty,
        })
        .to_string()
            + "\n";
        // At most seven fixed-stage records, all without externally supplied strings.
        if self.bytes.saturating_add(line.len()) > MAX_LOG_BYTES
            || file.write_all(line.as_bytes()).is_err()
        {
            self.file = None;
            return;
        }
        self.bytes += line.len();
    }

    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    /// The UI loop observed this conversation among its ordinary subscription intents.
    /// This is not the keyboard event time or successful WebSocket transmission.
    pub(crate) fn intend(&mut self, targets: &[String]) {
        if !self.intent && targets.iter().any(|target| target == &self.target) {
            self.intent = true;
            self.record("conversation_intent", None);
        }
    }

    pub(crate) fn enqueue(&mut self, targets: &[String], sent: bool) {
        if !self.queued && targets.iter().any(|target| target == &self.target) {
            self.queued = true;
            if sent {
                self.record("converse_enqueued", None);
            } else {
                self.record("converse_enqueue_failed", None);
                self.file = None;
            }
        }
    }

    pub(crate) fn received(&mut self, target: &str, entries: usize) {
        if target != self.target || !self.queued {
            return;
        }
        if !self.update {
            self.update = true;
            self.record("conversation_update", Some(entries));
        }
        if entries > 0 && !self.nonempty {
            self.nonempty = true;
            self.record("nonempty_update", Some(entries));
        }
    }

    fn presented(&mut self, useful: bool) {
        if useful && self.intent {
            self.record("first_useful_presented", None);
            // The one measurement is complete. Later updates/reopens cannot append to it.
            self.file = None;
        }
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        if self.file.is_some() {
            self.record(
                if std::thread::panicking() {
                    "panicked_before_useful_render"
                } else {
                    "stopped_before_useful_render"
                },
                None,
            );
        }
    }
}

/// Finish presenting before inspecting readiness or recording success. A failed terminal
/// write propagates unchanged and cannot create a useful-render marker.
pub(crate) fn present<T, E>(
    trace: &mut Option<Trace>,
    finish: impl FnOnce() -> Result<T, E>,
    useful: impl FnOnce(&str) -> bool,
) -> Result<T, E> {
    let result = finish()?;
    if let Some(trace) = trace.as_mut().filter(|trace| trace.file.is_some()) {
        let useful = useful(trace.target());
        trace.presented(useful);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, fs};

    const TARGET: &str = "agent/test/private-target";

    fn stages(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["stage"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn presentation_is_after_the_successful_terminal_write_and_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timing");
        let mut trace = Some(Trace::create(Instant::now(), &path, TARGET.into()).unwrap());
        let targets = vec![TARGET.into()];
        let selected = trace.as_mut().unwrap();
        selected.intend(&targets);
        selected.enqueue(&targets, true);
        selected.received("agent/test/other", 5);
        selected.received(TARGET, 0);
        present(&mut trace, || Ok::<_, ()>(()), |_| false).unwrap();
        assert!(!stages(&path).contains(&"first_useful_presented".into()));
        trace.as_mut().unwrap().received(TARGET, 2);
        // Placeholder/occluded/offscreen readiness is supplied by the actual viewport.
        present(&mut trace, || Ok::<_, ()>(()), |_| false).unwrap();
        let writing = Cell::new(false);
        assert_eq!(
            present(
                &mut trace,
                || {
                    assert!(!stages(&path).contains(&"first_useful_presented".into()));
                    writing.set(true);
                    Ok::<_, ()>(42)
                },
                |_| {
                    assert!(writing.get());
                    true
                }
            )
            .unwrap(),
            42
        );
        for _ in 0..1000 {
            trace.as_mut().unwrap().received(TARGET, 10);
            present(&mut trace, || Ok::<_, ()>(()), |_| true).unwrap();
        }
        drop(trace);
        assert_eq!(
            stages(&path),
            [
                "process_started",
                "conversation_intent",
                "converse_enqueued",
                "conversation_update",
                "nonempty_update",
                "first_useful_presented"
            ]
        );
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains(TARGET));
        assert!(text.len() <= MAX_LOG_BYTES);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn failed_write_and_unfinished_run_never_claim_a_useful_render() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timing");
        let mut trace = Some(Trace::create(Instant::now(), &path, TARGET.into()).unwrap());
        let selected = trace.as_mut().unwrap();
        selected.intend(&[TARGET.into()]);
        selected.enqueue(&[TARGET.into()], true);
        selected.received(TARGET, 1);
        assert_eq!(
            present(
                &mut trace,
                || Err::<(), _>("terminal failure"),
                |_| { panic!("readiness cannot run after a failed presentation") }
            ),
            Err("terminal failure")
        );
        drop(trace);
        assert_eq!(
            stages(&path).last().unwrap(),
            "stopped_before_useful_render"
        );
        assert!(!stages(&path).contains(&"first_useful_presented".into()));
    }

    #[test]
    fn cached_useful_presentation_is_not_delayed_until_a_live_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timing");
        let mut trace = Some(Trace::create(Instant::now(), &path, TARGET.into()).unwrap());
        let selected = trace.as_mut().unwrap();
        selected.intend(&[TARGET.into()]);
        selected.enqueue(&[TARGET.into()], true);
        present(&mut trace, || Ok::<_, ()>(()), |_| true).unwrap();
        trace.as_mut().unwrap().received(TARGET, 5);
        drop(trace);
        let text = fs::read_to_string(path).unwrap();
        let final_record: serde_json::Value =
            serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(final_record["stage"], "first_useful_presented");
        assert_eq!(final_record["live_update_seen"], false);
        assert_eq!(final_record["live_nonempty_update_seen"], false);
        assert!(!text.contains("conversation_update"));
    }

    #[test]
    fn disabled_path_preserves_the_result_without_readiness_work() {
        assert_eq!(
            present(
                &mut None,
                || Ok::<_, ()>(17),
                |_| { panic!("disabled tracing must not inspect the viewport") }
            ),
            Ok(17)
        );
        assert_eq!(present(&mut None, || Err::<(), _>(9), |_| true), Err(9));
    }

    #[test]
    fn existing_files_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timing");
        fs::write(&path, "keep").unwrap();
        assert!(Trace::create(Instant::now(), &path, TARGET.into()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "keep");
    }
}
