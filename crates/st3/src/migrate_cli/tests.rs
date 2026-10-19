use super::*;
use std::collections::BTreeMap;

struct FakeManager {
    services: Services,
    events: Vec<String>,
    idle_calls: usize,
    idle_failure: Option<usize>,
    probe_failure: bool,
    target_protection: Option<Protection>,
    fail_target_enable_once: bool,
    omit_missing: bool,
    fail_legacy_start_once: bool,
}

impl FakeManager {
    fn unit_mut(&mut self, name: &str) -> Result<&mut Unit> {
        self.services.legacy.iter_mut().chain(&mut self.services.target)
            .find(|unit| unit.name == name).context("unknown test service")
    }

    fn changed_services(&self) -> bool {
        self.events.iter().any(|event| ["stop:", "start:", "enable:", "reload"]
            .iter().any(|prefix| event.starts_with(prefix)))
    }
}

impl Manager for FakeManager {
    fn kind(&self) -> &'static str { "fake" }

    fn inspect(&mut self) -> Result<Services> {
        self.events.push("inspect".into());
        let mut services = self.services.clone();
        if self.omit_missing {
            services.legacy.retain(|unit| unit.file.exists());
            services.target.retain(|unit| unit.file.exists());
        }
        Ok(services)
    }

    fn stop(&mut self, unit: &Unit) -> Result<()> {
        self.events.push(format!("stop:{}", unit.name));
        self.unit_mut(&unit.name)?.running = false;
        Ok(())
    }

    fn start(&mut self, unit: &Unit) -> Result<()> {
        self.events.push(format!("start:{}", unit.name));
        if self.fail_legacy_start_once && self.services.legacy.iter().any(|old| old.name == unit.name) {
            self.fail_legacy_start_once = false;
            bail!("injected legacy restart interruption");
        }
        self.unit_mut(&unit.name)?.running = true;
        Ok(())
    }

    fn enable(&mut self, unit: &Unit, enabled: bool) -> Result<()> {
        self.events.push(format!("enable:{}:{enabled}", unit.name));
        if enabled && self.fail_target_enable_once {
            self.fail_target_enable_once = false;
            bail!("injected target enable failure");
        }
        self.unit_mut(&unit.name)?.enabled = enabled;
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        self.events.push("reload".into());
        Ok(())
    }

    fn protection(&mut self, unit: &Unit) -> Result<Protection> {
        self.events.push(format!("protection:{}", unit.name));
        if self.services.target.iter().any(|target| target.name == unit.name) {
            if let Some(protection) = &self.target_protection { return Ok(protection.clone()); }
        }
        Ok(memory_protection())
    }

    async fn idle(&mut self) -> Result<()> {
        self.idle_calls += 1;
        self.events.push("idle".into());
        ensure!(self.idle_failure != Some(self.idle_calls), "checkpoint/sync is not idle");
        Ok(())
    }

    async fn probe(&mut self) -> Result<()> {
        self.events.push("probe".into());
        ensure!(!self.probe_failure, "injected local API probe failure");
        Ok(())
    }
}

fn memory_protection() -> Protection {
    Protection { memory_swap_max: Some("0".into()), memory_low: Some("8589934592".into()) }
}

struct Fixture {
    root: tempfile::TempDir,
    layout: Layout,
    manager: FakeManager,
}

