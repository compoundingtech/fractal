//! Test-only probe for retirement custody's exact unread-count design. This is deliberately
//! separate from the production mailbox API: every page must share one pinned SQLite snapshot.

use super::*;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

const OLD: &str = "agent/fixture-old";
const PAGE: usize = 200;
const FAST_LIMIT: usize = 512;
const MAX_ATTEMPTS: usize = 3;
const MAX_ELAPSED: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Scan {
    unread: u64,
    candidates: u64,
    pages: u64,
    vm_steps: u64,
    fullscan_steps: u64,
    generation: i64,
    elapsed: Duration,
}

struct Fence {
    attempts: usize,
    busy: bool,
    count: Option<u64>,
    writer_micros: u128,
    fast: bool,
}

fn cheap_fast_precondition(connection: &Connection, recipient: &str) -> Result<bool> {
    let recipient = normalize_message_party(recipient);
    let bare = recipient
        .strip_prefix("agent/")
        .filter(|suffix| !suffix.contains('/'))
        .unwrap_or(&recipient);
    let desired_rows: usize = connection.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM desired LIMIT ?1)",
        [FAST_LIMIT + 1],
        |row| row.get(0),
    )?;
    if desired_rows > FAST_LIMIT {
        return Ok(false);
    }
    let sent_rows: usize = connection.query_row(
        "SELECT COUNT(*) FROM (
         SELECT 1 FROM claims INDEXED BY claims_message_to_index
         WHERE kind='message.sent' AND json_extract(body,'$.fields.to') IN (?1,?2)
         LIMIT ?3)",
        params![recipient, bare, FAST_LIMIT + 1],
        |row| row.get(0),
    )?;
    Ok(sent_rows <= FAST_LIMIT)
}

// A prototype of the no-storage completion gate, without writing a completion claim. The
// generation is intentionally broad: any digested graph mutation makes this attempt retry.
fn fenced_count(store: &Store, recipient: &str) -> Result<Fence> {
    let started = Instant::now();
    let mut writer = store.connection.write();
    let transaction = writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let fast_started = Instant::now();
    if cheap_fast_precondition(&transaction, recipient)? {
        let scan = scan_on_connection(&transaction, recipient, None)?;
        transaction.commit()?;
        return Ok(Fence {
            attempts: 1,
            busy: false,
            count: Some(scan.unread),
            writer_micros: fast_started.elapsed().as_micros(),
            fast: true,
        });
    }
    transaction.commit()?;
    drop(writer);
    let deadline = started + MAX_ELAPSED;
    let mut attempts = 0;
    for attempt in 1..=MAX_ATTEMPTS {
        if Instant::now() >= deadline {
            break;
        }
        attempts = attempt;
        let Some(scan) = count_snapshot_until(store, recipient, Some(deadline))? else {
            break;
        };
        let mut writer = store.connection.write();
        let transaction =
            writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let writer_started = Instant::now();
        let unchanged = smallclaims::store::graph_generation(&transaction)? == scan.generation;
        // Production would append agent.custody.completed here if unchanged and authorized.
        // This fixture records only the fence cost; the approved kind is not implemented here.
        transaction.commit()?;
        let writer_micros = writer_started.elapsed().as_micros();
        if unchanged {
            return Ok(Fence {
                attempts: attempt,
                busy: false,
                count: Some(scan.unread),
                writer_micros,
                fast: false,
            });
        }
    }
    Ok(Fence {
        attempts,
        busy: true,
        count: None,
        writer_micros: 0,
        fast: false,
    })
}

