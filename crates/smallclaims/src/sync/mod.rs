//! Sync between members: the signed HTTP exchange two members run to swap the envelopes each
//! lacks, the worker that keeps every member's store in step with its peers, and joining over
//! the same listener.
//!
//! Every request and response carries the fleet's HMAC and, on a member, its member key's
//! signature; [`fractal_auth::FleetAuth`] makes and checks both. The worker reaches its store through a
//! [`Backend`]: a program that holds the store itself uses [`Local`], and Fractal hands the
//! calls to its daemon, the store's only writer.


use axum::http::HeaderMap;
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

mod backend;
pub mod revision;
pub mod telemetry;
mod worker;

pub use backend::{Backend, Local, redemption_answer};
pub use worker::*;

/// `ST3_WORKER_INTERVAL_MS`, read once: a cap on every worker timer, and on the store's
/// catch-up and divergence intervals that follow them, so tests of several nodes on one machine
/// need not wait out the 30- and 60-second defaults. Unset, nothing is capped.
pub(crate) fn worker_timer_cap() -> Option<std::time::Duration> {
    static CAP: std::sync::OnceLock<Option<std::time::Duration>> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("ST3_WORKER_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .map(std::time::Duration::from_millis)
    })
}

/// `ST3_REPLICATION_IDLE_MAX_MS`, read once: raise or lower the idle polling ceiling.
/// Starts at 30 seconds, doubles to 10 minutes by default, and is bounded to 30s..=1h.
/// `ST3_WORKER_INTERVAL_MS` still caps the resulting timer when set.
pub(crate) fn replication_idle_max() -> std::time::Duration {
    static MAX: std::sync::LazyLock<std::time::Duration> = std::sync::LazyLock::new(|| {
        replication_idle_max_from(std::env::var("ST3_REPLICATION_IDLE_MAX_MS").ok().as_deref())
    });
    *MAX
}

pub(crate) fn replication_idle_max_from(value: Option<&str>) -> std::time::Duration {
    std::time::Duration::from_millis(
        value
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(600_000)
            .clamp(30_000, 3_600_000),
    )
}

/// `default`, or `cap` when that is shorter.
pub(crate) fn capped(
    default: std::time::Duration,
    cap: Option<std::time::Duration>,
) -> std::time::Duration {
    cap.map_or(default, |cap| cap.min(default))
}

pub const EXCHANGE_PATH: &str = "/v1/peer/exchange";

/// Where a member asks another for its answers per claim kind; see `crate::answers`.
pub const ANSWERS_PATH: &str = "/v1/peer/answers";
pub const JOIN_PATH: &str = "/v1/fleet/join";
pub const MAX_JOIN_BYTES: usize = 4096;
pub const MAX_EXCHANGE_BYTES: usize = 64 * 1024 * 1024;

/// The HTTP content coding for large exchange bodies: zlib-wrapped deflate. A requester asks
/// for it with `Accept-Encoding`, and a peer says with the same header in its answer that it
/// takes it in requests. Signatures cover the uncompressed JSON, so an older build, which
/// neither asks nor says, exchanges plain JSON as before.
const EXCHANGE_ENCODING: &str = "deflate";

/// Bodies smaller than this go uncompressed: a quiet exchange is a few kilobytes, while a page
/// of envelopes is megabytes and deflates to about a third.
const DEFLATE_MIN_BYTES: usize = 64 * 1024;

fn deflate(body: &[u8]) -> Result<Vec<u8>> {
    use std::io::Write as _;
    let mut encoder = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(body.len() / 3),
        flate2::Compression::fast(),
    );
    encoder.write_all(body)?;
    Ok(encoder.finish()?)
}

/// Inflate an exchange body, refusing one that would expand past `MAX_EXCHANGE_BYTES`. `what`
/// names the body (which path, request or answer, and the peer when known) so a refusal says
/// which part of an exchange grew past the limit; the message keeps its leading words, which
/// error classification reads.
fn inflate(body: &[u8], what: &str) -> Result<Vec<u8>> {
    use std::io::Read as _;
    let mut inflated = Vec::with_capacity(body.len() * 3);
    flate2::read::ZlibDecoder::new(body)
        .take(MAX_EXCHANGE_BYTES as u64 + 1)
        .read_to_end(&mut inflated)
        .with_context(|| format!("inflate the exchange body ({what})"))?;
    anyhow::ensure!(
        inflated.len() <= MAX_EXCHANGE_BYTES,
        "the exchange body inflates past {MAX_EXCHANGE_BYTES} bytes ({what}, {} compressed bytes)",
        body.len()
    );
    Ok(inflated)
}

fn deflated(headers: &HeaderMap) -> bool {
    headers.get("content-encoding").is_some_and(|value| {
        value
            .as_bytes()
            .eq_ignore_ascii_case(EXCHANGE_ENCODING.as_bytes())
    })
}

fn accepts_deflate(headers: &HeaderMap) -> bool {
    headers
        .get_all("accept-encoding")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|coding| {
            coding
                .split(';')
                .next()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case(EXCHANGE_ENCODING))
        })
}


/// The envelope of every signed peer answer. Its layout and version are the ones Fractal's
/// client API answers with, so members on older builds read it unchanged.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PeerResponse<T> {
    pub api_version: String,
    pub request_id: String,
    pub snapshot_host: String,
    pub store_index: u64,
    pub value: T,
}

/// The version every signed peer answer carries.
pub const PEER_API_VERSION: &str = "st3.v1";

// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
/// Accepted peer envelope labels. Writers still emit `PEER_API_VERSION`;
/// `st3.v1` remains a permanent read spelling, including after contraction.
pub const ACCEPTED_PEER_API_VERSIONS: &[&str] = &[PEER_API_VERSION, "fractal.v1"];
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both

impl<T> PeerResponse<T> {
    pub fn new(node: &str, store_index: u64, value: T) -> Self {
        Self {
            api_version: PEER_API_VERSION.into(),
            request_id: uuid::Uuid::now_v7().to_string(),
            snapshot_host: node.into(),
            store_index,
            value,
        }
    }
}

