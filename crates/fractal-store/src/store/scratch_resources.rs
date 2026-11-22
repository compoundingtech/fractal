//! Mission-owned dedicated directories, serialized by the existing host resource writer.
//! Intent commits before filesystem mutation. Inventory is observation only; it never reclaims.
use super::*;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Component, Path};

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
    /// and positively observe BOTH owned native runtimes and shells empty before
    /// calling this constructor. Missing/failed observations are not emptiness.
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Record {
    request: ScratchRequest,
    handle: ScratchHandle,
    state: String,
    root_device: u64,
    root_inode: u64,
    object: Option<(u64, u64)>,
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    // Every input is bounded before serialization; the serialized record has a hard SQL cap.
    connection.execute_batch("CREATE TABLE IF NOT EXISTS scratch_resources_v1 (
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
    connection.execute("INSERT INTO scratch_resources_v1(id,host,body) VALUES(?1,?2,?3)
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

fn inside(path: &str, root: &str) -> bool {
    let path = Path::new(path);
    if path.starts_with(root) { return true; }
    // Aliased cwd/disk paths must not evade custody checks.
    fs::canonicalize(path).is_ok_and(|path| path.starts_with(root))
}

fn consumers(connection: &Connection, record: &Record) -> Result<OwnerObservation> {
    let mut observation = OwnerObservation::Terminal;
    let mut members = connection.prepare("SELECT body FROM resource_memberships_v1 WHERE host=?1 AND state NOT IN ('retired','refused','cancelled')")?;
    for body in members.query_map([&record.request.host], |row| row.get::<_, String>(0))? {
        let member: Value = serde_json::from_str(&body?)?;
        if member.pointer("/request/diskPath").and_then(Value::as_str).is_some_and(|path| inside(path, &record.handle.path)) {
            if matches!(member.get("state").and_then(Value::as_str), Some("admitted" | "launched")) { return Ok(OwnerObservation::Live); }
            observation = OwnerObservation::Unknown;
        }
    }
    let mut commands = connection.prepare("SELECT body FROM shell_resources_v1 WHERE host=?1 AND state NOT IN ('completed','failed','cancelled','refused')")?;
    for body in commands.query_map([&record.request.host], |row| row.get::<_, String>(0))? {
        let command: Value = serde_json::from_str(&body?)?;
        if command.pointer("/command/cwd").and_then(Value::as_str).is_some_and(|path| inside(path, &record.handle.path)) {
            if matches!(command.get("state").and_then(Value::as_str), Some("running" | "queued")) { return Ok(OwnerObservation::Live); }
            observation = OwnerObservation::Unknown;
        }
    }
    Ok(observation)
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
            }, state: "intent".into(), root_device, root_inode, object: None };
            let transaction = connection.transaction()?;
            write(&transaction, &record)?;
            transaction.commit()?;
            #[cfg(test)]
            tests::interrupt_acquire("intent");
            record
        };
        record.handle.authority_epoch = authority;
        ensure_object(&mut record)?;
        #[cfg(test)]
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
        if consumers(&connection, &record)? != OwnerObservation::Terminal { return Ok(false); }
        custody(&record)?;
        match fs::symlink_metadata(&record.handle.path) {
            Ok(_) => {
                let observed = identity(Path::new(&record.handle.path))?;
                if let Some(expected) = record.object {
                    anyhow::ensure!(observed == expected, "scratch object identity changed");
                } else { record.object = Some(observed); }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                anyhow::ensure!(record.state == "intent" || record.state == "releasing", "held scratch disappeared; custody unknown");
            }
            Err(error) => return Err(error.into()),
        }
        record.handle.authority_epoch = handle.authority_epoch.clone();
        record.state = "releasing".into();
        let transaction = connection.transaction()?;
        write(&transaction, &record)?;
        transaction.commit()?;
        custody(&record)?;
        if Path::new(&record.handle.path).try_exists()? {
            anyhow::ensure!(Some(identity(Path::new(&record.handle.path))?) == record.object,
                "scratch object identity changed before removal");
        }
        match fs::remove_dir_all(&record.handle.path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
        fs::File::open(&record.request.root)?.sync_all()?;
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
                consumer_observation, handle: record.handle, state: record.state });
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests;
