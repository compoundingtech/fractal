//! Direct per-unit probes deliberately bypass ordinary old/new conflict lookup during staging.
use std::process::Output;
use std::time::Duration;

use serde_json::Value;
use fractal_daemon::client::Client;

use super::*;

pub(super) struct Native {
    client: Client,
    node: String,
    launchd_domain: Option<String>,
    checkpoint_worker_enabled: bool,
}

impl Native {
    pub fn new(endpoint: Endpoint, node: String, config: &Config) -> Self {
        Self { client: Client::new(endpoint), node, launchd_domain: None,
            checkpoint_worker_enabled: config.checkpoint.enabled || config.trim.local }
    }

    fn output(program: &str, arguments: &[&str]) -> Result<Output> {
        use std::io::{Read as _, Seek as _};
        use std::process::Stdio;
        if matches!(program, "launchctl" | "/bin/launchctl") {
            return fractal_daemon::service::launchctl::output(
                fractal_daemon::service::host_command(program).args(arguments),
            ).with_context(|| format!("run {program} {}", arguments.join(" ")));
        }
        // A wedged manager must not leave the host halfway through an unbounded gap.
        // Files avoid pipe-buffer deadlock while the child is polled.
        let mut stdout = tempfile::tempfile()?;
        let mut stderr = tempfile::tempfile()?;
        let mut child = fractal_daemon::service::host_command(program).args(arguments)
            .stdout(Stdio::from(stdout.try_clone()?)).stderr(Stdio::from(stderr.try_clone()?))
            .spawn().with_context(|| format!("run {program} {}", arguments.join(" ")))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait()? { break status; }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{program} {} timed out after 30 seconds", arguments.join(" "));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        stdout.rewind()?;
        stderr.rewind()?;
        let mut output = Output { status, stdout: Vec::new(), stderr: Vec::new() };
        stdout.read_to_end(&mut output.stdout)?;
        stderr.read_to_end(&mut output.stderr)?;
        Ok(output)
    }

    fn command(program: &str, arguments: &[&str]) -> Result<()> {
        let output = Self::output(program, arguments)?;
        ensure!(output.status.success(), "{program} {} failed: {}", arguments.join(" "), String::from_utf8_lossy(&output.stderr).trim());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn systemd(name: &str, base: &Path) -> Result<Unit> {
        let output = Self::output("systemctl", &["--user", "show", name, "--property=LoadState", "--property=ActiveState", "--property=UnitFileState"])?;
        let text = String::from_utf8_lossy(&output.stdout);
        let load = property(&text, "LoadState")?;
        let active = property(&text, "ActiveState")?;
        ensure!(output.status.success() || load == "not-found", "cannot inspect {name}: {}", String::from_utf8_lossy(&output.stderr));
        ensure!(matches!(load, "loaded" | "not-found" | "masked" | "error" | "bad-setting"), "cannot establish unit presence for {name}: {load}");
        let state = text.lines().find_map(|line| line.strip_prefix("UnitFileState=")).unwrap_or("");
        Ok(Unit {
            name: name.into(), file: base.join(name),
            installed: load != "not-found" || base.join(name).is_file(),
            running: !matches!(active, "inactive" | "failed"),
            // Static staged units have no enablement links. Unknown states fail closed.
            enabled: !matches!(state, "disabled" | "static") && load != "not-found",
        })
    }

    #[cfg(target_os = "macos")]
    fn domain(&self) -> Result<&str> { self.launchd_domain.as_deref().context("launchd domain has not been bound") }

    #[cfg(target_os = "macos")]
    fn launchd(&self, name: &str, base: &Path) -> Result<Unit> {
        let target = format!("{}/{name}", self.domain()?);
        let output = Self::output("launchctl", &["print", &target])?;
        ensure!(output.status.success() || output.status.code() == Some(113), "cannot inspect {target}: {}", String::from_utf8_lossy(&output.stderr));
        let disabled = Self::output("launchctl", &["print-disabled", self.domain()?])?;
        ensure!(disabled.status.success(), "cannot inspect launchd disabled state");
        let disabled = launchd_disabled(&String::from_utf8_lossy(&disabled.stdout), name)?.unwrap_or(false);
        Ok(Unit { name: name.into(), file: base.join(format!("{name}.plist")),
            installed: base.join(format!("{name}.plist")).is_file() || output.status.success(),
            // Loaded launchd jobs can restart themselves even when currently waiting.
            running: output.status.success(), enabled: !disabled })
    }
}

#[cfg(any(target_os = "macos", test))]
fn launchd_disabled(text: &str, label: &str) -> Result<Option<bool>> {
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once("=>") else { continue; };
        if key.trim().trim_matches('"') != label { continue; }
        return match value.trim() {
            "true" | "disabled" => Ok(Some(true)),
            "false" | "enabled" => Ok(Some(false)),
            value => bail!("unknown launchd disabled state for {label}: {value}"),
        };
    }
    Ok(None)
}

