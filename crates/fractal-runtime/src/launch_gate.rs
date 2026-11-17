//! Hold a native command until its actual incarnation has been registered by the daemon.
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Dropping an unreleased launch must never execute its user command.
pub trait LaunchRelease: Send {
    fn release(self: Box<Self>, incarnation: &str) -> Result<()>;
}

/// The shell is the allocated native process and execs the command in place. Thus the
/// registry's PID/start token remains the identity registered before the release.
pub struct NativeLaunchGate {
    directory: PathBuf,
    release: std::fs::File,
    acknowledged: std::fs::File,
}

impl NativeLaunchGate {
    pub fn prepare(root: &Path, operation: &str) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        anyhow::ensure!(!operation.is_empty() && operation.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "invalid launch gate operation");
        std::fs::create_dir_all(root)?;
        let directory = root.join(operation);
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let create = |name: &str| -> Result<std::fs::File> {
            let path = directory.join(name);
            let name = std::ffi::CString::new(path.as_os_str().as_bytes())?;
            // SAFETY: name is a live NUL-terminated path; the private directory is new.
            anyhow::ensure!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) } == 0,
                "create native launch gate: {}", std::io::Error::last_os_error());
            // Hold both ends so preparation does not depend on the child's scheduling.
            Ok(std::fs::OpenOptions::new().read(true).write(true)
                .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK).open(path)?)
        };
        match (|| Ok(Self { directory: directory.clone(), release: create("release")?, acknowledged: create("ack")? }))() {
            Ok(gate) => Ok(gate),
            Err(error) => {
                let _ = std::fs::remove_dir_all(&directory);
                Err(error)
            }
        }
    }

    pub fn wrap(&self, launch: &crate::Launch) -> crate::Launch {
        // All values remain argv entries, never interpolated shell text. An empty
        // release is cancellation, not permission to execute with a missing fence.
        let mut argv = vec!["sh".into(), "-c".into(),
            "IFS= read -r ST3_INCARNATION <\"$1\" && [ -n \"$ST3_INCARNATION\" ] || exit 125; FRACTAL_INCARNATION=$ST3_INCARNATION; export ST3_INCARNATION FRACTAL_INCARNATION; printf . >\"$2\" || exit 125; shift 2; exec \"$@\"".into(),
            "native-launch-gate".into(),
            self.directory.join("release").to_string_lossy().into_owned(),
            self.directory.join("ack").to_string_lossy().into_owned(),
        ];
        match launch {
            crate::Launch::Shell(source) => argv.extend(["sh".into(), "-c".into(), source.clone()]),
            crate::Launch::Argv(command) => argv.extend(command.iter().cloned()),
        }
        crate::Launch::Argv(argv)
    }
}

impl LaunchRelease for NativeLaunchGate {
    fn release(mut self: Box<Self>, incarnation: &str) -> Result<()> {
        anyhow::ensure!(!incarnation.is_empty() && !incarnation.contains(['\n', '\r']), "invalid native incarnation");
        writeln!(self.release, "{incarnation}").context("release registered native launch")?;
        let mut ready = libc::pollfd { fd: self.acknowledged.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        loop {
            // This is a failure bound, not a startup delay: a ready child proceeds
            // immediately. No observation timing determines claim authority.
            // SAFETY: ready is one initialized pollfd and remains live for the call.
            let result = unsafe { libc::poll(&raw mut ready, 1, 30_000) };
            if result < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            anyhow::ensure!(result > 0 && ready.revents & libc::POLLIN != 0, "native launch did not acknowledge release");
            break;
        }
        let mut acknowledged = [0];
        self.acknowledged.read_exact(&mut acknowledged)?;
        anyhow::ensure!(acknowledged == [b'.'], "invalid native launch acknowledgement");
        Ok(())
    }
}

impl Drop for NativeLaunchGate {
    fn drop(&mut self) {
        // Wake an already-open reader with cancellation; a late reader instead sees
        // the removed path and exits. Neither path can execute the user command.
        let _ = self.release.write_all(b"\n");
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(gate: &NativeLaunchGate, source: &str) -> std::process::Command {
        let crate::Launch::Argv(argv) = gate.wrap(&crate::Launch::Shell(source.into())) else {
            unreachable!()
        };
        let mut command = std::process::Command::new(&argv[0]);
        command.args(&argv[1..]).stdout(std::process::Stdio::piped());
        command
    }

    #[test]
    fn native_gate_exports_the_exact_incarnation_and_preserves_pid() {
        let root = tempfile::tempdir().unwrap();
        let gate = NativeLaunchGate::prepare(root.path(), "registered-launch").unwrap();
        let child = command(&gate, "printf '%s\\n%s\\n%s\\n' \"$ST3_INCARNATION\" \"$FRACTAL_INCARNATION\" \"$$\"")
            .env("ST3_INCARNATION", "stale").env("FRACTAL_INCARNATION", "also-stale")
            .spawn().unwrap();
        let pid = child.id();
        let incarnation = format!("{pid}:exact-native-created-at");
        Box::new(gate).release(&incarnation).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{incarnation}\n{incarnation}\n{pid}\n"));
        assert!(!root.path().join("registered-launch").exists());
    }

    #[test]
    fn dropping_an_unreleased_native_gate_never_executes_the_command() {
        let root = tempfile::tempdir().unwrap();
        let gate = NativeLaunchGate::prepare(root.path(), "cancelled-launch").unwrap();
        let child = command(&gate, "printf 'user command ran'").spawn().unwrap();
        drop(gate);
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!root.path().join("cancelled-launch").exists());
    }
}
