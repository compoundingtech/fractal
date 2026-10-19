//! Per-host, pre-staged directory/service swap. No copy and no live-store rewrite.
// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result, bail, ensure};
use clap::Args;
use serde::{Deserialize, Serialize};
use st3::client::Endpoint;
use st3::config::Config;
use st3_client::product_dirs::{MIGRATION_ID, RECEIPT_FILE};

mod native;
#[cfg(test)]
mod tests;

#[derive(Args)]
pub(super) struct HostArgs {
    /// Check staging, activity, paths and memory protection without changing anything.
    #[arg(long, conflicts_with = "rollback")]
    dry_run: bool,
    /// Recover an interrupted swap that has not passed its API probe.
    #[arg(long)]
    rollback: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Unit {
    name: String,
    file: PathBuf,
    installed: bool,
    running: bool,
    enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Services {
    // Daemon first; replication and declaration workers follow it.
    legacy: Vec<Unit>,
    target: Vec<Unit>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
struct Protection {
    memory_swap_max: Option<String>,
    memory_low: Option<String>,
}

#[derive(Debug, Serialize)]
struct Receipt {
    migration: &'static str,
    host: String,
    status: &'static str,
    manager: &'static str,
    local_api: &'static str,
    effective: Protection,
    gap_ms: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Layout {
    directories: [(PathBuf, PathBuf); 2],
    journal: PathBuf,
    lock: PathBuf,
}

impl Layout {
    fn discover() -> Result<Self> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
        let config = std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
        let state = std::env::var_os("XDG_STATE_HOME").map_or_else(|| home.join(".local/state"), PathBuf::from);
        ensure!(config.is_absolute() && state.is_absolute(), "XDG config/state directories must be absolute");
        Ok(Self {
            directories: [(config.join("st3"), config.join("fractal")), (state.join("st3"), state.join("fractal"))],
            journal: config.join(".fractal-host-cutover.json"),
            lock: config.join(".fractal-host-cutover.lock"),
        })
    }

    fn pairs(&self, services: &Services) -> Vec<(PathBuf, PathBuf)> {
        let mut pairs = self.directories.to_vec();
        for old in &services.legacy {
            let target_name = if old.name == "smalltalk.service" {
                "fractal.service".into()
            } else if old.name == "smalltalk-apply.service" {
                "fractal-apply.service".into()
            } else {
                old.name.replacen("st3", "fractal", 1)
            };
            if let Some(new) = services.target.iter().find(|new| new.name == target_name) {
                pairs.push((dropins(&old.file), dropins(&new.file)));
            }
        }
        pairs
    }
}

fn dropins(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".d");
    PathBuf::from(name)
}

// Write-ahead undo state survives a killed CLI. Indices refer to the deterministic pairs.
#[derive(Debug, Deserialize, Serialize)]
struct Journal {
    migration: String,
    services: Services,
    pairs: Vec<(PathBuf, PathBuf)>,
    moves: Vec<usize>,
    emptied: Vec<(usize, u32)>,
    receipts: Vec<usize>,
    links: Vec<usize>,
    committed: bool,
    before: Protection,
    #[serde(default)]
    after: Option<Protection>,
    #[serde(default)]
    gap_ms: u128,
}

trait Manager {
    fn kind(&self) -> &'static str;
    fn inspect(&mut self) -> Result<Services>;
    fn stop(&mut self, unit: &Unit) -> Result<()>;
    fn start(&mut self, unit: &Unit) -> Result<()>;
    fn enable(&mut self, unit: &Unit, enabled: bool) -> Result<()>;
    fn reload(&mut self) -> Result<()>;
    fn protection(&mut self, unit: &Unit) -> Result<Protection>;
    async fn idle(&mut self) -> Result<()>;
    async fn probe(&mut self) -> Result<()>;
}

pub(super) async fn run(endpoint: &Endpoint, args: HostArgs, json: bool) -> Result<()> {
    ensure!(st3::instance::current().is_none(), "host migration does not move named instances; run it for the default host");
    ensure!(matches!(endpoint, Endpoint::Unix(_)), "host migration requires the local Unix API, not a remote endpoint");
    let layout = Layout::discover()?;
    let mut config = Config::load_unvalidated(None)?;
    config.apply_fleet_file()?;
    st3::node_identity::resolve(&mut config)?;
    let expected_state = &layout.directories[1].0;
    ensure!(config.state_dir == *expected_state || config.state_dir == layout.directories[1].1,
        "host migration only moves the default XDG st3 state directory; configured state is {}", config.state_dir.display());
    let mut manager = native::Native::new(endpoint.clone(), config.node.clone());
    let receipt = cutover(&layout, &mut manager, &config.node, &args).await?;
    if json {
        super::print_value(&receipt, true)
    } else {
        st3::cli_println!("{}: {} · {} · {}", receipt.host, receipt.status, receipt.local_api, receipt.migration);
        st3::cli_println!("memory.swap.max={} MemoryLow={} gap={}ms manager={}",
            receipt.effective.memory_swap_max.as_deref().unwrap_or("not-applicable (launchd)"),
            receipt.effective.memory_low.as_deref().unwrap_or("not-applicable (launchd)"), receipt.gap_ms, receipt.manager);
        Ok(())
    }
}

fn lock(layout: &Layout) -> Result<File> {
    let file = OpenOptions::new().create(true).truncate(false).write(true).mode(0o600).open(&layout.lock)?;
    // SAFETY: file stays open until the operation (and any rollback) ends.
    ensure!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another host cutover holds {}", layout.lock.display());
    Ok(file)
}

fn save(layout: &Layout, journal: &Journal) -> Result<()> {
    let temporary = layout.journal.with_extension("json.new");
    let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary)?;
    file.write_all(&serde_json::to_vec(journal)?)?;
    file.sync_all()?;
    fs::rename(&temporary, &layout.journal)?;
    File::open(layout.journal.parent().context("journal has no parent")?)?.sync_all()?;
    Ok(())
}

fn destination_free(old: &Path, new: &Path) -> Result<()> {
    let source = fs::symlink_metadata(old).with_context(|| format!("inspect legacy directory {}", old.display()))?;
    ensure!(source.is_dir() && !source.file_type().is_symlink(), "{} must be a real legacy directory", old.display());
    let parent = new.parent().context("destination has no parent")?;
    ensure!(source.dev() == fs::metadata(parent)?.dev(), "{} and {} are on different filesystems; this cutover only performs same-filesystem renames", old.display(), new.display());
    if let Ok(destination) = fs::symlink_metadata(new) {
        ensure!(destination.is_dir() && !destination.file_type().is_symlink(), "destination {} is not a free directory", new.display());
        ensure!(fs::read_dir(new)?.next().is_none(),
            "destination {} is non-empty; refusing to overwrite it{}; move the dotfiles Fractal TUI state to fractalite first if applicable",
            new.display(), if new.join(RECEIPT_FILE).exists() { " (a receipt does not authorize merging two directories)" } else { " and has no migration receipt" });
    }
    Ok(())
}

fn validate_journal(layout: &Layout, journal: &Journal, current: &Services) -> Result<()> {
    ensure!(journal.migration == MIGRATION_ID, "unrecognized host cutover journal");
    let base = current.target.first().and_then(|unit| unit.file.parent()).context("missing target service directory")?;
    #[cfg(target_os = "linux")]
    let allowed = ["st3.service", "smalltalk.service", "st3-replication.service", "smalltalk-apply.service",
        "fractal.service", "fractal-replication.service", "fractal-apply.service"];
    #[cfg(target_os = "macos")]
    let allowed = ["com.compoundingtech.st3", "org.nix-community.home.smalltalk", "com.compoundingtech.st3.replication",
        "org.nix-community.home.smalltalk-apply", "com.compoundingtech.fractal", "com.compoundingtech.fractal.replication", "com.compoundingtech.fractal-apply"];
    ensure!(!journal.services.legacy.is_empty() && !journal.services.target.is_empty(), "journal has no daemon plan");
    for unit in journal.services.legacy.iter().chain(&journal.services.target) {
        let filename = if cfg!(target_os = "macos") { format!("{}.plist", unit.name) } else { unit.name.clone() };
        ensure!(allowed.contains(&unit.name.as_str()) && unit.file == base.join(filename),
            "journal service {} does not match this host", unit.name);
    }
    ensure!(journal.pairs == layout.pairs(&journal.services), "journal paths do not match this host");
    ensure!(journal.moves.iter().chain(journal.emptied.iter().map(|(index, _)| index)).all(|index| *index < journal.pairs.len())
        && journal.links.iter().chain(&journal.receipts).all(|index| *index < 2), "invalid host cutover journal indices");
    Ok(())
}

async fn cutover(layout: &Layout, manager: &mut impl Manager, host: &str, args: &HostArgs) -> Result<Receipt> {
    let _lock = if args.dry_run { None } else { Some(lock(layout)?) };
    let services = manager.inspect()?;
    if layout.journal.exists() {
        let mut journal: Journal = serde_json::from_slice(&fs::read(&layout.journal)?)?;
        validate_journal(layout, &journal, &services)?;
        if args.rollback {
            ensure!(!journal.committed, "the local API probe succeeded; rollback to legacy state is no longer permitted");
            rollback(layout, manager, &journal)?;
            return Ok(receipt(manager, host, "rolled-back", "legacy restored", journal.before, 0));
        }
        ensure!(journal.committed, "an interrupted host swap exists; run `fractal migrate host --rollback` before retrying");
        ensure!(services.target[0].running && !services.legacy.iter().any(|unit| unit.running),
            "committed migration requires the Fractal daemon running and all legacy services stopped; refusing ambiguous API ownership");
        manager.probe().await.context("completed migration's local API is not listening; state was not rolled back")?;
        let protection = manager.protection(&journal.services.target[0])?;
        ensure!(protection == journal.before, "effective memory protection differs from the legacy receipt; migration remains committed");
        if !args.dry_run { finish(manager, &mut journal)?; }
        return Ok(receipt(manager, host, if args.dry_run { "dry-run (already committed)" } else { "completed" }, "local API listening", protection, journal.gap_ms));
    }
    ensure!(!args.rollback, "no interrupted host cutover journal exists");
    ensure!(!services.legacy.is_empty() && !services.target.is_empty(), "legacy and pre-staged Fractal daemon definitions are required");
    ensure!(services.legacy[0].installed, "legacy daemon unit is not installed");
    ensure!(!services.legacy.iter().any(|unit| unit.running) || !services.target.iter().any(|unit| unit.running), "legacy and Fractal services both run; stop and investigate before cutover");
    ensure!(services.legacy[0].running, "legacy daemon must be running so checkpoint/sync quiescence can be checked");
    for unit in &services.target {
        ensure!(unit.installed && !unit.enabled && !unit.running, "{} must be pre-staged, disabled and stopped", unit.name);
    }
    let pairs = layout.pairs(&services);
    for (index, (old, new)) in pairs.iter().enumerate() {
        if index < 2 || old.exists() { destination_free(old, new)?; }
    }
    manager.idle().await?;
    let before = manager.protection(&services.legacy[0])?;
    if args.dry_run {
        return Ok(receipt(manager, host, "dry-run: preflight passed; no changes", "legacy API listening", before, 0));
    }
    let mut journal = Journal { migration: MIGRATION_ID.into(), services, pairs,
        moves: Vec::new(), emptied: Vec::new(), receipts: Vec::new(), links: Vec::new(), committed: false, before, after: None, gap_ms: 0 };
    save(layout, &journal)?;
    let gap = Instant::now();
    let attempt = swap(layout, manager, &mut journal).await;
    let protection = match attempt {
        Ok(protection) => protection,
        Err(error) => {
            return match rollback(layout, manager, &journal) {
                Ok(()) => Err(error).context("host cutover failed before API readiness; legacy paths and services restored"),
                Err(rollback) => Err(error).context(format!("host cutover failed and rollback needs repair: {rollback:#}; journal {} retained", layout.journal.display())),
            };
        }
    };
    // Point of no automatic return: the new daemon has served its local API.
    journal.committed = true;
    journal.after = Some(protection.clone());
    journal.gap_ms = gap.elapsed().as_millis();
    save(layout, &journal).context("local API listening, but commit journal could not be persisted; do not roll back state")?;
    finish(manager, &mut journal).context("local API listening; migration committed, rerun migrate host to finish retiring legacy units")?;
    Ok(receipt(manager, host, "completed", "local API listening", protection, journal.gap_ms))
}

fn receipt(manager: &impl Manager, host: &str, status: &'static str, local_api: &'static str, effective: Protection, gap_ms: u128) -> Receipt {
    Receipt { migration: MIGRATION_ID, host: host.into(), status, manager: manager.kind(), local_api, effective, gap_ms }
}

async fn swap(layout: &Layout, manager: &mut impl Manager, journal: &mut Journal) -> Result<Protection> {
    // Quiescence is checked again immediately before changing service state.
    manager.idle().await?;
    let stopped_at = Instant::now();
    for unit in journal.services.legacy.iter().rev().filter(|unit| unit.running) { manager.stop(unit)?; }
    let state_lock = st3::node_identity::lock_after_stop(&layout.directories[1].0).await
        .context("legacy daemon still owns its state after service stop")?;
    for index in 0..journal.pairs.len() {
        let (old, new) = &journal.pairs[index];
        if !old.exists() { continue; }
        if let Ok(metadata) = fs::symlink_metadata(new) {
            ensure!(metadata.is_dir() && fs::read_dir(new)?.next().is_none(), "destination changed since preflight: {}", new.display());
            journal.emptied.push((index, metadata.permissions().mode()));
            save(layout, journal)?;
            fs::remove_dir(new)?;
        }
        journal.moves.push(index);
        save(layout, journal)?;
        ensure!(!new.exists(), "destination appeared during cutover: {}", new.display());
        fs::rename(old, new).with_context(|| format!("rename {} to {}", old.display(), new.display()))?;
    }
    for index in 0..2 {
        let (old, new) = &journal.pairs[index];
        journal.receipts.push(index);
        save(layout, journal)?;
        fs::write(new.join(RECEIPT_FILE), format!("{MIGRATION_ID}\n"))?;
        journal.links.push(index);
        save(layout, journal)?;
        symlink(new, old)?;
        File::open(new.join(RECEIPT_FILE))?.sync_all()?;
        File::open(new)?.sync_all()?;
    }
    // launchd opens StandardOutPath before exec, so its parent must already exist.
    fs::create_dir_all(layout.directories[1].1.join("logs"))?;
    manager.reload()?;
    ensure!(stopped_at.elapsed() < std::time::Duration::from_secs(210),
        "host swap exhausted its five-minute gap budget before target startup; restoring legacy");
    drop(state_lock);
    manager.start(&journal.services.target[0])?;
    let protection = manager.protection(&journal.services.target[0])?;
    ensure!(protection == journal.before, "effective memory.swap.max/MemoryLow changed across the service swap: before {:?}, after {:?}", journal.before, protection);
    manager.probe().await?;
    Ok(protection)
}

fn rollback(layout: &Layout, manager: &mut impl Manager, journal: &Journal) -> Result<()> {
    for unit in journal.services.target.iter().rev() {
        manager.stop(unit)?;
        manager.enable(unit, false)?;
    }
    for index in journal.links.iter().rev() {
        let (old, new) = &journal.pairs[*index];
        if let Ok(target) = fs::read_link(old) {
            ensure!(target == *new, "refusing to remove unrelated rollback link {}", old.display());
            fs::remove_file(old)?;
        }
    }
    for index in journal.receipts.iter().rev() {
        let path = journal.pairs[*index].1.join(RECEIPT_FILE);
        if path.exists() {
            ensure!(fs::read_to_string(&path)? == format!("{MIGRATION_ID}\n"), "receipt changed during rollback: {}", path.display());
            fs::remove_file(path)?;
        }
    }
    for index in journal.moves.iter().rev() {
        let (old, new) = &journal.pairs[*index];
        if new.exists() {
            ensure!(!old.exists(), "both rollback paths exist: {} and {}", old.display(), new.display());
            fs::rename(new, old)?;
        }
    }
    for (index, mode) in &journal.emptied {
        let new = &journal.pairs[*index].1;
        if !new.exists() {
            fs::create_dir(new)?;
            fs::set_permissions(new, fs::Permissions::from_mode(*mode))?;
        }
    }
    manager.reload()?;
    for unit in journal.services.legacy.iter().filter(|unit| unit.running) {
        manager.start(unit)?;
        manager.enable(unit, unit.enabled)?;
    }
    fs::remove_file(&layout.journal)?;
    Ok(())
}

fn finish(manager: &mut impl Manager, journal: &mut Journal) -> Result<()> {
    for unit in &journal.services.legacy {
        manager.enable(unit, false)?;
        if fs::symlink_metadata(&unit.file).is_ok() { fs::remove_file(&unit.file)?; }
    }
    manager.reload()?;
    for (index, unit) in journal.services.target.iter().enumerate() {
        manager.enable(unit, true)?;
        if index > 0 { manager.start(unit)?; }
    }
    Ok(())
}
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-state-dirs-fractal
