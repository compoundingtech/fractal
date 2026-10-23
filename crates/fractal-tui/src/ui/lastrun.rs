//! Whether the Fractal TUI before this one ended cleanly.
//!
//! A Fractal TUI that is killed from outside (SIGKILL) or aborts cannot restore the terminal, so its
//! window keeps its last picture and looks frozen (Nathan, 2026-10-05). Each running Fractal TUI keeps
//! a small file under `$XDG_CACHE_HOME/st3/stui/runs/` (the cache, not the state: a device
//! with no st state must stay free of it), removed when it exits by any route that
//! runs destructors: a quit, a signal it handles or an error. A panic preserves its marker
//! for draft-recovery accounting on the next launch. A file whose process is
//! gone is the trace of one that did not, and the next Fractal TUI to start says so.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize)]
struct Mark {
    pid: u32,
    started_unix: u64,
    build: String,
    #[serde(default)]
    panicked: bool,
}

/// This Fractal TUI's file, removed on drop.
pub struct Run {
    file: Option<PathBuf>,
    started: std::time::Instant,
}

/// Why this Fractal TUI is ending, as the code that ends it knows. Said once; the first reason stands.
static EXIT_REASON: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Record why this Fractal TUI is ending, for `exits.log` (Nathan, 2026-10-13: "it crashed twice and I
/// cannot tell why"). A hard kill leaves no reason, and a run file that outlives its process.
pub fn note_exit(reason: impl Into<String>) {
    if let Ok(mut said) = EXIT_REASON.lock()
        && said.is_none()
    {
        *said = Some(reason.into());
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let panicking = std::thread::panicking();
        if let Some(file) = self
            .file
            .as_ref()
            .and_then(|file| file.parent())
            .and_then(Path::parent)
            .map(|dir| dir.join("exits.log"))
        {
            let reason = EXIT_REASON
                .lock()
                .ok()
                .and_then(|said| said.clone())
                .unwrap_or_else(|| {
                    if panicking {
                        "panic".to_owned()
                    } else {
                        "ended by a path that gave no reason".to_owned()
                    }
                });
            write_exit(&file, self.started.elapsed().as_secs(), &reason);
        }
        if panicking {
            return;
        }
        if let Some(file) = self.file.take() {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// When this Fractal TUI's own executable was last written, as it was at start.
static EXE_AT_START: std::sync::OnceLock<Option<std::time::SystemTime>> = std::sync::OnceLock::new();

fn executable_written() -> Option<std::time::SystemTime> {
    std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok())
}

/// Whether the executable this Fractal TUI runs from was replaced on disk since it started, once: a
/// deploy that writes over a running program can make the system stop it (macOS does for a
/// binary whose signature no longer matches), which looks like a crash with no panic. The fact
/// is written to `exits.log` before anything can stop it, so the cause is on record.
pub fn binary_replaced() -> bool {
    static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(Some(at_start)) = EXE_AT_START.get() else {
        return false;
    };
    if executable_written().is_none_or(|now| now == *at_start)
        || SAID.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        return false;
    }
    if let Some(file) = directory().and_then(|runs| runs.parent().map(|dir| dir.join("exits.log"))) {
        write_line(
            &file,
            &format!(
                "{} pid {}: the program was replaced on disk while this Fractal TUI ran (a deploy); the system may stop it",
                now_unix(),
                std::process::id()
            ),
        );
    }
    true
}

/// The terminal is gone and the watch that sees it is about to end this process without running
/// destructors: say so in `exits.log` (an exit with no line there is a silent one), take this
/// run's marker away so the next Fractal TUI does not count it as a crash, and keep what was typed.
pub fn terminal_gone() {
    if let Some(runs) = directory() {
        if let Some(log) = runs.parent().map(|dir| dir.join("exits.log")) {
            write_line(
                &log,
                &format!(
                    "{} pid {}: the terminal closed; the hangup watch ended this Fractal TUI",
                    now_unix(),
                    std::process::id()
                ),
            );
        }
        let _ = std::fs::remove_file(runs.join(format!("{}.json", std::process::id())));
    }
    super::drafts_store::flush_unsaved();
}

/// Say that a turn of the main loop was slow, and what was on screen, in `slow.log` beside the
/// panic log (Nathan, 2026-10-13: "understand why the tui crashes or is slow, if it ever is").
/// One line a ten seconds at most, bounded to 100 KB; the context holds kinds and counts, never
/// names, ids, paths or text.
pub fn note_slow(what: &str, context: &str, build: &str) {
    use std::sync::Mutex;
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let Ok(mut last) = LAST.lock() else {
        return;
    };
    if last.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(10)) {
        return;
    }
    *last = Some(std::time::Instant::now());
    if let Some(file) = directory()
        .and_then(|runs| runs.parent().map(|dir| dir.join("slow.log")))
    {
        write_line(&file, &format!("{} {build} pid {} {what}; {context}", now_unix(), std::process::id()));
    }
}

/// Append one line to a log in the cache that is cleared once it passes 100 KB.
fn write_line(file: &Path, line: &str) {
    use std::io::Write as _;
    let _ = std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")));
    if std::fs::metadata(file).is_ok_and(|meta| meta.len() > 100_000) {
        let _ = std::fs::remove_file(file);
    }
    if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(out, "{}", line.replace('\n', " "));
    }
}

