//! Seat freshness answered by the seat's host (contract C3 of the claims-store breakup).
//!
//! A re-stamp that changes nothing stays in its host's local observation log, so another member
//! learns how recently a seat was heard by asking that host. A member asks only a peer whose own
//! newest `daemon.started` advertises [`SEAT_FRESHNESS_FEATURE`]; an older peer is never sent the
//! request, and its seats keep replicating their re-stamps.

use super::*;

/// The member-authenticated route a seat's host answers.
pub const SEAT_FRESHNESS_PATH: &str = "/v1/peer/seat-freshness";
/// The daemon route the worker answers from.
pub const SEAT_FRESHNESS_INTERNAL_PATH: &str = "/v1/internal/seat-freshness";
/// The `daemon.started` feature of a member that serves [`SEAT_FRESHNESS_PATH`].
pub const SEAT_FRESHNESS_FEATURE: &str = "seat_freshness";
/// The most seats one request may name.
pub const MAX_SEAT_FRESHNESS_AGENTS: usize = 2_048;
const MAX_SEAT_FRESHNESS_AGENT_BYTES: usize = 256;
/// Bounds the request and the answer: each seat is a subject, an incarnation and a time.
pub const MAX_SEAT_FRESHNESS_BYTES: usize = 1_048_576;
const SEAT_FRESHNESS_TIMEOUT: Duration = Duration::from_secs(10);

/// The seats a member asks one host about: those its declarations place on that host.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatFreshnessRequest {
    pub agents: Vec<String>,
}

impl SeatFreshnessRequest {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.agents.len() <= MAX_SEAT_FRESHNESS_AGENTS,
            "a seat freshness request names at most {MAX_SEAT_FRESHNESS_AGENTS} seats"
        );
        for agent in &self.agents {
            anyhow::ensure!(
                agent.starts_with("agent/") && agent.len() <= MAX_SEAT_FRESHNESS_AGENT_BYTES,
                "a seat freshness request names agents only"
            );
        }
        Ok(())
    }
}

/// When the host last heard one seat: its newest local harness report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatHeard {
    pub agent: String,
    pub incarnation_id: String,
    pub heard_at_unix_ms: u64,
}

/// The host's answer. A seat it holds no local report of is left out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatFreshnessReply {
    pub seats: Vec<SeatHeard>,
}

pub(super) async fn receive_seat_freshness(
    State(state): State<PeerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match state.auth().verify_sender(&headers, "POST", SEAT_FRESHNESS_PATH, &body, None, None) {
        Ok(sender) if state.accept(&sender).is_ok() => state.note_activity(&sender.name),
        _ => return (StatusCode::UNAUTHORIZED, "untrusted fleet seat freshness").into_response(),
    }
    let digest = FleetAuth::body_digest(&body);
    let refuse = |status: StatusCode, message: String| {
        signed_error_response_for(&state, SEAT_FRESHNESS_PATH, &digest, 0, status, &message)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    };
    let request = match serde_json::from_slice::<SeatFreshnessRequest>(&body)
        .map_err(anyhow::Error::from)
        .and_then(|request| request.validate().map(|()| request))
    {
        Ok(request) => request,
        Err(error) => return refuse(StatusCode::BAD_REQUEST, format!("{error:#}")),
    };
    let local = crate::client::Client::unix(state.backend().socket());
    match local
        .post::<_, SeatFreshnessReply>(SEAT_FRESHNESS_INTERNAL_PATH, &request)
        .await
    {
        Ok(reply) => signed_response_for(&state, SEAT_FRESHNESS_PATH, &digest, 0, reply)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(error) => refuse(StatusCode::SERVICE_UNAVAILABLE, format!("{error:#}")),
    }
}

impl ClientRelay {
    /// Ask member `name` itself, over a route this node can dial, when it last heard each of
    /// `request`'s seats. `None` when no route reaches it directly, or when it does not serve
    /// the route (an unsigned 404 or 405). The caller asks only a member that advertises
    /// [`SEAT_FRESHNESS_FEATURE`], so that answer means its advertisement was ahead of its build.
    pub async fn seat_freshness(
        &self,
        name: &str,
        request: &SeatFreshnessRequest,
    ) -> Result<Option<SeatFreshnessReply>> {
        request.validate()?;
        let body = serde_json::to_vec(request)?;
        anyhow::ensure!(body.len() <= MAX_SEAT_FRESHNESS_BYTES, "the seat freshness request exceeds its bound");
        let digest = FleetAuth::body_digest(&body);
        let mut last_error = None;
        for peer in self.next_hops(name, &[]).into_iter().filter(|peer| peer.name == name) {
            match self.seat_freshness_from(&peer, &body, &digest).await {
                Ok(reply) => return Ok(reply),
                Err(error) => last_error = Some(error),
            }
        }
        match last_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    async fn seat_freshness_from(
        &self,
        peer: &PeerConfig,
        body: &[u8],
        digest: &str,
    ) -> Result<Option<SeatFreshnessReply>> {
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
            .timeout(SEAT_FRESHNESS_TIMEOUT)
            .body(body.to_vec())
            .send()
            .await
            .with_context(|| format!("ask {} for seat freshness", peer.name))?;
        let status = response.status();
        // A worker without the route answers with Axum's unsigned 404/405.
        if matches!(status, StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED) {
            return Ok(None);
        }
        anyhow::ensure!(status != StatusCode::UNAUTHORIZED, "{} does not accept this node's requests", peer.name);
        let response_headers = response.headers().clone();
        anyhow::ensure!(
            response.content_length().is_none_or(|length| length <= MAX_SEAT_FRESHNESS_BYTES as u64),
            "the seat freshness answer exceeds its bound"
        );
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
        anyhow::ensure!(envelope.api_version == "st3.v1", "the peer seat freshness protocol differs");
        anyhow::ensure!(status.is_success(), "{} refused seat freshness: {}", peer.name, envelope.value);
        Ok(Some(serde_json::from_value(envelope.value)?))
    }
}
