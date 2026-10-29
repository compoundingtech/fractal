//! Caller-bound read receipts leave the request path through one group-commit writer.
//!
//! A receipt is a read-used claim and the read-outcome claim that cites it. Both are appended
//! in one writer transaction, so the outcome's evidence and predecessor name the used claim's
//! ID as that transaction generated it. One background thread drains every receipt queued
//! behind the one it woke for into a single `batched` transaction per store; it never lingers,
//! so a lone receipt commits at once. At most `capacity` receipts are reserved, queued or in
//! flight: `reserve` beyond that awaits space and nothing is dropped. A caller reserves space
//! before its final authority checks, then queues synchronously, so no wait separates the two.
//!
//! The bodies are those of the synchronous `assistant::append`: `{fields, evidence}` with the
//! fields as one sorted map. Inputs are validated before they are queued, so a malformed receipt
//! fails its caller. A commit failure after enqueue returned is logged, counted, and answered by
//! the next `flush`. `shutdown` closes admission and commits everything admitted before it.
//! Dropping the writer commits everything queued and joins its thread.
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use opentelemetry::metrics::{Counter, Histogram};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};

use super::authorization::{OUTCOME, USED};
use crate::api::ApiError;
use crate::model::ClaimInput;
use crate::store::{Store, append_claim_tx};
use smallclaims::store::latest_claim_id_tx;

/// Receipts queued or being committed before an enqueue waits for space. Space returns only
/// when a receipt's transaction ends, so this also bounds what one drain takes.
const CAPACITY: usize = 1024;

/// Cumulative commit outcomes, for benchmarks and tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReceiptStats {
    /// Successful writer transactions.
    pub commits: u64,
    /// Receipts those transactions committed.
    pub receipts: u64,
    /// Receipts that failed to commit.
    pub failed: u64,
    /// Receipts per successful transaction → number of such transactions.
    pub batch_sizes: BTreeMap<usize, u64>,
}

pub struct ReceiptWriter {
    permits: Arc<Semaphore>,
    // Queueing and the shutdown marker are ordered under this lock.
    admission: Mutex<Admission>,
    stats: Arc<Mutex<ReceiptStats>>,
}

#[derive(Default)]
struct Admission {
    closed: bool,
    worker: Option<Worker>,
}

struct Worker {
    commands: mpsc::UnboundedSender<Command>,
    thread: JoinHandle<()>,
}

/// Space for one receipt in the writer that reserved it.
pub struct ReceiptPermit(OwnedSemaphorePermit);

pub(super) enum AdmissionError {
    ReadEnded(ApiError),
    Writer(ApiError),
}

impl AdmissionError {
    pub(super) fn into_api(self) -> ApiError {
        match self { Self::ReadEnded(error) | Self::Writer(error) => error }
    }
}

impl From<ApiError> for AdmissionError {
    fn from(error: ApiError) -> Self { Self::Writer(error) }
}

fn read_ended(error: smallclaims::error::Error) -> AdmissionError {
    AdmissionError::ReadEnded(ApiError::bad(error))
}

enum Command {
    Receipt(Receipt),
    Flush(oneshot::Sender<Result<(), String>>),
}

struct Receipt {
    // Held only while queued: the writer thread keeps no store between drains.
    store: Arc<Store>,
    subject: String,
    actor: Option<String>,
    used: BTreeMap<String, Value>,
    outcome: BTreeMap<String, Value>,
    _space: ReceiptPermit,
}

impl Default for ReceiptWriter {
    fn default() -> Self {
        Self::with_capacity(CAPACITY)
    }
}