// This is the visible mailbox's two candidate arms, including desired-only messages. The
// canonical message view, raw close, and accepted read checks below decide membership. The
// pair cursor avoids dropping two subjects with the same first-claim index.
const CANDIDATES: &str = "WITH candidates(subject) AS (
    SELECT subject FROM claims INDEXED BY claims_message_to_index
    WHERE kind='message.sent' AND json_extract(body, '$.fields.to') IN (?1, ?2)
    UNION
    SELECT desired.subject FROM desired, json_each(desired.body, '$.children') child
    WHERE desired.kind='message'
      AND json_extract(child.value, '$.name')='to'
      AND json_extract(child.value, '$.arguments[0]') IN (?1, ?2)
), created AS (
    SELECT subject, (SELECT MIN(store_index) FROM claims
                     WHERE claims.subject=candidates.subject) created_index FROM candidates
)
SELECT subject, created_index FROM created
WHERE created_index IS NOT NULL AND
      (created_index>?3 OR (created_index=?3 AND subject>?4))
ORDER BY created_index, subject LIMIT ?5";

fn count_snapshot(store: &Store, recipient: &str) -> Result<Scan> {
    count_snapshot_until(store, recipient, None)?
        .ok_or_else(|| anyhow::anyhow!("unexpected unbounded scan timeout"))
}

fn count_snapshot_until(
    store: &Store,
    recipient: &str,
    deadline: Option<Instant>,
) -> Result<Option<Scan>> {
    store.read_snapshot(|_| {
        let connection = store.readers.get();
        if let Some(deadline) = deadline {
            connection.progress_handler(1_000, Some(move || Instant::now() >= deadline));
        }
        let result = scan_on_connection(&connection, recipient, deadline);
        if deadline.is_some() {
            connection.progress_handler(0, None::<fn() -> bool>);
        }
        match result {
            Ok(scan) => Ok(Some(scan)),
            Err(_) if deadline.is_some_and(|deadline| Instant::now() >= deadline) => Ok(None),
            Err(error) => Err(error),
        }
    })
}

fn scan_on_connection(
    connection: &Connection,
    recipient: &str,
    deadline: Option<Instant>,
) -> Result<Scan> {
    let started = Instant::now();
    let generation = smallclaims::store::graph_generation(connection)?;
    let recipient = normalize_message_party(recipient);
    let bare = recipient
        .strip_prefix("agent/")
        .filter(|suffix| !suffix.contains('/'))
        .unwrap_or(&recipient);
    let mut scan = Scan {
        generation,
        ..Scan::default()
    };
    let mut cursor = (0_u64, String::new());
    loop {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            anyhow::bail!("custody count scan deadline reached");
        }
        let mut statement = connection.prepare(CANDIDATES)?;
        let rows = statement
            .query_map(params![recipient, bare, cursor.0, cursor.1, PAGE], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        scan.vm_steps += statement.get_status(rusqlite::StatementStatus::VmStep) as u64;
        scan.fullscan_steps += statement.get_status(rusqlite::StatementStatus::FullscanStep) as u64;
        if rows.is_empty() {
            break;
        }
        scan.pages += 1;
        for (subject, created_index) in &rows {
            scan.candidates += 1;
            // `message_view_tx` uses the same selected actual and current desired row as the
            // client mailbox. A raw close hides the message even when actual status differs.
            let view = message_view_tx(&connection, subject, *created_index)?;
            if view.to != recipient || view.status == "closed" {
                continue;
            }
            let closed: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_subject_kind_index
                     WHERE subject=?1 AND kind='message.closed')",
                [subject],
                |row| row.get(0),
            )?;
            if closed {
                continue;
            }
            let read: bool = connection.query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_subject_kind_index
                     WHERE subject=?1 AND kind='message.read' AND {UNREPAIRED_CLAIM})"
                ),
                [subject],
                |row| row.get(0),
            )?;
            if !read && view.status != "read" {
                scan.unread += 1;
            }
        }
        let (subject, index) = rows.last().expect("nonempty page");
        cursor = (*index, subject.clone());
        if rows.len() < PAGE {
            break;
        }
    }
    scan.elapsed = started.elapsed();
    Ok(scan)
}

fn append(store: &Store, subject: &str, kind: &str, fields: BTreeMap<String, Value>) {
    let actor = if kind == "message.sent" {
        "requester"
    } else {
        OLD
    };
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: Some(actor.into()),
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("fixture:{subject}:{kind}")),
        })
        .unwrap();
}

