//! The response stream owns export cancellation, independently of header admission.
//! Spooling completes before downloading, so backpressure never pins the SQLite snapshot.
use axum::body::Body;
use axum::response::{IntoResponse, Response};
use smallclaims::read_budget::{self, ReadBudget};
use tokio::io::AsyncReadExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex};
use super::super::ApiError;

static EXPORTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));

fn reserve_from(pool: &Arc<Semaphore>) -> Result<OwnedSemaphorePermit, ApiError> {
    pool.clone().try_acquire_owned().map_err(|_| ApiError::bad(crate::model::FractalError::new(
        "backup-busy", "another backup export is preparing or downloading; let it finish or cancel it",
    )))
}

pub(super) fn reserve() -> Result<OwnedSemaphorePermit, ApiError> {
    reserve_from(&EXPORTS)
}

struct Cancel(ReadBudget);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

enum Export {
    Preparing(tokio::task::JoinHandle<anyhow::Result<std::fs::File>>),
    Reading(tokio::fs::File),
    Finished,
}

struct Stream {
    export: Export,
    _cancel: Cancel,
    _permit: Arc<OwnedSemaphorePermit>,
}

pub(super) fn response(
    permit: OwnedSemaphorePermit,
    prepare: impl FnOnce() -> anyhow::Result<std::fs::File> + Send + 'static,
) -> Response {
    let cancellation = ReadBudget::cancellable("/v1/backup/stream");
    let worker_budget = cancellation.clone();
    let permit = Arc::new(permit);
    let worker_permit = permit.clone();
    let worker = tokio::task::spawn_blocking(move || {
        // A cancelled body cannot free the export slot until its worker has
        // actually released the reader and temporary archive.
        let _permit = worker_permit;
        read_budget::with(Some(worker_budget), prepare)
    });
    let stream = futures_util::stream::try_unfold(
        Stream { export: Export::Preparing(worker), _cancel: Cancel(cancellation), _permit: permit },
        |mut stream| async move {
            if let Export::Preparing(worker) = &mut stream.export {
                let result = worker.await.map_err(anyhow::Error::new).and_then(|file| file);
                let file = match result {
                    Ok(file) => file,
                    Err(error) => {
                        let error = ApiError::internal(error);
                        tracing::warn!(code = %error.code, error = %error.message, "backup export failed");
                        let failure = crate::backup::Record::Failure { code: error.code, message: error.message };
                        let mut bytes = serde_json::to_vec(&failure).map_err(std::io::Error::other)?;
                        bytes.push(b'\n');
                        stream.export = Export::Finished;
                        return Ok(Some((bytes, stream)));
                    }
                };
                stream.export = Export::Reading(tokio::fs::File::from_std(file));
            }
            let Export::Reading(file) = &mut stream.export else {
                return Ok::<_, std::io::Error>(None);
            };
            let mut bytes = vec![0_u8; 64 * 1024];
            let count = file.read(&mut bytes).await?;
            if count == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            bytes.truncate(count);
            Ok(Some((bytes, stream)))
        },
    );
    (
        [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
        Body::from_stream(stream),
    ).into_response()
}

fn wal_size(path: &std::path::Path) -> std::io::Result<u64> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

fn database_paths(
    graph: &rusqlite::Connection,
    shard: Option<&rusqlite::Connection>,
) -> anyhow::Result<BTreeMap<String, std::path::PathBuf>> {
    let mut databases = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for (prefix, connection) in [("graph", Some(graph)), ("shards/messages", shard)] {
        let Some(connection) = connection else { continue };
        let mut statement = connection.prepare("PRAGMA database_list")?;
        let attached = statement.query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        for database in attached {
            let (name, file) = database?;
            if file.is_empty() {
                continue;
            }
            let path = std::fs::canonicalize(file)?;
            if seen.insert(path.clone()) {
                let name = if name == "main" { prefix.to_owned() } else { format!("{prefix}/{name}") };
                databases.insert(name, path);
            }
        }
    }
    Ok(databases)
}

struct WalFile {
    name: String,
    path: std::path::PathBuf,
    initial: std::io::Result<u64>,
    peak: u64,
}

struct WalGrowth {
    files: Vec<WalFile>,
    peak: u64,
    limit: u64,
}

impl WalGrowth {
    fn new(databases: BTreeMap<String, std::path::PathBuf>, limit: u64) -> Self {
        let files = databases.into_iter().map(|(name, database)| {
            let mut path = database.into_os_string();
            path.push("-wal");
            let path = std::path::PathBuf::from(path);
            let initial = wal_size(&path);
            WalFile { name, path, initial, peak: 0 }
        }).collect();
        Self { files, peak: 0, limit }
    }

    fn sample(&mut self) -> std::io::Result<()> {
        let mut total = 0_u64;
        for file in &mut self.files {
            let initial = file.initial.as_ref().map_err(|error| {
                std::io::Error::other(format!("cannot inspect backup WAL {}: {error}", file.name))
            })?;
            let size = wal_size(&file.path).map_err(|error| {
                std::io::Error::other(format!("cannot inspect backup WAL {}: {error}", file.name))
            })?;
            let growth = size.saturating_sub(*initial);
            file.peak = file.peak.max(growth);
            total = total.saturating_add(growth);
        }
        self.peak = self.peak.max(total);
        if total > self.limit {
            return Err(std::io::Error::other(format!(
                "backup WAL growth {total} bytes exceeds --max-wal-growth {}; snapshot cancelled",
                self.limit
            )));
        }
        Ok(())
    }
}

/// Direct exports deliberately keep their read snapshot through downstream backpressure.
pub(super) fn direct_response(
    permit: OwnedSemaphorePermit,
    max_wal_growth: u64,
    produce: impl FnOnce(
        &mut dyn std::io::Write,
        &mut dyn FnMut(&rusqlite::Connection, Option<&rusqlite::Connection>) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> + Send + 'static,
) -> Response {
    let cancellation = ReadBudget::cancellable("/v1/backup/direct");
    let worker_budget = cancellation.clone();
    let monitor_budget = cancellation.clone();
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let (abort, aborted) = tokio::sync::watch::channel(None::<String>);
    let (done, mut finished) = tokio::sync::watch::channel(false);
    let growth = Arc::new(Mutex::new(WalGrowth::new(BTreeMap::new(), max_wal_growth)));
    let monitor_growth = growth.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = finished.changed() => break,
                _ = interval.tick() => {
                    let result = monitor_growth.lock()
                        .map_err(|_| std::io::Error::other("backup WAL monitor poisoned"))
                        .and_then(|mut growth| growth.sample());
                    if let Err(error) = result {
                        let _ = abort.send(Some(error.to_string()));
                        monitor_budget.cancel();
                        break;
                    }
                }
            }
        }
    });
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let register_growth = growth.clone();
        let mut pinned = move |graph: &rusqlite::Connection, shard: Option<&rusqlite::Connection>| {
            let mut guard = register_growth.lock()
                .map_err(|_| anyhow::anyhow!("backup WAL monitor poisoned"))?;
            *guard = WalGrowth::new(database_paths(graph, shard)?, max_wal_growth);
            guard.sample()?;
            Ok(())
        };
        let mut writer = DirectWriter {
            sender, aborted, handle, pending: Vec::new(), growth,
        };
        let result = read_budget::with(Some(worker_budget), || produce(&mut writer, &mut pinned));
        let result = result.and_then(|()| {
            anyhow::ensure!(writer.pending.is_empty(), "truncated producer record");
            writer.check_abort()?;
            Ok(())
        });
        if let Err(error) = result {
            let message = writer.aborted.borrow().clone().unwrap_or_else(|| format!("{error:#}"));
            let failure = crate::backup::Record::Failure { code: "backup-stream-failed".into(), message };
            if let Ok(mut bytes) = serde_json::to_vec(&failure) {
                bytes.push(b'\n');
                let _ = writer.sender.blocking_send(bytes.into());
            }
        }
        let _ = done.send(true);
    });
    let stream = futures_util::stream::unfold(
        (receiver, Cancel(cancellation)),
        |(mut receiver, cancel)| async move {
            receiver.recv().await.map(|bytes| (Ok::<_, std::io::Error>(bytes), (receiver, cancel)))
        },
    );
    (
        [(axum::http::header::CONTENT_TYPE, "application/x-ndjson"),
         (axum::http::header::HeaderName::from_static("x-fractal-backup-stream"), "1")],
        Body::from_stream(stream),
    ).into_response()
}

