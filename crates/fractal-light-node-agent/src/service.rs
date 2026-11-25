//! Explicit GUI-login launchd ownership. Never manages native children or graph services.
use std::path::{Path, PathBuf};
use anyhow::{Context as _, Result, ensure};
use serde_json::Value;
#[cfg(target_os = "macos")]
use serde_json::json;

pub const LABEL: &str = "com.compoundingtech.fractal-light-node-agent";
#[cfg(all(unix, any(target_os = "macos", test)))]
const LIMIT: u64 = 1024 * 1024;
#[cfg(target_os = "macos")]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation { Install, Start, Stop, Restart, Health, Uninstall }
impl Operation {
    pub fn parse(value: &std::ffi::OsStr) -> Result<Self> {
        match value.to_str() {
            Some("install") => Ok(Self::Install), Some("start") => Ok(Self::Start),
            Some("stop") => Ok(Self::Stop), Some("restart") => Ok(Self::Restart),
            Some("health") => Ok(Self::Health), Some("uninstall") => Ok(Self::Uninstall),
            _ => anyhow::bail!("expected service install|start|stop|restart|health|uninstall"),
        }
    }
}

fn path_text(path: &Path) -> Result<&str> {
    ensure!(path.is_absolute(), "service paths must be absolute");
    let text = path.to_str().context("service paths must be UTF-8")?;
    ensure!(!text.chars().any(|c| c.is_control()), "service paths cannot contain control characters");
    ensure!(text == "/" || text.split('/').skip(1).all(|part| !part.is_empty() && part != "." && part != ".."), "service paths require physical spelling without repeated/trailing separators or dot components");
    Ok(text)
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('"', "&quot;").replace('\'', "&apos;")
}

/// Pure renderer: no host probes or activation, also available on Linux.
pub fn render_plist(executable: &Path, config: &Path) -> Result<String> {
    let executable = xml(path_text(executable)?);
    let config = xml(path_text(config)?);
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{LABEL}</string>\n<key>ProgramArguments</key><array><string>{executable}</string><string>--config</string><string>{config}</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n<key>AbandonProcessGroup</key><true/>\n<key>Umask</key><integer>63</integer>\n<key>ThrottleInterval</key><integer>10</integer>\n</dict></plist>\n"))
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn uid() -> u32 {
    // SAFETY: geteuid has no pointers, failure state, or ownership effects.
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn trusted_directory(path: &Path, owner_required: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    path_text(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(metadata.is_dir() && !metadata.file_type().is_symlink()
        && (metadata.uid() == uid() || (!owner_required && metadata.uid() == 0))
        && (metadata.mode() & 0o022 == 0 || (!owner_required && metadata.mode() & 0o1000 != 0)),
        "service directory must be physical, trusted-owned, and deny group/other writes except sticky ancestors");
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn owned_home() -> Result<PathBuf> {
    use std::{ffi::{CStr, OsStr}, os::unix::ffi::OsStrExt as _};
    let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0_u8; 65536];
    // SAFETY: record and result are writable, buffer is live for its stated
    // capacity. getpwuid_r initializes record on success and keeps strings in
    // buffer. No shared/static passwd storage crosses this boundary.
    let status = unsafe {
        libc::getpwuid_r(uid(), record.as_mut_ptr(), buffer.as_mut_ptr().cast(),
            buffer.len(), &mut result)
    };
    ensure!(status == 0, "cannot resolve effective user's home: {}", std::io::Error::from_raw_os_error(status));
    ensure!(!result.is_null(), "effective user has no account home");
    // SAFETY: successful getpwuid_r with non-null result initialized record.
    let record = unsafe { record.assume_init() };
    ensure!(!record.pw_dir.is_null(), "effective user has no account home path");
    // SAFETY: getpwuid_r guarantees a NUL-terminated home string in live buffer.
    let home = unsafe { CStr::from_ptr(record.pw_dir) };
    Ok(PathBuf::from(OsStr::from_bytes(home.to_bytes())))
}

/// Fixed plist path in this effective user's owned home; no system/user-domain fallback.
#[cfg(unix)]
pub fn plist_path() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let home = owned_home()?;
    #[cfg(not(target_os = "macos"))]
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is required for the owned user home")?);
    trusted_directory(&home, true)?;
    for ancestor in home.ancestors().skip(1) { trusted_directory(ancestor, false)?; }
    Ok(home.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
}
#[cfg(not(unix))]
pub fn plist_path() -> Result<PathBuf> { anyhow::bail!("launchd service operations require Darwin") }

/// CLI cold path only. No persistent supervisor, graph lookup, or TCC requests.
pub fn run(operation: Operation, config_path: &Path) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        tokio::runtime::Builder::new_current_thread().enable_all().build()?
            .block_on(darwin::run(operation, config_path))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (operation, config_path);
        anyhow::bail!("launchd service operations require Darwin and an interactive GUI login domain")
    }
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn existing_plist(path: &Path, expected: &str) -> Result<bool> {
    use std::io::Read as _;
    for ancestor in path.ancestors().skip(1) {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => trusted_directory(ancestor, false)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
    }
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()), Ok(_) => {},
    }
    let file = crate::config::private_file(path).context("refusing foreign or non-private service plist")?;
    ensure!(file.metadata()?.len() <= LIMIT, "service plist exceeds size bound");
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes == expected.as_bytes(), "refusing service plist with different executable, configuration, or policy");
    Ok(true)
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn prepare_service_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    for ancestor in path.ancestors().skip(1) {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => trusted_directory(ancestor, false)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
    }
    let directory = path.parent().context("plist directory missing")?;
    let library = directory.parent().context("Library directory missing")?;
    for parent in [library, directory] {
        match std::fs::DirBuilder::new().mode(0o700).create(parent) {
            Ok(()) => {}, Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error.into()),
        }
        trusted_directory(parent, true)?;
    }
    Ok(())
}