fn seed_sent(store: &Store, count: usize, all_read: bool) {
    for number in 0..count {
        let subject = format!("message/custody-fixture-{number}");
        append(
            store,
            &subject,
            "message.sent",
            BTreeMap::from([
                ("from".into(), json!("requester")),
                ("to".into(), json!(OLD)),
                ("content".into(), json!("fixture")),
                ("status".into(), json!("sent")),
            ]),
        );
        if all_read {
            append(
                store,
                &subject,
                "message.delivered",
                BTreeMap::from([("status".into(), json!("delivered"))]),
            );
            append(
                store,
                &subject,
                "message.read",
                BTreeMap::from([("status".into(), json!("read"))]),
            );
        }
    }
}

fn seed_desired(store: &Store, old_count: usize, other_count: usize) {
    if old_count + other_count == 0 {
        return;
    }
    let mut kdl = String::from("version 2\n");
    kdl.push_str(
        "agent \"fixture-old\" { workspace \"/work\"; command \"true\"; restart \"never\" }\n",
    );
    kdl.push_str(
        "agent \"fixture-other\" { workspace \"/work\"; command \"true\"; restart \"never\" }\n",
    );
    for number in 0..old_count + other_count {
        let to = if number < old_count {
            "fixture-old"
        } else {
            "fixture-other"
        };
        kdl.push_str(&format!(
            "message \"custody-{number}\" {{ from \"requester\"; to \"{to}\"; content \"fixture\" }}\n"
        ));
    }
    let intent = crate::graph::parse_test_intent(&kdl, "local").unwrap();
    let mission = store
        .mission(
            &intent,
            IntentInput {
                kdl: "custody fixture".into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &mission.subject_tokens, "custody fixture")
        .unwrap();
}

fn unrelated_writes(
    writer: Store,
    prefix: &str,
    count: usize,
) -> thread::JoinHandle<(usize, Duration)> {
    let prefix = prefix.to_owned();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let started = Instant::now();
        started_tx.send(()).expect("fixture reader");
        for number in 0..count {
            append(
                &writer,
                &format!("message/{prefix}-{number}"),
                "message.sent",
                BTreeMap::from([
                    ("from".into(), json!("requester")),
                    ("to".into(), json!("agent/fixture-other")),
                    ("content".into(), json!("unrelated")),
                    ("status".into(), json!("sent")),
                ]),
            );
            // A synthetic 50 writes/s target. The output reports achieved rate; it is not a
            // claim about production traffic.
            thread::sleep(Duration::from_millis(20));
        }
        (count, started.elapsed())
    });
    started_rx.recv().expect("fixture writer");
    handle
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn exact_unread_generation_fence_probe() {
    for (sent, desired_old, desired_other) in [
        (100, 0, 0),
        (1_000, 0, 0),
        (10_000, 0, 0),
        (1_000, 10, 100),
        (1_000, 10, 1_000),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.sqlite3");
        let store = Store::open(&path, "host/fixture").unwrap();
        seed_sent(&store, sent, true);
        seed_desired(&store, desired_old, desired_other);
        let scan = count_snapshot(&store, OLD).unwrap();
        assert_eq!(scan.unread, desired_old as u64);
        println!(
            "custody_fixture sent={sent} desired_old={desired_old} desired_other={desired_other} unread={} candidates={} pages={} vm_steps={} fullscan_steps={} scan_ms={}",
            scan.unread,
            scan.candidates,
            scan.pages,
            scan.vm_steps,
            scan.fullscan_steps,
            scan.elapsed.as_millis()
        );
        let fence = fenced_count(&store, OLD).unwrap();
        assert!(!fence.busy);
        assert_eq!(fence.count, Some(desired_old as u64));
        println!(
            "custody_fence attempts={} busy={} fast={} writer_us={}",
            fence.attempts, fence.busy, fence.fast, fence.writer_micros
        );
    }
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn pinned_wal_and_unrelated_write_probe() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "host/fixture").unwrap();
    seed_sent(&store, 1_000, true);
    seed_desired(&store, 10, 1_000);
    let _: (i64, i64, i64) = store
        .connection
        .write()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    let wal_writer = Store::open(&path, "host/fixture-writer").unwrap();

    let (wal_bytes_before, wal_bytes_pinned, passive_ms, passive_busy, write_count, write_elapsed) =
        store
            .read_snapshot(|_| {
                // The first read fixes the snapshot before the unrelated writer starts.
                let reader = store.readers.get();
                let _generation = smallclaims::store::graph_generation(&reader)?;
                let wal_path = PathBuf::from(format!("{}-wal", path.display()));
                let wal_before = std::fs::metadata(&wal_path)
                    .map(|item| item.len())
                    .unwrap_or(0);
                let (write_count, write_elapsed) = unrelated_writes(wal_writer, "wal", 50)
                    .join()
                    .expect("writer thread");
                let wal_bytes = std::fs::metadata(wal_path)
                    .map(|item| item.len())
                    .unwrap_or(0);
                let started = Instant::now();
                let writer = store.connection.write();
                let (busy, _, _): (i64, i64, i64) =
                    writer.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?;
                Ok((
                    wal_before,
                    wal_bytes,
                    started.elapsed().as_millis(),
                    busy,
                    write_count,
                    write_elapsed,
                ))
            })
            .unwrap();
    let started = Instant::now();
    let (_, checkpoint_log, checkpoint_done): (i64, i64, i64) = store
        .connection
        .write()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    println!(
        "custody_wal writes={} achieved_writes_per_s={:.1} pinned_wal_before={} pinned_wal_after={} pinned_wal_growth={} passive_ms={} passive_busy={} truncate_ms={} checkpoint_log={} checkpoint_done={}",
        write_count,
        write_count as f64 / write_elapsed.as_secs_f64(),
        wal_bytes_before,
        wal_bytes_pinned,
        wal_bytes_pinned.saturating_sub(wal_bytes_before),
        passive_ms,
        passive_busy,
        started.elapsed().as_millis(),
        checkpoint_log,
        checkpoint_done
    );

    let mut success = 0;
    for trial in 0..5 {
        let writer_store = Store::open(&path, "host/fixture-writer").unwrap();
        let writer = unrelated_writes(writer_store, &format!("contention-{trial}"), 100);
        let fenced = fenced_count(&store, OLD).unwrap();
        let (writes, elapsed) = writer.join().expect("writer thread");
        success += usize::from(!fenced.busy);
        println!(
            "custody_contention trial={trial} target_writes_per_s=50 achieved_writes_per_s={:.1} attempts={} busy={} fast={} exact_count={:?} writer_us={}",
            writes as f64 / elapsed.as_secs_f64(),
            fenced.attempts,
            fenced.busy,
            fenced.fast,
            fenced.count,
            fenced.writer_micros
        );
    }
    println!("custody_contention_success={success}/5");
    let settled = fenced_count(&store, OLD).unwrap();
    assert_eq!(settled.count, Some(10));
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn projection_only_recipient_change_invalidates_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "host/fixture").unwrap();
    seed_desired(&store, 2, 0);
    let before = count_snapshot(&store, OLD).unwrap();
    assert_eq!(before.unread, 2);
    let writer = store.connection.write();
    let subject: String = writer
        .query_row(
            "SELECT subject FROM desired WHERE kind='message' AND body LIKE '%fixture-old%'
         ORDER BY subject LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    writer
        .execute(
            "UPDATE desired SET body=replace(body,'fixture-old','fixture-other')
         WHERE subject=?1",
            [&subject],
        )
        .unwrap();
    drop(writer);
    let after = count_snapshot(&store, OLD).unwrap();
    assert_eq!(after.unread, 1);
    assert_ne!(before.generation, after.generation);
    println!(
        "custody_projection_only old_count={} new_count={} fence_changed=true",
        before.unread, after.unread
    );
}