fn property<'a>(text: &'a str, name: &str) -> Result<&'a str> {
    text.lines().find_map(|line| line.split_once('=').filter(|(key, _)| *key == name).map(|(_, value)| value))
        .with_context(|| format!("service manager omitted {name}"))
}
fn checkpoint_idle(worker_enabled: bool, status: &Value) -> Result<()> {
    // Older daemons synthesize pending cuts even when no capture worker is spawned.
    ensure!(!worker_enabled || status.get("pending").is_some_and(Value::is_null),
        "checkpoint is pending or its status is unknown; do not cut over during a checkpoint");
    Ok(())
}


#[cfg(any(target_os = "linux", test))]
fn linux_services(probe: impl FnMut(&str) -> Result<Unit>) -> Result<Services> {
    inspect_services(
        &["st3.service", "smalltalk.service"],
        &[(&["st3-replication.service", "smalltalk-replication.service"], "fractal-replication.service"), (&["smalltalk-apply.service"], "fractal-apply.service")],
        "fractal.service",
        probe,
    )
}

#[cfg(any(target_os = "macos", test))]
fn macos_services(probe: impl FnMut(&str) -> Result<Unit>) -> Result<Services> {
    inspect_services(
        &["com.compoundingtech.st3", "org.nix-community.home.smalltalk"],
        &[(&["com.compoundingtech.st3.replication", "org.nix-community.home.smalltalk-replication"], "com.compoundingtech.fractal.replication"),
            (&["org.nix-community.home.smalltalk-apply"], "com.compoundingtech.fractal-apply")],
        "com.compoundingtech.fractal",
        probe,
    )
}

fn inspect_services(
    legacy_daemons: &[&str],
    auxiliary: &[(&[&str], &str)],
    target_name: &str,
    mut probe: impl FnMut(&str) -> Result<Unit>,
) -> Result<Services> {
    let daemons = legacy_daemons.iter().map(|name| probe(name)).collect::<Result<Vec<_>>>()?;
    let mut legacy: Vec<_> = daemons.iter().filter(|unit| unit.installed).cloned().collect();
    ensure!(legacy.len() <= 1, "multiple legacy daemon definitions exist; resolve which owns the default state before host cutover");
    // Keep a missing primary in the recovery plan after successful retirement.
    if legacy.is_empty() { legacy.push(daemons[0].clone()); }
    let mut target = vec![probe(target_name)?];
    for (aliases, target_name) in auxiliary {
        let mut old = None;
        for name in *aliases {
            let unit = probe(name)?;
            if unit.installed {
                ensure!(old.is_none(),
                    "multiple legacy definitions exist for {target_name}; resolve which owns the worker before host cutover");
                old = Some(unit);
            }
        }
        let new = probe(target_name)?;
        if let Some(old) = old {
            legacy.push(old);
            target.push(new);
        } else if new.running {
            // A stray active target auxiliary is still a preflight conflict.
            target.push(new);
        }
    }
    Ok(Services { legacy, target })
}

