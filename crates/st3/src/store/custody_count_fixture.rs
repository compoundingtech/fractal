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

thread_local! {
    static FIRST_PAGE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

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
    scan_millis: u128,
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

fn data_version(connection: &Connection) -> Result<i64> {
    connection
        .pragma_query_value(None, "data_version", |row| row.get(0))
        .map_err(Into::into)
}

fn readonly_observer(path: &std::path::Path) -> Result<Connection> {
    let observer = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    anyhow::ensure!(
        observer.is_autocommit(),
        "custody observer is inside a transaction"
    );
    Ok(observer)
}

// A read-only observer connection is kept for both data_version reads. It is distinct from the
// writer and the pinned snapshot reader, so every intervening main-database commit invalidates
// the count, including physical claim trims that preserve logical projection digests.
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
            scan_millis: scan.elapsed.as_millis(),
            fast: true,
        });
    }
    transaction.commit()?;
    drop(writer);
    let deadline = started + MAX_ELAPSED;
    let observer = readonly_observer(&store.path)?;
    let mut attempts = 0;
    let mut last_scan_millis = 0;
    for attempt in 1..=MAX_ATTEMPTS {
        if Instant::now() >= deadline {
            break;
        }
        attempts = attempt;
        anyhow::ensure!(
            observer.is_autocommit(),
            "custody observer is inside a transaction"
        );
        let before = data_version(&observer)?;
        let Some(scan) = count_snapshot_until(store, recipient, Some(deadline))? else {
            break;
        };
        last_scan_millis = scan.elapsed.as_millis();
        let mut writer = store.connection.write();
        let transaction =
            writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let writer_started = Instant::now();
        anyhow::ensure!(
            observer.is_autocommit(),
            "custody observer is inside a transaction"
        );
        let unchanged = data_version(&observer)? == before;
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
                scan_millis: last_scan_millis,
                fast: false,
            });
        }
    }
    Ok(Fence {
        attempts,
        busy: true,
        count: None,
        writer_micros: 0,
        scan_millis: last_scan_millis,
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

// The public mailbox pager is the oracle for visible rows. A read receipt makes a row read;
// closed rows are omitted by include_closed=false. The fixture compares this independent
// production path with its proposed direct count after each seeded lifecycle state.
fn visible_mailbox_unread(store: &Store, recipient: &str) -> Result<u64> {
    let through = store.index()?;
    let mut after = None;
    let mut unread = 0;
    loop {
        let (page, next) = store.messages_page(Some(recipient), false, after, through, PAGE)?;
        unread += page
            .iter()
            .filter(|message| message.status != "read")
            .count() as u64;
        match next {
            Some(cursor) => after = Some(cursor),
            None => return Ok(unread),
        }
    }
}

fn fresh_canonical_unread(path: &std::path::Path, recipient: &str) -> Result<u64> {
    let reader = readonly_observer(path)?;
    Ok(scan_on_connection(&reader, recipient, None)?.unread)
}

fn assert_visible_oracle(store: &Store, label: &str) {
    let counted = count_snapshot(store, OLD).unwrap();
    let visible = visible_mailbox_unread(store, OLD).unwrap();
    assert_eq!(
        counted.unread, visible,
        "{label}: direct count differs from mailbox page"
    );
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
        if scan.pages == 0 {
            FIRST_PAGE_HOOK.with(|hook| {
                if let Some(run) = hook.borrow_mut().take() {
                    run();
                }
            });
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

fn append(
    store: &Store,
    subject: &str,
    kind: &str,
    fields: BTreeMap<String, Value>,
) -> ClaimRecord {
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
        .unwrap()
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

fn trim_claim_as_checkpoint(store: &Store, claim: &ClaimRecord) {
    use smallclaims::store::checkpoint::{
        ClaimTombstone, delete_dropped_rows_tx, record_checkpoint_tombstones_tx,
    };
    let tombstone = ClaimTombstone {
        id: claim.id.clone(),
        writer: claim.origin.clone(),
        sequence: 0,
        envelope_hash: String::new(),
        subject: claim.subject.clone(),
        kind: claim.kind.clone(),
        actor: claim.actor.clone(),
        predecessors: claim.predecessors.clone(),
        operation_id: claim.operation_id.clone(),
        request_digest: claim.request_digest.clone(),
        accepted_at_unix_ms: claim.accepted_at_unix_ms,
    };
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    record_checkpoint_tombstones_tx(
        &transaction,
        "checkpoint/custody-fixture",
        &[],
        std::slice::from_ref(&tombstone),
    )
    .unwrap();
    delete_dropped_rows_tx(&transaction, &[], std::slice::from_ref(&tombstone)).unwrap();
    transaction.commit().unwrap();
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn visible_mailbox_lifecycle_oracle() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("state.sqlite3"), "host/fixture").unwrap();
    let subject = "message/custody-lifecycle-oracle";
    append(
        &store,
        subject,
        "message.sent",
        BTreeMap::from([
            ("from".into(), json!("requester")),
            ("to".into(), json!(OLD)),
            ("content".into(), json!("fixture")),
            ("status".into(), json!("sent")),
        ]),
    );
    assert_visible_oracle(&store, "sent only");
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 1);
    append(
        &store,
        subject,
        "message.delivered",
        BTreeMap::from([("status".into(), json!("delivered"))]),
    );
    assert_visible_oracle(&store, "sent and delivered");
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 1);
    append(
        &store,
        subject,
        "message.read",
        BTreeMap::from([("status".into(), json!("read"))]),
    );
    assert_visible_oracle(&store, "sent and read");
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 0);
    append(
        &store,
        subject,
        "message.closed",
        BTreeMap::from([("status".into(), json!("closed"))]),
    );
    assert_visible_oracle(&store, "sent and closed");

    append(
        &store,
        "message/custody-alias-oracle",
        "message.sent",
        BTreeMap::from([
            ("from".into(), json!("requester")),
            ("to".into(), json!("fixture-old")),
            ("content".into(), json!("fixture")),
            ("status".into(), json!("sent")),
        ]),
    );
    assert_visible_oracle(&store, "bare recipient alias");
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 1);

    seed_desired(&store, 1, 0);
    assert_visible_oracle(&store, "desired-only mail");
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 2);

    let out_of_order = "message/custody-out-of-order";
    append(
        &store,
        out_of_order,
        "message.sent",
        BTreeMap::from([
            ("from".into(), json!("requester")),
            ("to".into(), json!(OLD)),
            ("content".into(), json!("fixture")),
            ("status".into(), json!("sent")),
        ]),
    );
    // Replication can leave a persisted read without this writer observing its delivery
    // boundary first. The test-only transaction writes that persisted shape directly.
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    append_claim_tx(
        &transaction,
        store.origin(),
        out_of_order,
        "message.read",
        Some(OLD),
        &json!({"fields":{"status":"read"}}),
        &[],
        None,
    )
    .unwrap();
    transaction.commit().unwrap();
    drop(writer);
    assert_visible_oracle(&store, "sent then read out of order");
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 2);
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn data_version_covers_mail_and_projection_mutations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "host/fixture").unwrap();
    let observer = readonly_observer(&path).unwrap();
    let subject = "message/custody-version-oracle";

    let before = data_version(&observer).unwrap();
    append(
        &store,
        subject,
        "message.sent",
        BTreeMap::from([
            ("from".into(), json!("requester")),
            ("to".into(), json!(OLD)),
            ("content".into(), json!("fixture")),
            ("status".into(), json!("sent")),
        ]),
    );
    let after = data_version(&observer).unwrap();
    assert_ne!(before, after, "ordinary sent claim must move data_version");
    assert_visible_oracle(&store, "ordinary append");
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 1);
    println!(
        "custody_version sent changed=true graph_generation={}",
        count_snapshot(&store, OLD).unwrap().generation
    );

    let before = after;
    append(
        &store,
        subject,
        "message.delivered",
        BTreeMap::from([("status".into(), json!("delivered"))]),
    );
    let after = data_version(&observer).unwrap();
    assert_ne!(before, after, "delivered claim must move data_version");
    let read = append(
        &store,
        subject,
        "message.read",
        BTreeMap::from([("status".into(), json!("read"))]),
    );
    let after_read = data_version(&observer).unwrap();
    assert_ne!(after, after_read, "read claim must move data_version");
    assert_visible_oracle(&store, "read claim");
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 0);

    let graph_before = count_snapshot(&store, OLD).unwrap().generation;
    store
        .connection
        .write()
        .execute(
            "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)",
            [&read.id],
        )
        .unwrap();
    let after_repair = data_version(&observer).unwrap();
    assert_ne!(
        after_read, after_repair,
        "repair commit must move data_version"
    );
    let canonical = count_snapshot(&store, OLD).unwrap();
    assert_eq!(canonical.unread, 1);
    let cached_page = visible_mailbox_unread(&store, OLD).unwrap();
    let fresh_page = fresh_canonical_unread(&path, OLD).unwrap();
    assert_eq!(canonical.unread, fresh_page);
    println!(
        "custody_repair cached_page={cached_page} fresh_canonical={fresh_page} graph_generation_before={graph_before} graph_generation_after={}",
        canonical.generation
    );

    let checkpoint_before = data_version(&observer).unwrap();
    let _: (i64, i64, i64) = store
        .connection
        .write()
        .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(
        checkpoint_before,
        data_version(&observer).unwrap(),
        "WAL checkpoint without content change must not move data_version"
    );
    let reader = readonly_observer(&path).unwrap();
    let _: i64 = reader
        .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        checkpoint_before,
        data_version(&observer).unwrap(),
        "a read-only connection must not move data_version"
    );
    let child_before = data_version(&observer).unwrap();
    let status = std::process::Command::new("python3")
        .arg("-c")
        .arg("import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute(\"INSERT OR REPLACE INTO meta(key,value) VALUES('custody_child_commit','1')\"); c.commit()")
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success(), "second-process SQLite commit failed");
    assert_ne!(
        child_before,
        data_version(&observer).unwrap(),
        "second-process commit must move data_version"
    );
    println!("custody_second_process data_version_changed=true");

    seed_desired(&store, 2, 0);
    let subject: String = store
        .connection
        .write()
        .query_row(
            "SELECT subject FROM desired WHERE kind='message' AND body LIKE '%fixture-old%'
         ORDER BY subject LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let before_update = data_version(&observer).unwrap();
    let before_update_scan = count_snapshot(&store, OLD).unwrap();
    let graph_before = before_update_scan.generation;
    let _warm_page = visible_mailbox_unread(&store, OLD).unwrap();
    store
        .connection
        .write()
        .execute(
            "UPDATE desired SET body=replace(body,'fixture-old','fixture-other')
         WHERE subject=?1",
            [&subject],
        )
        .unwrap();
    assert_ne!(before_update, data_version(&observer).unwrap());
    let direct = count_snapshot(&store, OLD).unwrap();
    assert_eq!(direct.unread + 1, before_update_scan.unread);
    let cached_page = visible_mailbox_unread(&store, OLD).unwrap();
    let fresh_page = fresh_canonical_unread(&path, OLD).unwrap();
    assert_eq!(direct.unread, fresh_page);
    println!(
        "custody_desired_update cached_page={cached_page} fresh_canonical={fresh_page} graph_generation_before={graph_before} graph_generation_after={}",
        direct.generation
    );

    let before_delete = data_version(&observer).unwrap();
    let direct_before = direct.unread;
    let remaining: String = store
        .connection
        .write()
        .query_row(
            "SELECT subject FROM desired WHERE kind='message' AND body LIKE '%fixture-old%'
         ORDER BY subject LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    store
        .connection
        .write()
        .execute("DELETE FROM desired WHERE subject=?1", [&remaining])
        .unwrap();
    assert_ne!(before_delete, data_version(&observer).unwrap());
    let direct_after = count_snapshot(&store, OLD).unwrap();
    assert_eq!(direct_after.unread + 1, direct_before);
    println!(
        "custody_desired_delete old_count={direct_before} new_count={} graph_generation_before={} graph_generation_after={}",
        direct_after.unread, direct.generation, direct_after.generation
    );
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn data_version_covers_checkpoint_trim_without_digest_change() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "host/fixture").unwrap();
    let observer = readonly_observer(&path).unwrap();
    let subject = "message/custody-trim-oracle";
    append(
        &store,
        subject,
        "message.sent",
        BTreeMap::from([
            ("from".into(), json!("requester")),
            ("to".into(), json!(OLD)),
            ("content".into(), json!("fixture")),
            ("status".into(), json!("sent")),
        ]),
    );
    append(
        &store,
        subject,
        "message.delivered",
        BTreeMap::from([("status".into(), json!("delivered"))]),
    );
    let read = append(
        &store,
        subject,
        "message.read",
        BTreeMap::from([("status".into(), json!("read"))]),
    );
    assert_visible_oracle(&store, "before read trim");
    let before = count_snapshot(&store, OLD).unwrap();
    let before_version = data_version(&observer).unwrap();
    let before_digest =
        smallclaims::store::projection_digest::generation(&store.readers.get()).unwrap();
    trim_claim_as_checkpoint(&store, &read);
    let after_version = data_version(&observer).unwrap();
    assert_ne!(
        before_version, after_version,
        "physical read trim must move data_version"
    );
    let after = count_snapshot(&store, OLD).unwrap();
    assert_eq!(before.unread, 0);
    assert_eq!(after.unread, 1);
    assert_visible_oracle(&store, "after read trim");
    let after_digest =
        smallclaims::store::projection_digest::generation(&store.readers.get()).unwrap();
    println!(
        "custody_trim_read graph_before={} graph_after={} digest_before={before_digest} digest_after={after_digest} data_version_changed=true unread_before={} unread_after={}",
        before.generation, after.generation, before.unread, after.unread
    );

    let close_subject = "message/custody-close-trim";
    append(
        &store,
        close_subject,
        "message.sent",
        BTreeMap::from([
            ("from".into(), json!("requester")),
            ("to".into(), json!(OLD)),
            ("content".into(), json!("fixture")),
            ("status".into(), json!("sent")),
        ]),
    );
    append(
        &store,
        close_subject,
        "message.delivered",
        BTreeMap::from([("status".into(), json!("delivered"))]),
    );
    append(
        &store,
        close_subject,
        "message.read",
        BTreeMap::from([("status".into(), json!("read"))]),
    );
    let closed = append(
        &store,
        close_subject,
        "message.closed",
        BTreeMap::from([("status".into(), json!("closed"))]),
    );
    let before_close_trim = data_version(&observer).unwrap();
    trim_claim_as_checkpoint(&store, &closed);
    assert_ne!(
        before_close_trim,
        data_version(&observer).unwrap(),
        "physical close trim must move data_version"
    );
    assert_visible_oracle(&store, "after close trim");
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 1);
    println!("custody_trim_close data_version_changed=true visible_read=true");
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn pinned_scan_connection_is_not_the_observer() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "host/fixture").unwrap();
    let scan_connection = store.readers.get();
    scan_connection.execute_batch("BEGIN").unwrap();
    let _: i64 = scan_connection
        .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
        .unwrap();
    let pinned_before = data_version(&scan_connection).unwrap();
    append(
        &store,
        "message/custody-pinned-observer",
        "message.sent",
        BTreeMap::from([
            ("from".into(), json!("requester")),
            ("to".into(), json!(OLD)),
            ("content".into(), json!("fixture")),
            ("status".into(), json!("sent")),
        ]),
    );
    let pinned_after = data_version(&scan_connection).unwrap();
    scan_connection.execute_batch("COMMIT").unwrap();
    let released_after = data_version(&scan_connection).unwrap();
    assert_eq!(
        pinned_before, pinned_after,
        "a pinned scan connection can falsely report no change"
    );
    assert_ne!(
        pinned_before, released_after,
        "the same connection sees the commit only after release"
    );
    println!("custody_pinned_observer false_equality=true released_change=true");
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn other_connection_commits_during_a_pinned_page_scan() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "host/fixture").unwrap();
    seed_sent(&store, 100, true);
    let other = Store::open(&path, "host/fixture-writer").unwrap();
    let observer = readonly_observer(&path).unwrap();
    let before = data_version(&observer).unwrap();
    let (start_tx, start_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let writer = thread::spawn(move || {
        start_rx.recv().unwrap();
        append(
            &other,
            "message/custody-during-scan",
            "message.sent",
            BTreeMap::from([
                ("from".into(), json!("requester")),
                ("to".into(), json!(OLD)),
                ("content".into(), json!("during scan")),
                ("status".into(), json!("sent")),
            ]),
        );
        done_tx.send(()).unwrap();
    });
    FIRST_PAGE_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            start_tx.send(()).unwrap();
            done_rx.recv().unwrap();
        }));
    });
    let old_scan = count_snapshot(&store, OLD).unwrap();
    writer.join().unwrap();
    assert_eq!(
        old_scan.unread, 0,
        "the pinned snapshot must stay at its original cut"
    );
    let mut writer_lock = store.connection.write();
    let transaction = writer_lock
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let after = data_version(&observer).unwrap();
    assert_ne!(
        before, after,
        "commit during scan must fail the observer fence"
    );
    transaction.commit().unwrap();
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 1);
    println!("custody_during_scan old_count=0 new_count=1 data_version_changed=true");
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn eval_run_desired_delete_moves_data_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "host/fixture").unwrap();
    let source = r#"version 2
