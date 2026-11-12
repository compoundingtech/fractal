//! Controlled inherited-lock fixture shared by shutdown regressions.
use std::fs;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

const FIXTURE: &str = "FRACTAL_INHERITED_LOCK_FIXTURE";
const LEGACY_ONLY: &str = "FRACTAL_INHERITED_LOCK_LEGACY_ONLY";
const IGNORE_TERM: &str = "FRACTAL_INHERITED_LOCK_IGNORE_TERM";

/// Called first by each regression; the re-executed test acts as the daemon leader.
pub fn run_if_requested() -> Result<bool> {
    let Some(directory) = std::env::var_os(FIXTURE) else {
        return Ok(false);
    };
    let directory = Path::new(&directory);
    let lock = if std::env::var_os(LEGACY_ONLY).is_some() {
        let path = directory.join("node-identity.lock");
        fs::write(&path, [])?;
        super::lock_directory_for(&path, "fixture lock file")?
    } else {
        super::lock_directory(directory)?
    };
    if std::env::var_os(IGNORE_TERM).is_some() {
        // SAFETY: installing SIG_IGN has no handler or shared-memory access.
        unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
    }
    // SAFETY: the child performs only async-signal-safe syscalls before _exit.
    // It never resumes the test harness or accesses inherited Rust locks.
    let child = unsafe { libc::fork() };
    anyhow::ensure!(
        child >= 0,
        "fork inherited lock holder: {}",
        std::io::Error::last_os_error()
    );
    if child == 0 {
        let mut byte = 0_u8;
        loop {
            // Control EOF releases the inherited descriptor even when the leader is gone.
            let read = unsafe { libc::read(0, (&mut byte as *mut u8).cast(), 1) };
            if read >= 0 {
                break;
            }
        }
        unsafe {
            libc::close(lock.as_raw_fd());
            libc::_exit(0);
        }
    }
    println!("inherited-lock-holder:{child}");
    std::io::stdout().flush()?;
    // The leader keeps its original lock until the stop under test signals it.
    loop {
        std::thread::park();
    }
}

/// The control socket owns only the fixture child's lifetime, not the leader's stop.
pub struct RetainedLock {
    pub leader: Child,
    pub holder: u32,
    control: Option<UnixStream>,
    state_dir: PathBuf,
}

impl RetainedLock {
    pub fn spawn(test: &str, state_dir: &Path, ignore_term: bool) -> Result<Self> {
        Self::spawn_lock(test, state_dir, ignore_term, false)
    }

    pub fn spawn_legacy(test: &str, state_dir: &Path) -> Result<Self> {
        Self::spawn_lock(test, state_dir, false, true)
    }

    fn spawn_lock(test: &str, state_dir: &Path, ignore_term: bool, legacy_only: bool) -> Result<Self> {
        fs::create_dir_all(state_dir)?;
        let (control, input) = UnixStream::pair()?;
        let mut command = Command::new(std::env::current_exe()?);
        // Keep a daemon-shaped argv while running the selected libtest function.
        command
            .arg0("fractal")
            .args([
                "--exact",
                test,
                "--nocapture",
                "--test-threads=1",
                "--skip",
                "up",
            ])
            .env(FIXTURE, state_dir)
            .stdin(Stdio::from(OwnedFd::from(input)))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if legacy_only {
            command.env(LEGACY_ONLY, "1");
        } else {
            command.env_remove(LEGACY_ONLY);
        }
        if ignore_term {
            command.env(IGNORE_TERM, "1");
        } else {
            command.env_remove(IGNORE_TERM);
        }
        let leader = command.spawn()?;
        let mut fixture = Self {
            leader,
            holder: 0,
            control: Some(control),
            state_dir: state_dir.into(),
        };
        let stdout = fixture.leader.stdout.take().context("fixture stdout")?;
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = lines
                .next()
                .context("fixture exited before retaining the lock")??;
            if let Some((_, holder)) = line.split_once("inherited-lock-holder:") {
                fixture.holder = holder.parse()?;
                return Ok(fixture);
            }
        }
    }

    pub fn release(&mut self) -> Result<()> {
        drop(self.control.take());
        super::wait_for_release(&self.state_dir, Instant::now() + Duration::from_secs(3))
    }

    pub fn assert_held_error(&mut self, error: &anyhow::Error) -> Result<()> {
        let text = format!("{error:#}");
        assert!(text.contains("lock still held"), "{text}");
        #[cfg(target_os = "linux")]
        assert!(text.contains(&format!("pid {} fd ", self.holder)), "{text}");
        assert!(
            self.leader.try_wait()?.is_some(),
            "the leader has not exited"
        );
        assert!(
            super::try_stop_locks(&self.state_dir).is_err(),
            "child must still own the lock"
        );
        Ok(())
    }
}

impl Drop for RetainedLock {
    fn drop(&mut self) {
        let _ = self.leader.kill();
        let _ = self.leader.wait();
        if let Err(error) = self.release() {
            eprintln!("inherited-lock fixture cleanup failed: {error:#}");
        }
    }
}
