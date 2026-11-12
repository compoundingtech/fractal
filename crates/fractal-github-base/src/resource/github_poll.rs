//! Completed reads belong to one unfinished poll, never to a published snapshot.
use std::{cell::RefCell, collections::BTreeMap, future::Future, time::{SystemTime, UNIX_EPOCH}};

use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::{LocalObserverBudget, ObservationRequest, ProviderObservation};

const MAX_CHECKPOINT_BYTES: usize = 1024 * 1024;
const MAX_AGE_MS: u128 = 2 * 60 * 60 * 1000;

tokio::task_local! {
    static POLL: RefCell<Checkpoint>;
}

#[cfg(test)]
tokio::task_local! {
    pub(super) static CUTOFF: usize;
}

#[derive(Default, Deserialize, Serialize)]
struct Checkpoint {
    poll_started_ms: u128,
    cursor: Option<String>,
    scope: String,
    reads: BTreeMap<String, Value>,
}

/// Durable position after an unfinished budget-cut poll; never a published snapshot.
#[derive(Debug)]
pub struct BudgetContinuation(pub Option<String>, String);

impl std::fmt::Display for BudgetContinuation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}
impl std::error::Error for BudgetContinuation {}

/// Mirror continuity follows completed observations, not private staging envelopes.
pub fn completed_poll_cursor(cursor: Option<&str>) -> Option<std::borrow::Cow<'_, str>> {
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(rename = "poll_started_ms")]
        _poll_started_ms: u128,
        cursor: Option<String>,
    }
    let cursor = cursor?;
    match serde_json::from_str::<Envelope>(cursor) {
        Ok(poll) => poll.cursor.map(std::borrow::Cow::Owned),
        Err(_) => Some(std::borrow::Cow::Borrowed(cursor)),
    }
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

pub(super) fn started() -> SystemTime {
    POLL.with(|poll| UNIX_EPOCH + std::time::Duration::from_millis(
        u64::try_from(poll.borrow().poll_started_ms).unwrap_or(u64::MAX),
    ))
}

impl Checkpoint {
    fn restore(cursor: Option<&str>, scope: &str, now: u128) -> Option<Self> {
        let checkpoint: Self = serde_json::from_str(cursor?).ok()?;
        (checkpoint.scope == scope && now.saturating_sub(checkpoint.poll_started_ms) <= MAX_AGE_MS)
            .then_some(checkpoint)
    }

    fn encode(&self) -> Result<Option<String>> {
        let mut value = serde_json::to_value(self)?;
        if let Some(cursor) = self.cursor.as_deref()
            && let Ok(completed) = serde_json::from_str::<serde_json::Map<String, Value>>(cursor)
        {
            value.as_object_mut().expect("checkpoint is an object").extend(completed);
        }
        let encoded = serde_json::to_string(&value)?;
        if encoded.len() > MAX_CHECKPOINT_BYTES {
            tracing::debug!("GitHub poll checkpoint exceeds size bound; retaining previous cursor");
            return Ok(None);
        }
        Ok(Some(encoded))
    }
}

pub(super) async fn observe<F, Fut>(mut request: ObservationRequest, run: F) -> Result<ProviderObservation>
where
    F: FnOnce(ObservationRequest) -> Fut,
    Fut: Future<Output = Result<ProviderObservation>>,
{
    let scope = format!("{} {:?}", request.locator, request.fields);
    let now = now_ms();
    let checkpoint = Checkpoint::restore(request.cursor.as_deref(), &scope, now)
        .unwrap_or_else(|| Checkpoint {
            poll_started_ms: now,
            // Expired checkpoints restart from the last completed observation, not staged facts.
            cursor: request.cursor.as_deref().and_then(|cursor| {
                serde_json::from_str::<Checkpoint>(cursor).map_or_else(|_| Some(cursor.to_owned()), |poll| poll.cursor)
            }),
            scope,
            ..Checkpoint::default()
        });
    request.cursor.clone_from(&checkpoint.cursor);
    let previous_cursor = request.cursor.clone();
    let resumed = !checkpoint.reads.is_empty();
    POLL.scope(RefCell::new(checkpoint), async move {
        let result = run(request).await;
        match result {
            Err(error) if error.downcast_ref::<LocalObserverBudget>().is_some() => {
                let cursor = POLL.with(|poll| poll.borrow().encode())?;
                let reason = error.to_string();
                Err(match cursor {
                    Some(cursor) => error.context(BudgetContinuation(Some(cursor), reason)),
                    None => error.context(BudgetContinuation(previous_cursor, reason)),
                })
            }
            Err(error) if resumed => {
                let reason = error.to_string();
                Err(error.context(BudgetContinuation(previous_cursor, reason)))
            }
            other => other,
        }
    }).await
}

pub(super) async fn read<T, F>(key: String, fetch: F) -> Result<T>
where
    T: Serialize + DeserializeOwned,
    F: Future<Output = Result<T>>,
{
    if let Some(value) = POLL.try_with(|poll| poll.borrow().reads.get(&key).cloned()).ok().flatten() {
        return Ok(serde_json::from_value(value)?);
    }
    #[cfg(test)]
    if CUTOFF.try_with(|cutoff| POLL.with(|poll| poll.borrow().reads.len() >= *cutoff)).unwrap_or(false) {
        return Err(LocalObserverBudget { retry_at_unix_ms: now_ms() + 60_000 }.into());
    }
    let value = fetch.await?;
    if POLL.try_with(|_| ()).is_ok() {
        let staged = serde_json::to_value(&value)?;
        POLL.with(|poll| poll.borrow_mut().reads.insert(key, staged));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_size_is_bounded() {
        let mut checkpoint = Checkpoint::default();
        checkpoint.reads.insert("page".into(), Value::String("x".repeat(MAX_CHECKPOINT_BYTES)));
        assert!(checkpoint.encode().unwrap().is_none());
        checkpoint.reads.clear();
        assert!(checkpoint.encode().unwrap().is_some());
    }

    #[test]
    fn checkpoint_expires_after_two_hours() {
        let checkpoint = Checkpoint { poll_started_ms: 1, scope: "repo".into(), ..Checkpoint::default() };
        let encoded = checkpoint.encode().unwrap().unwrap();
        assert!(Checkpoint::restore(Some(&encoded), "repo", MAX_AGE_MS + 1).is_some());
        assert!(Checkpoint::restore(Some(&encoded), "repo", MAX_AGE_MS + 2).is_none());
    }

    #[test]
    fn checkpoint_preserves_completed_cursor_for_old_readers_and_mirrors() {
        let completed = r#"{"items_since":"2026-10-30T00:00:00Z","comments_since":"2026-10-30T01:00:00Z","seen":{"items":"etag"},"resume_next":"page-2"}"#;
        let checkpoint = Checkpoint { cursor: Some(completed.into()), ..Checkpoint::default() };
        let encoded = checkpoint.encode().unwrap().unwrap();
        type Cursor = super::super::github_repository::RepositoryCursor;
        assert_eq!(serde_json::from_str::<Cursor>(&encoded).unwrap(), serde_json::from_str::<Cursor>(completed).unwrap());
        assert_eq!(completed_poll_cursor(Some(&encoded)).as_deref(), Some(completed));
    }
}
