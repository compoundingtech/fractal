use super::*;
use crate::OwnedSetsStore;
use std::io::{BufRead as _, Read as _, Seek as _, Write as _};
use std::path::PathBuf;

static RUNNING: LazyLock<Mutex<BTreeSet<PathBuf>>> = LazyLock::new(Mutex::default);

struct ReplayGuard(PathBuf);
impl ReplayGuard {
    fn acquire(path: PathBuf) -> Result<Self, ApiError> {
        if !RUNNING.lock().unwrap().insert(path.clone()) {
            return Err(ApiError::bad(FractalError::new(
                "replication-replay-in-progress", "a replication replay is already running",
            )));
        }
        Ok(Self(path))
    }
}
impl Drop for ReplayGuard {
    fn drop(&mut self) { RUNNING.lock().unwrap().remove(&self.0); }
}

#[derive(Deserialize)]
pub struct ReplayRequest {
    #[serde(default)]
    confirm: bool,
}

fn audit(path: &Path, record: &Value) -> anyhow::Result<()> {
    let mut file = OpenOptions::new().create(true).read(true).append(true).mode(0o600).open(path)?;
    if file.metadata()?.len() != 0 {
        file.seek(std::io::SeekFrom::End(-1))?;
        let mut last = [0];
        file.read_exact(&mut last)?;
        if last[0] != b'\n' { file.write_all(b"\n")?; }
    }
    serde_json::to_writer(&mut file, record)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    File::open(path.parent().expect("audit has a parent"))?.sync_all()?;
    Ok(())
}

fn last_duration(path: &Path) -> anyhow::Result<Option<u64>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut duration = None;
    for (line, bytes) in std::io::BufReader::new(file).split(b'\n').enumerate() {
        let bytes = bytes?;
        if bytes.iter().all(u8::is_ascii_whitespace) { continue; }
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(record) => {
                if let Some(measured) = record["duration_ms"].as_u64() { duration = Some(measured); }
            }
            // Audit history is diagnostic, never replay input. Preserve damaged records and
            // warn, but do not make a repair unavailable because an older append was torn.
            Err(error) => tracing::warn!(line = line + 1, %error, "skipping malformed replication replay audit record"),
        }
    }
    Ok(duration)
}

struct AuditAttempt {
    path: PathBuf,
    record: Value,
    timer: Instant,
    completed: bool,
}
impl Drop for AuditAttempt {
    fn drop(&mut self) {
        if self.completed { return; }
        self.record["result"] = json!("aborted");
        self.record["duration_ms"] = json!(self.timer.elapsed().as_millis());
        if let Err(error) = audit(&self.path, &self.record) {
            tracing::error!(%error, "could not persist aborted replication replay audit");
        }
    }
}

pub async fn replay(
    State(state): State<AppState>,
    Extension(transport): Extension<ClientTransportBoundary>,
    Json(request): Json<ReplayRequest>,
) -> Result<Json<Value>, ApiError> {
    if !matches!(transport, ClientTransportBoundary::Unix) {
        return Err(ApiError::bad(FractalError::new(
            "replication-replay-local-only", "replication replay requires the local Unix endpoint",
        )));
    }
    // Admit before any writer-worker or SQLite wait. The worker owns the guard even if the
    // requesting CLI disconnects: another call must never queue behind a running replay.
    let guard = ReplayGuard::acquire(state.state_dir.clone())?;
    let operation = blocking_external(move || -> Result<Json<Value>, ApiError> {
        let _guard = guard;
        let path = state.state_dir.join("replication-replay-audit.jsonl");
        let previous = last_duration(&path).map_err(ApiError::internal)?;
        let claims: u64 = state.store.readers.get().query_row("SELECT count(*) FROM claims", [], |row| row.get(0)).map_err(ApiError::internal)?;
        let blocked_owned_sets: Vec<_> = state.store.owned_sets().map_err(ApiError::internal)?
            .into_iter().filter(|set| !set.blockers.is_empty()).map(|set| set.id).collect();
        let mut warning = "The writer is held for the whole replay. Use a planned window; do not restart the daemon to fix a replay.".to_owned();
        if !blocked_owned_sets.is_empty() {
            warning.push_str(&format!(" Blocked owned sets: {}. Replay removes their retained physical desired and mission-definition rows without rebuilding them while blocked; authoritative claims and set receipts remain.", blocked_owned_sets.join(", ")));
        }
        if !request.confirm {
            return Ok(Json(json!({"confirmed":false,"claim_count":claims,"last_replay_duration_ms":previous,"warning":warning,"blocked_owned_sets":blocked_owned_sets})));
        }
        let actor = state.owning_person.as_deref().ok_or_else(|| ApiError::bad(FractalError::new(
            "replication-replay-owner-required", "configure the local owning person before replaying",
        )))?;
        let started = client_now_ms();
        let record = json!({"actor":actor,"started_at_unix_ms":started,"claim_count":claims,"result":"running"});
        audit(&path, &record).map_err(ApiError::internal)?;
        let mut attempt = AuditAttempt { path, record, timer: Instant::now(), completed: false };
        tracing::warn!(actor, claims, %warning, "replication replay starting; writer held for the whole replay");
        let result = state.store.replay_replication_graph();
        attempt.record["duration_ms"] = json!(attempt.timer.elapsed().as_millis());
        attempt.record["result"] = json!(if result.is_ok() { "healthy" } else { "failed" });
        attempt.record["error"] = json!(result.as_ref().err().map(ToString::to_string));
        audit(&attempt.path, &attempt.record).map_err(ApiError::internal)?;
        attempt.completed = true;
        tracing::warn!(duration_ms = %attempt.record["duration_ms"], result = %attempt.record["result"], "replication replay finished");
        result.map_err(ApiError::internal)?;
        signal_changed(&state);
        Ok(Json(std::mem::take(&mut attempt.record)))
    });
    tokio::pin!(operation);
    loop {
        tokio::select! {
            result = &mut operation => return result?,
            () = tokio::time::sleep(Duration::from_secs(30)) => {
                tracing::warn!("replication replay is still running; do not restart the daemon");
            }
        }
    }
}

#[cfg(test)]
#[path = "replication_replay_tests.rs"]
mod tests;