impl Fixture {
    fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let config = root.path().join("config");
        let state = root.path().join("state");
        let units = config.join("systemd/user");
        fs::create_dir_all(&units)?;
        fs::create_dir_all(&state)?;
        let layout = Layout {
            directories: [(config.join("st3"), config.join("fractal")), (state.join("st3"), state.join("fractal"))],
            journal: config.join(".fractal-host-cutover.json"),
            lock: config.join(".fractal-host-cutover.lock"),
        };
        for (old, _) in &layout.directories {
            fs::create_dir_all(old.join("nested"))?;
            fs::write(old.join("nested/authority"), b"preserve these exact bytes\0\xff")?;
            fs::write(old.join(".hidden"), b"hidden state")?;
        }
        // A running daemon already owns this lock file and has its log parent.
        fs::write(layout.directories[1].0.join("node-identity.lock"), b"")?;
        fs::create_dir(layout.directories[1].0.join("logs"))?;
        let mut legacy = Vec::new();
        let mut target = Vec::new();
        let names: &[(&str, &str)] = if cfg!(target_os = "macos") {
            &[
                ("com.compoundingtech.st3", "com.compoundingtech.fractal"),
                ("com.compoundingtech.st3.replication", "com.compoundingtech.fractal.replication"),
            ]
        } else {
            &[
                ("st3.service", "fractal.service"),
                ("st3-replication.service", "fractal-replication.service"),
                ("smalltalk-apply.service", "fractal-apply.service"),
            ]
        };
        for &(old_name, new_name) in names {
            let filename = |name: &str| if cfg!(target_os = "macos") { format!("{name}.plist") } else { name.into() };
            let old = Unit { name: old_name.into(), file: units.join(filename(old_name)), installed: true, running: true, enabled: true };
            let new = Unit { name: new_name.into(), file: units.join(filename(new_name)), installed: true, running: false, enabled: false };
            fs::write(&old.file, format!("legacy {old_name}\n"))?;
            fs::write(&new.file, format!("staged {new_name}\n"))?;
            let directory = dropins(&old.file);
            fs::create_dir_all(directory.join("nested"))?;
            fs::write(directory.join("10-memory.conf"), b"[Service]\nMemorySwapMax=0\nMemoryLow=8G\n")?;
            fs::write(directory.join(".operator-setting"), b"preserve hidden drop-in")?;
            fs::write(directory.join("nested/evidence"), b"preserve whole directory")?;
            legacy.push(old);
            target.push(new);
        }
        let services = Services { legacy, target };
        Ok(Self {
            root, layout,
            manager: FakeManager { services, events: Vec::new(), idle_calls: 0,
                idle_failure: None, probe_failure: false, target_protection: None,
                fail_target_enable_once: false, omit_missing: false, fail_legacy_start_once: false },
        })
    }

    async fn run(&mut self, dry_run: bool, rollback: bool) -> Result<Receipt> {
        cutover(&self.layout, &mut self.manager, "test-host", &HostArgs { dry_run, rollback }).await
    }

    fn snapshot(&self) -> Result<Snapshot> {
        snapshot(self.root.path(), &self.layout.lock)
    }

    fn assert_restored(&self) -> Result<()> {
        for (old, new) in &self.layout.directories {
            assert!(fs::symlink_metadata(old)?.is_dir());
            assert!(!new.exists());
            assert!(!old.join(RECEIPT_FILE).exists());
            assert_eq!(fs::read(old.join("nested/authority"))?, b"preserve these exact bytes\0\xff");
        }
        assert!(self.manager.services.legacy.iter().all(|unit| unit.running && unit.file.exists()));
        assert!(self.manager.services.target.iter().all(|unit| !unit.running && !unit.enabled));
        assert!(!self.layout.journal.exists());
        Ok(())
    }
}

type Snapshot = BTreeMap<PathBuf, (u64, u32, Vec<u8>)>;