impl ReceiptWriter {
    /// A writer that holds at most `capacity` receipts reserved, queued or in flight.
    ///
    /// # Panics
    /// When `capacity` is zero: no receipt could ever be reserved.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0, "an assistant read receipt writer needs space for one receipt");
        Self { permits: Arc::new(Semaphore::new(capacity)), admission: Mutex::default(), stats: Arc::default() }
    }

    /// Reserve space for one receipt, waiting while the writer is full.
    ///
    /// # Errors
    /// The writer was shut down, or the current read reached its deadline or was cancelled.
    pub async fn reserve(&self) -> Result<ReceiptPermit, ApiError> {
        self.reserve_for_read().await.map_err(AdmissionError::into_api)
    }

    pub(super) async fn reserve_for_read(&self) -> Result<ReceiptPermit, AdmissionError> {
        let budget = smallclaims::read_budget::current();
        if let Some(budget) = &budget { budget.check().map_err(read_ended)?; }
        // Ready admission does not allocate a cancellation subscription or install a timer.
        let space = match self.permits.clone().try_acquire_owned() {
            Ok(space) => space,
            Err(tokio::sync::TryAcquireError::Closed) => return Err(closed().into()),
            Err(tokio::sync::TryAcquireError::NoPermits) => {
                let admission = self.permits.clone().acquire_owned();
                if let Some(budget) = &budget {
                    tokio::select! {
                        biased;
                        _ = budget.cancelled() => {
                            budget.check().map_err(read_ended)?;
                            unreachable!("a read budget wakes only after its deadline or cancellation");
                        }
                        space = admission => space.map_err(|_| closed())?,
                    }
                } else {
                    admission.await.map_err(|_| closed())?
                }
            }
        };
        if let Some(budget) = &budget { budget.check().map_err(read_ended)?; }
        Ok(ReceiptPermit(space))
    }

    /// Queue one caller-bound receipt into reserved space without waiting. It returns once the
    /// receipt is queued, not committed.
    ///
    /// # Errors
    /// A body that is not a valid claim object, space reserved from another writer, a writer that
    /// was shut down, or a writer thread that stopped.
    pub fn enqueue_reserved(&self, store: &Arc<Store>, subject: String, actor: String,
                            used: Value, outcome: Value, space: ReceiptPermit) -> Result<(), ApiError> {
        if !Arc::ptr_eq(space.0.semaphore(), &self.permits) {
            return Err(ApiError::internal("receipt space was reserved from another writer"));
        }
        // Validate both claims with one input, retaining only their shared identity
        // and fields in the queue rather than two copies of all claim metadata.
        let mut claim = input(store, subject, OUTCOME, actor, outcome)?;
        let outcome = std::mem::replace(&mut claim.fields,
            serde_json::from_value(used).map_err(ApiError::internal)?);
        claim.kind.clear();
        claim.kind.push_str(USED);
        store.validate_claim_input(&claim).map_err(ApiError::bad)?;
        let mut admission = self.admission();
        if admission.closed { return Err(closed()); }
        let worker = match &mut admission.worker {
            Some(worker) => worker,
            empty => empty.insert(self.start()?),
        };
        worker.commands.send(Command::Receipt(Receipt { store: store.clone(),
            subject: claim.subject, actor: claim.actor, used: claim.fields, outcome, _space: space }))
            .map_err(|_| stopped())
    }

    /// Reserve space, then queue one caller-bound receipt.
    ///
    /// # Errors
    /// As `reserve`, then as `enqueue_reserved`.
    pub async fn enqueue(&self, store: &Arc<Store>, subject: String, actor: String,
                         used: Value, outcome: Value) -> Result<(), ApiError> {
        let space = self.reserve().await?;
        self.enqueue_reserved(store, subject, actor, used, outcome, space)
    }

    /// Wait until every receipt queued before this call has committed or failed.
    ///
    /// # Errors
    /// Any receipt that failed to commit since the previous flush, or a writer thread that stopped.
    pub async fn flush(&self) -> Result<(), ApiError> {
        let answer = self.mark()?;
        answered(answer).await
    }

    /// Refuse every later reservation and receipt, wake callers waiting for space with an error,
    /// and wait until every receipt admitted before this call has committed or failed.
    ///
    /// # Errors
    /// As `flush`.
    pub async fn shutdown(&self) -> Result<(), ApiError> {
        let answer = {
            let mut admission = self.admission();
            admission.closed = true;
            self.permits.close();
            Self::marker(&admission)?
        };
        answered(answer).await
    }

    #[must_use]
    pub fn stats(&self) -> ReceiptStats {
        self.stats.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn admission(&self) -> MutexGuard<'_, Admission> {
        self.admission.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn mark(&self) -> Result<Option<oneshot::Receiver<Result<(), String>>>, ApiError> {
        Self::marker(&self.admission())
    }

    /// Queue a flush marker behind everything already admitted; `None` when nothing ever was.
    fn marker(admission: &Admission) -> Result<Option<oneshot::Receiver<Result<(), String>>>, ApiError> {
        let Some(worker) = &admission.worker else { return Ok(None) };
        let (reply, answer) = oneshot::channel();
        worker.commands.send(Command::Flush(reply)).map_err(|_| stopped())?;
        Ok(Some(answer))
    }

    fn start(&self) -> Result<Worker, ApiError> {
        let (commands, received) = mpsc::unbounded_channel();
        let stats = self.stats.clone();
        let thread = std::thread::Builder::new().name("st3-assistant-receipts".into())
            .spawn(move || run(received, &stats))
            .map_err(ApiError::internal)?;
        Ok(Worker { commands, thread })
    }
}