/// One fixed lock inode serializes the entire lifecycle ownership transaction,
/// including old-binary uninstall against new-binary install. Never unlink it:
/// a successor opening a different inode would evade an incumbent's lock.
#[cfg(all(unix, any(target_os = "macos", test)))]
fn lifecycle_lock(plist: &Path) -> Result<std::fs::File> {
    prepare_service_directory(plist)?;
    let path = plist.parent().context("plist directory missing")?
        .join(format!(".{LABEL}.lifecycle.lock"));
    let lock = crate::config::private_state_file(&path)?;
    lock.try_lock().context("another lifecycle operation owns this GUI service; ownership is unchanged")?;
    Ok(lock)
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn install_plist(path: &Path, expected: &str) -> Result<()> {
    use std::{io::Write as _, os::unix::fs::OpenOptionsExt as _};
    prepare_service_directory(path)?;
    let directory = path.parent().context("plist directory missing")?;
    if existing_plist(path, expected)? { return Ok(()); }
    let temporary = directory.join(format!(".{LABEL}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary)?;
        file.write_all(expected.as_bytes())?;
        file.sync_all()?;
        // An exclusive link publishes complete bytes without ever overwriting an intervening file.
        match std::fs::hard_link(&temporary, path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => { existing_plist(path, expected)?; },
            Err(error) => return Err(error.into()),
        }
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    let cleanup = std::fs::remove_file(&temporary);
    result?;
    cleanup?;
    Ok(())
}

/// Strict loaded-job fence. launchctl's human output is not treated as a plist.
/// Ambiguous or unrecognized argument layouts fail closed.
pub fn validate_loaded_job(text: &str, executable: &Path, config: &Path, plist: &Path) -> Result<Option<u32>> {
    let executable = path_text(executable)?;
    let config = path_text(config)?;
    let plist = path_text(plist)?;
    let mut program = None;
    let mut path = None;
    let mut pid = None;
    let mut arguments = Vec::new();
    let mut in_arguments = false;
    let mut argument_blocks = 0;
    for line in text.lines().map(str::trim) {
        if in_arguments {
            if line == "}" { in_arguments = false; } else { arguments.push(line); }
            continue;
        }
        if line == "arguments = {" { in_arguments = true; argument_blocks += 1; }
        if let Some(value) = line.strip_prefix("program = ") { ensure!(program.replace(value).is_none(), "duplicate loaded program"); }
        if let Some(value) = line.strip_prefix("path = ") { ensure!(path.replace(value).is_none(), "duplicate loaded plist"); }
        if let Some(value) = line.strip_prefix("pid = ") { ensure!(pid.is_none(), "duplicate loaded PID"); pid = Some(value.parse::<u32>().context("invalid launchd PID")?); }
    }
    ensure!(!in_arguments && argument_blocks == 1 && program == Some(executable) && path == Some(plist)
        && arguments == [executable, "--config", config], "loaded launchd job does not match this exact agent executable/configuration/plist");
    ensure!(pid != Some(0), "invalid launchd PID");
    Ok(pid)
}

#[cfg(any(target_os = "macos", test))]
fn full_node_label(label: &str) -> bool {
    label == "org.nix-community.home.smalltalk"
        || label.starts_with("com.compoundingtech.st3")
        || label == "com.compoundingtech.fractal"
        || label.starts_with("com.compoundingtech.fractal.")
}

#[cfg(any(target_os = "macos", test))]
fn full_node_present(domain_output: &str) -> bool {
    domain_output.split_whitespace().any(|word| full_node_label(word.trim_matches('"')))
}

#[cfg(target_os = "macos")]
mod darwin {
    use super::*;
    use std::{os::unix::fs::{FileTypeExt as _, MetadataExt as _}, process::Stdio};
    use tokio::{io::{AsyncReadExt as _, AsyncWriteExt as _}, process::Command, time::timeout};

    async fn launchctl(args: &[&str]) -> Result<(bool, String, String)> {
        let mut child = Command::new("/bin/launchctl").args(args).env("LC_ALL", "C")
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn()?;
        let stdout = child.stdout.take().context("launchctl stdout unavailable")?;
        let stderr = child.stderr.take().context("launchctl stderr unavailable")?;
        let work = async {
            let read = |stream: std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>| async move {
                let mut bytes = Vec::new();
                stream.take(LIMIT + 1).read_to_end(&mut bytes).await?;
                ensure!(bytes.len() as u64 <= LIMIT, "launchctl output exceeds bound");
                Ok::<_, anyhow::Error>(String::from_utf8(bytes)?)
            };
            let (out, err, status) = tokio::try_join!(read(Box::pin(stdout)), read(Box::pin(stderr)), async { Ok::<_, anyhow::Error>(child.wait().await?) })?;
            Ok::<_, anyhow::Error>((status.success(), out, err))
        };
        match timeout(TIMEOUT, work).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => { let _ = child.kill().await; Err(error) },
            Err(_) => { child.kill().await.context("kill timed-out launchctl")?; anyhow::bail!("launchctl exceeded three-second deadline") },
        }
    }

    async fn loaded(target: &str) -> Result<Option<String>> {
        let (success, output, error) = launchctl(&["print", target]).await?;
        if success { return Ok(Some(output)); }
        // Missing job is distinct from denied access, unavailable GUI domain, or a failed RPC.
        ensure!(error.lines().any(|line| line.contains("Could not find service") && line.contains(LABEL)), "cannot determine launchd job ownership: {error}");
        Ok(None)
    }

    async fn checked_loaded(target: &str, executable: &Path, config: &Path, plist: &Path) -> Result<Option<Option<u32>>> {
        loaded(target).await?.map(|text| validate_loaded_job(&text, executable, config, plist)).transpose()
    }

    #[allow(unsafe_code)]
    fn socket_peer_pid(socket: &tokio::net::UnixStream) -> Result<u32> {
        use std::os::fd::AsRawFd as _;
        let mut pid: libc::pid_t = 0;
        let mut length = std::mem::size_of_val(&pid) as libc::socklen_t;
        // SAFETY: socket owns a live AF_UNIX descriptor; pid and length are valid,
        // correctly sized writable buffers. LOCAL_PEERPID is Darwin's peer PID query.
        let result = unsafe {
            libc::getsockopt(socket.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID,
                std::ptr::from_mut(&mut pid).cast(), &mut length)
        };
        ensure!(result == 0, "cannot establish performance peer PID: {}", std::io::Error::last_os_error());
        ensure!(length as usize == std::mem::size_of_val(&pid), "invalid performance peer PID size");
        let pid = u32::try_from(pid)?;
        ensure!(pid != 0, "invalid performance peer PID");
        Ok(pid)
    }

    async fn performance(config: &crate::config::Config, pid: u32) -> Result<Value> {
        // Reuse the runtime's physical/private namespace validation; no directory creation.
        trusted_directory(&config.state_dir, true)?;
        ensure!(std::fs::symlink_metadata(&config.state_dir)?.mode() & 0o077 == 0, "performance state directory must be private");
        for ancestor in config.state_dir.ancestors().skip(1) { trusted_directory(ancestor, false)?; }
        let path = config.socket_path();
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(metadata.file_type().is_socket() && metadata.uid() == uid() && metadata.mode() & 0o077 == 0, "performance socket must be private and owned");
        let response = timeout(TIMEOUT, async {
            let mut socket = tokio::net::UnixStream::connect(path).await?;
            let peer = socket.peer_cred()?;
            ensure!(peer.uid() == uid() && socket_peer_pid(&socket)? == pid, "performance socket peer is not the loaded launchd PID");
            socket.write_all(b"GET /v1/performance HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await?;
            let mut bytes = Vec::new();
            socket.take(LIMIT + 1).read_to_end(&mut bytes).await?;
            ensure!(bytes.len() as u64 <= LIMIT, "performance response exceeds bound");
            Ok::<_, anyhow::Error>(bytes)
        }).await.context("private performance request exceeded three-second deadline")??;
        let split = response.windows(4).position(|bytes| bytes == b"\r\n\r\n").context("invalid performance HTTP response")?;
        let headers = std::str::from_utf8(&response[..split])?;
        ensure!(headers.starts_with("HTTP/1.1 200 ") || headers.starts_with("HTTP/1.0 200 "), "performance endpoint is not healthy");
        ensure!(!headers.lines().any(|line| line.to_ascii_lowercase().starts_with("transfer-encoding:")), "unsupported performance transfer encoding");
        let value: Value = serde_json::from_slice(&response[split + 4..])?;
        validate_performance(&value, pid)?;
        Ok(value)
    }

    pub async fn run(operation: Operation, config_path: &Path) -> Result<Value> {
        path_text(config_path)?;
        ensure!(uid() != 0, "service commands require an ordinary effective user; no sudo/system domain");
        let executable = std::env::current_exe()?;
        let plist = plist_path()?;
        let expected = render_plist(&executable, config_path)?;
        // Hold before the first ownership observation through the final
        // launchctl/filesystem mutation. A health read may race a transition
        // and report unavailable, but cannot mutate an owner.
        let _lifecycle_lock = if operation == Operation::Health { None }
            else { Some(lifecycle_lock(&plist)?) };
        let domain = format!("gui/{}", uid());
        let target = format!("{domain}/{LABEL}");
        let (available, domain_output, error) = launchctl(&["print", &domain]).await?;
        ensure!(available, "interactive GUI login domain is unavailable: {error}");
        let config = if matches!(operation, Operation::Stop | Operation::Uninstall) { None } else { Some(crate::config::Config::read(config_path)?) };
        let installed = existing_plist(&plist, &expected)?;
        let loaded = checked_loaded(&target, &executable, config_path, &plist).await?;
        ensure!(loaded.is_none() || installed, "loaded job lacks the exact owned installed plist");
        if matches!(operation, Operation::Install | Operation::Start | Operation::Restart) {
            ensure!(!full_node_present(&domain_output),
                "full-node service is present in the GUI domain; native custody ownership is unknown, refusing activation");
        }
        match operation {
            Operation::Install => { install_plist(&plist, &expected)?; },
            Operation::Start | Operation::Restart => {
                ensure!(installed, "service is not installed; explicitly run service install first");
                if operation == Operation::Restart && loaded.is_some() {
                    bootout(&target).await?;
                }
                if loaded.is_none() || operation == Operation::Restart {
                    let (success, _, error) = launchctl(&["bootstrap", &domain, path_text(&plist)?]).await?;
                    ensure!(success, "launchd bootstrap failed: {error}");
                }
            },
            Operation::Stop | Operation::Uninstall => {
                if loaded.is_some() { bootout(&target).await?; }
                if operation == Operation::Uninstall && installed {
                    // Fence again after launchd returned; never remove a substituted foreign plist.
                    ensure!(existing_plist(&plist, &expected)?, "owned service plist disappeared during uninstall");
                    std::fs::remove_file(&plist)?;
                    std::fs::File::open(plist.parent().context("plist directory missing")?)?.sync_all()?;
                }
            },
            Operation::Health => {
                let Some(Some(pid)) = loaded else {
                    return Ok(json!({"label": LABEL, "domain": domain, "installed": installed, "loaded": loaded.is_some(), "local_status": "unhealthy", "reason": "no_running_loaded_pid", "pid": null, "process_instance_id": null, "graph_ready": false}));
                };
                let performance = performance(config.as_ref().context("health configuration missing")?, pid).await;
                let after = checked_loaded(&target, &executable, config_path, &plist).await?;
                ensure!(after == Some(Some(pid)), "launchd PID changed during health observation");
                return Ok(match performance {
                    Ok(performance) => json!({"label": LABEL, "domain": domain, "installed": installed, "loaded": true, "local_status": "healthy", "home_status": "not_observed", "graph_ready": false, "pid": pid, "process_instance_id": performance["process_instance_id"], "performance": performance}),
                    Err(error) => json!({"label": LABEL, "domain": domain, "installed": installed, "loaded": true, "local_status": "unhealthy", "graph_ready": false, "pid": pid, "process_instance_id": null, "reason": format!("{error:#}")}),
                });
            },
        }
        Ok(json!({"label": LABEL, "domain": domain, "operation": format!("{operation:?}").to_lowercase(), "ok": true}))
    }

    async fn bootout(target: &str) -> Result<()> {
        let (success, _, error) = launchctl(&["bootout", target]).await?;
        ensure!(success, "launchd bootout failed: {error}");
        ensure!(loaded(target).await?.is_none(), "launchd job still loaded after bootout");
        Ok(())
    }
}

/// Validate self-accounting identity, not merely an HTTP listener or launchctl state.
pub fn validate_performance(value: &Value, pid: u32) -> Result<()> {
    ensure!(value["schema_version"].as_u64() == Some(1) && value["pid"].as_u64() == Some(u64::from(pid)), "performance identity differs from loaded PID");
    let instance = value["process_instance_id"].as_str().context("missing performance process identity")?;
    ensure!(!uuid::Uuid::parse_str(instance)?.is_nil(), "invalid performance process identity");
    ensure!(value["process_cpu_clock"] == "CLOCK_PROCESS_CPUTIME_ID", "missing own-process CPU clock");
    for field in ["accounting_started_unix_ns", "process_cpu_baseline_ns", "accounting_uptime_ns", "process_cpu_ns", "cpu_sample_uptime_before_ns", "cpu_sample_uptime_after_ns"] {
        ensure!(value[field].as_u64().is_some(), "missing or invalid performance counter {field}");
    }
    ensure!(value["process_cpu_ns"].as_u64() >= value["process_cpu_baseline_ns"].as_u64()
        && value["cpu_sample_uptime_before_ns"].as_u64() <= value["cpu_sample_uptime_after_ns"].as_u64(), "inconsistent performance CPU counters");
    Ok(())
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
