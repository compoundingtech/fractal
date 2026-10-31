//! Per-host, pre-staged directory/service swap. No copy and no live-store rewrite.
// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-state-dirs-fractal — DELETE at contraction — #3032
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result, bail, ensure};
use clap::Args;
use serde::{Deserialize, Serialize};
use fractal_daemon::client::Endpoint;
use fractal_daemon::config::Config;
use fractal_client::product_dirs::{MIGRATION_ID, RECEIPT_FILE};

mod native;
mod home_manager;
mod mounts;
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
    /// Match the daemon unit's --config override, if any.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Match the daemon unit's --state-dir override, if any.
    #[arg(long)]
    state_dir: Option<PathBuf>,
    /// Match the daemon unit's --store-dir override, if any.
    #[arg(long)]
    store_dir: Option<PathBuf>,
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
    home: PathBuf,
    profiles: Option<Vec<PathBuf>>,
    directories: [(PathBuf, PathBuf); 2],
    // The whole product root moves, but daemon identity/locks may live in store/.
    daemon_state: PathBuf,
    store_dir: Option<PathBuf>,
    journal: PathBuf,
    lock: PathBuf,
}

impl Layout {
    fn discover() -> Result<Self> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
        let config = std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
        let state = std::env::var_os("XDG_STATE_HOME").map_or_else(|| home.join(".local/state"), PathBuf::from);
        ensure!(config.is_absolute() && state.is_absolute(), "XDG config/state directories must be absolute");
        let profiles = std::env::var_os("USER").map(|user| {
            let nix_state = std::env::var_os("NIX_STATE_DIR").map_or_else(|| PathBuf::from("/nix/var/nix"), PathBuf::from);
            let mut profiles = vec![state.join("nix/profiles"), nix_state.join("profiles/per-user").join(user)];
            let default = home.join(".local/state/nix/profiles");
            if profiles[0] != default { profiles.push(default); }
            profiles
        });
        Ok(Self {
            home, profiles,
            directories: [(config.join("st3"), config.join("fractal")), (state.join("st3"), state.join("fractal"))],
            daemon_state: state.join("st3"),
            store_dir: None,
            journal: config.join(".fractal-host-cutover.json"),
            lock: config.join(".fractal-host-cutover.lock"),
        })
    }

    fn configure_state(&mut self, state: &Path) -> Result<()> {
        ensure!(
            !state.components().any(|part| matches!(part, std::path::Component::ParentDir))
                && (state.starts_with(&self.directories[1].0) || state.starts_with(&self.directories[1].1)),
            "host migration requires configured state within the default XDG st3 or Fractal state root; configured state is {}",
            state.display()
        );
        self.daemon_state = state.to_owned();
        Ok(())
    }

    fn target_daemon_state(&self) -> PathBuf {
        let (old, new) = &self.directories[1];
        self.daemon_state.strip_prefix(old).map_or_else(
            |_| self.daemon_state.clone(),
            |suffix| new.join(suffix),
        )
    }

    fn configure_store(&mut self, config: &Config) {
        let store = fractal_daemon::config::store_dir(config);
        self.store_dir = (store != config.state_dir).then_some(store);
    }

    fn target_store_dir(&self) -> Option<PathBuf> {
        self.store_dir.as_ref().map(|store| {
            for (old, new) in &self.directories {
                if let Ok(suffix) = store.strip_prefix(old) { return new.join(suffix); }
            }
            store.clone()
        })
    }

    fn check_store(&self, target: bool) -> Result<()> {
        let store = if target { self.target_store_dir() } else { self.store_dir.clone() };
        let Some(store) = store else { return Ok(()); };
        ensure!(store.is_absolute() && !store.components().any(|part| matches!(part, std::path::Component::ParentDir)),
            "host migration requires an absolute store_dir without parent traversal; configured store_dir is {}", store.display());
        let metadata = fs::symlink_metadata(&store)
            .with_context(|| format!("configured store_dir {} is not reachable; mount or restore it before host migration", store.display()))?;
        ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(),
            "configured store_dir {} must be an ordinary directory or an external mount, not a symlink", store.display());
        check_store_files(&store)?;
        if target { return Ok(()); }
        let resolved = fs::canonicalize(&store)?;
        for (root, destination) in &self.directories {
            ensure!(!store.starts_with(destination) && !resolved.starts_with(mounts::resolved(destination)?),
                "configured store_dir {} is inside the destination product root {}; keep it outside both product roots or in an ordinary directory below the legacy root before cutover", store.display(), destination.display());
            if store.starts_with(root) {
                ensure!(store != *root && resolved == mounts::resolved(root)?.join(store.strip_prefix(root)?),
                    "configured store_dir {} must be an ordinary directory below the product root without symlinked ancestors", store.display());
            } else {
                ensure!(!resolved.starts_with(mounts::resolved(root)?),
                    "configured store_dir {} resolves inside product root {}; configure its ordinary path before host migration", store.display(), root.display());
            }
        }
        Ok(())
    }

    fn lock_store(&self) -> Result<Option<File>> {
        self.store_dir.as_deref().map(|store| {
            let guard = fractal_daemon::node_identity::lock_directory(store)?;
            check_store_files(store)?;
            Ok::<_, anyhow::Error>(guard)
        }).transpose()
            .context("cannot lock configured store directory after legacy service stop; stop its owner or restore the directory before host migration")
    }

    fn pairs(&self, services: &Services) -> Result<Vec<(PathBuf, PathBuf)>> {
        let mut pairs = self.directories.to_vec();
        for old in &services.legacy {
            let target_name = if old.name == "smalltalk.service" {
                "fractal.service".into()
            } else if old.name == "smalltalk-replication.service" {
                "fractal-replication.service".into()
            } else if old.name == "smalltalk-apply.service" {
                "fractal-apply.service".into()
            } else if old.name == "org.nix-community.home.smalltalk" {
                "com.compoundingtech.fractal".into()
            } else if old.name == "org.nix-community.home.smalltalk-apply" {
                "com.compoundingtech.fractal-apply".into()
            } else {
                old.name.replacen("st3", "fractal", 1)
            };
            if let Some(new) = services.target.iter().find(|new| new.name == target_name) {
                pairs.push((dropins(&old.file), dropins(&new.file)));
                if let (Some(old), Some(new)) = (control_dropins(&old.file), control_dropins(&new.file)) {
                    let source = control_directory_exists(&old)?;
                    let destination = control_directory_exists(&new)?;
                    if source || destination { pairs.push((old, new)); }
                }
            }
        }
        Ok(pairs)
    }
}