async fn answered(answer: Option<oneshot::Receiver<Result<(), String>>>) -> Result<(), ApiError> {
    let Some(answer) = answer else { return Ok(()) };
    answer.await.map_err(|_| stopped())?.map_err(ApiError::internal)
}

impl Drop for ReceiptWriter {
    /// Closing the queue lets the thread commit what remains before it exits. A caller must not
    /// hold this store's writer guard while the last state reference drops.
    fn drop(&mut self) {
        let Some(Worker { commands, thread }) = self.admission.get_mut()
            .unwrap_or_else(PoisonError::into_inner).worker.take() else { return };
        drop(commands);
        if thread.join().is_err() {
            tracing::error!("the assistant read receipt writer panicked; queued receipts were not committed");
        }
    }
}

fn input(store: &Store, subject: String, kind: &str, actor: String, fields: Value) -> Result<ClaimInput, ApiError> {
    let input = ClaimInput {
        subject, kind: kind.into(), actor: Some(actor),
        fields: serde_json::from_value(fields).map_err(ApiError::internal)?,
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    };
    store.validate_claim_input(&input).map_err(ApiError::bad)?;
    Ok(input)
}

fn stopped() -> ApiError {
    ApiError::internal("the assistant read receipt writer stopped")
}

fn closed() -> ApiError {
    ApiError::internal("the assistant read receipt writer is shut down")
}

#[derive(Default)]
struct Failures {
    count: u64,
    first: Option<String>,
}

impl Failures {
    fn take(&mut self) -> Result<(), String> {
        let Self { count, first } = std::mem::take(self);
        match first {
            None => Ok(()),
            Some(first) => Err(format!("{count} assistant read receipts failed to commit; first: {first}")),
        }
    }
}

fn run(mut commands: mpsc::UnboundedReceiver<Command>, stats: &Mutex<ReceiptStats>) {
    let mut failures = Failures::default();
    let mut receipts = Vec::new();
    while let Some(first) = commands.blocking_recv() {
        let mut next = Some(first);
        while let Some(command) = next {
            match command {
                Command::Receipt(receipt) => receipts.push(receipt),
                Command::Flush(reply) => {
                    commit_all(&mut receipts, stats, &mut failures);
                    let _ = reply.send(failures.take());
                }
            }
            next = commands.try_recv().ok();
        }
        commit_all(&mut receipts, stats, &mut failures);
    }
}

/// Commit queued receipts in order, one transaction per run of receipts for the same store.
fn commit_all(receipts: &mut Vec<Receipt>, stats: &Mutex<ReceiptStats>, failures: &mut Failures) {
    let mut rest = &receipts[..];
    while let Some(first) = rest.first() {
        let length = rest.iter().take_while(|receipt| Arc::ptr_eq(&receipt.store, &first.store)).count();
        let (run, after) = rest.split_at(length);
        commit_run(&first.store, run, stats, failures);
        rest = after;
    }
    receipts.clear();
}

fn commit_run(store: &Store, run: &[Receipt], stats: &Mutex<ReceiptStats>, failures: &mut Failures) {
    let started = Instant::now();
    match commit(store, run) {
        Ok(()) => {
            tracing::debug!(receipt_count = run.len(), elapsed_ms = started.elapsed().as_secs_f64() * 1000.,
                "committed assistant read receipts");
            if crate::otel::metrics_enabled() {
                INSTRUMENTS.batch_size.record(run.len() as u64, &[]);
            }
            let mut stats = stats.lock().unwrap_or_else(PoisonError::into_inner);
            stats.commits += 1;
            stats.receipts += run.len() as u64;
            *stats.batch_sizes.entry(run.len()).or_default() += 1;
        }
        // A failed transaction wrote none of its receipts: every one of them failed.
        Err(error) => {
            for receipt in run {
                tracing::error!(subject = %receipt.subject, %error, receipt_count = run.len(),
                    "an assistant read receipt failed to commit");
            }
            if crate::otel::metrics_enabled() {
                INSTRUMENTS.failures.add(run.len() as u64, &[]);
            }
            stats.lock().unwrap_or_else(PoisonError::into_inner).failed += run.len() as u64;
            failures.count += run.len() as u64;
            failures.first.get_or_insert(error);
        }
    }
}

