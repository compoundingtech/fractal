//! Read-position invalidations have their own socket so all inventory slots remain available.
use super::*;

fn unreachable(error: &ClientError) -> bool {
    matches!(error, ClientError::Transport(_) | ClientError::Unreachable(_)
        | ClientError::Api(ErrorCode::RemoteUnavailable, _, _))
}

/// Scoped to the selected member connection: dropping this future closes its socket and retry wait.
/// Refusals terminate the subscription visibly; only transport failures reconnect automatically.
pub(super) async fn run(client: &Client, person: &str, updates: &mpsc::Sender<Update>) {
    let mut failures = 0;
    loop {
        let outcome = match tokio::time::timeout(Duration::from_secs(5), client.collection_stream()).await {
            Ok(Ok(mut stream)) => follow(&mut stream, person, updates, &mut failures).await,
            Ok(Err(error)) if unreachable(&error) => Err(error.human()),
            Ok(Err(error)) => {
                failed(updates, error.human());
                return;
            }
            Err(_) => Err("Read-position connection timed out".into()),
        };
        match outcome {
            Ok(()) => return,
            Err(reason) => {
                if !failed(updates, reason) { return; }
                tokio::time::sleep(retry_delay(failures)).await;
                failures = failures.saturating_add(1);
            }
        }
    }
}

fn failed(updates: &mpsc::Sender<Update>, reason: String) -> bool {
    connection_log(&format!("read positions: {reason}"), true);
    updates.send(Update::WindowFailed(Window::ReadPositions, reason)).is_ok()
}

async fn follow(stream: &mut CollectionStream, person: &str, updates: &mpsc::Sender<Update>, failures: &mut usize) -> Result<(), String> {
    let window = Window::ReadPositions;
    if let Err(error) = stream.subscribe_read_positions(window.id(), person, window.limit()).await {
        if unreachable(&error) { return Err(error.human()); }
        failed(updates, error.human());
        return Ok(());
    }
    let heard = stream.heard();
    let mut pinged: Option<Instant> = None;
    let mut probe = tokio::time::interval(timer(QUIET_CHECK));
    probe.tick().await;
    loop {
        tokio::select! {
            _ = probe.tick() => {
                if heard.quiet_for() < timer(QUIET_BEFORE_PING) {
                    pinged = None;
                } else if let Some(at) = pinged {
                    if at.elapsed() >= timer(PONG_WAIT) {
                        return Err("Read-position connection stopped responding".into());
                    }
                } else {
                    stream.ping().await.map_err(|error| error.human())?;
                    pinged = Some(Instant::now());
                }
            }
            event = stream.next_event() => {
                let event = match event {
                    Ok(Some(event)) => event,
                    Ok(None) => return Err("Read-position connection closed".into()),
                    Err(error) if unreachable(&error) => return Err(error.human()),
                    Err(error) => {
                        failed(updates, error.human());
                        return Ok(());
                    }
                };
                match event {
                    CollectionEvent::Snapshot { id, snapshot, .. }
                    | CollectionEvent::Changes { id, snapshot, .. } if id == window.id() => {
                        *failures = 0;
                        // Rows are deliberately ignored: the UI re-lists all pages on invalidation.
                        if updates.send(Update::Window {
                            window, received: std::time::Instant::now(), snapshot,
                            items: Vec::new(), has_more: false,
                        }).is_err() { return Ok(()); }
                    }
                    CollectionEvent::Resync { id, code, message, retryable, .. } if id == window.id() => {
                        if code == Some(ErrorCode::RemoteUnavailable) {
                            return Err(message.unwrap_or_else(|| "Read-position peer is unreachable".into()));
                        }
                        if retryable == Some(false) || message.is_some() {
                            failed(updates, fractal_client::human_message(code.as_ref(),
                                message.as_deref().unwrap_or("Read-position subscription refused")));
                            return Ok(());
                        }
                        if let Err(error) = stream.subscribe_read_positions(window.id(), person, window.limit()).await {
                            if unreachable(&error) { return Err(error.human()); }
                            failed(updates, error.human());
                            return Ok(());
                        }
                    }
                    CollectionEvent::Error { code, message, .. } => {
                        if code == Some(ErrorCode::RemoteUnavailable) {
                            return Err(message);
                        }
                        failed(updates, fractal_client::human_message(code.as_ref(), &message));
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
}
