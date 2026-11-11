use super::*;
use std::io::{BufRead as _, Write as _};
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
    let mut file = OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
    serde_json::to_writer(&mut file, record)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    File::open(path.parent().expect("audit has a parent"))?.sync_all()?;
    Ok(())
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
    blocking_external(move || -> Result<Json<Value>, ApiError> {
        let _guard = guard;
        let path = state.state_dir.join("replication-replay-audit.jsonl");
        let previous = match File::open(&path) {
            Ok(file) => {
                let mut duration = None;
                for line in std::io::BufReader::new(file).lines() {
                    let record: Value = serde_json::from_str(&line.map_err(ApiError::internal)?).map_err(ApiError::internal)?;
                    if let Some(measured) = record["duration_ms"].as_u64() { duration = Some(measured); }
                }
                duration
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(ApiError::internal(error)),
        };
        let claims: u64 = state.store.readers.get().query_row("SELECT count(*) FROM claims", [], |row| row.get(0)).map_err(ApiError::internal)?;
        let warning = "The writer is held for the whole replay. Use a planned window; do not restart the daemon to fix a replay.";
        if !request.confirm {
            return Ok(Json(json!({"confirmed":false,"claim_count":claims,"last_replay_duration_ms":previous,"warning":warning})));
        }
        let actor = state.owning_person.as_deref().ok_or_else(|| ApiError::bad(FractalError::new(
            "replication-replay-owner-required", "configure the local owning person before replaying",
        )))?;
        let started = client_now_ms();
        let mut record = json!({"actor":actor,"started_at_unix_ms":started,"claim_count":claims,"result":"running"});
        audit(&path, &record).map_err(ApiError::internal)?;
        tracing::warn!(actor, claims, "replication replay starting; writer held for the whole replay");
        let timer = Instant::now();
        let result = state.store.replay_replication_graph();
        record["duration_ms"] = json!(timer.elapsed().as_millis());
        record["result"] = json!(if result.is_ok() { "healthy" } else { "failed" });
        record["error"] = json!(result.as_ref().err().map(ToString::to_string));
        audit(&path, &record).map_err(ApiError::internal)?;
        tracing::warn!(duration_ms = %record["duration_ms"], result = %record["result"], "replication replay finished");
        result.map_err(ApiError::internal)?;
        signal_changed(&state);
        Ok(Json(record))
    }).await?
}

#[cfg(test)]
#[path = "replication_replay_tests.rs"]
mod tests;
