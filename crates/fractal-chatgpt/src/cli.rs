//! Shared transport runner; each binary owns its closed flag surface.
use crate::{Adapter, Profile, http_router};
use rmcp::ServiceExt;
use std::{
    io,
    net::SocketAddr,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};
fn is_call_telemetry_target(target: &str) -> bool {
    let permitted = target == "fractal_chatgpt::call";
    // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-11-07-fractal-chatgpt-telemetry-target — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
    // Deployments must enable both targets in RUST_LOG before upgrading.
    let permitted = permitted || target == "st_chatgpt::call";
    // LIVE-MIGRATION END arn:lmig:smalltalk:2026-11-07-fractal-chatgpt-telemetry-target
    permitted
}

#[test]
fn call_telemetry_target_allowlist() {
    assert!(is_call_telemetry_target("fractal_chatgpt::call"));
    // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-11-07-fractal-chatgpt-telemetry-target — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
    assert!(is_call_telemetry_target("st_chatgpt::call"));
    // LIVE-MIGRATION END arn:lmig:smalltalk:2026-11-07-fractal-chatgpt-telemetry-target
    assert!(!is_call_telemetry_target("reqwest::connect"));
}

/// Eight times fractal-delivery-base's message_body::MAX_BYTES (256 KiB),
/// allowing worst-case JSON escaping (6x) and an MCP envelope. Excludes LF;
/// a CR, if present, counts toward the ceiling.
const MAX_STDIO_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// Bounds rmcp's otherwise unbounded read_until buffer without copying input.
/// An overflow is terminal, including when the peer never sends a newline.
struct FrameLimitedRead<R> {
    inner: R,
    line_bytes: usize,
    failed: bool,
}

impl<R> FrameLimitedRead<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            line_bytes: 0,
            failed: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for FrameLimitedRead<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stdio MCP frame exceeds byte ceiling",
            )));
        }
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        // Read at most one byte beyond the remaining budget, so rmcp cannot
        // accumulate more than the ceiling even for an unterminated frame.
        let capacity = output
            .remaining()
            .min(MAX_STDIO_FRAME_BYTES - this.line_bytes + 1);
        let mut input = ReadBuf::new(output.initialize_unfilled_to(capacity));
        match Pin::new(&mut this.inner).poll_read(cx, &mut input) {
            Poll::Ready(Ok(())) => {}
            other => return other,
        }
        for &byte in input.filled() {
            if byte == b'\n' {
                this.line_bytes = 0;
            } else {
                this.line_bytes += 1;
                if this.line_bytes > MAX_STDIO_FRAME_BYTES {
                    this.failed = true;
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "stdio MCP frame exceeds byte ceiling",
                    )));
                }
            }
        }
        let length = input.filled().len();
        output.advance(length);
        Poll::Ready(Ok(()))
    }
}

pub async fn run(
    socket: &Path,
    credential_file: &Path,
    profile: Profile,
    listen: Option<SocketAddr>,
) -> Result<(), Box<dyn std::error::Error>> {
    // stderr is essential for stdio framing; dependency logs stay disabled.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off"));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::io::stderr)
                .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                    is_call_telemetry_target(metadata.target())
                }))
                .with_filter(filter),
        )
        .try_init()?;
    let adapter = Adapter::with_profile(socket, credential_file, profile)?;
    if let Some(listen) = listen {
        if !listen.ip().is_loopback() {
            return Err("the MCP listener must be loopback-only".into());
        }
        axum::serve(
            tokio::net::TcpListener::bind(listen).await?,
            http_router(adapter),
        )
        .await?;
    } else {
        adapter
            .serve((
                FrameLimitedRead::new(tokio::io::stdin()),
                tokio::io::stdout(),
            ))
            .await?
            .waiting()
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod stdio_tests {
    use super::*;
    use rmcp::{RoleServer, transport::Transport};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    fn ping_frame(bytes: usize) -> Vec<u8> {
        let mut frame = br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_vec();
        frame.resize(bytes, b' ');
        frame.push(b'\n');
        frame
    }

    #[tokio::test]
    async fn oversized_frames_end_rmcp_transport() {
        for terminated in [false, true] {
            let mut frame = ping_frame(MAX_STDIO_FRAME_BYTES + 1);
            if !terminated {
                frame.pop();
            }
            // A valid following frame must not be processed after overflow.
            frame.extend_from_slice(&ping_frame(64));
            let mut transport =
                rmcp::transport::async_rw::AsyncRwTransport::<RoleServer, _, _>::new(
                    FrameLimitedRead::new(frame.as_slice()),
                    tokio::io::sink(),
                );
            assert!(transport.receive().await.is_none());
            assert!(transport.receive().await.is_none());
        }
    }

    #[tokio::test]
    async fn unterminated_stream_is_bounded_and_failure_is_sticky() {
        // An endless source cannot reach EOF or supply a newline.
        let mut reader = BufReader::new(FrameLimitedRead::new(tokio::io::repeat(b'x')));
        let mut line = Vec::new();
        let error = reader.read_until(b'\n', &mut line).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(line.len() <= MAX_STDIO_FRAME_BYTES);
        let mut byte = [0];
        assert_eq!(
            reader.read(&mut byte).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[tokio::test]
    async fn exact_limit_and_following_frame_are_accepted() {
        let mut frames = ping_frame(MAX_STDIO_FRAME_BYTES);
        frames.extend_from_slice(&ping_frame(MAX_STDIO_FRAME_BYTES));
        let mut transport = rmcp::transport::async_rw::AsyncRwTransport::<RoleServer, _, _>::new(
            FrameLimitedRead::new(frames.as_slice()),
            tokio::io::sink(),
        );
        assert!(transport.receive().await.is_some());
        assert!(transport.receive().await.is_some());
        assert!(transport.receive().await.is_none());
    }
}