fn snapshot(root: &Path, ignored: &Path) -> Result<Snapshot> {
    fn walk(path: &Path, ignored: &Path, result: &mut Snapshot) -> Result<()> {
        if path == ignored { return Ok(()); }
        let metadata = fs::symlink_metadata(path)?;
        let contents = if metadata.file_type().is_symlink() {
            fs::read_link(path)?.as_os_str().as_encoded_bytes().to_vec()
        } else if metadata.is_file() { fs::read(path)? } else { Vec::new() };
        result.insert(path.to_owned(), (metadata.ino(), metadata.permissions().mode(), contents));
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? { walk(&entry?.path(), ignored, result)?; }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    walk(root, ignored, &mut result)?;
    Ok(result)
}

#[tokio::test]
async fn swaps_authority_by_rename_preserving_all_dropins_and_receipting_memory() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let pairs = fixture.layout.pairs(&fixture.manager.services);
    let before = pairs.iter().map(|(old, _)| fs::metadata(old).map(|metadata| metadata.ino()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let receipt = fixture.run(false, false).await?;
    assert_eq!(receipt.status, "completed");
    assert_eq!(receipt.host, "test-host");
    assert_eq!(receipt.migration, MIGRATION_ID);
    assert_eq!(receipt.local_api, "local API listening");
    assert_eq!(receipt.effective, memory_protection());
    let json = serde_json::to_value(&receipt)?;
    assert_eq!(json["effective"]["memory_swap_max"], "0");
    assert_eq!(json["effective"]["memory_low"], "8589934592");
    for (index, (old, new)) in pairs.iter().enumerate() {
        assert_eq!(fs::metadata(new)?.ino(), before[index]);
        if index < 2 {
            assert_eq!(fs::read_link(old)?, *new);
            assert_eq!(fs::read(new.join(RECEIPT_FILE))?, format!("{MIGRATION_ID}\n").as_bytes());
            assert_eq!(fs::read(new.join(".hidden"))?, b"hidden state");
            assert_eq!(fs::read(old.join("nested/authority"))?, b"preserve these exact bytes\0\xff");
        } else {
            assert!(!old.exists());
            assert_eq!(fs::read(new.join("10-memory.conf"))?, b"[Service]\nMemorySwapMax=0\nMemoryLow=8G\n");
            assert_eq!(fs::read(new.join(".operator-setting"))?, b"preserve hidden drop-in");
            assert_eq!(fs::read(new.join("nested/evidence"))?, b"preserve whole directory");
        }
    }
    assert!(fixture.manager.services.legacy.iter().all(|unit| !unit.running && !unit.enabled && !unit.file.exists()));
    assert!(fixture.manager.services.target.iter().all(|unit| unit.running && unit.enabled));
    assert_eq!(fixture.manager.idle_calls, 2);
    let probe = fixture.manager.events.iter().position(|event| event == "probe").unwrap();
    let enable_event = format!("enable:{}:true", fixture.manager.services.target[0].name);
    let enable = fixture.manager.events.iter().position(|event| *event == enable_event).unwrap();
    let journal: Journal = serde_json::from_slice(&fs::read(&fixture.layout.journal)?)?;
    assert_eq!(journal.before, memory_protection());
    assert_eq!(journal.after, Some(memory_protection()));
    assert_eq!(journal.gap_ms, receipt.gap_ms);
    assert!(probe < enable);
    Ok(())
}

#[tokio::test]
async fn dry_run_has_no_filesystem_or_service_mutations() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let before = fixture.snapshot()?;
    let receipt = fixture.run(true, false).await?;
    assert!(receipt.status.starts_with("dry-run"));
    assert_eq!(fixture.snapshot()?, before);
    assert!(!fixture.layout.lock.exists());
    assert!(!fixture.manager.changed_services());
    Ok(())
}

#[tokio::test]
async fn nonempty_fractal_dotfiles_are_rejected_before_stop_even_with_a_receipt() -> Result<()> {
    for with_receipt in [false, true] {
        let mut fixture = Fixture::new()?;
        let destination = &fixture.layout.directories[0].1;
        fs::create_dir(destination)?;
        fs::write(destination.join(".tui-state"), b"unrelated dotfiles fractal state")?;
        if with_receipt { fs::write(destination.join(RECEIPT_FILE), format!("{MIGRATION_ID}\n"))?; }
        let before = fixture.snapshot()?;
        let error = fixture.run(false, false).await.unwrap_err();
        assert!(format!("{error:#}").contains("non-empty"));
        assert_eq!(fixture.snapshot()?, before);
        assert!(!fixture.manager.changed_services());
    }
    Ok(())
}

#[tokio::test]
async fn checkpoint_or_sync_activity_refuses_before_any_stop() -> Result<()> {
    for failure_call in [1, 2] {
        let mut fixture = Fixture::new()?;
        fixture.manager.idle_failure = Some(failure_call);
        let before = fixture.snapshot()?;
        let error = fixture.run(false, false).await.unwrap_err();
        assert!(format!("{error:#}").contains("checkpoint/sync"));
        assert_eq!(fixture.snapshot()?, before);
        let legacy_stops: Vec<_> = fixture.manager.services.legacy.iter()
            .map(|unit| format!("stop:{}", unit.name)).collect();
        assert!(!fixture.manager.events.iter().any(|event| legacy_stops.contains(event)));
        assert!(fixture.manager.services.legacy.iter().all(|unit| unit.running));
    }
    Ok(())
}

#[tokio::test]
async fn rejects_unstaged_enabled_or_running_targets_before_stop() -> Result<()> {
    for invalid in ["not-installed", "missing-definition", "enabled", "running"] {
        let mut fixture = Fixture::new()?;
        match invalid {
            "not-installed" => fixture.manager.services.target[0].installed = false,
            "missing-definition" => fs::remove_file(&fixture.manager.services.target[0].file)?,
            "enabled" => fixture.manager.services.target[0].enabled = true,
            "running" => fixture.manager.services.target[0].running = true,
            _ => unreachable!(),
        }
        let before = fixture.snapshot()?;
        assert!(fixture.run(false, false).await.is_err());
        assert_eq!(fixture.snapshot()?, before);
        assert!(!fixture.manager.changed_services());
    }
    Ok(())
}

#[tokio::test]
async fn both_running_service_families_are_a_hard_error() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.manager.services.target[0].running = true;
    let error = fixture.run(false, false).await.unwrap_err();
    assert!(format!("{error:#}").contains("both run"));
    assert!(!fixture.manager.changed_services());
    Ok(())
}

