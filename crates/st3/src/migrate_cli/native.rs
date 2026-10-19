//! Direct per-unit probes deliberately bypass ordinary old/new conflict lookup during staging.
use std::process::Output;
use std::time::Duration;

use serde_json::Value;
use st3::client::Client;

use super::*;

pub(super) struct Native {
    client: Client,
    node: String,
}

impl Native {
    pub fn new(endpoint: Endpoint, node: String) -> Self {
        Self { client: Client::new(endpoint), node }
    }

    fn output(program: &str, arguments: &[&str]) -> Result<Output> {
        use std::io::{Read as _, Seek as _};
        use std::process::Stdio;
        // A wedged manager must not leave the host halfway through an unbounded gap.
        // Files avoid pipe-buffer deadlock while the child is polled.
        let mut stdout = tempfile::tempfile()?;
        let mut stderr = tempfile::tempfile()?;
        let mut child = st3::service::host_command(program).args(arguments)
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
    fn domain() -> String { format!("gui/{}", unsafe { libc::getuid() }) }

    #[cfg(target_os = "macos")]
    fn launchd(name: &str, base: &Path) -> Result<Unit> {
        let target = format!("{}/{name}", Self::domain());
        let output = Self::output("launchctl", &["print", &target])?;
        ensure!(output.status.success() || output.status.code() == Some(113), "cannot inspect {target}: {}", String::from_utf8_lossy(&output.stderr));
        let disabled = Self::output("launchctl", &["print-disabled", &Self::domain()])?;
        ensure!(disabled.status.success(), "cannot inspect launchd disabled state");
        let disabled = String::from_utf8_lossy(&disabled.stdout).lines().find_map(|line| {
            let (key, value) = line.trim().split_once(" => ")?;
            (key.trim_matches('"') == name).then_some(value.trim() == "true")
        }).unwrap_or(false);
        Ok(Unit { name: name.into(), file: base.join(format!("{name}.plist")),
            installed: base.join(format!("{name}.plist")).is_file() || output.status.success(),
            // Loaded launchd jobs can restart themselves even when currently waiting.
            running: output.status.success(), enabled: !disabled })
    }
}

fn property<'a>(text: &'a str, name: &str) -> Result<&'a str> {
    text.lines().find_map(|line| line.split_once('=').filter(|(key, _)| *key == name).map(|(_, value)| value))
        .with_context(|| format!("service manager omitted {name}"))
}

impl Manager for Native {
    fn kind(&self) -> &'static str { if cfg!(target_os = "linux") { "systemd-user" } else { "launchd-user" } }

    fn inspect(&mut self) -> Result<Services> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
        #[cfg(target_os = "linux")]
        let (legacy_daemons, auxiliary, base): (&[&str], &[(&str, &str)], PathBuf) = (
            &["st3.service", "smalltalk.service"],
            &[("st3-replication.service", "fractal-replication.service"), ("smalltalk-apply.service", "fractal-apply.service")],
            std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from).join("systemd/user"),
        );
        #[cfg(target_os = "macos")]
        let (legacy_daemons, auxiliary, base): (&[&str], &[(&str, &str)], PathBuf) = (
            &["com.compoundingtech.st3", "org.nix-community.home.smalltalk"],
            &[("com.compoundingtech.st3.replication", "com.compoundingtech.fractal.replication"), ("org.nix-community.home.smalltalk-apply", "com.compoundingtech.fractal-apply")],
            home.join("Library/LaunchAgents"),
        );
        #[cfg(target_os = "linux")]
        let probe = |name| Self::systemd(name, &base);
        #[cfg(target_os = "macos")]
        let probe = |name| Self::launchd(name, &base);
        let daemons = legacy_daemons.iter().map(|name| probe(name)).collect::<Result<Vec<_>>>()?;
        let mut legacy: Vec<_> = daemons.iter().filter(|unit| unit.installed).cloned().collect();
        ensure!(legacy.len() <= 1, "multiple legacy daemon definitions exist; resolve which owns the default state before host cutover");
        // Keep a missing primary in the recovery plan after successful retirement.
        if legacy.is_empty() { legacy.push(daemons[0].clone()); }
        let target_name = if cfg!(target_os = "linux") { "fractal.service" } else { "com.compoundingtech.fractal" };
        let mut target = vec![probe(target_name)?];
        for (old, new) in auxiliary {
            let old = probe(old)?;
            let new = probe(new)?;
            if old.installed {
                legacy.push(old);
                target.push(new);
            } else if new.running {
                // A stray active target auxiliary is still a preflight conflict.
                target.push(new);
            }
        }
        Ok(Services { legacy, target })
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
            let target = format!("{}/{}", Self::domain(), unit.name);
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
            let target = format!("{}/{}", Self::domain(), unit.name);
            let loaded = Self::output("launchctl", &["print", &target])?;
            if !loaded.status.success() {
                ensure!(loaded.status.code() == Some(113), "cannot inspect {target} before bootstrap");
                Self::command("launchctl", &["bootstrap", &Self::domain(), unit.file.to_str().context("non-UTF8 launchd plist path")?])?;
            }
            Self::command("launchctl", &["kickstart", &target])?;
        }
        Ok(())
    }

    fn enable(&mut self, unit: &Unit, enabled: bool) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            let output = Self::output("systemctl", &["--user", if enabled { "enable" } else { "disable" }, &unit.name])?;
            if !output.status.success() {
                let status = Self::systemd(&unit.name, unit.file.parent().context("unit has no parent")?)?;
                ensure!(!enabled && !status.installed && !status.running, "cannot change enablement of {}: {}", unit.name, String::from_utf8_lossy(&output.stderr));
            }
        }
        #[cfg(target_os = "macos")]
        Self::command("launchctl", &[if enabled { "enable" } else { "disable" }, &format!("{}/{}", Self::domain(), unit.name)])?;
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
            return Ok(Protection { memory_swap_max: Some(swap.trim().into()), memory_low: Some(low.trim().into()) });
        }
        #[cfg(target_os = "macos")]
        { let _ = unit; Ok(Protection::default()) }
    }

    async fn idle(&mut self) -> Result<()> {
        let checkpoint: Value = self.client.get("/v1/checkpoint/status").await.context("cannot establish checkpoint quiescence")?;
        ensure!(checkpoint.get("pending").is_some_and(Value::is_null), "checkpoint is pending or its status is unknown; do not cut over during a checkpoint");
        let replication: st3::model::ReplicationStatus = self.client.get("/v1/replication/status").await.context("cannot establish sync quiescence")?;
        ensure!(replication.pending_records == 0, "replication has pending records; wait until sync is idle");
        if let Some(first) = replication.first_sync {
            ensure!(first.state == "verified", "first sync is not complete");
        }
        for peer in replication.peers {
            ensure!(peer.worker.as_ref().is_some_and(|worker| matches!(worker.phase.as_str(), "idle" | "backoff")),
                "sync with {} is active or unobserved; retry after the worker reports idle/backoff", peer.peer);
        }
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