/// Append one line to `exits.log`, which is cleared once it passes 100 KB: when this Fractal TUI ended,
/// how long it had run, and why.
fn write_exit(file: &Path, ran_seconds: u64, reason: &str) {
    use std::io::Write as _;
    if std::fs::metadata(file).is_ok_and(|meta| meta.len() > 100_000) {
        let _ = std::fs::remove_file(file);
    }
    if let Ok(mut out) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
    {
        let reason = reason.replace('\n', " ");
        let _ = writeln!(
            out,
            "{} pid {} ran {ran_seconds}s: {reason}",
            now_unix(),
            std::process::id()
        );
    }
}

fn directory() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    base.is_absolute()
        .then(|| base.join("st3").join("stui").join("runs"))
}

/// Keep a trace of what a panic said and where, in `panics.log` beside the run files, so a
/// crash that the terminal's own restore hides can be read afterwards (Nathan, 2026-10-07: "it
/// keeps crashing"). The default hook still runs, so the terminal is restored as before.
pub fn log_panics(build: &str) {
    let build = build.to_owned();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        mark_panicked();
        super::slo::panic();
        let what = format!("{info}\n{}", std::backtrace::Backtrace::force_capture());
        // Beside the run files in the cache, and beside `connection.log` in the state
        // directory, where a person looking at what Fractal TUI left behind looks first.
        let state = super::prefs::path().and_then(|prefs| {
            prefs
                .parent()
                .filter(|dir| dir.is_dir())
                .map(|dir| dir.join("panics.log"))
        });
        let cache = directory().and_then(|runs| runs.parent().map(|dir| dir.join("panics.log")));
        for file in [cache, state].into_iter().flatten() {
            write_panic(&file, &build, &what);
        }
        // What was typed and not yet written is not lost with the panic.
        super::drafts_store::flush_unsaved();
        previous(info);
    }));
}

fn mark_panicked() {
    let Some(dir) = directory() else { return };
    let file = dir.join(format!("{}.json", std::process::id()));
    if let Some(mut mark) = std::fs::read(&file)
        .ok()
        .and_then(|raw| serde_json::from_slice::<Mark>(&raw).ok())
    {
        mark.panicked = true;
        if let Ok(raw) = serde_json::to_vec(&mark) {
            let _ = std::fs::write(file, raw);
        }
    }
}

