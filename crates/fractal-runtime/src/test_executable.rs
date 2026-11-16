//! Executables that tests write and then run.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Writes `body` as the executable `root/name`. A parallel test can fork while this process holds
/// a file open for writing, and the child keeps that writer until it execs, so running the file in
/// that window fails with ETXTBSY. `install` creates and closes the executable in its own process,
/// so no child of this one inherits a writer for it.
pub(crate) fn fake_executable(root: &Path, name: &str, body: &str) -> PathBuf {
    let source = root.join(format!("{name}.source"));
    let binary = root.join(name);
    std::fs::write(&source, body).unwrap();
    let output = Command::new("install")
        .args(["-m", "700"])
        .arg(&source)
        .arg(&binary)
        .output()
        .unwrap();
    assert!(output.status.success(), "install failed: {output:?}");
    binary
}

mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Stops the forking threads however the runs end, so a panic cannot leave the scope waiting.
    struct Stop<'a>(&'a AtomicBool);

    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    // With a writer in this process, 7 of 8 runs of 300 execs saw one to five fail with ETXTBSY
    // under these two forking threads, so 600 fail almost every time; through `install` none can.
    #[test]
    fn a_fake_executable_runs_while_other_threads_fork() {
        let root = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let runs = std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    while !stop.load(Ordering::Relaxed) {
                        let _ = Command::new("true").status();
                    }
                });
            }
            let _stop = Stop(&stop);
            (0..600)
                .map(|n| {
                    let binary = fake_executable(root.path(), &format!("probe-{n}"), "#!/bin/sh\nexit 0\n");
                    Command::new(&binary).status()
                })
                .collect::<Vec<_>>()
        });
        let busy = runs
            .iter()
            .filter(|run| matches!(run, Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy))
            .count();
        assert_eq!(busy, 0, "a forked child held a writer for the executable");
        assert!(runs.into_iter().all(|run| run.unwrap().success()));
    }
}
