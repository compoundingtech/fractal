//! Reads a member answers about its own seats, on the seat-freshness relay (db break-up step 4b).
//!
//! A consumer's delivery, read and close of a message stay on the consumer's host, never
//! replicated (`local_message_marks`, and a stream item's `stream_lifecycle`). Another member that
//! needs how far a message got asks that host, which answers for the seats its declarations place
//! on it. Nothing is pushed or kept: a host that does not answer in time reads as unknown.
//!
//! Each request is one tagged operation, so a later operation joins without a new route. A member
//! asks only a host whose own newest `daemon.started` advertises [`SEAT_FRESHNESS_READS_FEATURE`]
//! at the operation's version.

use super::*;

/// The member-authenticated route a member answers its peers' reads on.
pub const SEAT_FRESHNESS_READS_PATH: &str = "/v1/peer/seat-freshness/reads";
/// The daemon route the worker answers a read through.
pub const SEAT_FRESHNESS_READS_INTERNAL_PATH: &str = "/v1/internal/seat-freshness/reads";
/// The `daemon.started` feature of a member that answers [`SEAT_FRESHNESS_READS_PATH`], and the
/// version of the operations it answers.
pub const SEAT_FRESHNESS_READS_FEATURE: &str = "seat_freshness_reads";
/// The most messages one read may name.
pub const MAX_READ_MESSAGES: usize = 512;
const MAX_READ_SUBJECT_BYTES: usize = 256;
/// Bounds a read's request.
pub const MAX_READ_REQUEST_BYTES: usize = 262_144;
/// Bounds a read's answer: each message's marks, at most one per kind and consumer.
pub const MAX_READ_ANSWER_BYTES: usize = 4_194_304;

/// One read a member asks another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum SeatFreshnessRead {
    /// How far the host's seats got with each of these messages.
    MessageMarks { messages: Vec<String> },
}

impl SeatFreshnessRead {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::MessageMarks { messages } => {
                anyhow::ensure!(
                    messages.len() <= MAX_READ_MESSAGES,
                    "a message marks read names at most {MAX_READ_MESSAGES} messages"
                );
                for message in messages {
                    anyhow::ensure!(
                        message.starts_with("message/") && message.len() <= MAX_READ_SUBJECT_BYTES,
                        "a message marks read names messages only"
                    );
                }
            }
        }
        Ok(())
    }
}

/// What the worker hands its daemon: the read, and the authenticated member that asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceivedSeatFreshnessRead {
    pub from: String,
    pub read: SeatFreshnessRead,
}

/// One consumer's delivery, read or close of a message, as the consumer's host recorded it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayedMark {
    pub kind: String,
    pub consumer: String,
    pub recorded_at_unix_ms: u64,
    pub receipt: String,
}

/// The host's answer for one message: whether it holds it, and its seats' marks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageMarksAnswer {
    pub subject: String,
    pub held: bool,
    pub marks: Vec<RelayedMark>,
}

/// The answer to [`SeatFreshnessRead::MessageMarks`], one entry per message asked, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageMarksAnswers {
    pub messages: Vec<MessageMarksAnswer>,
}

pub(super) async fn receive_seat_freshness_read(
    State(state): State<PeerState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let sender = match state.auth().verify_sender(
        &headers,
        "POST",
        SEAT_FRESHNESS_READS_PATH,
        &body,
        None,
        None,
    ) {
        Ok(sender) if state.accept(&sender).is_ok() => sender,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                "untrusted fleet seat freshness read",
            )
                .into_response();
        }
    };
    state.note_activity(&sender.name);
    let digest = FleetAuth::body_digest(&body);
    let refuse = |status: StatusCode, message: String| {
        signed_error_response_for(
            &state,
            SEAT_FRESHNESS_READS_PATH,
            &digest,
            0,
            status,
            &message,
        )
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    };
    let read = match serde_json::from_slice::<SeatFreshnessRead>(&body)
        .map_err(anyhow::Error::from)
        .and_then(|read| read.validate().map(|()| read))
    {
        Ok(read) => read,
        Err(error) => return refuse(StatusCode::BAD_REQUEST, format!("{error:#}")),
    };
    let received = ReceivedSeatFreshnessRead {
        from: sender.name,
        read,
    };
    let local = crate::client::Client::unix(state.backend().socket());
    match local
        .post::<_, Value>(SEAT_FRESHNESS_READS_INTERNAL_PATH, &received)
        .await
    {
        Ok(answer) => signed_response_for(&state, SEAT_FRESHNESS_READS_PATH, &digest, 0, answer)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(error) => refuse(StatusCode::SERVICE_UNAVAILABLE, format!("{error:#}")),
    }
}

