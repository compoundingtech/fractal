use super::*;
use anyhow::ensure;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::io::{BufRead as _, Read as _};
use std::os::unix::net::{UnixListener, UnixStream};

const TEST: &str = "process::namespace_tests::spawn_binds_the_created_child_in_the_procfs_namespace";
const PHASE: &str = "FRACTAL_PROCESS_IDENTITY_TEST_PHASE";
const SOCKET: &str = "FRACTAL_PROCESS_IDENTITY_TEST_SOCKET";
const EXPECT_REFUSAL: &str = "FRACTAL_PROCESS_IDENTITY_EXPECT_REFUSAL";

#[derive(Serialize, Deserialize)]
struct OwnIdentity {
    local_pid: u32,
    proc_pid: u32,
    start_token: u64,
}

#[test]
fn spawn_binds_the_created_child_in_the_procfs_namespace() {
    let executable = std::env::current_exe().expect("resolve archived test executable");
    match std::env::var(PHASE).as_deref() {
        Ok("producer") => {
            // Independent oracle: /proc/self denotes this exact process even
            // when numeric PIDs in the mounted procfs differ from getpid().
            let proc_pid = fs::read_link("/proc/self").expect("producer /proc/self");
            let stat = fs::read_to_string("/proc/self/stat").expect("producer /proc/self/stat");
            let tail = stat.rsplit_once(") ").expect("producer stat command boundary").1;
            let identity = OwnIdentity {
                local_pid: std::process::id(),
                proc_pid: proc_pid.to_str().unwrap().parse().unwrap(),
                start_token: tail.split_whitespace().nth(19).unwrap().parse().unwrap(),
            };
            let socket = std::env::var(SOCKET).expect("producer readiness socket path");
            let mut stream = UnixStream::connect(socket).expect("producer connect readiness socket");
            serde_json::to_writer(&mut stream, &identity).unwrap();
            stream.write_all(b"\n").unwrap();
            let mut release = [0];
            let _ = stream.read_exact(&mut release);
        }
        Ok("supervisor") => {
            // This dedicated re-exec owns isolation initialization. It needs
            // no systemd, shell, sleep program, PATH entry or extra executable.
            let mut environment = BTreeMap::from([("PATH".into(), String::new())]);
            if let Ok(libraries) = std::env::var("LD_LIBRARY_PATH") {
                environment.insert("LD_LIBRARY_PATH".into(), libraries);
            }
            crate::initialize_isolation(&environment);
            let root = tempfile::tempdir().unwrap();
            let socket = root.path().join("ready.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            environment.insert(PHASE.into(), "producer".into());
            environment.insert(SOCKET.into(), socket.to_str().unwrap().into());
            let runtime = ExecRuntime::new(root.path().join("exec"), root.path().join("logs"));
            let result = runtime.spawn("work", &crate::Launch::Argv(vec![
                executable.to_str().unwrap().into(), "--exact".into(), TEST.into(), "--nocapture".into(),
            ]), root.path(), &environment);
            if std::env::var_os(EXPECT_REFUSAL).is_some() {
                let error = result.expect_err("sandbox must refuse the pidfd identity operation");
                let refusal = error.downcast_ref::<ProcessIdentityUnavailable>().expect("typed process identity refusal");
                assert_eq!(refusal.stage, "pidfd-open", "{error:#}");
                assert_eq!(refusal.errno, Some(libc::EPERM), "{error:#}");
                assert!(error.downcast_ref::<StartRejected>().is_none(), "producer was created: {error:#}");
                assert!(runtime.observe("work").unwrap().is_none(), "refused identity must not publish a generation");
                assert_eq!(unsafe { libc::kill(refusal.pid as i32, 0) }, -1);
                assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH), "refused child must be killed and reaped");
                eprintln!("{TEST}: typed refusal; no generation; refused child killed and reaped: {error:#}");
                return;
            }
            let generation = result.unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(&mut stream).read_line(&mut line).unwrap();
            let oracle: OwnIdentity = serde_json::from_str(&line).unwrap();
            let proof = (|| -> Result<()> {
                ensure!(generation.pid == oracle.local_pid, "generation must retain the signal/wait PID");
                ensure!(generation.start_token == oracle.start_token, "generation must bind the producer, not an unrelated procfs PID");
                ensure!(process_start_token(generation.pid)? == oracle.start_token, "subsequent token lookup must resolve the same process");
                ensure!(process_group_is_live(generation.pid)?, "procfs group scan must resolve the caller's group namespace");
                ensure!(matches!(runtime.observe("work")?, Some(ExecObservation::Running(_))), "real child must be observed running");
                eprintln!("{TEST}: correct created child: local_pid={} procfs_pid={} start_token={} caller_pid={} caller_procfs_pid={}",
                    oracle.local_pid, oracle.proc_pid, oracle.start_token, std::process::id(), fs::read_link("/proc/self")?.display());
                Ok(())
            })();
            // Wait on an exact kernel exit event, not a scheduling delay.
            // SAFETY: this is the fixture's still-owned child.
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, generation.pid, 0_u32) };
            assert!(fd >= 0, "pin fixture child for cleanup: {}", std::io::Error::last_os_error());
            let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
            let cleanup = runtime.kill("work");
            // A failed observation must not leave this test producer alive.
            unsafe { libc::kill(-(generation.pid as i32), libc::SIGKILL); }
            let mut event = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            assert_eq!(unsafe { libc::poll(&mut event, 1, 10_000) }, 1, "fixture child did not exit");
            cleanup.unwrap();
            assert!(matches!(runtime.observe("work").unwrap(), Some(ExecObservation::Exited(_))));
            proof.unwrap();
        }
        _ => {
            let status = std::process::Command::new(&executable)
                .args(["--exact", TEST, "--nocapture"])
                .env(PHASE, "supervisor").env("PATH", "")
                .status().expect("re-exec archived test supervisor");
            assert!(status.success(), "isolated process-identity supervisor failed: {status}");
        }
    }
}
