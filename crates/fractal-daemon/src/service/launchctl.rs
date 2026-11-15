//! Bounded local launchd RPCs, with pipe draining and deterministic child cleanup.

use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

// launchctl talks to the local launchd, not a remote service. Three seconds is
// ample for that RPC; an unresponsive launchd must not stall daemon work.
const TIMEOUT: Duration = Duration::from_secs(3);
const EXIT_POLL: Duration = Duration::from_millis(10);

/// Apply a three-second launchctl RPC deadline, then kill and reap on timeout.
///
/// Standard input is closed; stdout and stderr are captured independently.
/// At the deadline, publish unresponsive health before killing and reaping the
/// child, then return `io::ErrorKind::TimedOut`. Reaping may outlast the deadline.
/// All other runner errors also kill and reap it. No reader threads can remain
/// blocked by descendants retaining inherited output descriptors.
/// The shared launchd health gate may return `WouldBlock` without spawning.
pub fn output(command: &mut Command) -> io::Result<Output> {
    super::launchd_health::run(|publish_timeout| {
        let deadline = Instant::now() + TIMEOUT;
        output_with_remaining(command, |_| {
            Ok(deadline.saturating_duration_since(Instant::now()))
        }, publish_timeout)
    })
}

// Keeping the clock injectable lets tests expire a real child immediately,
// without relying on sleeps or races with a short wall-clock timeout.
fn output_with_remaining(
    command: &mut Command,
    mut remaining: impl FnMut(&Child) -> io::Result<Duration>,
    publish_timeout: impl Fn(),
) -> io::Result<Output> {
    let mut child = ReapOnDrop(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let mut stdout = child.0.stdout.take().expect("stdout was piped");
    let mut stderr = child.0.stderr.take().expect("stderr was piped");
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut status = None;

    loop {
        let time_left = remaining(&child.0)?;
        if time_left.is_zero() {
            publish_timeout();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "launchctl exceeded its three-second deadline",
            ));
        }
        // Read at most one buffer per stream per iteration, so a continuously
        // writing child cannot starve either stderr or the deadline check.
        if !stdout_eof {
            stdout_eof = drain(&mut stdout, &mut output)?;
        }
        if !stderr_eof {
            stderr_eof = drain(&mut stderr, &mut errors)?;
        }
        if status.is_none() {
            status = child.0.try_wait()?;
        }
        if let Some(status) = status
            && stdout_eof
            && stderr_eof
        {
            return Ok(Output {
                status,
                stdout: output,
                stderr: errors,
            });
        }

        let mut descriptors = [
            libc::pollfd {
                fd: if stdout_eof { -1 } else { stdout.as_raw_fd() },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if stderr_eof { -1 } else { stderr.as_raw_fd() },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // A pipe can close before its child exits, so periodically check waitpid
        // even when there are no remaining descriptors to wake poll.
        let milliseconds = time_left.min(EXIT_POLL).as_millis();
        let timeout = i32::try_from(milliseconds).unwrap_or(10);
        // SAFETY: descriptors is a live array of two initialized pollfd values.
        let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, timeout) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

fn nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
    let fd = pipe.as_raw_fd();
    // SAFETY: pipe owns the live descriptor throughout both fcntl calls.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: F_SETFL changes only this live pipe's descriptor status flags.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>) -> io::Result<bool> {
    let mut buffer = [0; 8192];
    match pipe.read(&mut buffer) {
        Ok(0) => Ok(true),
        Ok(count) => {
            bytes.extend_from_slice(&buffer[..count]);
            Ok(false)
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

struct ReapOnDrop(Child);

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        // Child caches a successfully reaped status, so kill is harmless after
        // try_wait succeeds. Always wait, even when kill fails or it already
        // exited, and retry interruptions rather than leaving a zombie.
        let _ = self.0.kill();
        loop {
            match self.0.wait() {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                _ => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn fake_launchctl(script: &str) -> (tempfile::TempDir, Command) {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("launchctl");
        fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        (directory, Command::new(binary))
    }

    fn capture(command: &mut Command) -> io::Result<Output> {
        let deadline = Instant::now() + TIMEOUT;
        output_with_remaining(command, |_| {
            Ok(deadline.saturating_duration_since(Instant::now()))
        }, || {})
    }

    fn assert_reaped(pid: u32) {
        let mut status = 0;
        // SAFETY: status is writable; WNOHANG never waits on an unreaped child.
        let result = unsafe {
            libc::waitpid(i32::try_from(pid).unwrap(), &mut status, libc::WNOHANG)
        };
        assert_eq!(result, -1, "runner left a live child or zombie");
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ECHILD));
    }

    #[test]
    fn captures_both_pipes_beyond_pipe_capacity() {
        let (_directory, mut command) = fake_launchctl(
            "i=0; while [ \"$i\" -lt 8192 ]; do printf 'stdout0123456789\\n'; printf 'stderr0123456789\\n' >&2; i=$((i + 1)); done",
        );
        let output = capture(&mut command).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"stdout0123456789\n".repeat(8192));
        assert_eq!(output.stderr, b"stderr0123456789\n".repeat(8192));
    }

    #[test]
    fn preserves_nonzero_status_and_output() {
        let (_directory, mut command) =
            fake_launchctl("printf 'output'; printf 'failure' >&2; exit 113");
        let output = capture(&mut command).unwrap();
        assert_eq!(output.status.code(), Some(113));
        assert_eq!(output.stdout, b"output");
        assert_eq!(output.stderr, b"failure");
    }

    #[test]
    fn timeout_kills_and_reaps_actual_child() {
        let (_directory, mut command) = fake_launchctl("while :; do :; done");
        let pid = std::cell::Cell::new(None);
        let published = std::cell::Cell::new(false);
        let error = output_with_remaining(&mut command, |child| {
            pid.set(Some(child.id()));
            Ok(Duration::ZERO)
        }, || {
            let mut status = 0;
            // SAFETY: status is writable and the runner still owns this child.
            // WNOHANG proves publication precedes both killing and reaping.
            assert_eq!(unsafe {
                libc::waitpid(i32::try_from(pid.get().unwrap()).unwrap(), &mut status, libc::WNOHANG)
            }, 0);
            published.set(true);
        })
        .unwrap_err();
        assert!(published.get());
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_reaped(pid.get().unwrap());
    }

    #[test]
    fn runner_error_also_kills_and_reaps_actual_child() {
        let (_directory, mut command) = fake_launchctl("while :; do :; done");
        let mut pid = None;
        let error = output_with_remaining(&mut command, |child| {
            pid = Some(child.id());
            Err(io::Error::other("injected runner failure"))
        }, || {})
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_reaped(pid.unwrap());
    }
}