mission "eval/custody" state="ready" timeout="1m" {
  goal "Own a desired-only fixture message."
}
"#;
    let definition = crate::graph::parse_test_intent(source, "host/fixture").unwrap();
    let published = store
        .mission(
            &definition,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&definition, &published.subject_tokens, "eval fixture")
        .unwrap();
    let root = store
        .create_mission_run(&MissionRunRequest {
            mission: "eval/custody".into(),
            revision: None,
            workspace: "/eval".into(),
            requester: Some("person/test".into()),
            mode: Some("eval".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "custody-eval-root".into(),
        })
        .unwrap();
    let mut message = crate::graph::parse_test_intent(
        "version 2\nmessage \"custody-eval-mail\" { from \"requester\"; to \"fixture-old\"; content \"fixture\" }\n",
        "host/fixture",
    ).unwrap();
    for subject in message.subjects.values_mut() {
        subject.owner_run = Some(root.subject.clone());
    }
    store.apply_internal(&message, "custody-eval-mail").unwrap();
    let observer = readonly_observer(&path).unwrap();
    let before = data_version(&observer).unwrap();
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 1);
    assert_visible_oracle(&store, "before eval-run desired delete");
    assert!(
        store
            .retire_eval_owned_desired(&root.subject)
            .unwrap()
            .is_empty()
    );
    assert_ne!(
        before,
        data_version(&observer).unwrap(),
        "eval-run desired DELETE must move data_version"
    );
    assert_eq!(count_snapshot(&store, OLD).unwrap().unread, 0);
    println!("custody_eval_delete unread_before=1 unread_after=0 data_version_changed=true");
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn exact_unread_generation_fence_probe() {
    for (sent, desired_old, desired_other) in [
        (100, 0, 0),
        (500, 0, 0),
        (2_000, 0, 0),
        (500, 10, 100),
        (500, 10, 700),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.sqlite3");
        let store = Store::open(&path, "host/fixture").unwrap();
        seed_sent(&store, sent, true);
        seed_desired(&store, desired_old, desired_other);
        let scan = count_snapshot(&store, OLD).unwrap();
        assert_eq!(scan.unread, desired_old as u64);
        assert_visible_oracle(&store, "all-read size probe");
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
            "custody_fence attempts={} busy={} fast={} scan_ms={} writer_us={}",
            fence.attempts, fence.busy, fence.fast, fence.scan_millis, fence.writer_micros
        );
    }
}

#[test]
#[ignore = "hosted fixture only; run explicitly after Intake source admission"]
fn pinned_wal_and_unrelated_write_probe() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let store = Store::open(&path, "host/fixture").unwrap();
    seed_sent(&store, 500, true);
    seed_desired(&store, 10, 700);
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
            "custody_contention trial={trial} target_writes_per_s=50 achieved_writes_per_s={:.1} attempts={} busy={} fast={} scan_ms={} exact_count={:?} writer_us={}",
            writes as f64 / elapsed.as_secs_f64(),
            fenced.attempts,
            fenced.busy,
            fenced.fast,
            fenced.scan_millis,
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
