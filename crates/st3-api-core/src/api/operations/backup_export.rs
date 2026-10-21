//! The response stream owns export cancellation, independently of header admission.
//! Spooling completes before downloading, so backpressure never pins the SQLite snapshot.
use axum::body::Body;
use axum::response::{IntoResponse, Response};
use smallclaims::read_budget::{self, ReadBudget};
use tokio::io::AsyncReadExt;

struct Cancel(ReadBudget);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

enum Export {
    Preparing(tokio::task::JoinHandle<anyhow::Result<std::fs::File>>),
    Reading(tokio::fs::File),
}

struct Stream {
    export: Export,
    _cancel: Cancel,
}

pub(super) fn response(
    prepare: impl FnOnce() -> anyhow::Result<std::fs::File> + Send + 'static,
) -> Response {
    let cancellation = ReadBudget::cancellable("/v1/backup/stream");
    let worker_budget = cancellation.clone();
    let worker = tokio::task::spawn_blocking(move || {
        read_budget::with(Some(worker_budget), prepare)
    });
    let stream = futures_util::stream::try_unfold(
        Stream { export: Export::Preparing(worker), _cancel: Cancel(cancellation) },
        |mut stream| async move {
            if let Export::Preparing(worker) = &mut stream.export {
                let file = worker.await.map_err(std::io::Error::other)?
                    .map_err(|error| {
                        tracing::warn!(error = %error, "backup export failed");
                        std::io::Error::other(format!("backup export failed: {error:#}"))
                    })?;
                stream.export = Export::Reading(tokio::fs::File::from_std(file));
            }
            let Export::Reading(file) = &mut stream.export else { unreachable!() };
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
        let response = read_budget::with(Some(request.clone()), || response(move || {
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
        let response = response(move || {
            started.send(()).unwrap();
            gate.recv()?;
            let cancelled = read_budget::check().is_err();
            finished.send(cancelled).unwrap();
            anyhow::bail!("cancelled export")
        });
        begun.await.unwrap();
        drop(response);
        release.send(()).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), result).await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn a_failed_producer_cannot_finish_a_successful_download() {
        let response = response(|| anyhow::bail!("export fixture failure"));
        assert!(axum::body::to_bytes(response.into_body(), 1024).await.is_err());
    }
}
