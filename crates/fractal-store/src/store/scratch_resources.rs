//! Mission-owned dedicated directories, serialized by the existing host resource writer.
//! Intent commits before filesystem mutation. Inventory is observation only; it never reclaims.
use super::*;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Component, Path};
use rustix::fd::OwnedFd;
use rustix::fs::{Mode, OFlags};
#[cfg(target_os = "linux")]
use rustix::fs::{AtFlags, Dir, FileType};

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

// rustix uses platform-native stat fields: device IDs can be signed on macOS
// while Linux already supplies u64. Keep the checked portable conversion here.
#[allow(clippy::useless_conversion)]
fn stat_identity(stat: &rustix::fs::Stat) -> Result<(u64, u64)> {
    Ok((stat.st_dev.try_into()?, stat.st_ino.try_into()?))
}

fn fd_identity(directory: &OwnedFd) -> Result<(u64, u64)> {
    stat_identity(&rustix::fs::fstat(directory)?)
}

/// Walk from the filesystem root without ever resolving a symlink component.
/// Once opened, renaming any ancestor cannot redirect this directory capability.
fn open_custody_root(record: &Record) -> Result<OwnedFd> {
    let path = Path::new(&record.request.root);
    anyhow::ensure!(path.is_absolute() && path.parent().is_some(), "scratch root must be an absolute dedicated directory");
    anyhow::ensure!(Path::new(&record.handle.path).parent() == Some(path), "scratch object escaped dedicated root");
    let mut directory = rustix::fs::open("/", DIRECTORY_FLAGS, Mode::empty())?;
    for component in path.components() {
        match component {
            Component::RootDir => {},
            Component::Normal(name) => {
                directory = rustix::fs::openat(&directory, name, DIRECTORY_FLAGS, Mode::empty())?;
            }
            _ => anyhow::bail!("noncanonical scratch root"),
        }
    }
    anyhow::ensure!(fd_identity(&directory)? == (record.root_device, record.root_inode), "scratch root identity changed");
    anyhow::ensure!(rustix::fs::fstat(&directory)?.st_mode & 0o077 == 0, "scratch root must be private (0700)");
    Ok(directory)
}

#[cfg(target_os = "linux")]
fn verify_entry(parent: &OwnedFd, name: &std::ffi::CStr, expected: (u64, u64)) -> Result<()> {
    let stat = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    let observed = stat_identity(&stat)?;
    anyhow::ensure!(observed == expected, "scratch entry identity changed before removal");
    Ok(())
}