#[tokio::test]
async fn missing_daemon_families_and_stopped_legacy_are_not_cutovers() -> Result<()> {
    for missing in ["both", "legacy", "target", "legacy-file", "stopped"] {
        let mut fixture = Fixture::new()?;
        match missing {
            "both" => { fixture.manager.services.legacy.clear(); fixture.manager.services.target.clear(); }
            "legacy" => fixture.manager.services.legacy.clear(),
            "target" => fixture.manager.services.target.clear(),
            "legacy-file" => fs::remove_file(&fixture.manager.services.legacy[0].file)?,
            "stopped" => fixture.manager.services.legacy.iter_mut().for_each(|unit| unit.running = false),
            _ => unreachable!(),
        }
        let before = fixture.snapshot()?;
        assert!(fixture.run(false, false).await.is_err());
        assert_eq!(fixture.snapshot()?, before);
        assert!(!fixture.manager.changed_services());
    }
    Ok(())
}

#[tokio::test]
async fn failed_api_probe_rolls_back_every_path_receipt_symlink_and_service() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.manager.probe_failure = true;
    let before = fixture.snapshot()?;
    let error = fixture.run(false, false).await.unwrap_err();
    assert!(format!("{error:#}").contains("local API probe failure"));
    fixture.assert_restored()?;
    assert_eq!(fixture.snapshot()?, before);
    Ok(())
}

#[tokio::test]
async fn changed_effective_memory_protection_rolls_back_before_api_probe() -> Result<()> {
    for protection in [
        Protection { memory_swap_max: Some("max".into()), ..memory_protection() },
        Protection { memory_low: Some("0".into()), ..memory_protection() },
    ] {
        let mut fixture = Fixture::new()?;
        fixture.manager.target_protection = Some(protection);
        let before = fixture.snapshot()?;
        let error = fixture.run(false, false).await.unwrap_err();
        assert!(format!("{error:#}").contains("effective memory"));
        assert!(!fixture.manager.events.iter().any(|event| event == "probe"));
        fixture.assert_restored()?;
        assert_eq!(fixture.snapshot()?, before);
    }
    Ok(())
}