fn commit(store: &Store, run: &[Receipt]) -> Result<(), String> {
    let origin = store.origin();
    match store.connection.batched(|tx| -> anyhow::Result<()> {
        for receipt in run { append(tx, origin, receipt)?; }
        Ok(())
    }) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("{error:#}")),
        Err(error) => Err(error),
    }
}

fn append(tx: &rusqlite::Transaction<'_>, origin: &str, receipt: &Receipt) -> anyhow::Result<()> {
    let Receipt { subject, actor, used, outcome, .. } = receipt;
    // The synchronous append opened the subject with `expected_subject: Some(None)`.
    if let Some(head) = latest_claim_id_tx(tx, subject)? {
        anyhow::bail!("stale-subject: read receipt subject `{subject}` already has claim `{head}`");
    }
    let consumed = append_claim_tx(tx, origin, subject, USED, actor.as_deref(),
        &json!({"fields": used, "evidence": []}), &[], None)?;
    let link = std::slice::from_ref(&consumed.id);
    append_claim_tx(tx, origin, subject, OUTCOME, actor.as_deref(),
        &json!({"fields": outcome, "evidence": link}), link, None)?;
    Ok(())
}

struct Instruments {
    batch_size: Histogram<u64>,
    failures: Counter<u64>,
}

static INSTRUMENTS: LazyLock<Instruments> = LazyLock::new(|| {
    let meter = opentelemetry::global::meter("st3.assistant");
    Instruments {
        batch_size: meter.u64_histogram("st.assistant.read_receipts.batch.size")
            .with_unit("{receipt}")
            .with_description("Caller-bound assistant read receipts committed by one writer transaction")
            .with_boundaries(vec![1., 2., 4., 8., 16., 32., 64., 128., 256.])
            .build(),
        failures: meter.u64_counter("st.assistant.read_receipts.failures")
            .with_unit("{receipt}")
            .with_description("Caller-bound assistant read receipts that failed to commit after their read returned")
            .build(),
    }
});

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt as _;

    const ACTOR: &str = "assistant/chatgpt/session/assistant-grant/0123456789abcdef01234567";

    fn subject(number: usize) -> String { format!("custom/client/assistant-v1-read-{number:08}") }
    fn used() -> Value {
        json!({"grant_id":"assistant-grant/0123456789abcdef01234567","mode":"caller-bound-allow",
            "projection":"summary","request_digest":"digest","max_response_bytes":4096,
            "timestamp":"2026-10-26T00:00:00Z"})
    }
    fn outcome() -> Value {
        json!({"version":1,"outcome":"delivered","returned_bytes":12,"timestamp":"2026-10-26T00:00:01Z"})
    }
    fn claims(store: &Store, subject: &str, kind: &str) -> Vec<crate::model::ClaimRecord> {
        store.claims_for_subject_kind_at(subject, kind, None, false, 10).unwrap().claims
    }
    async fn enqueue(writer: &ReceiptWriter, store: &Arc<Store>, number: usize) {
        writer.enqueue(store, subject(number), ACTOR.into(), used(), outcome()).await.unwrap();
    }

    #[tokio::test]
    async fn enqueue_returns_while_the_writer_is_held_and_flush_waits_for_the_linked_commit() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let writer = ReceiptWriter::default();
        let held = store.connection.write();
        for number in 0..3 { enqueue(&writer, &store, number).await; }
        let mut flush = std::pin::pin!(writer.flush());
        assert!(flush.as_mut().now_or_never().is_none());
        assert!(claims(&store, &subject(0), USED).is_empty());
        drop(held);
        flush.await.unwrap();
        for number in 0..3 {
            let [used] = &claims(&store, &subject(number), USED)[..] else { panic!("one used claim") };
            let [outcome] = &claims(&store, &subject(number), OUTCOME)[..] else { panic!("one outcome claim") };
            assert!(used.predecessors.is_empty());
            assert_eq!(outcome.body["evidence"], json!([used.id]));
            assert_eq!(outcome.predecessors, vec![used.id.clone()]);
            assert_eq!(used.actor.as_deref(), Some(ACTOR));
        }
        // Receipts queued behind a held writer share at most one transaction after the first.
        let stats = writer.stats();
        assert_eq!((stats.receipts, stats.failed), (3, 0));
        assert!(stats.commits <= 2, "{stats:?}");
        assert_eq!(stats.batch_sizes.iter().map(|(size, count)| *size as u64 * count).sum::<u64>(), 3);
    }

    #[tokio::test]
    async fn queued_bodies_match_the_synchronous_append() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let writer = ReceiptWriter::default();
        enqueue(&writer, &store, 0).await;
        writer.flush().await.unwrap();
        let used_input = input(&store, subject(1), USED, ACTOR.into(), used()).unwrap();
        let direct_used = store.append_claim(&ClaimInput { expected_subject: Some(None), ..used_input }).unwrap();
        let outcome_input = input(&store, subject(1), OUTCOME, ACTOR.into(), outcome()).unwrap();
        store.append_claim(&ClaimInput {
            evidence: vec![direct_used.id.clone()], expected_subject: Some(Some(direct_used.id.clone())), ..outcome_input
        }).unwrap();
        let queued_used = claims(&store, &subject(0), USED).remove(0);
        let queued_outcome = claims(&store, &subject(0), OUTCOME).remove(0);
        // Compare the stored representation on both paths. append_claim returns its
        // insertion-ordered body, while reading a stored claim decodes canonical JSON.
        let direct_used = claims(&store, &subject(1), USED).remove(0);
        let direct_outcome = claims(&store, &subject(1), OUTCOME).remove(0);
        assert_eq!(queued_used.body.to_string(), direct_used.body.to_string());
        assert_eq!(queued_outcome.body.to_string().replace(&queued_used.id, "USED"),
            direct_outcome.body.to_string().replace(&direct_used.id, "USED"));
    }

    #[tokio::test]
    async fn a_full_queue_waits_for_space_and_drops_nothing() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let writer = ReceiptWriter::with_capacity(1);
        let held = store.connection.write();
        enqueue(&writer, &store, 0).await;
        let mut second = std::pin::pin!(enqueue(&writer, &store, 1));
        assert!(second.as_mut().now_or_never().is_none());
        drop(held);
        second.await;
        writer.flush().await.unwrap();
        for number in 0..2 {
            assert_eq!(claims(&store, &subject(number), OUTCOME).len(), 1);
        }
        assert_eq!(writer.stats().receipts, 2);
    }

    #[tokio::test]
    async fn dropping_the_writer_commits_what_is_queued() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let writer = ReceiptWriter::default();
        let held = store.connection.write();
        for number in 0..4 { enqueue(&writer, &store, number).await; }
        drop(held);
        drop(writer);
        for number in 0..4 {
            assert_eq!(claims(&store, &subject(number), USED).len(), 1);
            assert_eq!(claims(&store, &subject(number), OUTCOME).len(), 1);
        }
    }

    #[tokio::test]
    async fn invalid_bodies_fail_the_caller_before_queueing() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let writer = ReceiptWriter::default();
        assert!(writer.enqueue(&store, subject(0), ACTOR.into(), json!([1]), outcome()).await.is_err());
        assert!(writer.admission().worker.is_none());
        writer.flush().await.unwrap();
        let foreign = ReceiptWriter::default().reserve().await.unwrap();
        assert!(writer.enqueue_reserved(&store, subject(0), ACTOR.into(), used(), outcome(), foreign).is_err());
        assert!(writer.admission().worker.is_none());
    }

    #[tokio::test]
    async fn flush_reports_a_receipt_that_failed_after_enqueue_returned() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let writer = ReceiptWriter::default();
        enqueue(&writer, &store, 0).await;
        writer.flush().await.unwrap();
        // The same subject again: its used claim cannot open a second head.
        enqueue(&writer, &store, 0).await;
        assert!(writer.flush().await.is_err());
        assert_eq!(claims(&store, &subject(0), USED).len(), 1);
        assert_eq!(writer.stats().failed, 1);
        writer.flush().await.unwrap();
        enqueue(&writer, &store, 1).await;
        writer.flush().await.unwrap();
    }

    #[test]
    fn a_failed_transaction_fails_every_receipt_in_it() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let space = Arc::new(Semaphore::new(3));
        let receipt = |number| Receipt {
            store: store.clone(),
            subject: subject(number),
            actor: Some(ACTOR.into()),
            used: serde_json::from_value(used()).unwrap(),
            outcome: serde_json::from_value(outcome()).unwrap(),
            _space: ReceiptPermit(space.clone().try_acquire_owned().unwrap()),
        };
        let (stats, mut failures) = (Mutex::default(), Failures::default());
        commit_all(&mut vec![receipt(0)], &stats, &mut failures);
        failures.take().unwrap();
        // One stale subject between two fresh ones fails the transaction they share.
        commit_all(&mut vec![receipt(1), receipt(0), receipt(2)], &stats, &mut failures);
        let error = failures.take().unwrap_err();
        assert!(error.starts_with("3 assistant read receipts failed to commit"), "{error}");
        for number in [1, 2] {
            assert!(claims(&store, &subject(number), USED).is_empty());
            assert!(claims(&store, &subject(number), OUTCOME).is_empty());
        }
        assert_eq!(claims(&store, &subject(0), USED).len(), 1);
        let stats = stats.into_inner().unwrap();
        assert_eq!((stats.commits, stats.receipts, stats.failed), (1, 1, 3));
        assert_eq!(space.available_permits(), 3);
    }

    #[tokio::test]
    async fn shutdown_refuses_later_receipts_and_commits_those_admitted_before_it() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let writer = ReceiptWriter::default();
        let held = store.connection.write();
        for number in 0..2 { enqueue(&writer, &store, number).await; }
        let reserved = writer.reserve().await.unwrap();
        let mut shutdown = std::pin::pin!(writer.shutdown());
        assert!(shutdown.as_mut().now_or_never().is_none());
        assert!(writer.enqueue_reserved(&store, subject(2), ACTOR.into(), used(), outcome(), reserved).is_err());
        assert!(writer.reserve().await.is_err());
        drop(held);
        shutdown.await.unwrap();
        for number in 0..2 {
            assert_eq!(claims(&store, &subject(number), OUTCOME).len(), 1);
        }
        assert!(claims(&store, &subject(2), USED).is_empty());
        writer.flush().await.unwrap();
        assert_eq!(writer.stats().receipts, 2);
    }

    #[tokio::test]
    async fn shutdown_fails_a_caller_waiting_for_space() {
        let store = Arc::new(Store::open_memory("node").unwrap());
        let writer = ReceiptWriter::with_capacity(1);
        let held = store.connection.write();
        enqueue(&writer, &store, 0).await;
        let mut waiting = std::pin::pin!(writer.reserve());
        assert!(waiting.as_mut().now_or_never().is_none());
        let mut shutdown = std::pin::pin!(writer.shutdown());
        assert!(shutdown.as_mut().now_or_never().is_none());
        assert!(matches!(waiting.as_mut().now_or_never(), Some(Err(_))));
        drop(held);
        shutdown.await.unwrap();
        assert_eq!(claims(&store, &subject(0), OUTCOME).len(), 1);
    }

    #[tokio::test]
    async fn receipt_admission_releases_its_blocking_worker_when_the_read_ends() {
        use smallclaims::read_budget::{self, ReadBudget};
        use std::time::Duration;
        for cancelled in [false, true] {
            let store = Arc::new(Store::open_memory("node").unwrap());
            let writer = Arc::new(ReceiptWriter::with_capacity(1));
            let held = store.connection.write();
            enqueue(&writer, &store, 0).await;
            let (started, waiting) = oneshot::channel();
            let runtime = tokio::runtime::Handle::current();
            let queue = writer.clone();
            let target = store.clone();
            let worker = tokio::task::spawn_blocking(move || {
                let budget = if cancelled {
                    ReadBudget::cancellable("/v1/client/assistant/summary")
                } else {
                    ReadBudget::new("/v1/client/assistant/summary", Duration::from_millis(200))
                };
                read_budget::with(Some(budget.clone()), || runtime.block_on(async {
                    let mut request = std::pin::pin!(queue.enqueue(&target, subject(1), ACTOR.into(), used(), outcome()));
                    assert!(request.as_mut().now_or_never().is_none());
                    assert!(started.send(budget).is_ok());
                    request.await
                }))
            });
            let budget = waiting.await.unwrap();
            if cancelled { budget.cancel(); }
            let error = tokio::time::timeout(Duration::from_secs(2), worker).await
                .expect("receipt admission retained its blocking worker").unwrap().unwrap_err();
            assert_eq!(error.code, "read-deadline");
            assert_eq!(error.status, axum::http::StatusCode::GATEWAY_TIMEOUT);
            assert!(claims(&store, &subject(1), USED).is_empty());
            assert_eq!(writer.stats().receipts, 0);
            // Cancellation affected admission only, not the receipt admitted earlier.
            drop(held);
            writer.flush().await.unwrap();
            assert_eq!(claims(&store, &subject(0), OUTCOME).len(), 1);
            assert!(claims(&store, &subject(1), OUTCOME).is_empty());
            assert_eq!(writer.stats().receipts, 1);
        }
    }
}