struct DirectWriter {
    sender: tokio::sync::mpsc::Sender<axum::body::Bytes>,
    aborted: tokio::sync::watch::Receiver<Option<String>>,
    handle: tokio::runtime::Handle,
    pending: Vec<u8>,
    growth: Arc<Mutex<WalGrowth>>,
}

impl DirectWriter {
    fn check_abort(&self) -> std::io::Result<()> {
        if let Some(error) = self.aborted.borrow().as_ref() {
            return Err(std::io::Error::other(error.clone()));
        }
        read_budget::check().map_err(std::io::Error::other)
    }

    fn send(&mut self, bytes: axum::body::Bytes) -> std::io::Result<()> {
        self.check_abort()?;
        self.handle.block_on(async {
            tokio::select! {
                result = self.sender.send(bytes) => result.map_err(std::io::Error::other),
                result = self.aborted.changed() => {
                    result.map_err(std::io::Error::other)?;
                    Err(std::io::Error::other(self.aborted.borrow().clone().unwrap_or_else(|| "backup cancelled".into())))
                }
            }
        })
    }
}

impl std::io::Write for DirectWriter {
    fn write(&mut self, mut bytes: &[u8]) -> std::io::Result<usize> {
        self.check_abort()?;
        let length = bytes.len();
        while !bytes.is_empty() {
            let count = bytes.iter().position(|byte| *byte == b'\n').map_or(bytes.len(), |index| index + 1);
            if self.pending.len() as u64 + count as u64 > crate::backup::MAX_RECORD_BYTES {
                return Err(std::io::Error::other("oversized backup record"));
            }
            self.pending.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.pending.last() == Some(&b'\n') {
                let mut line = std::mem::take(&mut self.pending);
                if line.starts_with(b"{\"record\":\"end\",") {
                    let mut growth = self.growth.lock()
                        .map_err(|_| std::io::Error::other("backup WAL monitor poisoned"))?;
                    growth.sample()?;
                    let by_file = growth.files.iter()
                        .map(|file| (file.name.to_owned(), file.peak))
                        .collect::<BTreeMap<String, u64>>();
                    let mut end: serde_json::Value = serde_json::from_slice(&line)?;
                    end["data"]["peak_wal_growth"] = growth.peak.into();
                    end["data"]["peak_wal_growth_by_file"] = serde_json::to_value(by_file)?;
                    drop(growth);
                    line = serde_json::to_vec(&end)?;
                    line.push(b'\n');
                }
                let line = axum::body::Bytes::from(line);
                for offset in (0..line.len()).step_by(64 * 1024) {
                    self.send(line.slice(offset..(offset + 64 * 1024).min(line.len())))?;
                }
            }
        }
        Ok(length)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.check_abort()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};
    use std::time::Duration;