#[tokio::test]
async fn explicit_rollback_recovers_an_interrupted_uncommitted_swap() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let before = fixture.snapshot()?;
    let services = fixture.manager.services.clone();
    let pairs = fixture.layout.pairs(&services);
    for (old, new) in &pairs { fs::rename(old, new)?; }
    for (old, new) in pairs.iter().take(2) {
        fs::write(new.join(RECEIPT_FILE), format!("{MIGRATION_ID}\n"))?;
        symlink(new, old)?;
    }
    let journal = Journal { migration: MIGRATION_ID.into(), services, moves: (0..pairs.len()).collect(),
        pairs, emptied: Vec::new(), receipts: vec![0, 1], links: vec![0, 1], config_fence: false, committed: false,
        before: memory_protection(), after: None, gap_ms: 0 };
    save(&fixture.layout, &journal)?;
    fixture.manager.services.legacy.iter_mut().for_each(|unit| unit.running = false);
    fixture.manager.services.target[0].running = true;
    let error = fixture.run(false, false).await.unwrap_err();
    assert!(format!("{error:#}").contains("interrupted host swap"));
    assert!(!fixture.manager.changed_services());
    let receipt = fixture.run(false, true).await?;
    assert_eq!(receipt.status, "rolled-back");
    fixture.assert_restored()?;
    assert_eq!(fixture.snapshot()?, before);
    Ok(())
}

#[tokio::test]
async fn committed_journal_rerun_finishes_retirement_without_reswapping_state() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.manager.fail_target_enable_once = true;
    let error = fixture.run(false, false).await.unwrap_err();
    assert!(format!("{error:#}").contains("migration committed"));
    let journal: Journal = serde_json::from_slice(&fs::read(&fixture.layout.journal)?)?;
    assert!(journal.committed);
    assert!(fixture.manager.services.legacy.iter().all(|unit| !unit.file.exists()));
    fixture.manager.omit_missing = true;
    fixture.manager.events.clear();
    let before = fixture.snapshot()?;
    let receipt = fixture.run(false, false).await?;
    assert_eq!(receipt.status, "completed");
    assert_eq!(fixture.snapshot()?, before);
    assert!(fixture.manager.services.target.iter().all(|unit| unit.enabled && unit.running));
    assert!(!fixture.manager.events.iter().any(|event| event.starts_with("stop:")));
    assert!(fixture.run(false, true).await.is_err());
    Ok(())
}

#[test]
fn home_manager_launchd_labels_map_to_target_dropin_directories() -> Result<()> {
    let fixture = Fixture::new()?;
    let base = fixture.root.path().join("LaunchAgents");
    let unit = |name: &str| Unit {
        name: name.into(), file: base.join(format!("{name}.plist")),
        installed: true, running: false, enabled: false,
    };
    let services = Services {
        legacy: vec![unit("org.nix-community.home.smalltalk"), unit("org.nix-community.home.smalltalk-apply")],
        target: vec![unit("com.compoundingtech.fractal"), unit("com.compoundingtech.fractal-apply")],
    };
    let pairs = fixture.layout.pairs(&services);
    assert_eq!(pairs.len(), 4);
    for (index, (old, new)) in pairs.iter().skip(2).enumerate() {
        assert_eq!(*old, dropins(&services.legacy[index].file));
        assert_eq!(*new, dropins(&services.target[index].file));
    }
    Ok(())
}

fn interrupted_journal(fixture: &Fixture) -> Journal {
    Journal {
        migration: MIGRATION_ID.into(), services: fixture.manager.services.clone(),
        pairs: fixture.layout.pairs(&fixture.manager.services), moves: Vec::new(),
        emptied: Vec::new(), receipts: Vec::new(), links: Vec::new(), config_fence: false,
        committed: false, before: memory_protection(), after: None, gap_ms: 0,
    }
}