impl Manager for Native {
    fn kind(&self) -> &'static str { if cfg!(target_os = "linux") { "systemd-user" } else { "launchd-user" } }
    fn domain(&self) -> Option<&str> { self.launchd_domain.as_deref() }
    fn checkpoint_gate(&self) -> &'static str {
        if self.checkpoint_worker_enabled { "pending-null" } else { "worker-disabled" }
    }
    fn bind_domain(&mut self, recorded: Option<&str>, recovery: bool) -> Result<()> {
        #[cfg(target_os = "macos")]
        { self.launchd_domain = Some(super::launchd::resolve(unsafe { libc::getuid() }, recorded, recovery, |domain| {
            super::launchd::probe(domain, Self::output("launchctl", &["print", domain])?)
        })?); }
        #[cfg(not(target_os = "macos"))]
        let _ = (recorded, recovery);
        Ok(())
    }

    fn inspect(&mut self) -> Result<Services> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
        #[cfg(target_os = "linux")]
        {
            let base = std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from).join("systemd/user");
            linux_services(|name| Self::systemd(name, &base))
        }
        #[cfg(target_os = "macos")]
        {
            let base = home.join("Library/LaunchAgents");
            macos_services(|name| self.launchd(name, &base))
        }
    }

    fn stop(&mut self, unit: &Unit) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            let output = Self::output("systemctl", &["--user", "stop", &unit.name])?;
            if !output.status.success() {
                let status = Self::systemd(&unit.name, unit.file.parent().context("unit has no parent")?)?;
                ensure!(!status.installed && !status.running, "failed stopping {}: {}", unit.name, String::from_utf8_lossy(&output.stderr));
            }
        }
        #[cfg(target_os = "macos")]
        {
            let target = format!("{}/{}", self.domain()?, unit.name);
            let output = Self::output("launchctl", &["bootout", &target])?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let state = Self::output("launchctl", &["print", &target])?;
                if state.status.code() == Some(113) { break; }
                ensure!(output.status.success() && state.status.success() && Instant::now() < deadline,
                    "launchd has not removed {target}; refusing to run two daemons");
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        Ok(())
    }

    fn start(&mut self, unit: &Unit) -> Result<()> {
        #[cfg(target_os = "linux")]
        Self::command("systemctl", &["--user", "start", &unit.name])?;
        #[cfg(target_os = "macos")]
        {
            self.enable(unit, true)?;
            let target = format!("{}/{}", self.domain()?, unit.name);
            let loaded = Self::output("launchctl", &["print", &target])?;
            if !loaded.status.success() {
                ensure!(loaded.status.code() == Some(113), "cannot inspect {target} before bootstrap");
                Self::command("launchctl", &["bootstrap", self.domain()?, unit.file.to_str().context("non-UTF8 launchd plist path")?])?;
            }
            Self::command("launchctl", &["kickstart", &target])?;
        }
        Ok(())
    }

    fn enable(&mut self, unit: &Unit, enabled: bool) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if enabled && unit.name.starts_with("fractal") {
                // HM stages static units (no WantedBy) specifically to prevent activation.
                // Add the boot relationship only at the explicit, committed host cutover.
                return Self::command("systemctl", &["--user", "add-wants", "default.target", &unit.name]);
            }
            let output = Self::output("systemctl", &["--user", if enabled { "enable" } else { "disable" }, &unit.name])?;
            if !output.status.success() {
                let status = Self::systemd(&unit.name, unit.file.parent().context("unit has no parent")?)?;
                ensure!(!enabled && !status.installed && !status.running, "cannot change enablement of {}: {}", unit.name, String::from_utf8_lossy(&output.stderr));
            }
        }
        #[cfg(target_os = "macos")]
        Self::command("launchctl", &[if enabled { "enable" } else { "disable" }, &format!("{}/{}", self.domain()?, unit.name)])?;
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        Self::command("systemctl", &["--user", "daemon-reload"])?;
        Ok(())
    }

    fn protection(&mut self, unit: &Unit) -> Result<Protection> {
        #[cfg(target_os = "linux")]
        {
            let output = Self::output("systemctl", &["--user", "show", &unit.name, "--property=ControlGroup"])?;
            ensure!(output.status.success(), "cannot inspect effective memory protection for {}", unit.name);
            let text = String::from_utf8_lossy(&output.stdout);
            let group = property(&text, "ControlGroup")?;
            ensure!(group.starts_with('/') && group != "/" && !group.split('/').any(|part| part == ".."), "invalid service cgroup path");
            let cgroup = Path::new("/sys/fs/cgroup").join(group.trim_start_matches('/'));
            let swap = fs::read_to_string(cgroup.join("memory.swap.max"))
                .context("read effective daemon memory.swap.max (cgroup v2 required)")?;
            let low = fs::read_to_string(cgroup.join("memory.low"))
                .context("read effective daemon MemoryLow from cgroup memory.low")?;
            Ok(Protection { memory_swap_max: Some(swap.trim().into()), memory_low: Some(low.trim().into()) })
        }
        #[cfg(target_os = "macos")]
        { let _ = unit; Ok(Protection::default()) }
    }

    async fn idle(&mut self) -> Result<()> {
        if self.checkpoint_worker_enabled {
            let checkpoint: Value = self.client.get("/v1/checkpoint/status").await.context("cannot establish checkpoint quiescence")?;
            checkpoint_idle(self.checkpoint_worker_enabled, &checkpoint)?;
        }
        let replication: fractal_daemon::model::ReplicationStatus = self.client.get("/v1/replication/status").await.context("cannot establish sync quiescence")?;
        ensure!(replication.pending_records == 0, "replication has pending records; wait until local admission completes");
        if let Some(first) = replication.first_sync {
            ensure!(first.state == "verified", "first sync is not complete");
        }
        // Peer phases are observations, not a write fence: healthy anti-entropy can
        // stay in exchange indefinitely and idle can immediately start another round.
        // Each received page commits atomically and retries deduplicate envelopes.
        // stop_legacy stops the replication auxiliary before the daemon; swap then
        // fences state/store ownership before moving the complete durable store,
        // including any page admitted after this status read.
        Ok(())
    }

    async fn probe(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let response = tokio::time::timeout(Duration::from_secs(2), self.client.get::<Value>("/v1/health")).await;
            if let Ok(Ok(health)) = response {
                ensure!(health["node"].as_str() == Some(self.node.as_str()), "local API belongs to another node; not accepting this readiness probe");
                return Ok(());
            }
            ensure!(Instant::now() < deadline, "Fractal local API did not listen within 30 seconds (Type=simple is not readiness)");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint_config(enabled: bool, local: bool) -> Result<Config> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("daemon.toml");
        fs::write(&path, format!("[checkpoint]\nenabled = {enabled}\n[trim]\nlocal = {local}\n"))?;
        load_daemon_config(Some(&path))
    }

    #[test]
    fn checkpoint_disabled_accepts_synthesized_pending() -> Result<()> {
        let config = checkpoint_config(false, false)?;
        let native = Native::new(Endpoint::Unix("/unused.sock".into()), "example".into(), &config);
        checkpoint_idle(native.checkpoint_worker_enabled, &serde_json::json!({"pending": {"cut": 42}}))?;
        assert_eq!(native.checkpoint_gate(), "worker-disabled");
        Ok(())
    }

    #[test]
    fn checkpoint_enabled_refuses_pending() -> Result<()> {
        let config = checkpoint_config(true, false)?;
        assert!(checkpoint_idle(config.checkpoint.enabled || config.trim.local,
            &serde_json::json!({"pending": {"cut": 42}})).is_err());
        checkpoint_idle(true, &serde_json::json!({"pending": null}))?;
        assert!(checkpoint_idle(true, &serde_json::json!({})).is_err());
        Ok(())
    }

    #[test]
    fn checkpoint_trim_local_refuses_pending() -> Result<()> {
        let config = checkpoint_config(false, true)?;
        assert!(checkpoint_idle(config.checkpoint.enabled || config.trim.local,
            &serde_json::json!({"pending": {"cut": 42}})).is_err());
        Ok(())
    }

    #[test]
    fn checkpoint_unreadable_config_fails_closed() -> Result<()> {
        let directory = tempfile::tempdir()?;
        assert!(load_daemon_config(Some(directory.path())).is_err());
        assert!(load_daemon_config(Some(&directory.path().join("missing.toml"))).is_err());
        let path = directory.path().join("invalid.toml");
        fs::write(&path, "[checkpoint]\nenabled = 'not a boolean'\n")?;
        assert!(load_daemon_config(Some(&path)).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn replication_gate_accepts_active_or_unobserved_peers_but_requires_local_readiness() -> Result<()> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let directory = tempfile::tempdir()?;
        let socket = directory.path().join("api.sock");
        let listener = tokio::net::UnixListener::bind(&socket)?;
        let mut ready = serde_json::to_value(fractal_daemon::model::ReplicationStatus::default())?;
        ready["peers"] = serde_json::json!([
            {"peer": "birch", "status": "up", "worker": {"phase": "exchange", "last_attempt_at_unix_ms": 1}},
            {"peer": "cedar", "status": "up", "worker": {"phase": "exchange", "last_attempt_at_unix_ms": 1}}
        ]);
        let mut unobserved = ready.clone();
        unobserved["peers"][0]["worker"] = Value::Null;
        let mut pending = ready.clone();
        pending["pending_records"] = 1.into();
        let mut syncing = ready.clone();
        let mut first = serde_json::to_value(smallclaims::replication::ReplicationFirstSync::default())?;
        first["state"] = "syncing".into();
        syncing["first_sync"] = first.clone();
        first["state"] = "verified".into();
        let mut verified = ready.clone();
        verified["first_sync"] = first;
        let server = tokio::spawn(async move {
            for status in [ready, unobserved, pending, syncing, verified] {
                let (mut stream, _) = listener.accept().await?;
                let mut request = [0; 4096];
                let count = stream.read(&mut request).await?;
                ensure!(request[..count].starts_with(b"GET /v1/replication/status "), "unexpected API probe");
                let body = serde_json::to_vec(&serde_json::json!({"api_version": "st3.v1", "value": status}))?;
                stream.write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                ).as_bytes()).await?;
                stream.write_all(&body).await?;
            }
            Ok::<_, anyhow::Error>(())
        });
        let config = checkpoint_config(false, false)?;
        let mut native = Native::new(Endpoint::Unix(socket), "example".into(), &config);
        native.idle().await?;
        native.idle().await?;
        assert!(native.idle().await.unwrap_err().to_string().contains("pending records"));
        assert!(native.idle().await.unwrap_err().to_string().contains("first sync is not complete"));
        native.idle().await?;
        server.await??;
        Ok(())
    }

    #[test]
    fn parses_launchd_disabled_boolean_and_named_states() -> Result<()> {
        let label = "com.compoundingtech.fractal";
        for (value, expected) in [("true", true), ("disabled", true), ("false", false), ("enabled", false)] {
            let text = format!("disabled services = {{\n    \"{label}\" => {value}\n}}\n");
            assert_eq!(launchd_disabled(&text, label)?, Some(expected), "{value}");
        }
        Ok(())
    }

    #[test]
    fn absent_launchd_disabled_override_is_distinct() -> Result<()> {
        assert_eq!(launchd_disabled("disabled services = {\n    \"com.example.other\" => disabled\n}\n",
            "com.compoundingtech.fractal")?, None);
        Ok(())
    }

    #[test]
    fn unknown_launchd_disabled_state_fails_closed() {
        let error = launchd_disabled("disabled services = {\n    \"com.compoundingtech.fractal\" => unknown\n}\n",
            "com.compoundingtech.fractal").unwrap_err();
        assert_eq!(error.to_string(), "unknown launchd disabled state for com.compoundingtech.fractal: unknown");
    }

    fn linux_plan(installed: &[&str], running: &[&str]) -> Result<Services> {
        linux_services(|name| Ok(Unit {
            name: name.into(),
            file: Path::new("/home/example/.config/systemd/user").join(name),
            installed: installed.contains(&name),
            running: running.contains(&name),
            enabled: installed.contains(&name) && !name.starts_with("fractal"),
        }))
    }

    fn macos_plan(installed: &[&str], running: &[&str]) -> Result<Services> {
        macos_services(|name| Ok(Unit {
            name: name.into(),
            file: Path::new("/Users/example/Library/LaunchAgents").join(format!("{name}.plist")),
            installed: installed.contains(&name),
            running: running.contains(&name),
            enabled: installed.contains(&name) && !name.starts_with("com.compoundingtech.fractal"),
        }))
    }

    #[test]
    fn discovers_both_replication_aliases_through_native_macos_plan() -> Result<()> {
        for (daemon, worker) in [
            ("org.nix-community.home.smalltalk", "org.nix-community.home.smalltalk-replication"),
            ("com.compoundingtech.st3", "com.compoundingtech.st3.replication"),
        ] {
            let services = macos_plan(
                &[daemon, worker, "com.compoundingtech.fractal", "com.compoundingtech.fractal.replication"],
                &[daemon, worker],
            )?;
            assert_eq!(services.legacy.iter().map(|unit| unit.name.as_str()).collect::<Vec<_>>(), [daemon, worker]);
            assert_eq!(services.target.iter().map(|unit| unit.name.as_str()).collect::<Vec<_>>(),
                ["com.compoundingtech.fractal", "com.compoundingtech.fractal.replication"]);
            assert!(services.legacy.iter().all(|unit| unit.running));
        }
        Ok(())
    }

    #[test]
    fn installed_macos_replication_aliases_are_ambiguous_even_when_stopped() {
        let error = macos_plan(
            &["org.nix-community.home.smalltalk", "com.compoundingtech.st3.replication", "org.nix-community.home.smalltalk-replication"],
            &[],
        ).unwrap_err();
        assert_eq!(error.to_string(),
            "multiple legacy definitions exist for com.compoundingtech.fractal.replication; resolve which owns the worker before host cutover");
    }

    #[test]
    fn running_macos_target_replication_is_retained_once_without_legacy_worker() -> Result<()> {
        let services = macos_plan(&["org.nix-community.home.smalltalk", "com.compoundingtech.fractal.replication"],
            &["com.compoundingtech.fractal.replication"])?;
        assert_eq!(services.target.iter().filter(|unit| unit.name == "com.compoundingtech.fractal.replication").count(), 1);
        assert!(services.target.iter().find(|unit| unit.name == "com.compoundingtech.fractal.replication").unwrap().running);
        Ok(())
    }

    #[test]
    fn both_macos_legacy_daemon_aliases_remain_a_hard_error() {
        let error = macos_plan(&["com.compoundingtech.st3", "org.nix-community.home.smalltalk"], &[]).unwrap_err();
        assert_eq!(error.to_string(),
            "multiple legacy daemon definitions exist; resolve which owns the default state before host cutover");
    }

    #[test]
    fn discovers_smalltalk_replication_through_native_linux_plan() -> Result<()> {
        let services = linux_plan(
            &["smalltalk.service", "smalltalk-replication.service", "fractal.service", "fractal-replication.service"],
            &["smalltalk.service", "smalltalk-replication.service"],
        )?;
        assert_eq!(services.legacy.iter().map(|unit| unit.name.as_str()).collect::<Vec<_>>(),
            ["smalltalk.service", "smalltalk-replication.service"]);
        assert_eq!(services.target.iter().map(|unit| unit.name.as_str()).collect::<Vec<_>>(),
            ["fractal.service", "fractal-replication.service"]);
        Ok(())
    }

    #[test]
    fn installed_legacy_replication_aliases_are_ambiguous_even_when_stopped() {
        let error = linux_plan(
            &["smalltalk.service", "st3-replication.service", "smalltalk-replication.service", "fractal.service"],
            &[],
        ).unwrap_err();
        assert_eq!(error.to_string(),
            "multiple legacy definitions exist for fractal-replication.service; resolve which owns the worker before host cutover");
    }

    #[test]
    fn running_target_replication_is_retained_once_without_legacy_worker() -> Result<()> {
        let services = linux_plan(&["smalltalk.service", "fractal-replication.service"], &["fractal-replication.service"])?;
        assert_eq!(services.target.iter().filter(|unit| unit.name == "fractal-replication.service").count(), 1);
        assert!(services.target.iter().find(|unit| unit.name == "fractal-replication.service").unwrap().running);
        Ok(())
    }

    #[test]
    fn both_legacy_daemon_aliases_remain_a_hard_error() {
        let error = linux_plan(&["st3.service", "smalltalk.service"], &[]).unwrap_err();
        assert_eq!(error.to_string(),
            "multiple legacy daemon definitions exist; resolve which owns the default state before host cutover");
    }
}