fn check_store_files(directory: &Path) -> Result<()> {
    for name in fractal_daemon::config::STORE_DATABASE_FILES {
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(!metadata.file_type().is_symlink(),
                "configured store database file {} must not be a symlink", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect configured store database file before host migration"),
        }
    }
    Ok(())
}

fn dropins(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".d");
    PathBuf::from(name)
}

fn control_dropins(file: &Path) -> Option<PathBuf> {
    let unit_directory = file.parent()?;
    let systemd = unit_directory.parent()?;
    if unit_directory.file_name()? != "user" || systemd.file_name()? != "systemd" { return None; }
    let mut filename = file.file_name()?.to_os_string();
    filename.push(".d");
    Some(systemd.join("user.control").join(filename))
}

fn control_directory_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(),
                "{} must be an ordinary systemd control drop-in directory", path.display());
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("inspect systemd control drop-ins {}", path.display())),
    }
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
    #[serde(default)]
    config_fence: bool,
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
    ensure!(fractal_daemon::instance::current().is_none(), "host migration does not move named instances; run it for the default host");
    ensure!(matches!(endpoint, Endpoint::Unix(_)), "host migration requires the local Unix API, not a remote endpoint");
    let mut layout = Layout::discover()?;
    let mut config = Config::load_unvalidated(args.config.as_deref())?;
    if let Some(state) = &args.state_dir { config.state_dir = state.clone(); }
    if let Some(store) = &args.store_dir { config.store_dir = Some(store.clone()); }
    layout.configure_state(&config.state_dir)?;
    layout.configure_store(&config);
    config.apply_fleet_file()?;
    fractal_daemon::node_identity::resolve(&mut config)?;
    let mut manager = native::Native::new(endpoint.clone(), config.node.clone());
    let receipt = cutover(&layout, &mut manager, &config.node, &args).await?;
    if json {
        super::print_value(&receipt, true)
    } else {
        fractal_daemon::cli_println!("{}: {} · {} · {}", receipt.host, receipt.status, receipt.local_api, receipt.migration);
        fractal_daemon::cli_println!("memory.swap.max={} MemoryLow={} gap={}ms manager={}",
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

const PENDING_RECEIPT: &str = ".fractal-migration-completed.pending";

fn publish_receipt(directory: &Path) -> Result<()> {
    let pending = directory.join(PENDING_RECEIPT);
    let mut file = OpenOptions::new().create_new(true).write(true).mode(0o600).open(&pending)?;
    file.write_all(format!("{MIGRATION_ID}\n").as_bytes())?;
    file.sync_all()?;
    fs::rename(&pending, directory.join(RECEIPT_FILE))?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

const CONFIG_BACKING: &str = "config.home-manager.toml";
const CONFIG_BACKUP: &str = ".fractal-config-before-cutover";
const CONFIG_PENDING: &str = ".fractal-config.pending";
const CONFIG_MARKER: &str = "\n# arn:lmig:smalltalk:2026-10-16-service-labels-fractal Home Manager legacy-generation fence\n";

fn home_manager(services: &Services) -> bool {
    matches!(services.legacy[0].name.as_str(), "smalltalk.service" | "org.nix-community.home.smalltalk")
}

fn install_config_fence(directory: &Path) -> Result<()> {
    let mut content = fs::read(directory.join("config.toml"))?;
    content.extend_from_slice(CONFIG_MARKER.as_bytes());
    let mut file = OpenOptions::new().create_new(true).write(true).mode(0o600).open(directory.join(CONFIG_PENDING))?;
    file.write_all(&content)?;
    file.sync_all()?;
    fs::rename(directory.join(CONFIG_PENDING), directory.join(CONFIG_BACKING))?;
    symlink(CONFIG_BACKING, directory.join(CONFIG_BACKUP))?;
    File::open(directory)?.sync_all()?;
    // Neither a crash nor an immutable activation can observe config.toml
    // absent: exchange its original object with the already-published fence.
    exchange(&directory.join("config.toml"), &directory.join(CONFIG_BACKUP))?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

fn restore_config_fence(directory: &Path) -> Result<()> {
    let backup = directory.join(CONFIG_BACKUP);
    if fs::symlink_metadata(&backup).is_ok() {
        if fs::read_link(&backup).ok().as_deref() != Some(Path::new(CONFIG_BACKING)) {
            let config = directory.join("config.toml");
            ensure!(fs::read_link(&config)? == Path::new(CONFIG_BACKING), "config fence changed during rollback");
            exchange(&config, &backup)?;
            File::open(directory)?.sync_all()?;
        }
        // A staged fence means either forward exchange has not happened or
        // reverse exchange already completed before interruption.
        ensure!(fs::read_link(&backup)? == Path::new(CONFIG_BACKING), "invalid staged config fence");
        fs::remove_file(backup)?;
    }
    for name in [CONFIG_BACKING, CONFIG_PENDING] {
        if fs::symlink_metadata(directory.join(name)).is_ok() { fs::remove_file(directory.join(name))?; }
    }
    File::open(directory)?.sync_all()?;
    Ok(())
}

fn exchange(old: &Path, new: &Path) -> Result<()> {
    let old = std::ffi::CString::new(old.as_os_str().as_bytes())?;
    let new = std::ffi::CString::new(new.as_os_str().as_bytes())?;
    // SAFETY: both C strings remain alive; exchange is atomic on the same filesystem.
    #[cfg(target_os = "linux")]
    let result = unsafe { libc::renameat2(libc::AT_FDCWD, old.as_ptr(), libc::AT_FDCWD, new.as_ptr(), libc::RENAME_EXCHANGE) };
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(old.as_ptr(), new.as_ptr(), libc::RENAME_SWAP) };
    if result != 0 { return Err(std::io::Error::last_os_error()).context("atomic directory/bridge exchange"); }
    Ok(())
}

fn reverse_root(old: &Path, new: &Path) -> Result<()> {
    if let Ok(target) = fs::read_link(old) {
        ensure!(target == new && fs::symlink_metadata(new)?.is_dir(), "invalid rollback bridge");
        exchange(old, new)?;
    }
    if let Ok(target) = fs::read_link(new) {
        ensure!(target == new && fs::symlink_metadata(old)?.is_dir(), "invalid staged bridge");
        fs::remove_file(new)?;
    } else if new.exists() {
        ensure!(!old.exists(), "both rollback roots exist");
        fs::rename(new, old)?;
    }
    File::open(old.parent().context("root has no parent")?)?.sync_all()?;
    Ok(())
}

fn destination_free(old: &Path, new: &Path) -> Result<()> {
    let source = fs::symlink_metadata(old).with_context(|| format!("inspect legacy directory {}", old.display()))?;
    ensure!(source.is_dir() && !source.file_type().is_symlink(), "{} must be a real legacy directory", old.display());
    let parent = new.parent().context("destination has no parent")?;
    ensure!(source.dev() == fs::metadata(parent)?.dev(), "{} and {} are on different filesystems; this cutover only performs same-filesystem renames", old.display(), new.display());
    let destination = match fs::symlink_metadata(new) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("inspect destination {}", new.display())),
    };
    ensure!(destination.is_dir() && !destination.file_type().is_symlink(), "destination {} is not a free directory", new.display());
    ensure!(fs::read_dir(new)?.next().is_none(),
        "destination {} is non-empty; refusing to overwrite it{}; move the dotfiles Fractal TUI state to fractalite first if applicable",
        new.display(), if new.join(RECEIPT_FILE).exists() { " (a receipt does not authorize merging two directories)" } else { " and has no migration receipt" });
    Ok(())
}

fn validate_journal(layout: &Layout, journal: &Journal, current: &Services) -> Result<()> {
    ensure!(journal.migration == MIGRATION_ID, "unrecognized host cutover journal");
    let base = current.target.first().and_then(|unit| unit.file.parent()).context("missing target service directory")?;
    #[cfg(target_os = "linux")]
    let allowed = ["st3.service", "smalltalk.service", "st3-replication.service", "smalltalk-replication.service", "smalltalk-apply.service",
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
    ensure!(journal.pairs == layout.pairs(&journal.services)?, "journal paths do not match this host");
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
            mounts::rollback_preflight(&journal.pairs)?;
            ensure!(!journal.committed, "the local API probe succeeded; rollback to legacy state is no longer permitted");
            rollback(layout, manager, &mut journal)?;
            return Ok(receipt(manager, host, "rolled-back", "legacy restored", journal.before, 0));
        }
        ensure!(journal.committed, "an interrupted host swap exists; run `fractal migrate host --rollback` before retrying");
        mounts::committed_preflight(&journal.pairs, layout.target_store_dir().as_deref())?;
        layout.check_store(true)?;
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
    ensure!(services.legacy.iter().all(|unit| unit.file.is_file()),
        "legacy definitions must be present in the user's service directory; global or orphaned loaded units need operator repair before cutover");
    for unit in &services.target {
        ensure!(unit.installed && unit.file.is_file() && !unit.enabled && !unit.running, "{} must be pre-staged, disabled and stopped", unit.name);
    }
    let pairs = layout.pairs(&services)?;
    mounts::preflight(&pairs)?;
    layout.check_store(false)?;
    for (index, (old, new)) in pairs.iter().enumerate() {
        if index < 2 || old.exists() { destination_free(old, new)?; }
        if index < 2 {
            for name in [RECEIPT_FILE, PENDING_RECEIPT] {
                ensure!(fs::symlink_metadata(old.join(name)).is_err(), "legacy source contains reserved migration marker {name}");
            }
        }
    }
    if home_manager(&services) {
        home_manager::preflight(layout)?;
        let config = &layout.directories[0].0;
        for name in [CONFIG_BACKING, CONFIG_BACKUP, CONFIG_PENDING] {
            ensure!(fs::symlink_metadata(config.join(name)).is_err(), "legacy config contains reserved migration file {name}");
        }
        fs::read(config.join("config.toml")).context("read Home Manager config before fencing")?;
    }
    manager.idle().await?;
    let before = manager.protection(&services.legacy[0])?;
    if args.dry_run {
        return Ok(receipt(manager, host, "dry-run: preflight passed; no changes", "legacy API listening", before, 0));
    }
    let mut journal = Journal { migration: MIGRATION_ID.into(), services, pairs,
        moves: Vec::new(), emptied: Vec::new(), receipts: Vec::new(), links: Vec::new(), config_fence: false, committed: false, before, after: None, gap_ms: 0 };
    save(layout, &journal)?;
    let attempt = swap(layout, manager, &mut journal).await;
    let (protection, gap_ms) = match attempt {
        Ok(result) => result,
        Err(error) => {
            return match rollback(layout, manager, &mut journal) {
                Ok(()) => Err(error).context("host cutover failed before API readiness; legacy paths and services restored"),
                Err(rollback) => Err(error).context(format!("host cutover failed and rollback needs repair: {rollback:#}; journal {} retained", layout.journal.display())),
            };
        }
    };
    // Point of no automatic return: the new daemon has served its local API.
    journal.committed = true;
    journal.after = Some(protection.clone());
    journal.gap_ms = gap_ms;
    save(layout, &journal).context("local API listening, but commit journal could not be persisted; do not roll back state")?;
    finish(manager, &mut journal).context("local API listening; migration committed, rerun migrate host to finish retiring legacy units")?;
    Ok(receipt(manager, host, "completed", "local API listening", protection, journal.gap_ms))
}

fn receipt(manager: &impl Manager, host: &str, status: &'static str, local_api: &'static str, effective: Protection, gap_ms: u128) -> Receipt {
    Receipt { migration: MIGRATION_ID, host: host.into(), status, manager: manager.kind(), local_api, effective, gap_ms }
}

fn stop_legacy(manager: &mut impl Manager, journal: &Journal) -> Result<()> {
    // The original boot/login enablement is already durable in the journal.
    // Disable before stop/rename so a reboot cannot recreate an empty st3 root.
    for unit in &journal.services.legacy { manager.enable(unit, false)?; }
    for unit in journal.services.legacy.iter().rev().filter(|unit| unit.running) { manager.stop(unit)?; }
    Ok(())
}

async fn swap(layout: &Layout, manager: &mut impl Manager, journal: &mut Journal) -> Result<(Protection, u128)> {
    // Quiescence is checked again immediately before changing service state.
    manager.idle().await?;
    let stopped_at = Instant::now();
    stop_legacy(manager, journal)?;
    let state_lock = fractal_daemon::node_identity::lock_after_stop(&layout.daemon_state).await
        .context("legacy daemon still owns its state after service stop")?;
    // Directory-FD ownership follows a nested store's inode across the root exchange.
    let store_lock = layout.lock_store()?;
    if home_manager(&journal.services) {
        journal.config_fence = true;
        save(layout, journal)?;
        install_config_fence(&layout.directories[0].0)?;
    }
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
        if index < 2 { journal.links.push(index); }
        save(layout, journal)?;
        ensure!(fs::symlink_metadata(new).is_err(), "destination appeared during cutover: {}", new.display());
        if index < 2 {
            // A staged self-link becomes the legacy bridge in one atomic swap:
            // old config/state is never absent, even across a crash or reboot.
            symlink(new, new)?;
            File::open(old.parent().context("source has no parent")?)?.sync_all()?;
            exchange(old, new)?;
        } else {
            fs::rename(old, new).with_context(|| format!("rename {} to {}", old.display(), new.display()))?;
        }
        File::open(old.parent().context("source has no parent")?)?.sync_all()?;
    }
    for index in 0..2 {
        let (_, new) = &journal.pairs[index];
        journal.receipts.push(index);
        save(layout, journal)?;
        publish_receipt(new)?;
    }
    layout.check_store(true)?;
    // launchd opens StandardOutPath before exec, so its parent must already exist.
    fs::create_dir_all(layout.target_daemon_state().join("logs"))?;
    manager.reload()?;
    // Reserve bounded target start/protection/probe calls, including launchd's
    // enable + print + bootstrap + kickstart, inside the five-minute budget.
    let pre_start_budget = if cfg!(target_os = "macos") { 140 } else { 200 };
    ensure!(stopped_at.elapsed() < std::time::Duration::from_secs(pre_start_budget),
        "host swap exhausted its five-minute gap budget before target startup; restoring legacy");
    drop(store_lock);
    drop(state_lock);
    manager.start(&journal.services.target[0])?;
    let protection = manager.protection(&journal.services.target[0])?;
    ensure!(protection == journal.before, "effective memory.swap.max/MemoryLow changed across the service swap: before {:?}, after {:?}", journal.before, protection);
    manager.probe().await?;
    Ok((protection, stopped_at.elapsed().as_millis()))
}