#[tokio::test]
async fn reboot_after_root_exchange_cannot_start_legacy_or_fork_state() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.manager.services.legacy[1].running = false;
    let mut journal = interrupted_journal(&fixture);
    save(&fixture.layout, &journal)?;
    stop_legacy(&mut fixture.manager, &journal)?;
    assert!(fixture.manager.services.legacy.iter().all(|unit| !unit.enabled && !unit.running));
    let first_stop = fixture.manager.events.iter().position(|event| event.starts_with("stop:")).unwrap();
    assert_eq!(first_stop, journal.services.legacy.len());
    journal.moves.push(1);
    journal.links.push(1);
    save(&fixture.layout, &journal)?;
    let (old, new) = &journal.pairs[1];
    symlink(new, new)?;
    exchange(old, new)?;
    // Reboot starts enabled jobs only. No legacy job can recreate a root;
    // immutable generations still resolve the same authority through old.
    assert!(!fixture.manager.services.legacy.iter().any(|unit| unit.enabled));
    assert_eq!(fs::metadata(old)?.ino(), fs::metadata(new)?.ino());
    fixture.run(false, true).await?;
    assert!(fixture.manager.services.legacy.iter().all(|unit| unit.enabled));
    assert!(!fixture.manager.services.legacy[1].running);
    assert_eq!(fs::read(old.join("nested/authority"))?, b"preserve these exact bytes\0\xff");
    Ok(())
}

#[tokio::test]
async fn rollback_replay_after_empty_destination_recreation_and_restart_failure() -> Result<()> {
    let mut fixture = Fixture::new()?;
    for (_, new) in &fixture.layout.directories {
        fs::create_dir(new)?;
        fs::set_permissions(new, fs::Permissions::from_mode(0o750))?;
    }
    fixture.manager.probe_failure = true;
    fixture.manager.fail_legacy_start_once = true;
    let error = fixture.run(false, false).await.unwrap_err();
    assert!(format!("{error:#}").contains("rollback needs repair"));
    let journal: Journal = serde_json::from_slice(&fs::read(&fixture.layout.journal)?)?;
    assert!(journal.moves.is_empty());
    fixture.run(false, true).await?;
    for (old, new) in &fixture.layout.directories {
        assert_eq!(fs::read(old.join("nested/authority"))?, b"preserve these exact bytes\0\xff");
        assert!(fs::read_dir(new)?.next().is_none());
        assert_eq!(fs::metadata(new)?.permissions().mode() & 0o777, 0o750);
    }
    assert!(!fixture.layout.journal.exists());
    Ok(())
}

#[tokio::test]
async fn partial_receipt_publication_is_owned_and_recoverable() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let before = fixture.snapshot()?;
    let mut journal = interrupted_journal(&fixture);
    journal.moves.push(0);
    journal.links.push(0);
    journal.receipts.push(0);
    save(&fixture.layout, &journal)?;
    stop_legacy(&mut fixture.manager, &journal)?;
    let (old, new) = &journal.pairs[0];
    symlink(new, new)?;
    exchange(old, new)?;
    fs::write(new.join(PENDING_RECEIPT), b"arn:partial")?;
    assert!(!new.join(RECEIPT_FILE).exists());
    fixture.run(false, true).await?;
    assert_eq!(fixture.snapshot()?, before);
    Ok(())
}

#[tokio::test]
async fn reserved_source_markers_are_never_claimed_by_rollback() -> Result<()> {
    for name in [PENDING_RECEIPT, RECEIPT_FILE] {
        let mut fixture = Fixture::new()?;
        fs::write(fixture.layout.directories[0].0.join(name), b"foreign")?;
        let before = fixture.snapshot()?;
        assert!(fixture.run(false, false).await.is_err());
        assert_eq!(fixture.snapshot()?, before);
        assert!(!fixture.manager.changed_services());
    }
    Ok(())
}