impl ClientRelay {
    /// Ask member `name` itself how far its seats got with `messages`, over a route this node can
    /// dial, within `timeout`. `None` when no route reaches it directly, or when it does not
    /// answer the route (an unsigned 404 or 405): its advertisement was ahead of its build.
    pub async fn read_message_marks(
        &self,
        name: &str,
        messages: &[String],
        timeout: Duration,
    ) -> Result<Option<MessageMarksAnswers>> {
        let read = SeatFreshnessRead::MessageMarks {
            messages: messages.to_vec(),
        };
        read.validate()?;
        let body = serde_json::to_vec(&read)?;
        anyhow::ensure!(
            body.len() <= MAX_READ_REQUEST_BYTES,
            "the seat freshness read exceeds its bound"
        );
        let digest = FleetAuth::body_digest(&body);
        let mut last_error = None;
        for peer in self
            .next_hops(name, &[])
            .into_iter()
            .filter(|peer| peer.name == name)
        {
            match tokio::time::timeout(
                timeout,
                self.read_seat_freshness_from(&peer, &body, &digest),
            )
            .await
            {
                Ok(Ok(Some(answer))) => {
                    let answers: MessageMarksAnswers = serde_json::from_value(answer)?;
                    anyhow::ensure!(
                        answers.messages.len() == messages.len()
                            && answers
                                .messages
                                .iter()
                                .zip(messages)
                                .all(|(answer, asked)| &answer.subject == asked),
                        "{name} answered other messages than it was asked"
                    );
                    return Ok(Some(answers));
                }
                Ok(Ok(None)) => return Ok(None),
                Ok(Err(error)) => last_error = Some(error),
                Err(_) => {
                    last_error = Some(anyhow::anyhow!(
                        "{} did not answer in {timeout:?}",
                        peer.name
                    ))
                }
            }
        }
        match last_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    async fn read_seat_freshness_from(
        &self,
        peer: &PeerConfig,
        body: &[u8],
        digest: &str,
    ) -> Result<Option<Value>> {
        let url = match parse_route(&peer.url).context("invalid peer route")? {
            Route::Http(url) => url,
            Route::Fabric { node, protocol } => {
                let fabric = self.fabric.as_ref().context("Fabric is unavailable")?;
                format!("http://{}", fabric.dial(&node, &protocol).await?)
            }
        };
        let headers = self.auth.request_headers_method(
            "POST",
            SEAT_FRESHNESS_READS_PATH,
            &self.node,
            body,
        )?;
        let mut response = self
            .http
            .post(format!(
                "{}{}",
                url.trim_end_matches('/'),
                SEAT_FRESHNESS_READS_PATH
            ))
            .headers(headers)
            .header("content-type", "application/json")
            .body(body.to_vec())
            .send()
            .await
            .with_context(|| format!("read seat freshness from {}", peer.name))?;
        let status = response.status();
        // A worker without the route answers with Axum's unsigned 404/405.
        if matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
        ) {
            return Ok(None);
        }
        anyhow::ensure!(
            status != StatusCode::UNAUTHORIZED,
            "{} does not accept this node's requests",
            peer.name
        );
        let response_headers = response.headers().clone();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_READ_ANSWER_BYTES,
                "the seat freshness answer exceeds its bound"
            );
            bytes.extend_from_slice(&chunk);
        }
        self.auth.verify(
            &response_headers,
            "RESPONSE",
            SEAT_FRESHNESS_READS_PATH,
            &bytes,
            Some(&peer.name),
            Some(digest),
        )?;
        let envelope: ApiResponse<Value> = serde_json::from_slice(&bytes)?;
        // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
        anyhow::ensure!(
            smallclaims::sync::ACCEPTED_PEER_API_VERSIONS.contains(&envelope.api_version.as_str()),
            "the peer seat freshness protocol differs"
        );
        // LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both
        anyhow::ensure!(
            status.is_success(),
            "{} refused the seat freshness read: {}",
            peer.name,
            envelope.value
        );
        Ok(Some(envelope.value))
    }
}
