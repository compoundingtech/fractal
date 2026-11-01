//! Seat freshness pushed by the seat's host (contract C3 of the claims-store breakup).
//!
//! A re-stamp that changes nothing stays in its host's local observation log, so the host tells
//! the other members when it last heard each of its seats. It pushes only what changed, to a
//! member whose own newest `daemon.started` advertises [`SEAT_FRESHNESS_FEATURE`]; an older member
//! is never sent the push, and the host's seats keep replicating their re-stamps for it.

use super::*;

/// The member-authenticated route a member receives its peers' seat freshness on.
pub const SEAT_FRESHNESS_PATH: &str = "/v1/peer/seat-freshness";
/// The daemon route the worker records a push through.
pub const SEAT_FRESHNESS_INTERNAL_PATH: &str = "/v1/internal/seat-freshness";
/// The `daemon.started` feature of a member that receives [`SEAT_FRESHNESS_PATH`].
pub const SEAT_FRESHNESS_FEATURE: &str = "seat_freshness";
/// The most seats one push may name.
pub const MAX_SEAT_FRESHNESS_SEATS: usize = 2_048;
const MAX_SEAT_FRESHNESS_FIELD_BYTES: usize = 256;
/// Bounds a push: each seat is a subject, an incarnation and a time.
pub const MAX_SEAT_FRESHNESS_BYTES: usize = 1_048_576;

/// When the host last heard one of its seats: its newest local harness report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatHeard {
    pub agent: String,
    pub incarnation_id: String,
    pub heard_at_unix_ms: u64,
}

/// A host's push: the seats it heard since its last push to this member.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatFreshnessPush {
    pub seats: Vec<SeatHeard>,
}

impl SeatFreshnessPush {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.seats.len() <= MAX_SEAT_FRESHNESS_SEATS,
            "a seat freshness push names at most {MAX_SEAT_FRESHNESS_SEATS} seats"
        );
        for seat in &self.seats {
            anyhow::ensure!(
                seat.agent.starts_with("agent/")
                    && seat.agent.len() <= MAX_SEAT_FRESHNESS_FIELD_BYTES
                    && seat.incarnation_id.len() <= MAX_SEAT_FRESHNESS_FIELD_BYTES,
                "a seat freshness push names agents only"
            );
        }
        Ok(())
    }
}

/// What the worker hands its daemon: the push, and the authenticated member that sent it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceivedSeatFreshness {
    pub from: String,
    pub seats: Vec<SeatHeard>,
}

/// The receiver's answer: the seats it recorded, those its declarations place on the sender. A
/// seat it left out, say one whose declaration has not reached it yet, is still owed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatFreshnessAck {
    pub recorded: Vec<String>,
}

pub(super) async fn receive_seat_freshness(
    State(state): State<PeerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let sender = match state.auth().verify_sender(&headers, "POST", SEAT_FRESHNESS_PATH, &body, None, None) {
        Ok(sender) if state.accept(&sender).is_ok() => sender,
        _ => return (StatusCode::UNAUTHORIZED, "untrusted fleet seat freshness").into_response(),
    };
    state.note_activity(&sender.name);
    let digest = FleetAuth::body_digest(&body);
    let refuse = |status: StatusCode, message: String| {
        signed_error_response_for(&state, SEAT_FRESHNESS_PATH, &digest, 0, status, &message)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    };
    let push = match serde_json::from_slice::<SeatFreshnessPush>(&body)
        .map_err(anyhow::Error::from)
        .and_then(|push| push.validate().map(|()| push))
    {
        Ok(push) => push,
        Err(error) => return refuse(StatusCode::BAD_REQUEST, format!("{error:#}")),
    };
    let received = ReceivedSeatFreshness { from: sender.name, seats: push.seats };
    let local = crate::client::Client::unix(state.backend().socket());
    match local
        .post::<_, SeatFreshnessAck>(SEAT_FRESHNESS_INTERNAL_PATH, &received)
        .await
    {
        Ok(ack) => signed_response_for(&state, SEAT_FRESHNESS_PATH, &digest, 0, ack)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(error) => refuse(StatusCode::SERVICE_UNAVAILABLE, format!("{error:#}")),
    }
}

impl ClientRelay {
    /// Push `push` to member `name` itself, over a route this node can dial, within `timeout`.
    /// `None` when no route reaches it directly, or when it does not receive the route (an
    /// unsigned 404 or 405): its advertisement was ahead of its build.
    pub async fn push_seat_freshness(
        &self,
        name: &str,
        push: &SeatFreshnessPush,
        timeout: Duration,
    ) -> Result<Option<SeatFreshnessAck>> {
        push.validate()?;
        let body = serde_json::to_vec(push)?;
        anyhow::ensure!(body.len() <= MAX_SEAT_FRESHNESS_BYTES, "the seat freshness push exceeds its bound");
        let digest = FleetAuth::body_digest(&body);
        let mut last_error = None;
        for peer in self.next_hops(name, &[]).into_iter().filter(|peer| peer.name == name) {
            match tokio::time::timeout(timeout, self.push_seat_freshness_to(&peer, &body, &digest)).await {
                Ok(Ok(ack)) => return Ok(ack),
                Ok(Err(error)) => last_error = Some(error),
                Err(_) => last_error = Some(anyhow::anyhow!("{} did not answer in {timeout:?}", peer.name)),
            }
        }
        match last_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    async fn push_seat_freshness_to(
        &self,
        peer: &PeerConfig,
        body: &[u8],
        digest: &str,
    ) -> Result<Option<SeatFreshnessAck>> {
        let url = match parse_route(&peer.url).context("invalid peer route")? {
            Route::Http(url) => url,
            Route::Fabric { node, protocol } => {
                let fabric = self.fabric.as_ref().context("Fabric is unavailable")?;
                format!("http://{}", fabric.dial(&node, &protocol).await?)
            }
        };
        let headers = self.auth.request_headers_method("POST", SEAT_FRESHNESS_PATH, &self.node, body)?;
        let mut response = self
            .http
            .post(format!("{}{}", url.trim_end_matches('/'), SEAT_FRESHNESS_PATH))
            .headers(headers)
            .header("content-type", "application/json")
            .body(body.to_vec())
            .send()
            .await
            .with_context(|| format!("push seat freshness to {}", peer.name))?;
        let status = response.status();
        // A worker without the route answers with Axum's unsigned 404/405.
        if matches!(status, StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED) {
            return Ok(None);
        }
        anyhow::ensure!(status != StatusCode::UNAUTHORIZED, "{} does not accept this node's requests", peer.name);
        let response_headers = response.headers().clone();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_SEAT_FRESHNESS_BYTES,
                "the seat freshness answer exceeds its bound"
            );
            bytes.extend_from_slice(&chunk);
        }
        self.auth.verify(&response_headers, "RESPONSE", SEAT_FRESHNESS_PATH, &bytes, Some(&peer.name), Some(digest))?;
        let envelope: ApiResponse<Value> = serde_json::from_slice(&bytes)?;
        // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
        anyhow::ensure!(
            smallclaims::sync::ACCEPTED_PEER_API_VERSIONS.contains(&envelope.api_version.as_str()),
            "the peer seat freshness protocol differs"
        );
        // LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both
        anyhow::ensure!(status.is_success(), "{} refused seat freshness: {}", peer.name, envelope.value);
        Ok(Some(serde_json::from_value(envelope.value)?))
    }
}
