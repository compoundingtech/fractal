//! The part of `store::agent_messages` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use serde::Deserialize;
pub use st3_store::store::agent_messages::*;
pub use st3_store::store::agent_messages::{BACKFILL_PAGE, DAY_MS, backfill_page, create_schema, flush, open, ready, synthetic};

pub struct DailyUsage {
    pub since: u64,
    pub until: u64,
    pub cost: u64,
    pub unpriced: u64,
}

const ALLOWANCES: &str = "doc/usage/agent-message-allowances";

// The rest of the backfill in one transaction, for an estimate that arrives before the
// daemon's background pages finish.
fn ensure_ready(transaction: &Transaction<'_>) -> Result<()> {
    backfill_page(transaction, usize::MAX)?;
    Ok(())
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowance {
    low_microusd: u64,
    high_microusd: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    source: String,
    method: String,
    recipients: BTreeMap<String, Allowance>,
}

/// At most the last 31 UTC days, clipped to the requested period. Zero-message days stay.
pub fn windows(since: u64, until: u64) -> Vec<(u64, u64)> {
    if since >= until {
        return Vec::new();
    }
    let last = (until - 1) / DAY_MS * DAY_MS;
    let first = (since / DAY_MS * DAY_MS).max(last.saturating_sub(30 * DAY_MS));
    (first..=last)
        .step_by(DAY_MS as usize)
        .map(|day| (day.max(since), day.saturating_add(DAY_MS).min(until)))
        .collect()
}

// A caller's pinned snapshot cannot be initialized by a later writer commit. Fold that
// snapshot's retained sends instead, with the same earliest-send arbitration and eligibility.
fn snapshot_sends(connection: &Connection) -> Result<Vec<(u64, String)>> {
    let mut statement = connection.prepare_cached(
        "SELECT sent_ms,body FROM (
             SELECT CAST(accepted_at_unix_ms AS INTEGER) AS sent_ms,body,
                    ROW_NUMBER() OVER (
                        PARTITION BY subject ORDER BY CAST(accepted_at_unix_ms AS INTEGER),id
                    ) AS rank
             FROM claims WHERE kind='message.sent'
         ) WHERE rank=1"
    )?;
    let mut rows = statement.query([])?;
    let mut sends = Vec::new();
    while let Some(row) = rows.next()? {
        let body: Value = serde_json::from_str(&row.get::<_, String>(1)?)?;
        if eligible(&body["fields"]) {
            sends.push((row.get(0)?, body["fields"]["to"].as_str().unwrap().to_owned()));
        }
    }
    Ok(sends)
}

pub trait AgentMessagesStore {
    fn agent_message_estimate(&self, days: &[DailyUsage]) -> Result<Value>;
    /// One bounded page of the usage backfill on the background writer: at most
    /// [`BACKFILL_PAGE`] sends. The daemon awaits it and pauses between pages. Returns whether
    /// the counts are ready.
    fn advance_agent_message_days(&self) -> Result<bool>;
}

impl AgentMessagesStore for Store {

    fn advance_agent_message_days(&self) -> Result<bool> {
        let mut connection = self.connection.write_background();
        let transaction = connection.transaction()?;
        let ready = backfill_page(&transaction, BACKFILL_PAGE)?;
        transaction.commit()?;
        Ok(ready)
    }

