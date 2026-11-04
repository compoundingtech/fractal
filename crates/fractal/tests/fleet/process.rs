//! Restart owns the daemon's descendants too: a child before exec still holds its state lock.
use std::fs::File;
use std::io;
use std::process::{Child, Command, ExitStatus, Stdio};

pub(super) struct DaemonProcess {
    child: Child,
    pid: u32,
    #[cfg(target_os = "linux")]
    _control: std::os::unix::net::UnixStream,
}

impl DaemonProcess {
    /// Run a fleet daemon with a complete, explicit environment (as Node::command supplies).
    /// Program lookup uses that environment too; inherited variables are intentionally excluded.
    pub(super) fn spawn_isolated(command: Command, stdout: File, stderr: File) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            use std::io::Read as _;
            use std::os::unix::net::UnixStream;
            let environment = command
                .get_envs()
                .filter_map(|(key, value)| {
                    value.map(|value| {
                        (
                            key.to_string_lossy().into_owned(),
                            value.to_string_lossy().into_owned(),
                        )
                    })
                })
                .collect();
            let program = command.get_program().to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "fixture program must be UTF-8")
            })?;
            let program = fractal_runtime::resolve_executable(program, &environment)
                .map_err(io::Error::other)?;
            let python = fractal_runtime::resolve_executable("python3", &environment)
                .map_err(io::Error::other)?;
            let (mut control, input) = UnixStream::pair()?;
            let script = std::path::Path::new(test_env!("CARGO_MANIFEST_DIR"))
                .join("../../scripts/fractal_test_process.py");
            let mut scope = Command::new(python);
            scope
                .arg(script)
                .args(["--scope", "--owner", &std::process::id().to_string(), "--"])
                .arg(program)
                .args(command.get_args())
                .env_clear()
                .envs(
                    command
                        .get_envs()
                        .filter_map(|(key, value)| value.map(|value| (key, value))),
                )
                .stdin(Stdio::from(std::os::fd::OwnedFd::from(input)))
                .stdout(stdout)
                .stderr(stderr);
            if let Some(directory) = command.get_current_dir() {
                scope.current_dir(directory);
            }
            let mut daemon = Self {
                child: scope.spawn()?,
                pid: 0,
                _control: control.try_clone()?,
            };
            let mut pid = [0; 4];
            control.read_exact(&mut pid)?;
            daemon.pid = u32::from_be_bytes(pid);
            Ok(daemon)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let mut command = command;
            let child = command
                .stdin(Stdio::null())
                .stdout(stdout)
                .stderr(stderr)
                .spawn()?;
            let pid = child.id();
            Ok(Self { child, pid })
        }
    }

    pub(super) fn id(&self) -> u32 {
        self.pid
    }
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    pub(super) fn kill(&mut self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            // The guardian handles TERM, kills its owned tree, and reaps every descendant
            // before exiting. The unreaped direct child keeps this PID reserved.
            if unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        #[cfg(not(target_os = "linux"))]
        self.child.kill()
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn stopping_a_daemon_reaps_a_child_that_inherited_its_state_lock() {
    use std::io::{BufRead as _, BufReader};
    use std::os::unix::net::UnixListener;
    if fractal_daemon::test_support::supervise_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let ready = UnixListener::bind(root.path().join("ready")).unwrap();
    let mut command = fractal_daemon::test_support::command("python3");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap());
    command
        .args([
            "-c",
            r#"
import fcntl, os, socket, sys
lock = open(sys.argv[1] + '/state.lock', 'w')
fcntl.flock(lock, fcntl.LOCK_EX)
read, write = os.pipe()
child = os.fork()
if child == 0:
    # Keep both ends: the blocked child stays alive after its parent dies.
    os.read(read, 1)
    os._exit(0)
ready = socket.socket(socket.AF_UNIX)
ready.connect(sys.argv[1] + '/ready')
ready.sendall((str(child) + '\n').encode())
ready.recv(1)
"#,
        ])
        .arg(root.path());
    let mut daemon = DaemonProcess::spawn_isolated(
        command,
        File::create(root.path().join("out")).unwrap(),
        File::create(root.path().join("err")).unwrap(),
    )
    .unwrap();
    let (connection, _) = ready.accept().unwrap();
    let mut pid = String::new();
    BufReader::new(&connection).read_line(&mut pid).unwrap();
    let lock = File::open(root.path().join("state.lock")).unwrap();
    assert!(matches!(
        lock.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    daemon.kill().unwrap();
    daemon.wait().unwrap();
    lock.try_lock()
        .expect("stop must close the inherited state lock before restart");
    assert!(
        !std::path::Path::new("/proc").join(pid.trim()).exists(),
        "stop must reap the inherited child"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn daemon_scopes_use_only_the_supplied_path_and_do_not_inject_evidence_environment() {
    use std::os::unix::fs::PermissionsExt as _;
    if fractal_daemon::test_support::supervise_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let python =
        fractal_runtime::resolve_executable("python3", &std::env::vars().collect()).unwrap();
    let python_json = serde_json::to_string(&python).unwrap();
    let marker = root.path().join("private-python-used");
    let wrapper = bin.join("python3");
    std::fs::write(&wrapper, format!(
        "#!{}\nimport os, sys\nwith open({}, 'a') as marker: marker.write('launch\\n')\nos.execv({}, [{}, *sys.argv[1:]])\n",
        python.display(), serde_json::to_string(&marker).unwrap(), python_json, python_json,
    )).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new("python3");
    command.env_clear().env("PATH", &bin).args([
        "-c",
        r#"
import json, os
print(json.dumps([key for key in os.environ if key.startswith('FRACTAL_TEST_')]))
"#,
    ]);
    let output = root.path().join("out");
    let mut daemon = DaemonProcess::spawn_isolated(
        command,
        File::create(&output).unwrap(),
        File::create(root.path().join("err")).unwrap(),
    )
    .unwrap();
    assert!(daemon.wait().unwrap().success());
    assert_eq!(
        std::fs::read_to_string(marker).unwrap().lines().count(),
        2,
        "the guardian and daemon must both use the private Python, without a default PATH"
    );
    assert_eq!(
        std::fs::read_to_string(output).unwrap().trim(),
        "[]",
        "a daemon scope must preserve the explicit fixture environment"
    );
}
