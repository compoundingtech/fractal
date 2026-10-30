use super::*;

#[derive(Clone, Copy, Debug, clap::ValueEnum, PartialEq, Eq)]
pub(super) enum Phase { Prepare, Commit }

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct Prepared {
    mount: mounts::Mount,
    identity: Identity,
    daemon_suffix: PathBuf,
    #[serde(default)]
    pub(super) begun: bool,
}

// The node pin is the daemon's stable writer/origin identity, not a hostname.
// Device/inode additionally bind that writer to this exact database, not a copy.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Identity { node: String, database_device: u64, database_inode: u64 }

fn identity(state: &Path) -> Result<Identity> {
    ensure!(fs::symlink_metadata(state.join("node-identity.json"))?.is_file(), "adopted node identity must be a regular file");
    let node: String = serde_json::from_slice(&fs::read(state.join("node-identity.json"))?)?;
    ensure!(!node.trim().is_empty() && node.trim() == node, "invalid adopted store node identity");
    let database = fs::symlink_metadata(state.join("claims.sqlite3"))?;
    ensure!(database.is_file() && !database.file_type().is_symlink(), "adopted claims.sqlite3 must be a regular file");
    Ok(Identity { node, database_device: database.dev(), database_inode: database.ino() })
}

fn root_mount(layout: &Layout, at_destination: bool, observed: &[mounts::Mount]) -> Result<mounts::Mount> {
    let (old, new) = &layout.directories[1];
    let old = mounts::resolved(old)?;
    let new = mounts::resolved(new)?;
    let expected = if at_destination { &new } else { &old };
    let mut root = None;
    for mount in observed {
        if mount.path.starts_with(&old) || mount.path.starts_with(&new) {
            ensure!(mount.path == *expected && root.is_none(), "adoption requires exactly one state-root mount and no nested or destination mounts");
            root = Some(mount.clone());
        }
    }
    root.context("adoption requires the state root to be an exact mount point")
}

pub(super) fn prepare(layout: &Layout, observed: &[mounts::Mount]) -> Result<Prepared> {
    let mount = root_mount(layout, false, observed)?;
    let (old, new) = &layout.directories[1];
    ensure!(fs::symlink_metadata(old)?.is_dir(), "adopted legacy state must be a real directory");
    if let Ok(metadata) = fs::symlink_metadata(new) {
        ensure!(metadata.is_dir() && !metadata.file_type().is_symlink()
            && metadata.dev() == fs::metadata(new.parent().context("state has no parent")?)?.dev()
            && fs::read_dir(new)?.next().is_none(), "adoption destination must be absent or an empty ordinary directory");
    } else {
        ensure!(!new.try_exists()?, "adoption destination must be absent or an empty ordinary directory");
    }
    let daemon_suffix = layout.daemon_state.strip_prefix(old).context("adoption requires daemon state under the legacy root")?.to_owned();
    Ok(Prepared { mount, identity: identity(&layout.daemon_state)?, daemon_suffix, begun: false })
}

fn same_mount(prepared: &Prepared, mount: &mounts::Mount) -> Result<()> {
    // Mount IDs change after unmount/remount. Keep the original ID for audit;
    // filesystem source, type, device and subtree must all survive the move.
    ensure!(prepared.mount.source == mount.source && prepared.mount.fstype == mount.fstype
        && prepared.mount.device == mount.device && prepared.mount.root == mount.root,
        "adopted mount identity differs from prepared source/filesystem/device/root");
    Ok(())
}

pub(super) fn validate(layout: &Layout, prepared: &Prepared, observed: &[mounts::Mount], destination: bool) -> Result<()> {
    same_mount(prepared, &root_mount(layout, destination, observed)?)?;
    let root = if destination { &layout.directories[1].1 } else { &layout.directories[1].0 };
    ensure!(fs::symlink_metadata(root)?.is_dir(), "adopted state root must be a real directory");
    let expected = layout.target_daemon_state();
    ensure!(expected.strip_prefix(&layout.directories[1].1)? == prepared.daemon_suffix
        && !prepared.daemon_suffix.components().any(|part| matches!(part, std::path::Component::ParentDir)),
        "adopted daemon state path differs from prepared path");
    ensure!(identity(&root.join(&prepared.daemon_suffix))? == prepared.identity, "adopted store identity differs from prepared writer/database");
    Ok(())
}

pub(super) fn integrity(state: &Path) -> Result<()> {
    let connection = rusqlite::Connection::open_with_flags(state.join("claims.sqlite3"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("open adopted claims.sqlite3 read-only")?;
    let mut query = connection.prepare("PRAGMA integrity_check").context("adopted claims.sqlite3 integrity_check failed")?;
    let mut rows = query.query([]).context("adopted claims.sqlite3 integrity_check failed")?;
    let first = rows.next().context("adopted claims.sqlite3 integrity_check failed")?.context("integrity_check returned no result")?;
    ensure!(first.get::<_, String>(0)? == "ok" && rows.next()?.is_none(), "adopted claims.sqlite3 integrity_check did not return exactly ok");
    Ok(())
}

pub(super) fn steps(layout: &Layout, journal: &Journal, back: bool) -> String {
    let (old, new) = &layout.directories[1];
    let units = journal.services.legacy.iter().map(|unit| unit.name.as_str()).collect::<Vec<_>>().join(" ");
    let (from, to) = if back { (new, old) } else { (old, new) };
    let mount = &journal.adopt.as_ref().expect("adoption journal").mount;
    let from = quote(from.as_os_str().as_bytes());
    let to = quote(to.as_os_str().as_bytes());
    let source = quote(mount.source.as_bytes());
    let remount = if mount.fstype == "zfs" {
        format!("sudo zfs set mountpoint={to} {source}")
    } else {
        format!("mkdir -p {to}\nsudo umount {from}\nsudo mount -t {} {source} {to}", quote(mount.fstype.as_bytes()))
    };
    format!("{}\nsystemctl --user disable --now {units}\n{remount}\nif test -d {from}; then rmdir {from}; fi\nfractal migrate host {}",
        if back { "Rollback paused; legacy units remain stopped until the prepared store is mounted at the legacy path." } else { "Mounted-state adoption prepared; no services or product paths changed." },
        if back { "--rollback" } else { "--adopt-mounted-state commit" })
}

fn quote(value: &[u8]) -> String {
    format!("'{}'", String::from_utf8_lossy(value).replace('\'', "'\\''"))
}