    fn agent_message_estimate(&self, days: &[DailyUsage]) -> Result<Value> {
        // The daemon counts retained sends in background pages. An estimate that arrives
        // before they finish completes the rest atomically rather than return partial counts.
        // Subsequent writes maintain the ready index as before.
        let ready: bool = self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key='agent_message_days_v1')",
            [],
            |row| row.get(0),
        )?;
        let pinned = self.readers.has_pinned_reader();
        if !ready && !pinned {
            self.connection.batched(ensure_ready).map_err(anyhow::Error::msg)??;
        }
        let connection = self.readers.get();
        let snapshot_sends = (!ready && pinned).then(|| snapshot_sends(&connection)).transpose()?;
        let calibration: Option<(String, Vec<u8>)> = connection
            .query_row(
                "SELECT d.hash,b.bytes FROM documents d JOIN blobs b ON b.hash=d.hash
             WHERE d.name=?1 ORDER BY d.binding_key DESC LIMIT 1",
                [ALLOWANCES],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        // An absent or invalid calibration must not break spend reads or imply zero cost.
        let calibration = calibration.and_then(|(hash, bytes)| {
            let value: Calibration = serde_json::from_slice(&bytes).ok()?;
            if value.source.is_empty()
                || value.method.is_empty()
                || value.recipients.len() > 4096
                || value.recipients.iter().any(|(seat, a)| {
                    !seat.starts_with("agent/") || a.low_microusd > a.high_microusd
                })
            {
                return None;
            }
            Some((format!("{ALLOWANCES}@{hash}"), value))
        });
        let fallback = Allowance {
            low_microusd: 220_000,
            high_microusd: 330_000,
        };
        let mut result = Vec::new();
        for &DailyUsage {
            since,
            until,
            cost,
            unpriced,
        } in days
        {
            let day = since / DAY_MS * DAY_MS;
            let counts = if let Some(sends) = &snapshot_sends {
                let mut counts = BTreeMap::<String, u64>::new();
                for (sent_ms, recipient) in sends {
                    if since <= *sent_ms && *sent_ms < until {
                        *counts.entry(recipient.clone()).or_default() += 1;
                    }
                }
                counts.into_iter().collect()
            } else if since == day && until == day.saturating_add(DAY_MS) {
                connection
                    .prepare_cached(
                        "SELECT recipient,count FROM agent_message_days WHERE day_ms=?1",
                    )?
                    .query_map([day], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                // Only the two clipped edge days need the indexed timestamp range.
                connection
                    .prepare_cached(
                        "SELECT recipient,COUNT(*) FROM agent_message_sends
                     WHERE sent_ms>=?1 AND sent_ms<?2 GROUP BY recipient",
                    )?
                    .query_map(params![since, until], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let (mut messages, mut calibrated, mut low, mut high) = (0_u64, 0_u64, 0_u64, 0_u64);
            for (recipient, count) in counts {
                let allowance = calibration
                    .as_ref()
                    .and_then(|(_, c)| c.recipients.get(&recipient));
                if allowance.is_some() {
                    calibrated = calibrated.saturating_add(count);
                }
                let allowance = allowance.copied().unwrap_or(fallback);
                messages = messages.saturating_add(count);
                low = low.saturating_add(count.saturating_mul(allowance.low_microusd));
                high = high.saturating_add(count.saturating_mul(allowance.high_microusd));
            }
            result.push(json!({
                "day_start_ms": day, "since_ms": since, "until_ms": until,
                "messages": messages, "calibrated_messages": calibrated,
                "low_microusd": low, "high_microusd": high,
                "usage_cost_microusd": cost, "unpriced_tokens": unpriced,
                "low_percent": (cost > 0).then(|| low as f64 * 100.0 / cost as f64),
                "high_percent": (cost > 0).then(|| high as f64 * 100.0 / cost as f64),
            }));
        }
        Ok(json!({
            "calibration": calibration.as_ref().map(|(reference, _)| reference.as_str()),
            "source": calibration.as_ref().map_or("fleet fallback", |(_, c)| c.source.as_str()),
            "method": calibration.as_ref().map_or("Count times fleet allowance", |(_, c)| c.method.as_str()),
            "fallback_low_microusd": fallback.low_microusd,
            "fallback_high_microusd": fallback.high_microusd,
            "days": result,
        }))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use st3_store::store::agent_messages_tests_shared_delivery::*;
    use st3_store::store::agent_messages_tests_shared_delivery::{send};

    fn estimate_days(store: &Store, days: &[(u64, u64, u64, u64)]) -> Result<Value> {
        let days = days
            .iter()
            .map(|&(since, until, cost, unpriced)| DailyUsage {
                since,
                until,
                cost,
                unpriced,
            })
            .collect::<Vec<_>>();
        store.agent_message_estimate(&days)
    }

    #[test]
    #[ignore = "requires an exclusively owned copied corpus in ST3_AGENT_MESSAGES_CORPUS"]
    fn copied_corpus_lazy_backfill_benchmark() {
        let path = std::env::var("ST3_AGENT_MESSAGES_CORPUS").unwrap();
        let mut connection = Connection::open(path).unwrap();
        let transaction = connection.transaction().unwrap();
        transaction.execute("DELETE FROM meta WHERE key='agent_message_days_v1'", []).unwrap();
        let started = std::time::Instant::now();
        ensure_ready(&transaction).unwrap();
        let eager = started.elapsed();
        let expected = transaction.prepare(
            "SELECT day_ms,recipient,count FROM agent_message_days ORDER BY day_ms,recipient"
        ).unwrap().query_map([], |row| Ok((
            row.get::<_, u64>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?
        ))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        transaction.rollback().unwrap();
        let transaction = connection.transaction().unwrap();
        transaction.execute("DELETE FROM meta WHERE key='agent_message_days_v1'", []).unwrap();
        let started = std::time::Instant::now();
        open(&transaction).unwrap();
        let lazy_open = started.elapsed();
        let started = std::time::Instant::now();
        ensure_ready(&transaction).unwrap();
        let lazy_first_estimate = started.elapsed();
        let actual = transaction.prepare(
            "SELECT day_ms,recipient,count FROM agent_message_days ORDER BY day_ms,recipient"
        ).unwrap().query_map([], |row| Ok((
            row.get::<_, u64>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?
        ))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(actual, expected);
        println!("eager_open={eager:?} lazy_open={lazy_open:?} first_estimate={lazy_first_estimate:?} messages={}",
            actual.iter().map(|(_, _, count)| count).sum::<u64>());
        transaction.rollback().unwrap();
    }

    #[test]
    fn first_estimate_in_pinned_snapshot_does_not_write_or_see_later_sends() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("pinned.sqlite3"), "node").unwrap();
        let fields = json!({"from":"agent/alder","to":"agent/birch","status":"sent"});
        send(&store, "message/old", DAY_MS + 1, fields.clone());
        let day = [(DAY_MS, 2 * DAY_MS, 1_000_000, 0)];
        let pinned = store.read_snapshot(|_| {
            std::thread::scope(|scope| {
                scope.spawn(|| send(&store, "message/new", DAY_MS + 2, fields));
            });
            estimate_days(&store, &day)
        }).unwrap();
        assert_eq!(pinned["days"][0]["messages"], 1);
        let ready: bool = store.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key='agent_message_days_v1')",
            [], |row| row.get(0)
        ).unwrap();
        assert!(!ready);
        assert_eq!(estimate_days(&store, &day).unwrap()["days"][0]["messages"], 2);
    }

    #[test]
    fn legacy_open_defers_estimate_backfill_without_losing_updates() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lazy.sqlite3");
        let fields = json!({"from":"agent/alder","to":"agent/birch","status":"sent"});
        {
            let store = Store::open(&path, "node").unwrap();
            send(&store, "message/old", DAY_MS + 1, fields.clone());
            store.connection.batched(|tx| {
                tx.execute("DELETE FROM meta WHERE key='agent_message_days_v1'", [])?;
                tx.execute("DELETE FROM agent_message_sends", [])?;
                tx.execute("DELETE FROM local_agent_message_pending", [])?;
                Ok::<_, anyhow::Error>(())
            }).unwrap().unwrap();
        }
        let store = Store::open(&path, "node").unwrap();
        let counts: (u64, u64) = store.readers.get().query_row(
            "SELECT (SELECT COUNT(*) FROM agent_message_sends),
                    (SELECT COUNT(*) FROM local_agent_message_pending)",
            [], |row| Ok((row.get(0)?, row.get(1)?))
        ).unwrap();
        assert_eq!(counts, (0, 0));
        send(&store, "message/new", DAY_MS + 2, fields);
        let day = [(DAY_MS, 2 * DAY_MS, 1_000_000, 0)];
        assert_eq!(estimate_days(&store, &day).unwrap()["days"][0]["messages"], 2);
        store.connection.batched(|tx| {
            tx.execute("DELETE FROM claims WHERE subject='message/old'", [])?;
            flush(tx)
        }).unwrap().unwrap();
        assert_eq!(estimate_days(&store, &day).unwrap()["days"][0]["messages"], 1);
        drop(store);
        let reopened = Store::open(&path, "node").unwrap();
        assert_eq!(estimate_days(&reopened, &day).unwrap()["days"][0]["messages"], 1);
    }

    /// `count` agent-to-agent sends, each its own message, in one transaction.
    fn send_history(store: &Store, prefix: &str, count: usize) {
        store.connection.batched(|tx| {
            for number in 0..count {
                let to = if number % 2 == 0 { "agent/birch" } else { "agent/cedar" };
                append_claim_record_tx(tx, "node", &format!("message/{prefix}-{number:05}"), "message.sent", None,
                    &json!({"fields": {"from":"agent/alder", "to":to, "status":"sent"}}), &[], None)?;
            }
            flush(tx)
        }).unwrap().unwrap();
    }

    /// An upgraded store: its retained sends were never counted.
    fn withdraw_counts(store: &Store) {
        store.connection.batched(|tx| {
            tx.execute_batch(
                "DELETE FROM meta WHERE key IN ('agent_message_days_v1','agent_message_days_v1_through');
                 DELETE FROM agent_message_sends;
                 DELETE FROM agent_message_days;",
            )?;
            Ok::<_, anyhow::Error>(())
        }).unwrap().unwrap();
    }

    /// Each counted message's subject, send time and recipient.
    type Sends = Vec<(String, u64, String)>;
    /// Each day's count per recipient.
    type Days = Vec<(u64, String, u64)>;

    fn counts(store: &Store) -> (Sends, Days) {
        let connection = store.readers.get();
        let sends = connection.prepare("SELECT subject,sent_ms,recipient FROM agent_message_sends ORDER BY subject").unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        let days = connection.prepare("SELECT day_ms,recipient,count FROM agent_message_days ORDER BY day_ms,recipient").unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        (sends, days)
    }

    fn counts_ready(store: &Store) -> bool {
        store.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key='agent_message_days_v1')", [], |row| row.get(0),
        ).unwrap()
    }

    /// The counts a rebuild of every retained send in one transaction gives the same claims.
    fn assert_counts_match_a_full_rebuild(store: &Store) {
        let paged = counts(store);
        withdraw_counts(store);
        store.connection.batched(ensure_ready).unwrap().unwrap();
        assert_eq!(paged, counts(store));
    }

    /// Run `estimate` while this thread holds the writer: it finishes only if it never needs it.
    fn estimate_without_the_writer(store: &Store, day: &[(u64, u64, u64, u64)]) -> Value {
        std::thread::scope(|scope| {
            let held = store.connection.write_background();
            let estimate = scope.spawn(|| estimate_days(store, day).unwrap());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !estimate.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(estimate.is_finished(), "the estimate waited for the writer");
            drop(held);
            estimate.join().unwrap()
        })
    }

    #[test]
    fn background_pages_count_a_large_history_so_the_first_estimate_never_takes_the_writer() {
        let store = Store::open_memory("node").unwrap();
        let history = 4 * BACKFILL_PAGE + 7;
        send_history(&store, "old", history);
        withdraw_counts(&store);
        let mut counted = 0;
        let mut pages = 1;
        while !store.advance_agent_message_days().unwrap() {
            // One writer turn counts at most one page of sends.
            let now = counts(&store).0.len();
            assert!(now - counted <= BACKFILL_PAGE, "page {pages} counted {} sends", now - counted);
            assert!(!counts_ready(&store));
            counted = now;
            if pages == 2 {
                // Writes between pages: a new send past the cursor, a later duplicate send of a
                // counted message, and the removal of one counted and one uncounted message.
                send_history(&store, "new", 3);
                store.connection.batched(|tx| {
                    append_claim_record_tx(tx, "node", "message/old-00000", "message.sent", None,
                        &json!({"fields": {"from":"agent/alder", "to":"agent/cedar", "status":"sent"}}), &[], None)?;
                    tx.execute("DELETE FROM claims WHERE subject='message/old-00001'", [])?;
                    tx.execute(&format!("DELETE FROM claims WHERE subject='message/old-{:05}'", history - 1), [])?;
                    flush(tx)
                }).unwrap().unwrap();
            }
            pages += 1;
        }
        assert_eq!(pages, history.div_ceil(BACKFILL_PAGE));
        assert!(counts_ready(&store));
        let day = [(0, u64::MAX / 2, 1_000_000, 0)];
        let estimate = estimate_without_the_writer(&store, &day);
        assert_eq!(estimate["days"][0]["messages"], history - 2 + 3);
        assert_counts_match_a_full_rebuild(&store);
    }

    #[test]
    fn the_backfill_resumes_after_a_restart_and_an_early_estimate_completes_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("resume.sqlite3");
        let store = Store::open(&path, "node").unwrap();
        send_history(&store, "old", 3 * BACKFILL_PAGE + 3);
        withdraw_counts(&store);
        assert!(!store.advance_agent_message_days().unwrap());
        assert_eq!(counts(&store).0.len(), BACKFILL_PAGE);
        drop(store);
        // Opening the store neither restarts the backfill nor finishes it.
        let store = Store::open(&path, "node").unwrap();
        assert!(!counts_ready(&store));
        assert_eq!(counts(&store).0.len(), BACKFILL_PAGE);
        assert!(!store.advance_agent_message_days().unwrap());
        assert_eq!(counts(&store).0.len(), 2 * BACKFILL_PAGE, "the second page continues from the first");
        // An estimate before the last page completes the rest itself, in one transaction.
        send_history(&store, "new", 2);
        let day = [(0, u64::MAX / 2, 1_000_000, 0)];
        assert_eq!(estimate_days(&store, &day).unwrap()["days"][0]["messages"], 3 * BACKFILL_PAGE + 5);
        assert!(counts_ready(&store));
        assert!(store.advance_agent_message_days().unwrap());
        assert_counts_match_a_full_rebuild(&store);
    }

    #[test]
    fn counts_distinct_sends_excludes_explicit_tests_and_rebuilds() {
        let store = Store::open_memory("node").unwrap();
        let fields =
            json!({"from":"agent/alder", "to":"agent/birch", "status":"sent", "title":"Update"});
        send(&store, "message/one", DAY_MS + 10, fields.clone());
        // A legacy/replicated second send claim for the same subject cannot double count.
        store.connection.batched(|tx| {
            append_claim_record_tx(tx, "node", "message/one", "message.sent", None,
                &json!({"fields": {"from":"agent/alder", "to":"agent/birch", "status":"sent", "title":"Retry"}}), &[], None)?;
            append_claim_record_tx(tx, "node", "message/one", "message.delivered", None,
                &json!({"fields":{"status":"delivered"}}), &[], None)?;
            flush(tx)
        }).unwrap().unwrap();
        send(&store, "message/two", 2 * DAY_MS, fields.clone());
        for (i, (from, to, title, tags)) in [
            ("person/ada", "agent/birch", "Question", json!([])),
            ("daemon/runtime", "agent/birch", "Ready", json!([])),
            ("agent/alder", "person/ada", "Answer", json!([])),
            (
                "agent/alder",
                "agent/birch",
                "Re: Re: Channel check",
                json!([]),
            ),
            (
                "agent/alder",
                "agent/birch",
                "Hello",
                json!(["delivery-probe"]),
            ),
            (
                "agent/delivery-soak/worker",
                "agent/birch",
                "Hello",
                json!([]),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            send(
                &store,
                &format!("message/exclude-{i}"),
                DAY_MS + 100,
                json!({"from":from,"to":to,"title":title,"tags":tags,"status":"sent"}),
            );
        }
        let daily = [
            (DAY_MS, 2 * DAY_MS, 1_000_000, 0),
            (2 * DAY_MS, 3 * DAY_MS, 0, 0),
        ];
        let estimate = estimate_days(&store, &daily).unwrap();
        assert_eq!(estimate["days"][0]["messages"], 1);
        assert_eq!(estimate["days"][1]["messages"], 1);
        assert_eq!(estimate["days"][0]["low_percent"], 22.0);
        assert!(estimate["days"][1]["low_percent"].is_null());
        assert_eq!(
            estimate_days(&store, &[(DAY_MS + 11, 2 * DAY_MS, 1, 0)]).unwrap()["days"][0]["messages"],
            0
        );
        store
            .connection
            .batched(|tx| {
                tx.execute("DELETE FROM meta WHERE key='agent_message_days_v1'", [])?;
                open(tx)
            })
            .unwrap()
            .unwrap();
        assert_eq!(estimate_days(&store, &daily).unwrap(), estimate);
        store
            .connection
            .batched(|tx| {
                tx.execute("DELETE FROM claims WHERE subject='message/one'", [])?;
                flush(tx)
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            estimate_days(&store, &daily).unwrap()["days"][0]["messages"],
            0
        );
        let plan: String = store
            .readers
            .get()
            .query_row(
                "EXPLAIN QUERY PLAN SELECT recipient,count FROM agent_message_days WHERE day_ms=1",
                [],
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("PRIMARY KEY"), "{plan}");
    }

    #[test]
    fn receiving_seat_calibration_and_fallback_use_the_same_daily_denominator() {
        let store = Store::open_memory("node").unwrap();
        let binding = store
            .put_document(
                ALLOWANCES,
                serde_json::to_vec(&json!({
                    "source":"dated audit", "method":"associated budgets / messages",
                    "recipients":{"agent/birch":{"low_microusd":100_000,"high_microusd":200_000}}
                }))
                .unwrap()
                .as_slice(),
                &None,
                "allowances",
            )
            .unwrap();
        for (subject, to) in [("message/a", "agent/birch"), ("message/b", "agent/cedar")] {
            send(
                &store,
                subject,
                DAY_MS + 1,
                json!({"from":"agent/alder","to":to,"title":"Update","status":"sent"}),
            );
        }
        let estimate = estimate_days(&store, &[(DAY_MS, 2 * DAY_MS, 2_000_000, 999)]).unwrap();
        let day = &estimate["days"][0];
        assert_eq!(day["messages"], 2);
        assert_eq!(day["calibrated_messages"], 1);
        assert_eq!(day["low_microusd"], 320_000);
        assert_eq!(day["high_microusd"], 530_000);
        assert_eq!(day["low_percent"], 16.0);
        assert_eq!(day["unpriced_tokens"], 999);
        assert!(
            estimate["calibration"]
                .as_str()
                .unwrap()
                .starts_with(ALLOWANCES)
        );
        store
            .put_document(
                ALLOWANCES,
                b"{}",
                &Some(binding.binding_claim_id),
                "invalid-allowances",
            )
            .unwrap();
        let fallback = estimate_days(&store, &[(DAY_MS, 2 * DAY_MS, 1_000_000, 0)]).unwrap();
        assert_eq!(fallback["days"][0]["calibrated_messages"], 0);
        assert_eq!(fallback["days"][0]["low_microusd"], 440_000);
    }
}