#[test]
fn atomic_bridge_recovery_handles_both_exchange_crash_boundaries() -> Result<()> {
    for exchanged in [false, true] {
        let fixture = Fixture::new()?;
        let (old, new) = &fixture.layout.directories[0];
        let inode = fs::metadata(old)?.ino();
        symlink(new, new)?;
        if exchanged { exchange(old, new)?; }
        assert_eq!(fs::metadata(old)?.ino(), inode);
        reverse_root(old, new)?;
        reverse_root(old, new)?;
        assert_eq!(fs::metadata(old)?.ino(), inode);
        assert!(fs::symlink_metadata(new).is_err());
    }
    Ok(())
}

#[test]
fn home_manager_config_fence_restores_original_symlink_at_every_publication_boundary() -> Result<()> {
    for boundary in ["intent", "partial", "backing", "staged", "published", "reversed"] {
        let fixture = Fixture::new()?;
        let directory = &fixture.layout.directories[0].0;
        let original = fixture.root.path().join("immutable-config");
        fs::write(&original, b"node = \"test\"\n")?;
        symlink(&original, directory.join("config.toml"))?;
        let inode = fs::symlink_metadata(directory.join("config.toml"))?.ino();
        if matches!(boundary, "published" | "reversed") {
            install_config_fence(directory)?;
            assert_eq!(fs::read_link(directory.join("config.toml"))?, Path::new(CONFIG_BACKING));
            assert!(fs::read_to_string(directory.join(CONFIG_BACKING))?.ends_with(CONFIG_MARKER));
            if boundary == "reversed" {
                exchange(&directory.join("config.toml"), &directory.join(CONFIG_BACKUP))?;
            }
        } else if boundary == "partial" {
            fs::write(directory.join(CONFIG_PENDING), b"partial")?;
        } else if matches!(boundary, "backing" | "staged") {
            fs::write(directory.join(CONFIG_BACKING), b"completed backing")?;
            if boundary == "staged" { symlink(CONFIG_BACKING, directory.join(CONFIG_BACKUP))?; }
        }
        assert!(fs::read(directory.join("config.toml")).is_ok());
        restore_config_fence(directory)?;
        restore_config_fence(directory)?;
        assert_eq!(fs::read_link(directory.join("config.toml"))?, original);
        assert_eq!(fs::symlink_metadata(directory.join("config.toml"))?.ino(), inode);
        assert!(!directory.join(CONFIG_PENDING).exists());
        assert!(!directory.join(CONFIG_BACKING).exists());
    }
    Ok(())
}

#[tokio::test]
async fn home_manager_cutover_retains_fence_and_failure_restores_original_config() -> Result<()> {
    for failed in [false, true] {
        let mut fixture = Fixture::new()?;
        let old = &mut fixture.manager.services.legacy[0];
        let name = if cfg!(target_os = "macos") { "org.nix-community.home.smalltalk" } else { "smalltalk.service" };
        let filename = if cfg!(target_os = "macos") { format!("{name}.plist") } else { name.into() };
        let file = old.file.parent().unwrap().join(filename);
        fs::rename(&old.file, &file)?;
        fs::rename(dropins(&old.file), dropins(&file))?;
        old.name = name.into();
        old.file = file;
        let immutable = fixture.root.path().join("immutable-config");
        fs::write(&immutable, b"node = \"test\"\n")?;
        symlink(&immutable, fixture.layout.directories[0].0.join("config.toml"))?;
        let before = fixture.snapshot()?;
        fixture.manager.probe_failure = failed;
        let result = fixture.run(false, false).await;
        if failed {
            assert!(result.is_err());
            assert_eq!(fixture.snapshot()?, before);
        } else {
            result?;
            let (legacy, target) = &fixture.layout.directories[0];
            assert_eq!(fs::read_link(target.join("config.toml"))?, Path::new(CONFIG_BACKING));
            assert_eq!(fs::read(legacy.join("config.toml"))?, fs::read(target.join(CONFIG_BACKING))?);
            assert!(fs::read_to_string(target.join(CONFIG_BACKING))?.ends_with(CONFIG_MARKER));
            assert!(!target.join(PENDING_RECEIPT).exists());
        }
    }
    Ok(())
}