fn rollback(layout: &Layout, manager: &mut impl Manager, journal: &mut Journal) -> Result<()> {
    mounts::rollback_preflight(&journal.pairs)?;
    for unit in journal.services.target.iter().rev() {
        manager.stop(unit)?;
        manager.enable(unit, false)?;
    }
    for index in journal.receipts.iter().rev() {
        let path = journal.pairs[*index].1.join(RECEIPT_FILE);
        let pending = journal.pairs[*index].1.join(PENDING_RECEIPT);
        if pending.exists() { fs::remove_file(pending)?; }
        if path.exists() {
            ensure!(fs::read_to_string(&path)? == format!("{MIGRATION_ID}\n"), "receipt changed during rollback: {}", path.display());
            fs::remove_file(path)?;
        }
    }
    while let Some(index) = journal.moves.last().copied() {
        let (old, new) = &journal.pairs[index];
        if index < 2 {
            reverse_root(old, new)?;
        } else if new.exists() {
            ensure!(!old.exists(), "both rollback paths exist: {} and {}", old.display(), new.display());
            fs::rename(new, old)?;
        }
        File::open(old.parent().context("source has no parent")?)?.sync_all()?;
        // Persist reverse progress before recreating an empty destination.
        // If killed after rename but before save, new is absent on replay.
        journal.moves.pop();
        save(layout, journal)?;
    }
    if journal.config_fence { restore_config_fence(&layout.directories[0].0)?; }
    for (index, mode) in &journal.emptied {
        let new = &journal.pairs[*index].1;
        if !new.exists() {
            fs::create_dir(new)?;
            fs::set_permissions(new, fs::Permissions::from_mode(*mode))?;
        }
    }
    manager.reload()?;
    for unit in &journal.services.legacy {
        if unit.running { manager.start(unit)?; }
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