/// Append one panic to the log, which is cleared once it passes 200 KB.
fn write_panic(file: &Path, build: &str, what: &str) {
    use std::io::Write as _;
    let _ = std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")));
    if std::fs::metadata(file).is_ok_and(|meta| meta.len() > 200_000) {
        let _ = std::fs::remove_file(file);
    }
    if let Ok(mut out) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
    {
        let _ = writeln!(out, "{} {build} {what}\n", now_unix());
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Whether a process with this ID exists.
fn alive(pid: u32) -> bool {
    // Signal 0 only checks: it succeeds, or fails with EPERM for a process of another user.
    // SAFETY: kill with signal 0 sends nothing.
    let found = unsafe { libc::kill(pid as libc::pid_t, 0) };
    found == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Record that this Fractal TUI is running, and say whether an earlier one left without exiting
/// cleanly: its file is there and its process is gone. The newest such trace is returned
/// (and every one is cleared), as "started 3h ago".
pub fn begin(build: &str) -> (Run, Option<String>) {
    let _ = EXE_AT_START.set(executable_written());
    let Some(directory) = directory() else {
        return (
            Run {
                file: None,
                started: std::time::Instant::now(),
            },
            None,
        );
    };
    let _ = std::fs::create_dir_all(&directory);
    let note = left_without_exiting(&directory, std::process::id(), now_unix());
    let file = directory.join(format!("{}.json", std::process::id()));
    let mark = Mark {
        pid: std::process::id(),
        started_unix: now_unix(),
        build: build.to_owned(),
        panicked: false,
    };
    let written = serde_json::to_vec(&mark)
        .ok()
        .is_some_and(|bytes| std::fs::write(&file, bytes).is_ok());
    (
        Run {
            file: written.then_some(file),
            started: std::time::Instant::now(),
        },
        note,
    )
}

fn left_without_exiting(directory: &Path, own: u32, now: u64) -> Option<String> {
    let mut newest: Option<Mark> = None;
    for entry in std::fs::read_dir(directory).ok()?.flatten() {
        let path = entry.path();
        let Some(mark) = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Mark>(&bytes).ok())
        else {
            continue;
        };
        if mark.pid == own || alive(mark.pid) {
            continue;
        }
        // Only the process that successfully consumes a stale marker counts it.
        if std::fs::remove_file(&path).is_ok() && !mark.panicked {
            super::slo::METRICS.add("silent_exits", 1);
        }
        if newest
            .as_ref()
            .is_none_or(|known| mark.started_unix > known.started_unix)
        {
            newest = Some(mark);
        }
    }
    newest.map(|mark| {
        format!(
            "An earlier Fractal TUI (pid {}, started {} ago, {}) ended without exiting cleanly: killed or crashed",
            mark.pid,
            elapsed(now.saturating_sub(mark.started_unix)),
            mark.build
        )
    })
}

fn elapsed(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..86_400 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(directory: &Path, pid: u32, started: u64) {
        let mark = Mark {
            pid,
            started_unix: started,
            build: "0.1.0+test".into(),
            panicked: false,
        };
        std::fs::write(
            directory.join(format!("{pid}.json")),
            serde_json::to_vec(&mark).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn a_trace_whose_process_is_gone_is_reported_once_and_a_live_one_is_not() {
        let directory = tempfile::tempdir().unwrap();
        // A child that has exited: its ID is free (and not ours).
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let gone = child.id();
        child.wait().unwrap();
        mark(directory.path(), gone, 1_000);
        // This test's own process is alive; so is another that is still running.
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        mark(directory.path(), sleeper.id(), 2_000);
        let note = left_without_exiting(directory.path(), 1, 1_000 + 3 * 3_600).unwrap();
        assert!(note.contains(&format!("pid {gone}")), "{note}");
        assert!(note.contains("started 3h ago"), "{note}");
        assert!(note.contains("without exiting cleanly"), "{note}");
        // It was cleared, so the next start does not say it again; the live one stayed.
        assert!(left_without_exiting(directory.path(), 1, 5_000).is_none());
        assert!(
            directory
                .path()
                .join(format!("{}.json", sleeper.id()))
                .exists()
        );
        // Once that one is gone too, its trace is the next to be reported.
        sleeper.kill().unwrap();
        sleeper.wait().unwrap();
        let later = left_without_exiting(directory.path(), 1, 5_000).unwrap();
        assert!(later.contains(&format!("pid {}", sleeper.id())), "{later}");
    }

    #[test]
    fn dropping_the_run_removes_its_file() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("1.json");
        std::fs::write(&file, b"{}").unwrap();
        drop(Run {
            file: Some(file.clone()),
            started: std::time::Instant::now(),
        });
        assert!(!file.exists());
    }

    #[test]
    fn unwinding_preserves_the_run_marker_for_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("run.json");
        std::fs::write(&file, b"{}").unwrap();
        let kept = file.clone();
        let outcome = std::panic::catch_unwind(|| {
            let _run = Run {
                file: Some(kept),
                started: std::time::Instant::now(),
            };
            panic!("recovery probe");
        });
        assert!(outcome.is_err());
        assert!(file.exists());
    }

    #[test]
    fn a_panic_is_written_to_the_log_and_the_log_stays_small() {
        let root = std::env::temp_dir().join(format!("stui-panic-{}", std::process::id()));
        let file = root.join("stui").join("panics.log");
        write_panic(&file, "build-1", "panicked at src/x.rs:1:1: boom");
        let first = std::fs::read_to_string(&file).unwrap();
        assert!(
            first.contains("build-1") && first.contains("boom"),
            "{first}"
        );
        write_panic(&file, "build-1", "second");
        assert!(std::fs::read_to_string(&file).unwrap().contains("second"));
        std::fs::write(&file, "x".repeat(200_001)).unwrap();
        write_panic(&file, "build-1", "after the cut");
        let cut = std::fs::read_to_string(&file).unwrap();
        assert!(cut.len() < 1_000 && cut.contains("after the cut"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_exit_is_logged_with_when_how_long_and_why_and_the_log_stays_small() {
        let root = std::env::temp_dir().join(format!("stui-exit-{}", std::process::id()));
        let file = root.join("stui").join("exits.log");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_exit(&file, 3725, "terminal closed (stdin hung up)");
        write_exit(&file, 4, "error: the daemon\nwent away");
        let log = std::fs::read_to_string(&file).unwrap();
        assert_eq!(log.lines().count(), 2, "{log}");
        assert!(log.contains("ran 3725s: terminal closed (stdin hung up)"), "{log}");
        assert!(log.contains("ran 4s: error: the daemon went away"), "{log}");
        std::fs::write(&file, "x".repeat(100_001)).unwrap();
        write_exit(&file, 1, "quit (Ctrl+Q)");
        let cut = std::fs::read_to_string(&file).unwrap();
        assert!(cut.len() < 200 && cut.contains("quit (Ctrl+Q)"), "{cut}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_log_line_is_one_line_and_the_log_stays_small() {
        let root = std::env::temp_dir().join(format!("stui-slow-{}", std::process::id()));
        let file = root.join("stui").join("slow.log");
        write_line(&file, "draw 300 ms, turn 400 ms; pane agent\nsecond");
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);
        std::fs::write(&file, "x".repeat(100_001)).unwrap();
        write_line(&file, "after the cut");
        assert!(std::fs::read_to_string(&file).unwrap().len() < 100);
        let _ = std::fs::remove_dir_all(root);
    }
}