    fn fixture_database_paths(root: &std::path::Path) -> [std::path::PathBuf; 2] {
        [
            root.join("claims.sqlite3"),
            fractal_store::store::backup_shards::database_path(root).unwrap(),
        ]
    }

    fn wal_database(path: &std::path::Path) -> rusqlite::Connection {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE payload (value BLOB);
             INSERT INTO payload VALUES (zeroblob(16384));
             PRAGMA wal_checkpoint(TRUNCATE);"
        ).unwrap();
        connection
    }

    fn pin_snapshot(path: &std::path::Path) -> rusqlite::Connection {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch("BEGIN;").unwrap();
        let count: u64 = connection.query_row("SELECT count(*) FROM payload", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
        connection
    }

    async fn final_record_after_writes(writes: [bool; 2], limit: u64) -> (serde_json::Value, [u64; 2]) {
        let root = tempfile::tempdir().unwrap();
        let databases = fixture_database_paths(root.path());
        let writers = databases.each_ref().map(|path| wal_database(path));
        let snapshots = databases.clone();
        let (started, begun) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let response = direct_response(
            reserve_from(&Arc::new(Semaphore::new(1))).unwrap(), limit,
            move |output, pinned| {
                let graph = pin_snapshot(&snapshots[0]);
                let shard = pin_snapshot(&snapshots[1]);
                pinned(&graph, Some(&shard))?;
                started.send(()).unwrap();
                gate.recv()?;
                output.write_all(b"{\"record\":\"end\",\"data\":{}}\n")?;
                Ok(())
            },
        );
        begun.await.unwrap();
        for (writer, write) in writers.iter().zip(writes) {
            if write {
                writer.execute("INSERT INTO payload VALUES (zeroblob(16384))", []).unwrap();
            }
        }
        let sizes = databases.map(|database| {
            let mut wal = database.into_os_string();
            wal.push("-wal");
            wal_size(std::path::Path::new(&wal)).unwrap()
        });
        release.send(()).unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        (serde_json::from_slice(&bytes).unwrap(), sizes)
    }

    #[tokio::test(start_paused = true)]
    async fn final_sample_rejects_shard_only_growth() {
        let (record, sizes) = final_record_after_writes([false, true], 1).await;
        assert_eq!(sizes[0], 0);
        assert!(sizes[1] > 1);
        assert_eq!(record["record"], "failure");
        assert!(record["data"]["message"].as_str().unwrap().contains("--max-wal-growth 1"));
    }

    #[tokio::test(start_paused = true)]
    async fn wal_limit_applies_to_the_sum_not_each_database() {
        let limit = 32768;
        let (record, sizes) = final_record_after_writes([true, true], limit).await;
        assert!(sizes.iter().all(|size| *size <= limit), "{sizes:?}");
        assert!(sizes.iter().sum::<u64>() > limit, "{sizes:?}");
        assert_eq!(record["record"], "failure");
        assert!(record["data"]["message"].as_str().unwrap().contains("--max-wal-growth 32768"));
    }

    #[tokio::test(start_paused = true)]
    async fn end_metadata_includes_final_aggregate_and_per_file_peaks() {
        let (record, sizes) = final_record_after_writes([true, true], u64::MAX).await;
        assert!(sizes.iter().all(|size| *size > 0), "{sizes:?}");
        assert_eq!(record["record"], "end");
        assert_eq!(record["data"]["peak_wal_growth"], sizes.iter().sum::<u64>());
        assert_eq!(record["data"]["peak_wal_growth_by_file"], serde_json::json!({
            "graph": sizes[0], "shards/messages": sizes[1],
        }));
    }

    #[test]
    fn aggregate_peak_is_not_the_sum_of_independent_file_peaks() {
        let root = tempfile::tempdir().unwrap();
        let databases = fixture_database_paths(root.path());
        let graph = wal_database(&databases[0]);
        let shard = wal_database(&databases[1]);
        let graph_snapshot = pin_snapshot(&databases[0]);
        let shard_snapshot = pin_snapshot(&databases[1]);
        let mut growth = WalGrowth::new(database_paths(&graph_snapshot, Some(&shard_snapshot)).unwrap(), u64::MAX);
        graph.execute("INSERT INTO payload VALUES (zeroblob(16384))", []).unwrap();
        growth.sample().unwrap();
        let graph_peak = growth.peak;
        assert!(graph_peak > 0);
        drop(graph_snapshot);
        graph.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        shard.execute("INSERT INTO payload VALUES (zeroblob(16384))", []).unwrap();
        growth.sample().unwrap();
        let shard_peak = growth.files[1].peak;
        assert!(shard_peak > 0);
        assert_eq!(growth.files[0].peak, graph_peak);
        assert_eq!(growth.peak, graph_peak.max(shard_peak));
        assert!(growth.peak < graph_peak + shard_peak);
    }

    #[tokio::test]
    async fn headers_return_before_export_and_the_request_budget_does_not_cancel_the_stream() {
        let request = ReadBudget::new("/v1/backup", Duration::from_secs(15));
        let (release, gate) = std::sync::mpsc::channel();
        let response = read_budget::with(Some(request.clone()), || response(reserve_from(&Arc::new(Semaphore::new(1))).unwrap(), move || {
            gate.recv()?;
            read_budget::check()?;
            let mut file = tempfile::tempfile()?;
            file.write_all(b"complete archive\n")?;
            file.seek(SeekFrom::Start(0))?;
            Ok(file)
        }));
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        // The old request budget may expire after headers; the export has its own
        // cancellation owner and must still finish without lengthening that budget.
        request.cancel();
        release.send(()).unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&bytes[..], b"complete archive\n");
    }

    #[tokio::test]
    async fn dropping_the_response_cancels_the_export_worker() {
        let (started, begun) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let (finished, result) = tokio::sync::oneshot::channel();
        let pool = Arc::new(Semaphore::new(1));
        let response = response(reserve_from(&pool).unwrap(), move || {
            started.send(()).unwrap();
            gate.recv()?;
            let cancelled = read_budget::check().is_err();
            finished.send(cancelled).unwrap();
            anyhow::bail!("cancelled export")
        });
        begun.await.unwrap();
        let busy = reserve_from(&pool).err().unwrap();
        assert_eq!(busy.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(busy.code, "backup-busy");
        drop(response);
        assert!(reserve_from(&pool).is_err(), "a still-running producer owns the slot");
        release.send(()).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), result).await.unwrap().unwrap());
        let permit = tokio::time::timeout(Duration::from_secs(2), pool.acquire()).await.unwrap().unwrap();
        drop(permit);
    }

    #[tokio::test]
    async fn direct_backpressure_releases_the_worker_when_the_body_is_dropped() {
        let (started, begun) = tokio::sync::oneshot::channel();
        let (finished, result) = tokio::sync::oneshot::channel();
        let pool = Arc::new(Semaphore::new(1));
        let response = direct_response(
            reserve_from(&pool).unwrap(), u64::MAX,
            move |output, _pinned| {
                started.send(()).unwrap();
                let result = output.write_all(&vec![b'\n'; 256 * 1024]);
                finished.send(result.is_err()).unwrap();
                result.map_err(anyhow::Error::new)
            },
        );
        begun.await.unwrap();
        drop(response);
        assert!(result.await.unwrap());
        let permit = pool.acquire().await.unwrap();
        drop(permit);
    }

    async fn stream_with_attached_blob_write(target: &str, limit: u64) -> serde_json::Value {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(fractal_store::store::Store::open(
            &root.path().join("claims.sqlite3"), "backup-blobs",
        ).unwrap());
        let shard = fractal_store::store::messages_shard::open(&store, root.path()).unwrap();
        let databases = database_paths(&store.readers.get(), Some(&shard.readers.get())).unwrap();
        assert!(databases.contains_key("graph/blob_file"));
        assert!(databases.contains_key("shards/messages/blob_file"));
        let graph_blob = wal_database(&databases["graph/blob_file"]);
        let shard_blob = wal_database(&databases["shards/messages/blob_file"]);
        let (started, begun) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let response = direct_response(
            reserve_from(&Arc::new(Semaphore::new(1))).unwrap(), limit,
            move |output, pinned| store.read_snapshot(|_| shard.read_snapshot(|_| {
                let graph = store.readers.get();
                let shard = shard.readers.get();
                for connection in [&*graph, &*shard] {
                    let count: u64 = connection.query_row(
                        "SELECT count(*) FROM blob_file.payload", [], |row| row.get(0),
                    )?;
                    assert_eq!(count, 1);
                }
                pinned(&graph, Some(&shard))?;
                started.send(()).unwrap();
                gate.recv()?;
                output.write_all(b"{\"record\":\"end\",\"data\":{}}\n")?;
                Ok(())
            })),
        );
        begun.await.unwrap();
        let writer = match target {
            "graph/blob_file" => &graph_blob,
            "shards/messages/blob_file" => &shard_blob,
            _ => panic!("unknown test blob database"),
        };
        writer.execute("INSERT INTO payload VALUES (zeroblob(16384))", []).unwrap();
        release.send(()).unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn graph_and_shard_attached_blob_wals_both_trigger_the_limit() {
        for target in ["graph/blob_file", "shards/messages/blob_file"] {
            let record = stream_with_attached_blob_write(target, 1).await;
            assert_eq!(record["record"], "failure", "{target}: {record}");
            assert!(record["data"]["message"].as_str().unwrap().contains("--max-wal-growth 1"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn receipt_reports_attached_blob_wal_growth() {
        let record = stream_with_attached_blob_write("shards/messages/blob_file", u64::MAX).await;
        assert_eq!(record["record"], "end");
        let peaks = record["data"]["peak_wal_growth_by_file"].as_object().unwrap();
        assert_eq!(peaks.len(), 4);
        assert!(peaks["shards/messages/blob_file"].as_u64().unwrap() > 0);
        assert_eq!(peaks["graph/blob_file"], 0);
        assert!(record["data"]["peak_wal_growth"].as_u64().unwrap() >= peaks["shards/messages/blob_file"].as_u64().unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn relocated_store_shard_growth_cancels_a_direct_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("product-state");
        let store_dir = root.path().join("relocated-store");
        std::fs::create_dir_all(&state_dir).unwrap();
        let store = Arc::new(fractal_store::store::Store::open(
            &store_dir.join("claims.sqlite3"), "backup-relocation",
        ).unwrap());
        assert!(!database_paths(&store.readers.get(), None).unwrap().keys().any(|key| key.starts_with("shards/")));
        let opened_shard = fractal_store::store::messages_shard::open(&store, &store_dir).unwrap();
        let databases = database_paths(&store.readers.get(), Some(&opened_shard.readers.get())).unwrap();
        assert_eq!(databases.get("graph"), Some(&store.path));
        assert_eq!(databases.get("shards/messages"), Some(&opened_shard.path));
        assert!(!state_dir.join("shards/messages/claims.sqlite3").exists());
        let snapshots = [store.path.clone(), opened_shard.path.clone()];
        let graph = wal_database(&snapshots[0]);
        let shard = wal_database(&snapshots[1]);
        let (started, begun) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let response = direct_response(
            reserve_from(&Arc::new(Semaphore::new(1))).unwrap(), 1,
            move |output, pinned| {
                let graph = pin_snapshot(&snapshots[0]);
                let shard = pin_snapshot(&snapshots[1]);
                pinned(&graph, Some(&shard))?;
                started.send(()).unwrap();
                gate.recv()?;
                output.write_all(b"{\"record\":\"header\"}\n")?;
                Ok(())
            },
        );
        begun.await.unwrap();
        shard.execute("INSERT INTO payload VALUES (zeroblob(16384))", []).unwrap();
        assert_eq!(graph.query_row("SELECT count(*) FROM payload", [], |row| row.get::<_, u64>(0)).unwrap(), 1);
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        release.send(()).unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let failure: crate::backup::Record = serde_json::from_slice(&bytes).unwrap();
        let crate::backup::Record::Failure { message, .. } = failure else {
            panic!("WAL guard must reject the archive");
        };
        assert!(message.contains("--max-wal-growth 1"), "{message}");
    }

    #[tokio::test]
    async fn a_failed_producer_cannot_finish_a_successful_download() {
        let response = response(reserve_from(&Arc::new(Semaphore::new(1))).unwrap(), || anyhow::bail!("export fixture failure"));
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let crate::backup::Record::Failure { message, .. } = serde_json::from_slice(&bytes).unwrap() else {
            panic!("a producer failure must report its cause, never a successful archive");
        };
        assert!(message.contains("export fixture failure"));
    }
}
