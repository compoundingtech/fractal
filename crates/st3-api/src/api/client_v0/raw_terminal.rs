//! Full-fidelity terminal transport. The PTY, not a gateway emulator, owns replay and geometry.
use super::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;
pub mod lease;
pub mod peek_capability;
pub use lease::{Binding as LeaseBinding, Lease, ORIGIN_HEADER};

/// Run one lease query (revalidation, proof, or selected-use) as an admitted store read.
/// A lease is a per-socket authority handle, so each query owns its own clone.
pub async fn lease_query<T, F>(lease: Arc<Lease>, work: F) -> anyhow::Result<T>
where
    F: FnOnce(&Lease) -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    let store = lease
        .authority()
        .ok_or_else(|| anyhow::anyhow!("authority stopped"))?;
    crate::api::read_deadline::store_read(&store, move || work(&lease))
        .await
        .unwrap_or_else(|error| Err(anyhow::anyhow!(error.to_string())))
}

pub const CHUNK: usize = 16 * 1024;

/// One bounded byte splice; closing either direction drops the persistent owner connection.
/// A raw client may not use lifecycle/CAS/ancestry commands to escape terminal authority.
pub async fn splice(
    socket: WebSocket,
    transport: UnixStream,
    mode: Option<st3_client::RawTerminalMode>,
    lease: Option<Arc<Lease>>,
    control: Option<tokio::sync::mpsc::Sender<Value>>,
    _upgrade: Option<opentelemetry::trace::SpanContext>,
    mut first_frame: Option<tracing::Span>,
) {
    let (mut sink, mut source) = socket.split();
    let (mut reader, mut writer) = transport.into_split();
    let proof_ready = tokio::sync::Notify::new();
    let upload = async {
        let mut gate = FrameGate::new(mode);
        while let Some(Ok(message)) = source.next().await {
            match message {
                axum::extract::ws::Message::Text(text) => {
                    let Some(lease) = &lease else {
                        break;
                    };
                    let Ok(message) = serde_json::from_str::<lease::Control>(&text) else {
                        break;
                    };
                    let result = match message {
                        lease::Control::SelectedUse { sequence } => {
                            let result = lease_query(lease.clone(), move |lease| {
                                lease.selected_use(sequence)
                            })
                            .await;
                            if result.is_ok()
                                && let Some(control) = &control
                                && control
                                    .send(json!({"type":"selected-use","sequence":sequence}))
                                    .await
                                    .is_err()
                            {
                                break;
                            }
                            result
                        }
                        lease::Control::AuthorityProof {
                            lease_id,
                            watcher_epoch,
                            sequence,
                        } if lease.owner_side() && lease_id == lease.binding.lease_id => {
                            let result = lease_query(lease.clone(), move |lease| {
                                lease.proof(&watcher_epoch, sequence)
                            })
                            .await;
                            if result.is_ok() {
                                proof_ready.notify_one();
                            }
                            result
                        }
                        _ => break,
                    };
                    if result.is_err() {
                        break;
                    }
                }
                axum::extract::ws::Message::Binary(bytes) => {
                    if lease.as_ref().is_some_and(|lease| {
                        lease.check().is_err() || lease.owner_side() && !lease.has_proof()
                    }) {
                        break;
                    }
                    if gate.write(&mut writer, &bytes).await.is_err() {
                        break;
                    }
                }
                axum::extract::ws::Message::Ping(_) | axum::extract::ws::Message::Pong(_) => {}
                _ => break,
            }
        }
    };
    let download = async {
        let mut bytes = [0_u8; CHUNK];
        let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sequence = 0_u64;
        loop {
            tokio::select! {
                biased;
                _ = heartbeat.tick(), if lease.is_some() => {
                    let heartbeat_lease = lease.clone().expect("heartbeat requires lease");
                    if lease_query(heartbeat_lease.clone(), |lease| lease.revalidate())
                        .await
                        .is_err()
                    {
                        break;
                    }
                    let lease = heartbeat_lease;
                    sequence += 1;
                    if lease.owner_side() {
                        if sink.send(axum::extract::ws::Message::Text(json!({"type":"owner-proof","lease_id":lease.binding.lease_id,"watcher_epoch":lease.binding.gateway_epoch,"sequence":sequence,"issued_at_unix_ms":client_now_ms()}).to_string().into())).await.is_err() { break; }
                    } else if lease.binding.gateway == lease.binding.owner {
                        let epoch = lease.binding.gateway_epoch.clone();
                        let proofed = lease_query(lease.clone(), move |lease| {
                            lease.proof(&epoch, sequence)
                        })
                        .await
                        .is_ok();
                        if !proofed {
                            break;
                        }
                    }
                }
                // An accepted first proof enables reads immediately, not at the next heartbeat.
                () = proof_ready.notified() => {}
                read = reader.read(&mut bytes), if lease.as_ref().is_none_or(|lease| !lease.owner_side() || lease.has_proof()) => {
                    let Ok(count) = read else { break; };
                    if count == 0 || lease.as_ref().is_some_and(|lease| lease.check().is_err()) { break; }
                    if let Some(span) = &first_frame {
                        span.record("st.page.rows", 0_i64);
                        span.record("st.page.bytes", count as i64);
                        span.record("span.label", format!("terminal raw {count}"));
                    }
                    let first_span = first_frame.clone();
                    if first_frame_work(first_span, sink.send(axum::extract::ws::Message::Binary(bytes[..count].to_vec().into()))).await.is_err() { break; }
                    first_frame = None;
                }
            }
        }
    };
    let expired = async {
        if let Some(lease) = &lease {
            lease.expired().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! { biased; () = expired => {}, () = upload => {}, () = download => {} }
}

/// Validate frame headers before forwarding them, without copying or buffering payloads.
pub struct FrameGate {
    mode: Option<st3_client::RawTerminalMode>,
    header: [u8; 5],
    used: usize,
    remaining: usize,
    attached: bool,
}

impl FrameGate {
    pub fn new(mode: Option<st3_client::RawTerminalMode>) -> Self {
        Self {
            mode,
            header: [0; 5],
            used: 0,
            remaining: 0,
            attached: false,
        }
    }

    fn accept_header(&mut self) -> std::io::Result<()> {
        use pty_core::protocol::MessageType;
        let tag = MessageType::from_u8(self.header[0]);
        let length =
            u32::from_be_bytes(self.header[1..].try_into().expect("four-byte header")) as usize;
        let allowed = match self.mode {
            None => true,
            Some(st3_client::RawTerminalMode::Attach) if !self.attached => {
                tag == MessageType::Attach
            }
            Some(st3_client::RawTerminalMode::Peek) if !self.attached => tag == MessageType::Peek,
            Some(st3_client::RawTerminalMode::Attach) => matches!(
                tag,
                MessageType::Data
                    | MessageType::Resize
                    | MessageType::Detach
                    | MessageType::Status
                    | MessageType::ResetInputModes
            ),
            Some(st3_client::RawTerminalMode::Peek) => {
                matches!(tag, MessageType::Detach | MessageType::Status)
            }
        };
        if !allowed
            || length > 16 * 1024 * 1024
            || (tag == MessageType::ResetInputModes && length != 0)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "PTY frame exceeds raw terminal capability",
            ));
        }
        self.attached = true;
        self.remaining = length;
        Ok(())
    }

    pub async fn write(
        &mut self,
        writer: &mut (impl tokio::io::AsyncWrite + Unpin),
        mut bytes: &[u8],
    ) -> std::io::Result<()> {
        while !bytes.is_empty() {
            if self.remaining == 0 {
                let count = (5 - self.used).min(bytes.len());
                self.header[self.used..self.used + count].copy_from_slice(&bytes[..count]);
                self.used += count;
                bytes = &bytes[count..];
                if self.used < 5 {
                    continue;
                }
                self.accept_header()?;
                writer.write_all(&self.header).await?;
                self.used = 0;
            } else {
                let count = self.remaining.min(bytes.len());
                writer.write_all(&bytes[..count]).await?;
                bytes = &bytes[count..];
                self.remaining -= count;
            }
        }
        Ok(())
    }
}

