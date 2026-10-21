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
    pool.clone().try_acquire_owned().map_err(|_| ApiError::bad(crate::model::St3Error::new(
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
    async fn a_failed_producer_cannot_finish_a_successful_download() {
        let response = response(reserve_from(&Arc::new(Semaphore::new(1))).unwrap(), || anyhow::bail!("export fixture failure"));
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let crate::backup::Record::Failure { message, .. } = serde_json::from_slice(&bytes).unwrap() else {
            panic!("a producer failure must report its cause, never a successful archive");
        };
        assert!(message.contains("export fixture failure"));
    }
}