/// Mutations use only held parent capabilities and single-component names.
/// An entry replaced between stat and open is rejected; O_NOFOLLOW prevents a
/// replacement symlink from becoming a traversal. There is no pathname retry.
#[cfg(target_os = "linux")]
fn remove_contents(directory: &OwnedFd) -> Result<()> {
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." { continue; }
        let stat = rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
        let expected = stat_identity(&stat)?;
        if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
            let child = rustix::fs::openat(directory, name, DIRECTORY_FLAGS, Mode::empty())?;
            anyhow::ensure!(fd_identity(&child)? == expected, "scratch child identity changed before traversal");
            remove_contents(&child)?;
            verify_entry(directory, name, expected)?;
            rustix::fs::unlinkat(directory, name, AtFlags::REMOVEDIR)?;
        } else {
            verify_entry(directory, name, expected)?;
            rustix::fs::unlinkat(directory, name, AtFlags::empty())?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScratchRequest {
    pub id: String,
    pub host: String,
    pub owner: String,
    pub owner_generation: String,
    pub root: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScratchHandle {
    pub id: String,
    pub host: String,
    pub owner: String,
    pub owner_generation: String,
    pub path: String,
    pub generation: String,
    pub authority_epoch: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OwnerObservation { Live, Terminal, Unknown }

/// Positive native-finalization evidence, not a wire-deserializable release token.
#[derive(Clone, Debug)]
pub struct ScratchReleaseProof {
    owner: String,
    owner_generation: String,
}

impl ScratchReleaseProof {
    /// The native mission finalizer must stop intake, finish authored finalizers,
    /// and positively observe the owning run's entire execution tree (cgroup or
    /// scope) terminal. Missing/failed observations are not terminal evidence.
    pub fn after_quiescence(owner: &str, owner_generation: &str) -> Self {
        Self { owner: owner.into(), owner_generation: owner_generation.into() }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScratchInventoryRow {
    pub handle: ScratchHandle,
    pub state: String,
    pub owner_observation: OwnerObservation,
    pub consumer_observation: OwnerObservation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uninspectable: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Record {
    request: ScratchRequest,
    handle: ScratchHandle,
    state: String,
    root_device: u64,
    root_inode: u64,
    object: Option<(u64, u64)>,
    #[serde(default)]
    quarantine: Option<Quarantine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uninspectable: Option<u32>,
}

/// Names and inode receipts live in the allocation's existing custody row.
/// The random, mode-0700 directory is not published to consumers. Keep its empty
/// inode after retirement: unlinking it through the exposed root would recreate
/// the very namespace race this capability boundary removes.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Quarantine {
    name: String,
    entry: String,
    identity: Option<(u64, u64)>,
    phase: String,
    #[serde(default)]
    rejected: Option<(u64, u64)>,
}

#[cfg(target_os = "linux")]
fn entry_identity(parent: &OwnedFd, name: &std::ffi::OsStr) -> Result<Option<(u64, u64)>> {
    match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(stat_identity(&stat)?)),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Journal restoration before either namespace can change. Replay accepts only
/// the recorded inode in exactly one location; disappearance is never release.
#[cfg(target_os = "linux")]
fn restore_allocation(connection: &Connection, record: &mut Record, root: &OwnedFd,
    name: &std::ffi::OsStr, directory: &OwnedFd) -> Result<()> {
    record.quarantine.as_mut().expect("reserved quarantine").phase = "restoring".into();
    write(connection, record)?;
    #[cfg(test)]
    tests::quarantine_boundary("restoring");
    let quarantine = record.quarantine.as_ref().expect("reserved quarantine");
    let entry = std::ffi::OsStr::new(&quarantine.entry);
    let expected = record.object.ok_or_else(|| anyhow::anyhow!("missing allocation identity"))?;
    match (entry_identity(root, name)?, entry_identity(directory, entry)?) {
        (None, Some(observed)) if observed == expected => {
            rustix::fs::renameat_with(directory, entry, root, name, rustix::fs::RenameFlags::NOREPLACE)?;
        }
        (Some(observed), None) if observed == expected => {}
        _ => anyhow::bail!("scratch restoration locations conflict or custody is unknown"),
    }
    rustix::fs::fsync(directory)?;
    rustix::fs::fsync(root)?;
    #[cfg(test)]
    tests::quarantine_boundary("restored");
    record.quarantine.as_mut().expect("reserved quarantine").phase = "prepared".into();
    write(connection, record)
}

#[cfg(target_os = "linux")]
fn retire_quarantined(
    connection: &Connection,
    record: &mut Record,
    root: &OwnedFd,
    name: &std::ffi::OsStr,
    object: Option<OwnedFd>,
) -> Result<bool> {
    use rustix::fs::{renameat_with, RenameFlags};
    if record.quarantine.is_none() {
        // No physical removal is possible before quarantine intent. A missing
        // receipted allocation is unknown, not a legacy deletion shortcut.
        let Some(object) = object.as_ref() else {
            anyhow::ensure!(record.object.is_none(), "receipted allocation disappeared without quarantine; custody unknown");
            rustix::fs::fsync(root)?;
            return Ok(true);
        };
        record.quarantine = Some(Quarantine {
            name: format!(".retire-{}", uuid::Uuid::new_v4()),
            entry: uuid::Uuid::new_v4().to_string(),
            identity: None,
            phase: "planned".into(), rejected: None,
        });
        write(connection, record)?;
        let quarantine = record.quarantine.as_ref().expect("reserved quarantine");
        rustix::fs::mkdirat(root, quarantine.name.as_str(), Mode::RUSR | Mode::WUSR | Mode::XUSR)?;
        let directory = rustix::fs::openat(root, quarantine.name.as_str(), DIRECTORY_FLAGS, Mode::empty())?;
        rustix::fs::fsync(&directory)?;
        rustix::fs::fsync(root)?;
        let quarantine = record.quarantine.as_mut().expect("reserved quarantine");
        quarantine.identity = Some(fd_identity(&directory)?);
        quarantine.phase = "prepared".into();
        // Both identities precede the first rename, including on interruption.
        anyhow::ensure!(record.object == Some(fd_identity(object)?), "allocation receipt changed");
        write(connection, record)?;
    }
    let quarantine = record.quarantine.as_ref().expect("reserved quarantine").clone();
    let expected_directory = quarantine.identity
        .ok_or_else(|| anyhow::anyhow!("interrupted quarantine creation; custody unknown"))?;
    let directory = rustix::fs::openat(root, quarantine.name.as_str(), DIRECTORY_FLAGS, Mode::empty())?;
    anyhow::ensure!(fd_identity(&directory)? == expected_directory, "scratch quarantine identity changed");
    anyhow::ensure!(rustix::fs::fstat(&directory)?.st_mode & 0o077 == 0, "scratch quarantine is not private");
    let expected = record.object.ok_or_else(|| anyhow::anyhow!("missing allocation identity"))?;
    if quarantine.phase == "restoring" {
        restore_allocation(connection, record, root, name, &directory)?;
        return Ok(false);
    }
    let present = match rustix::fs::statat(&directory, quarantine.entry.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Some(stat_identity(&stat)?),
        Err(rustix::io::Errno::NOENT) => None,
        Err(error) => return Err(error.into()),
    };
    if quarantine.phase == "failed" {
        if present.is_some() && present == quarantine.rejected {
            let restored = renameat_with(&directory, quarantine.entry.as_str(), root, name, RenameFlags::NOREPLACE);
            rustix::fs::fsync(&directory)?;
            rustix::fs::fsync(root)?;
            restored?;
        }
        anyhow::bail!("scratch quarantine custody unknown");
    }
    if present.is_none() && quarantine.phase == "prepared" {
        let source = rustix::fs::openat(root, name, DIRECTORY_FLAGS, Mode::empty())?;
        anyhow::ensure!(fd_identity(&source)? == expected, "scratch allocation identity changed");
        let name_c = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(name))?;
        verify_entry(root, &name_c, expected)?;
        #[cfg(test)]
        tests::quarantine_boundary("handoff");
        // Capture whichever inode occupies the basename atomically, then check
        // it *inside* the private directory, before touching its contents.
        renameat_with(root, name, &directory, quarantine.entry.as_str(), RenameFlags::NOREPLACE)?;
        rustix::fs::fsync(root)?;
        rustix::fs::fsync(&directory)?;
        #[cfg(test)]
        tests::quarantine_boundary("moved");
    }
    let candidate = match rustix::fs::statat(&directory, quarantine.entry.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            let observed = stat_identity(&stat)?;
            if observed != expected {
                let receipt = record.quarantine.as_mut().expect("reserved quarantine");
                receipt.phase = "failed".into();
                receipt.rejected = Some(observed);
                write(connection, record)?;
                let restored = renameat_with(&directory, quarantine.entry.as_str(), root, name, RenameFlags::NOREPLACE);
                rustix::fs::fsync(&directory)?;
                rustix::fs::fsync(root)?;
                restored?;
                anyhow::bail!("scratch allocation changed during quarantine handoff");
            }
            Some(rustix::fs::openat(&directory, quarantine.entry.as_str(), DIRECTORY_FLAGS, Mode::empty())?)
        }
        Err(rustix::io::Errno::NOENT) => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(candidate) = candidate {
        anyhow::ensure!(fd_identity(&candidate)? == expected, "quarantined allocation identity changed");
        // The source descriptor is redundant after capture. Only the single
        // verified candidate descriptor is exempt from our same-UID host scan.
        drop(object);
        let scan = super::scratch_consumers::host_consumers(&candidate);
        record.uninspectable = Some(scan.uninspectable());
        if !matches!(scan, super::scratch_consumers::HostScan::Clear { .. }) {
            restore_allocation(connection, record, root, name, &directory)?;
            return Ok(false);
        }
        record.quarantine.as_mut().expect("reserved quarantine").phase = "deleting".into();
        write(connection, record)?;
        #[cfg(test)]
        tests::quarantine_boundary("deleting");
        remove_contents(&candidate)?;
        rustix::fs::fsync(&candidate)?;
        let entry = std::ffi::CString::new(quarantine.entry.as_str())?;
        verify_entry(&directory, &entry, expected)?;
        rustix::fs::unlinkat(&directory, &entry, AtFlags::REMOVEDIR)?;
    } else {
        anyhow::ensure!(quarantine.phase == "deleting" || quarantine.phase == "removed",
            "quarantined allocation disappeared before deletion intent");
        anyhow::ensure!(entry_identity(root, name)?.is_none(),
            "allocation exists outside quarantine after deletion intent; custody unknown");
    }
    rustix::fs::fsync(&directory)?;
    rustix::fs::fsync(root)?;
    record.quarantine.as_mut().expect("reserved quarantine").phase = "removed".into();
    write(connection, record)?;
    #[cfg(test)]
    tests::quarantine_boundary("removed");
    Ok(true)
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    // Every input is bounded before serialization; the serialized record has a hard SQL cap.
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hc855c0885bb7 owner=fractal_store::store::scratch_resources TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS scratch_resources_v1 (
        id TEXT PRIMARY KEY CHECK(length(id)<=4096),
        host TEXT NOT NULL CHECK(length(host)<=4096),
        body TEXT NOT NULL CHECK(length(CAST(body AS BLOB))<=32768));
        CREATE INDEX IF NOT EXISTS scratch_resources_host_v1 ON scratch_resources_v1(host,id);")?;
    Ok(())
}

fn read(connection: &Connection, id: &str) -> Result<Option<Record>> {
    let body: Option<String> = connection.query_row(
        "SELECT body FROM scratch_resources_v1 WHERE id=?1", [id], |row| row.get(0)).optional()?;
    body.map(|body| serde_json::from_str(&body).map_err(Into::into)).transpose()
}

fn write(connection: &Connection, record: &Record) -> Result<()> {
    connection.execute(/* sqlite-large-value: allow-for-now lv-hef0a8dd0d560 owner=fractal_store::store::scratch_resources TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO scratch_resources_v1(id,host,body) VALUES(?1,?2,?3)
        ON CONFLICT(id) DO UPDATE SET body=excluded.body",
        params![record.request.id, record.request.host, serde_json::to_string(record)?])?;
    Ok(())
}

fn epoch(connection: &Connection, host: &str, origin: &str) -> Result<String> {
    anyhow::ensure!(host == origin, "only the scratch host writer may mutate scratch");
    Ok(connection.query_row("SELECT epoch FROM resource_authorities_v1 WHERE host=?1", [host], |row| row.get(0))?)
}

fn identity(path: &Path) -> Result<(u64, u64)> {
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(), "scratch custody is not a directory");
    Ok((metadata.dev(), metadata.ino()))
}

fn private_root(path: &Path, create: bool) -> Result<(u64, u64)> {
    anyhow::ensure!(path.is_absolute() && path.parent().is_some(), "scratch root must be an absolute dedicated directory");
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        anyhow::ensure!(matches!(component, Component::RootDir | Component::Normal(_)), "noncanonical scratch root");
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => anyhow::ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(), "symlink or nondirectory in scratch root"),
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700).create(&current)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::ensure!(fs::canonicalize(path)? == path, "noncanonical scratch root");
    anyhow::ensure!(fs::symlink_metadata(path)?.mode() & 0o077 == 0, "scratch root must be private (0700)");
    identity(path)
}

fn custody(record: &Record) -> Result<()> {
    let root = Path::new(&record.request.root);
    anyhow::ensure!(private_root(root, false)? == (record.root_device, record.root_inode), "scratch root identity changed");
    anyhow::ensure!(Path::new(&record.handle.path).parent() == Some(root), "scratch object escaped dedicated root");
    Ok(())
}

fn ensure_object(record: &mut Record) -> Result<()> {
    custody(record)?;
    let path = Path::new(&record.handle.path);
    if record.state == "intent" {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => fs::File::open(&record.request.root)?.sync_all()?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error.into()),
        }
    }
    let observed = identity(path)?;
    if let Some(expected) = record.object {
        anyhow::ensure!(observed == expected, "scratch object identity changed");
    } else {
        // The random generation was committed before mkdir. Only this host writer can
        // populate the private root; the exact reserved name survives a lost receipt.
        record.object = Some(observed);
    }
    Ok(())
}

fn owner_observation(connection: &Connection, record: &Record) -> Result<OwnerObservation> {
    let run = record.request.owner.strip_prefix("mission-run/").unwrap_or(&record.request.owner);
    let observed: Option<(String, String, String)> = connection.query_row(
        "SELECT status,phase,current_generation_id FROM mission_runs WHERE id=?1", [run],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    let Some((status, phase, generation)) = observed else { return Ok(OwnerObservation::Unknown); };
    let expected = generation_id_from_subject(&record.request.owner_generation);
    if generation != expected { return Ok(OwnerObservation::Unknown); }
    Ok(if matches!(status.as_str(), "completed" | "failed" | "cancelled") && phase == "terminal" {
        OwnerObservation::Terminal
    } else if matches!(status.as_str(), "running" | "standing" | "blocked") && phase != "terminal" {
        OwnerObservation::Live
    } else { OwnerObservation::Unknown })
}


impl Store {
    pub fn acquire_scratch(&self, request: &ScratchRequest) -> Result<ScratchHandle> {
        for value in [&request.id, &request.host, &request.owner, &request.owner_generation, &request.root] {
            anyhow::ensure!(!value.is_empty() && value.len() <= 4096, "invalid scratch request field");
        }
        let mut connection = self.connection.write();
        let authority = epoch(&connection, &request.host, &self.origin)?;
        let owner = request.owner.strip_prefix("mission-run/").unwrap_or(&request.owner);
        let current: Option<(String, String, String, Option<String>)> = connection.query_row(
            "SELECT mission_runs.status,mission_runs.phase,mission_runs.current_generation_id,
                mission_run_deadlines.deadline_at_unix_ms
             FROM mission_runs LEFT JOIN mission_run_deadlines ON mission_run_deadlines.run_id=mission_runs.id
             WHERE mission_runs.id=?1", [owner],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).optional()?;
        let (status, phase, generation, deadline) = current.ok_or_else(|| anyhow::anyhow!("scratch owner unavailable"))?;
        anyhow::ensure!(generation == generation_id_from_subject(&request.owner_generation),
            "stale scratch owner generation");
        anyhow::ensure!(matches!(status.as_str(), "running" | "standing" | "blocked") && phase == "normal",
            "scratch owner is not accepting normal acquisitions");
        if let Some(deadline) = deadline {
            anyhow::ensure!(u128::from(deadline.parse::<u64>()?) > now_ms(), "scratch owner deadline elapsed");
        }
        let mut record = if let Some(record) = read(&connection, &request.id)? {
            anyhow::ensure!(record.request == *request, "scratch immutable specification mismatch");
            anyhow::ensure!(record.state != "released" && record.state != "releasing", "released scratch cannot be reacquired");
            record
        } else {
            let (root_device, root_inode) = private_root(Path::new(&request.root), true)?;
            let generation = uuid::Uuid::new_v4().to_string();
            let record = Record { request: request.clone(), handle: ScratchHandle {
                id: request.id.clone(), host: request.host.clone(), owner: request.owner.clone(),
                owner_generation: request.owner_generation.clone(),
                path: Path::new(&request.root).join(format!("scratch-{generation}")).to_string_lossy().into_owned(),
                generation, authority_epoch: authority.clone(),
            }, state: "intent".into(), root_device, root_inode, object: None, quarantine: None, uninspectable: None };
            let transaction = connection.transaction()?;
            write(&transaction, &record)?;
            transaction.commit()?;
            #[cfg(all(test, target_os = "linux"))]
            tests::interrupt_acquire("intent");
            record
        };
        record.handle.authority_epoch = authority;
        ensure_object(&mut record)?;
        #[cfg(all(test, target_os = "linux"))]
        tests::interrupt_acquire("created");
        record.state = "held".into();
        write(&connection, &record)?;
        Ok(record.handle)
    }

    /// Returns current custody without creating an object, including interrupted intents.
    pub fn scratch_handle(&self, id: &str) -> Result<Option<ScratchHandle>> {
        let connection = self.connection.write();
        let Some(mut record) = read(&connection, id)? else { return Ok(None); };
        record.handle.authority_epoch = epoch(&connection, &record.request.host, &self.origin)?;
        Ok(Some(record.handle))
    }

    pub fn release_scratch(&self, handle: &ScratchHandle, proof: &ScratchReleaseProof) -> Result<bool> {
        let mut connection = self.connection.write();
        let authority = epoch(&connection, &handle.host, &self.origin)?;
        anyhow::ensure!(handle.authority_epoch == authority, "stale scratch authority");
        let mut record = read(&connection, &handle.id)?.ok_or_else(|| anyhow::anyhow!("unknown scratch"))?;
        let mut expected = record.handle.clone();
        expected.authority_epoch = authority;
        anyhow::ensure!(*handle == expected, "stale scratch generation or owner");
        if record.state == "released" { return Ok(true); }
        anyhow::ensure!(proof.owner == record.request.owner
            && proof.owner_generation == record.request.owner_generation, "scratch finalization proof owner mismatch");
        if owner_observation(&connection, &record)? == OwnerObservation::Unknown { return Ok(false); }
        // Only Linux supplies the required atomic no-replace namespace handoff.
        // Other platforms retain custody rather than claiming physical retirement.
        if !cfg!(target_os = "linux") { return Ok(false); }
        let root = open_custody_root(&record)?;
        let name = Path::new(&record.handle.path).file_name()
            .ok_or_else(|| anyhow::anyhow!("scratch object has no basename"))?.to_os_string();
        let object = if record.quarantine.is_some() { None } else { match rustix::fs::openat(&root, &name, DIRECTORY_FLAGS, Mode::empty()) {
            Ok(directory) => {
                let observed = fd_identity(&directory)?;
                if let Some(expected) = record.object {
                    anyhow::ensure!(observed == expected, "scratch object identity changed");
                } else { record.object = Some(observed); }
                Some(directory)
            }
            Err(rustix::io::Errno::NOENT) => {
                anyhow::ensure!(record.state == "intent" || record.state == "releasing", "held scratch disappeared; custody unknown");
                None
            }
            Err(error) => return Err(error.into()),
        }};
        record.handle.authority_epoch = handle.authority_epoch.clone();
        record.state = "releasing".into();
        let transaction = connection.transaction()?;
        write(&transaction, &record)?;
        transaction.commit()?;
        // The verified allocation fd, not its absolute pathname, crosses the
        // durable releasing boundary and the final custody/mutation handoff.
        #[cfg(all(test, target_os = "linux"))]
        tests::before_remove();
        #[cfg(target_os = "linux")]
        if !retire_quarantined(&connection, &mut record, &root, &name, object)? { return Ok(false); }
        record.state = "released".into();
        write(&connection, &record)?;
        Ok(true)
    }

    pub fn has_scratch_for_owner(&self, host: &str, owner: &str, owner_generation: &str) -> Result<bool> {
        Ok(self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM scratch_resources_v1 WHERE host=?1
             AND json_extract(body,'$.request.owner')=?2
             AND json_extract(body,'$.request.ownerGeneration')=?3
             AND json_extract(body,'$.state')<>'released')",
            params![host, owner, owner_generation], |row| row.get(0))?)
    }

    pub fn release_scratch_for_owner(&self, host: &str, owner: &str, owner_generation: &str, proof: &ScratchReleaseProof) -> Result<bool> {
        anyhow::ensure!(proof.owner == owner && proof.owner_generation == owner_generation, "scratch finalization proof owner mismatch");
        let ids = {
            let connection = self.readers.get();
            let mut statement = connection.prepare("SELECT id FROM scratch_resources_v1 WHERE host=?1 AND json_extract(body,'$.request.owner')=?2 AND json_extract(body,'$.request.ownerGeneration')=?3 ORDER BY id")?;
            statement.query_map(params![host, owner, owner_generation], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut released = true;
        for id in ids {
            if let Some(handle) = self.scratch_handle(&id)? { released &= self.release_scratch(&handle, proof)?; }
        }
        Ok(released)
    }

    /// Keyset bounded, read-only inventory over the same authoritative custody rows.
    pub fn scratch_inventory(&self, host: &str, after: Option<&str>, limit: usize) -> Result<Vec<ScratchInventoryRow>> {
        anyhow::ensure!((1..=1000).contains(&limit), "scratch inventory limit must be 1..=1000");
        let connection = self.readers.get();
        let mut statement = connection.prepare("SELECT body FROM scratch_resources_v1 WHERE host=?1 AND id>?2 ORDER BY id LIMIT ?3")?;
        let mut rows = Vec::new();
        for body in statement.query_map(params![host, after.unwrap_or(""), limit], |row| row.get::<_, String>(0))? {
            let record: Record = serde_json::from_str(&body?)?;
            // Inventory does not scan unbounded host execution history or perform native
            // retirement probes. Only a persisted release receipt proves retirement here.
            let consumer_observation = if record.state == "released" {
                OwnerObservation::Terminal
            } else { OwnerObservation::Unknown };
            rows.push(ScratchInventoryRow { owner_observation: owner_observation(&connection, &record)?,
                consumer_observation, handle: record.handle, state: record.state, uninspectable: record.uninspectable });
        }
        Ok(rows)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;

#[cfg(all(test, not(target_os = "linux")))]
#[test]
fn unsupported_physical_retirement_retains_custody() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap().join("dedicated");
    let store = Store::open_memory("host-a").unwrap();
    store.start_resource_authority("host-a", "epoch-a").unwrap();
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h80ef1a553e5e owner=fractal_store::store::scratch_resources TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs
        (id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms)
        VALUES('owner','mission/test','revision','generation-a','revision','owner','workspace/test','person/test','{}','once','running','normal','1','1')", []).unwrap();
    let request = ScratchRequest { id: "scratch/platform".into(), host: "host-a".into(),
        owner: "mission-run/owner".into(), owner_generation: "run-generation/generation-a".into(),
        root: root.to_string_lossy().into_owned() };
    let handle = store.acquire_scratch(&request).unwrap();
    let proof = ScratchReleaseProof::after_quiescence(&handle.owner, &handle.owner_generation);
    assert!(!store.release_scratch(&handle, &proof).unwrap());
    assert!(Path::new(&handle.path).is_dir());
    assert_eq!(read(&store.connection.write(), &handle.id).unwrap().unwrap().state, "held");
}
