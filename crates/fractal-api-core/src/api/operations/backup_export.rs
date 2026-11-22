//! The response stream owns export cancellation, independently of header admission.
//! Spooling completes before downloading, so backpressure never pins the SQLite snapshot.
use axum::body::Body;
use axum::response::{IntoResponse, Response};
use smallclaims::read_budget::{self, ReadBudget};
use tokio::io::AsyncReadExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use std::sync::{Arc, LazyLock};
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

/// Direct exports deliberately keep their read snapshot through downstream backpressure.
pub(super) fn direct_response(
    permit: OwnedSemaphorePermit,
    database: std::path::PathBuf,
    max_wal_growth: u64,
    produce: impl FnOnce(&mut dyn std::io::Write) -> anyhow::Result<()> + Send + 'static,
) -> Response {
    use std::sync::atomic::{AtomicU64, Ordering};
    let cancellation = ReadBudget::cancellable("/v1/backup/direct");
    let worker_budget = cancellation.clone();
    let monitor_budget = cancellation.clone();
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let (abort, aborted) = tokio::sync::watch::channel(None::<String>);
    let (done, mut finished) = tokio::sync::watch::channel(false);
    let peak = Arc::new(AtomicU64::new(0));
    let monitor_peak = peak.clone();
    let mut wal_name = database.into_os_string();
    wal_name.push("-wal");
    let wal = std::path::PathBuf::from(wal_name);
    let initial = wal_size(&wal);
    let writer_wal = wal.clone();
    let initial_size = initial.as_ref().copied().unwrap_or(0);
    tokio::spawn(async move {
        let initial = match initial {
            Ok(initial) => initial,
            Err(error) => {
                let _ = abort.send(Some(format!("cannot inspect backup WAL: {error}")));
                monitor_budget.cancel();
                return;
            }
        };
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = finished.changed() => break,
                _ = interval.tick() => {
                    let result = wal_size(&wal).map(|size| size.saturating_sub(initial));
                    let error = match result {
                        Ok(growth) => {
                            monitor_peak.fetch_max(growth, Ordering::Relaxed);
                            (growth > max_wal_growth).then(|| format!(
                                "backup WAL growth {growth} bytes exceeds --max-wal-growth {max_wal_growth}; snapshot cancelled"
                            ))
                        }
                        Err(error) => Some(format!("cannot inspect backup WAL: {error}")),
                    };
                    if let Some(error) = error {
                        let _ = abort.send(Some(error));
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
        let mut writer = DirectWriter {
            sender, aborted, handle, pending: Vec::new(), peak,
            wal: writer_wal, initial_size, max_wal_growth,
        };
        let result = read_budget::with(Some(worker_budget), || produce(&mut writer));
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
    peak: Arc<std::sync::atomic::AtomicU64>,
    wal: std::path::PathBuf,
    initial_size: u64,
    max_wal_growth: u64,
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
                    let growth = wal_size(&self.wal)?.saturating_sub(self.initial_size);
                    self.peak.fetch_max(growth, std::sync::atomic::Ordering::Relaxed);
                    if growth > self.max_wal_growth {
                        return Err(std::io::Error::other(format!(
                            "backup WAL growth {growth} bytes exceeds --max-wal-growth {}; snapshot cancelled",
                            self.max_wal_growth
                        )));
                    }
                    let mut end: serde_json::Value = serde_json::from_slice(&line)?;
                    end["data"]["peak_wal_growth"] = self.peak.load(std::sync::atomic::Ordering::Relaxed).into();
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
        let root = tempfile::tempdir().unwrap();
        let (started, begun) = tokio::sync::oneshot::channel();
        let (finished, result) = tokio::sync::oneshot::channel();
        let pool = Arc::new(Semaphore::new(1));
        let response = direct_response(
            reserve_from(&pool).unwrap(), root.path().join("claims.sqlite3"), u64::MAX,
            move |output| {
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

    #[tokio::test(start_paused = true)]
    async fn wal_growth_cancels_a_direct_snapshot_and_reports_the_limit() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("claims.sqlite3");
        let wal = root.path().join("claims.sqlite3-wal");
        std::fs::write(&wal, b"").unwrap();
        let (release, gate) = std::sync::mpsc::channel();
        let response = direct_response(
            reserve_from(&Arc::new(Semaphore::new(1))).unwrap(), database, 1,
            move |output| {
                gate.recv()?;
                output.write_all(b"{\"record\":\"header\"}\n")?;
                Ok(())
            },
        );
        std::fs::write(&wal, b"growth").unwrap();
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
