//! Opt-in, disk-backed authority migration measurement, not an index-build proxy.
//!
//! Run alone in a fresh test process: Linux VmHWM is process-lifetime high water,
//! including fixture preparation. No claim history is retained in Rust memory.
//! Defaults describe a synthetic workload, not measured production proportions:
//! 1% authority-kind rows; five events per grant (grant, three policy revisions,
//! then revoke for 25% of groups or another policy); 10% malformed groups and
//! 20% unsigned remote groups, disjoint.
//! Unrelated JSON bodies are 70% quarter-size, 25% base-size, 5% four-times-size.
//! All ordinary Store indexes and durability settings stay enabled during setup.
//! Only authority projection work is disabled, so timed work is the actual
//! install + resumable backfill, not insert-time maintenance.
//!
//! Real-store cold-cache run (Linux, run alone in a fresh process):
//! `AUTHORITY_REAL_STORE=/restore/claims.sqlite3 AUTHORITY_REAL_ORIGIN=dev3 cargo test -p smallclaims --lib store::assistant_grant_authority_bench::assistant_grant_authority_measure_real_store -- --exact --ignored --nocapture --test-threads=1`
//! Use a freshly restored working copy from the current main build; this is the
//! cold-cache condition. The harness writes to it and does not reset projections,
//! checkpoint, or restore it. Restore again for each measurement. Agents cannot
//! use `echo 3 | sudo tee /proc/sys/vm/drop_caches`; `vmtouch -e` or
//! `dd iflag=nocache` eviction is optional, not performed by this test.
//! The origin defaults to dev3; override it with the source daemon's node name.
//! Store::open includes install and the committed two-second startup slice.
//! Test-only hooks time those exact operations, without repeating them. Background
//! slices call the production Store maintenance entrypoint, including membership
//! preparation. Readiness is the production authentication readiness check,
//! observed after each committed slice (not an HTTP request or credential probe).
//! One JSON measurement line is emitted; libtest may also print its own status.
//! Timings exclude final diagnostic queries; byte/RSS samples are taken before
//! those queries and while the store is still open (WAL bytes are physical size).
//! `total_backfill_ms` sums install and committed slice wall times, including
//! membership preparation; `elapsed_open_through_backfill_ms` also includes all
//! other open work and readiness probes. Slice counts include the startup slice.
//! Pending is a retained authentication evidence table, not necessarily queued
//! work; authenticated rows remain there after completion.
use super::*;
use std::time::{Duration, Instant};

const LOCAL: &str = "authority-bench-local";
const REMOTE: &str = "authority-bench-unsigned";
const EVENTS: u64 = 5;
const DENOMINATOR: u64 = 10_000;

struct Config {
    rows: u64,
    authority_bps: u64,
    malformed_bps: u64,
    unauthenticated_bps: u64,
    revoked_bps: u64,
    unrelated_body_bytes: usize,
    authority_body_bytes: usize,
    transaction_rows: u64,
    slice_ms: u64,
    startup_ms: u64,
    target_bytes: u64,
}

fn setting(name: &str, default: u64) -> u64 {
    match std::env::var(name) {
        Ok(value) => value.parse().unwrap_or_else(|_| panic!("{name} must be an unsigned integer")),
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => panic!("{name}: {error}"),
    }
}

impl Config {
    fn from_env() -> Self {
        let config = Self {
            rows: setting("AUTHORITY_BENCH_ROWS", 1_000_000),
            authority_bps: setting("AUTHORITY_BENCH_AUTHORITY_BPS", 100),
            malformed_bps: setting("AUTHORITY_BENCH_MALFORMED_BPS", 1_000),
            unauthenticated_bps: setting("AUTHORITY_BENCH_UNAUTHENTICATED_BPS", 2_000),
            revoked_bps: setting("AUTHORITY_BENCH_REVOKED_BPS", 2_500),
            unrelated_body_bytes: usize::try_from(setting("AUTHORITY_BENCH_BODY_BYTES", 4_096)).unwrap(),
            authority_body_bytes: usize::try_from(setting("AUTHORITY_BENCH_AUTHORITY_BODY_BYTES", 1_024)).unwrap(),
            transaction_rows: setting("AUTHORITY_BENCH_TRANSACTION_ROWS", 1_000),
            slice_ms: setting("AUTHORITY_BENCH_SLICE_MS", 100),
            startup_ms: setting("AUTHORITY_BENCH_STARTUP_MS", 2_000),
            target_bytes: setting("AUTHORITY_BENCH_TARGET_BYTES", 50_000_000_000),
        };
        assert!(config.rows >= DENOMINATOR && config.rows.is_multiple_of(DENOMINATOR));
        assert!(config.authority_bps > 0 && config.authority_bps < DENOMINATOR);
        assert!(config.authority_bps.is_multiple_of(EVENTS));
        assert!(config.malformed_bps + config.unauthenticated_bps < DENOMINATOR);
        assert!(config.revoked_bps <= DENOMINATOR);
        assert!(config.unrelated_body_bytes >= 256 && config.authority_body_bytes >= 1_024);
        assert!(config.transaction_rows > 0 && config.slice_ms > 0 && config.target_bytes > 0);
        config
    }
}

fn padded_body(mut body: Value, bytes: usize) -> String {
    body["fields"]["measurement_padding"] = json!("");
    let empty = serde_json::to_string(&body).unwrap();
    assert!(empty.len() <= bytes, "configured body size smaller than fixture fields");
    body["fields"]["measurement_padding"] = json!("x".repeat(bytes - empty.len()));
    let serialized = serde_json::to_string(&body).unwrap();
    assert_eq!(serialized.len(), bytes);
    serialized
}

fn authority_body(group: u64, event: u64, malformed: bool, bytes: usize) -> String {
    let grant_id = format!("assistant-grant/{group:024x}");
    let mut body = json!({"fields": {
        "grant_id": grant_id,
        "issuer": "person/perf-operator",
        "principal": "assistant/chatgpt",
        "profile": if group.is_multiple_of(4) { "service" } else { "assistant" },
        "session_actor": format!("assistant/chatgpt/session/{grant_id}"),
        "credential_hash": format!("{group:064x}"),
        "minimal_read_mode": "approval-once",
        "disclosure_consent": true,
        "expires_at_unix_ms": 4_102_444_800_000_u64 + event,
        "scopes": ["read.assistant.summary", "read.assistant.attention", "write.assistant.messages"]
    }});
    // Every event of a malformed group fails identity validation; valid remote
    // groups are structurally sound but have no signed envelope/membership proof.
    if malformed {
        body["fields"]["issuer"] = json!(17);
    }
    padded_body(body, bytes)
}

fn open_connection(path: &Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    projection_digest::register(&connection).unwrap();
    connection.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
         PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    ).unwrap();
    connection.pragma_update(None, "cache_size", -(crate::sqlite::WRITE_CACHE_KIB as i64)).unwrap();
    connection
}

fn physical_bytes(path: &Path) -> (u64, u64) {
    let db = fs::metadata(path).unwrap().len();
    let wal = fs::metadata(path.with_file_name("claims.sqlite3-wal"))
        .map_or_else(|error| {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            0
        }, |metadata| metadata.len());
    (db, wal)
}

fn memory_kib() -> (u64, u64) {
    let status = fs::read_to_string("/proc/self/status")
        .expect("authority backfill memory measurement requires Linux /proc/self/status");
    let field = |name: &str| -> u64 {
        status.lines().find_map(|line| {
            line.strip_prefix(name).map(|value| value.split_whitespace().next().unwrap().parse().unwrap())
        }).unwrap_or_else(|| panic!("missing {name} in /proc/self/status"))
    };
    (field("VmRSS:"), field("VmHWM:"))
}

fn cursors(connection: &Connection) -> Vec<(String, String)> {
    let mut statement = connection.prepare(
        "SELECT key,value FROM meta WHERE key LIKE 'assistant_grant_authority_cursor/%' ORDER BY key",
    ).unwrap();
    statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn slice(connection: &mut Connection, budget: Duration) -> (bool, Duration) {
    assert!(assistant_grant_authority::ready(connection).is_err());
    let started = Instant::now();
    let transaction = connection.transaction().unwrap();
    let done = assistant_grant_authority::backfill_slice(&transaction, budget).unwrap();
    assert_eq!(assistant_grant_authority::ready(&transaction).is_ok(), done);
    transaction.commit().unwrap();
    assert_eq!(assistant_grant_authority::ready(connection).is_ok(), done);
    (done, started.elapsed())
}

pub(super) enum OpenAuthorityPhase {
    BeforeInstall,
    Installed,
    StartupComplete,
}

struct OpenAuthorityTimings {
    open_started: Instant,
    phase_started: Instant,
    install: Duration,
    startup_slice: Duration,
    first_ready: Option<Duration>,
    ready_after_install: bool,
    ready_after_startup: bool,
}

thread_local! {
    static REAL_STORE_OPEN: std::cell::RefCell<Option<OpenAuthorityTimings>> =
        const { std::cell::RefCell::new(None) };
}

// Inert for all other tests; no instrumentation exists in non-test builds.
pub(super) fn observe_open_authority(connection: &Connection, phase: OpenAuthorityPhase) {
    REAL_STORE_OPEN.with_borrow_mut(|measurement| {
        let Some(measurement) = measurement else { return; };
        let now = Instant::now();
        match phase {
            OpenAuthorityPhase::BeforeInstall => measurement.phase_started = now,
            OpenAuthorityPhase::Installed => {
                measurement.install = now.duration_since(measurement.phase_started);
                measurement.ready_after_install = assistant_grant_authority::ready(connection).is_ok();
                if measurement.ready_after_install {
                    measurement.first_ready = Some(measurement.open_started.elapsed());
                }
                measurement.phase_started = Instant::now();
            }
            OpenAuthorityPhase::StartupComplete => {
                measurement.startup_slice = now.duration_since(measurement.phase_started);
                measurement.ready_after_startup = assistant_grant_authority::ready(connection).is_ok();
                if measurement.ready_after_startup && measurement.first_ready.is_none() {
                    measurement.first_ready = Some(measurement.open_started.elapsed());
                }
            }
        }
    });
}

#[test]
#[ignore = "writes a freshly restored real store; opt-in Linux cold-cache/RSS measurement"]
fn assistant_grant_authority_measure_real_store() {
    let path = PathBuf::from(std::env::var_os("AUTHORITY_REAL_STORE")
        .expect("set AUTHORITY_REAL_STORE to the restored claims.sqlite3 working copy"));
    assert!(path.is_file(), "AUTHORITY_REAL_STORE must be an existing store, not a new fixture");
    assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("claims.sqlite3"));
    let origin = std::env::var("AUTHORITY_REAL_ORIGIN").unwrap_or_else(|_| "dev3".into());
    let (db_bytes_before, wal_bytes_before) = physical_bytes(&path);
    let (baseline_rss_kib, baseline_peak_kib) = memory_kib();
    let started = Instant::now();
    REAL_STORE_OPEN.with_borrow_mut(|measurement| {
        *measurement = Some(OpenAuthorityTimings {
            open_started: started,
            phase_started: started,
            install: Duration::ZERO,
            startup_slice: Duration::ZERO,
            first_ready: None,
            ready_after_install: false,
            ready_after_startup: false,
        });
    });
    let store = Store::open(&path, &origin, Arc::new(runtime::Plain)).unwrap();
    let open_elapsed = started.elapsed();
    let timing = REAL_STORE_OPEN.with_borrow_mut(Option::take).unwrap();
    let mut first_ready = timing.first_ready;
    let mut slices = 1_u64;
    let mut background_slices = 0_u64;
    let mut max_slice = timing.startup_slice;
    let mut slice_sum = timing.startup_slice;
    // Always enter maintenance at least once: startup has not prepared the sealed
    // membership snapshot, even if its version/readiness marker already exists.
    loop {
        let slice_started = Instant::now();
        let done = store.advance_assistant_grant_authority_backfill().unwrap();
        let elapsed = slice_started.elapsed();
        slices += 1;
        background_slices += 1;
        max_slice = max_slice.max(elapsed);
        slice_sum += elapsed;
        let ready = assistant_grant_authority::ready(&store.readers.get()).is_ok();
        if ready && first_ready.is_none() {
            first_ready = Some(started.elapsed());
        }
        if done {
            assert!(ready, "maintenance completed without authentication readiness");
            break;
        }
    }
    let total_elapsed = started.elapsed();
    let (db_bytes_after, wal_bytes_after) = physical_bytes(&path);
    let (final_rss_kib, peak_rss_kib) = memory_kib();
    let connection = store.readers.get();
    let count = |sql: &str| -> u64 {
        connection.query_row(/* sqlite-large-value: allow-for-now lv-hd9109fd4002d owner=smallclaims::store::assistant_grant_authority_bench TODO=trace-statement-and-value-representation */ sql, [], |row| row.get(0)).unwrap()
    };
    let grant_kind_claims: u64 = connection.query_row(
        "SELECT COUNT(*) FROM claims WHERE kind IN (?1,?2,?3)",
        params![assistant_grant_authority::GRANTED, assistant_grant_authority::POLICY,
            assistant_grant_authority::REVOKED], |row| row.get(0),
    ).unwrap();
    let mut statement = connection.prepare(
        "SELECT pending.claim_id,pending.kind,pending.origin,
            CASE
              WHEN NOT EXISTS(SELECT 1 FROM meta WHERE key='assistant_grant_authority_fleet_id')
                THEN 'no fleet identity for member-writer authentication'
              WHEN NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=pending.claim_id)
                THEN 'no replica record and no durable local-append admission'
              ELSE 'no authenticated member-writer evidence and no durable local-append admission'
            END
         FROM assistant_grant_pending AS pending
         WHERE pending.authenticated=0 AND pending.kind=?1 ORDER BY pending.claim_id",
    ).unwrap();
    let pending_grants: Vec<Value> = statement.query_map([assistant_grant_authority::GRANTED], |row| {
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "kind": row.get::<_, String>(1)?,
            "origin": row.get::<_, String>(2)?,
            "reason": row.get::<_, String>(3)?,
        }))
    }).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let ms = |elapsed: Duration| elapsed.as_secs_f64() * 1_000.0;
    println!("{}", json!({
        "measurement": "assistant_grant_authority_real_store",
        "store": path,
        "origin": origin,
        "store_open_ms": ms(open_elapsed),
        "install_ms": ms(timing.install),
        "startup_slice_ms": ms(timing.startup_slice),
        "total_backfill_ms": ms(timing.install + slice_sum),
        "elapsed_open_through_backfill_ms": ms(total_elapsed),
        "slices": slices,
        "background_slices": background_slices,
        "max_slice_ms": ms(max_slice),
        "slice_sum_ms": ms(slice_sum),
        "authentication_ready_from_open_ms": first_ready.map(ms),
        "ready_after_install": timing.ready_after_install,
        "ready_after_startup": timing.ready_after_startup,
        "readiness_checked_each_slice": true,
        "baseline_rss_kib": baseline_rss_kib,
        "baseline_peak_rss_kib": baseline_peak_kib,
        "final_rss_kib": final_rss_kib,
        "peak_rss_kib": peak_rss_kib,
        "peak_rss_increase_kib": peak_rss_kib.saturating_sub(baseline_peak_kib),
        "db_bytes_before": db_bytes_before,
        "wal_bytes_before": wal_bytes_before,
        "db_bytes_after": db_bytes_after,
        "wal_bytes_after": wal_bytes_after,
        "grant_kind_claims": grant_kind_claims,
        "authority_rows": count("SELECT COUNT(*) FROM assistant_grant_authority"),
        "effective_grants": count("SELECT COUNT(*) FROM assistant_grant_effective"),
        "pending_unauthenticated_grant_claims": pending_grants.len(),
        "pending_unauthenticated_grants": pending_grants,
        "dirty_queue_size": count("SELECT COUNT(*) FROM assistant_grant_dirty"),
        "pending_queue_size": count("SELECT COUNT(*) FROM assistant_grant_pending"),
        "pending_unauthenticated_queue_size": count("SELECT COUNT(*) FROM assistant_grant_pending WHERE authenticated=0"),
        "membership_addition_queue_size": count("SELECT COUNT(*) FROM assistant_grant_additions"),
        "authentication_cursor_queue_size": count("SELECT COUNT(*) FROM assistant_grant_auth_cursor"),
    }));
}

#[test]
#[ignore = "opt-in disk/RSS authority backfill measurement; run alone with --ignored --nocapture"]
fn assistant_grant_authority_measure_realistic_backfill() {
    measure_backfill(Config::from_env());
}

#[test]
fn assistant_grant_authority_bench_unsigned_rows_reach_readiness() {
    measure_backfill(Config {
        rows: 10_000,
        authority_bps: 5_000,
        malformed_bps: 1_000,
        unauthenticated_bps: 4_000,
        revoked_bps: 2_500,
        unrelated_body_bytes: 1_024,
        authority_body_bytes: 1_024,
        transaction_rows: 1_000,
        slice_ms: 100,
        startup_ms: 2_000,
        target_bytes: 50_000_000_000,
    });
}

fn measure_backfill(config: Config) {
    let fixture_started = Instant::now();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let store = Store::open(&path, LOCAL, Arc::new(runtime::Plain)).unwrap();
    let mut expected_effective = 0_u64;
    let mut expected_revoked = 0_u64;
    let mut raw_revocations = 0_u64;
    let mut expected_malformed_rows = 0_u64;
    let mut expected_unauthenticated_rows = 0_u64;
    let mut authority_body_bytes = 0_u64;
    let mut unrelated_body_bytes = 0_u64;
    {
        let connection = store.connection.write();
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h54566dabc8dc owner=smallclaims::store::assistant_grant_authority_bench TODO=bound-or-reference-values-and-reclamation */
            "DROP TRIGGER IF EXISTS assistant_grant_dirty_insert;
             DELETE FROM assistant_grant_authority;
             DELETE FROM assistant_grant_effective;
             DELETE FROM assistant_grant_pending;
             DELETE FROM assistant_grant_dirty;
             DELETE FROM assistant_grant_auth_cursor;
             DELETE FROM assistant_grant_additions;
             DELETE FROM meta WHERE key='assistant_grant_authority_version'
                OR key LIKE 'assistant_grant_authority_cursor/%';",
        ).unwrap();
    }
    // Invalidating readiness above makes the production worker persist its
    // sealed snapshot, rather than taking the already-idle in-memory fast path.
    assert!(store.advance_assistant_grant_authority_backfill().unwrap());
    {
        let mut connection = store.connection.write();
        connection.execute(/* sqlite-large-value: allow-for-now lv-hb93a7253f42d owner=smallclaims::store::assistant_grant_authority_bench TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='assistant_grant_authority_version'", []).unwrap();
        // Fixed small pool of representative unrelated JSON sizes, not a history.
        let unrelated_bodies = [
            padded_body(json!({"fields": {"text": "synthetic conversation summary"}}), config.unrelated_body_bytes / 4),
            padded_body(json!({"fields": {"text": "synthetic conversation message"}}), config.unrelated_body_bytes),
            padded_body(json!({"fields": {"text": "synthetic terminal output"}}), config.unrelated_body_bytes.checked_mul(4).unwrap()),
        ];
        for start in (0..config.rows).step_by(usize::try_from(config.transaction_rows).unwrap()) {
            let transaction = connection.transaction().unwrap();
            {
                let mut batch = transaction.prepare_cached(
                    /* sqlite-large-value: allow-for-now lv-h2686d79044aa owner=smallclaims::store::assistant_grant_authority_bench TODO=chunk-or-reference-exact-wire-values */
                    "INSERT INTO batches(id,origin,replica_sequence,previous_hash,hash,accepted_at_unix_ms)
                     VALUES(?1,?2,?3,NULL,?4,'1791849600000')",
                ).unwrap();
                let mut claim = transaction.prepare_cached(
                    /* sqlite-large-value: allow-for-now lv-h1d162e19a0a1 owner=smallclaims::store::assistant_grant_authority_bench TODO=prepare-body-and-predecessor-values */
                    "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,'[]','1791849600000')",
                ).unwrap();
                // Two writer batches per bounded transaction; no per-row envelope
                // allocation and no growing collection of claim IDs or subjects.
                let sequence = start / config.transaction_rows + 1;
                for writer in [LOCAL, REMOTE] {
                    batch.execute(/* sqlite-large-value: allow-for-now lv-he7097c5f9f00 owner=smallclaims::store::assistant_grant_authority_bench TODO=trace-statement-and-value-representation */ params![format!("batch/{writer}/{sequence:064x}"), writer, sequence, format!("{sequence:064x}")]).unwrap();
                }
                for row in start..config.rows.min(start.saturating_add(config.transaction_rows)) {
                    let offset = row % DENOMINATOR;
                    let id = format!("{:064x}", row + 1);
                    if offset < config.authority_bps {
                        let group = (row / DENOMINATOR) * (config.authority_bps / EVENTS) + offset / EVENTS;
                        let event = offset % EVENTS;
                        // A coprime stride spreads categories across short samples.
                        let category = group.wrapping_mul(7_919) % DENOMINATOR;
                        let malformed = category < config.malformed_bps;
                        let unauthenticated = !malformed && category < config.malformed_bps + config.unauthenticated_bps;
                        let writer = if unauthenticated { REMOTE } else { LOCAL };
                        let revoked_group = group.wrapping_mul(3_571) % DENOMINATOR < config.revoked_bps;
                        let kind = match event {
                            0 => assistant_grant_authority::GRANTED,
                            4 if revoked_group => assistant_grant_authority::REVOKED,
                            _ => assistant_grant_authority::POLICY,
                        };
                        raw_revocations += u64::from(kind == assistant_grant_authority::REVOKED);
                        let body = authority_body(group, event, malformed, config.authority_body_bytes);
                        authority_body_bytes += body.len() as u64;
                        expected_malformed_rows += u64::from(malformed);
                        expected_unauthenticated_rows += u64::from(unauthenticated);
                        if event == 0 && !malformed && !unauthenticated {
                            expected_effective += 1;
                            expected_revoked += u64::from(revoked_group);
                        }
                        claim.execute(/* sqlite-large-value: allow-for-now lv-hb16dbad8dff5 owner=smallclaims::store::assistant_grant_authority_bench TODO=trace-statement-and-value-representation */ params![id, format!("batch/{writer}/{sequence:064x}"),
                            format!("custom/client/assistant-v1-{group:024x}"), kind, writer,
                            "person/perf-operator", body]).unwrap();
                        if writer == LOCAL {
                            // Model a genuine local append, not origin-based trust
                            // during upgrade of ambiguous historical raw rows.
                            transaction.execute(
                                /* sqlite-large-value: allow-for-now lv-h84ea96cecd57 owner=smallclaims::store::assistant_grant_authority_bench TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_local_appends(claim_id) VALUES(?1)", [&id],
                            ).unwrap();
                        }
                    } else {
                        let bucket = match row % 100 { 0..=69 => 0, 70..=94 => 1, _ => 2 };
                        let body = &unrelated_bodies[bucket];
                        unrelated_body_bytes += body.len() as u64;
                        claim.execute(/* sqlite-large-value: allow-for-now lv-h5820156720c0 owner=smallclaims::store::assistant_grant_authority_bench TODO=trace-statement-and-value-representation */ params![id, format!("batch/{LOCAL}/{sequence:064x}"),
                            format!("conversation/synthetic/{:08x}", row % 100_000),
                            format!("synthetic.message.kind{}", row % 16), LOCAL,
                            "person/perf-operator", body]).unwrap();
                    }
                }
            }
            transaction.commit().unwrap();
        }
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM assistant_grant_dirty", [], |row| row.get::<_, u64>(0)).unwrap(), 0);
        let checkpoint: (i64, i64, i64) = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        assert_eq!(checkpoint, (0, 0, 0));
    }
    drop(store);
    println!("authority_backfill_fixture_creation elapsed_ms={:.3}", fixture_started.elapsed().as_secs_f64() * 1_000.0);
    let mut connection = open_connection(&path);
    let raw_rows: u64 = connection.query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0)).unwrap();
    let raw_authority_rows: u64 = connection.query_row(
        "SELECT COUNT(*) FROM claims WHERE kind IN (?1,?2,?3)",
        params![assistant_grant_authority::GRANTED, assistant_grant_authority::POLICY, assistant_grant_authority::REVOKED], |row| row.get(0),
    ).unwrap();
    assert_eq!(raw_rows, config.rows);
    assert_eq!(raw_authority_rows, config.rows / DENOMINATOR * config.authority_bps);
    let (fixture_db_bytes, fixture_wal_bytes) = physical_bytes(&path);
    let (baseline_rss_kib, baseline_peak_kib) = memory_kib();

    let started = Instant::now();
    let install_started = Instant::now();
    assistant_grant_authority::install(&connection, LOCAL).unwrap();
    let install_elapsed = install_started.elapsed();
    assert!(assistant_grant_authority::ready(&connection).is_err());
    // Force a resume probe even when the configured startup budget would finish
    // the sample. Zero budget still processes and commits one real row.
    let (done, resume_probe_elapsed) = slice(&mut connection, Duration::ZERO);
    assert!(!done, "first slice must leave migration unready");
    let saved_cursors = cursors(&connection);
    assert!(!saved_cursors.is_empty());
    drop(connection);
    connection = open_connection(&path);
    assert_eq!(cursors(&connection), saved_cursors, "cursor must survive connection restart");
    assert!(assistant_grant_authority::ready(&connection).is_err());
    let (mut done, startup_slice_elapsed) = slice(&mut connection, Duration::from_millis(config.startup_ms));
    let mut slices = 2_u64;
    let mut max_slice = startup_slice_elapsed.max(resume_probe_elapsed);
    let mut slice_elapsed = startup_slice_elapsed + resume_probe_elapsed;
    while !done {
        let (finished, elapsed) = slice(&mut connection, Duration::from_millis(config.slice_ms));
        done = finished;
        slices += 1;
        max_slice = max_slice.max(elapsed);
        slice_elapsed += elapsed;
    }
    let backfill_elapsed = started.elapsed();
    let (final_db_bytes, final_wal_bytes) = physical_bytes(&path);
    let (final_rss_kib, peak_rss_kib) = memory_kib();
    let effective: u64 = connection.query_row("SELECT COUNT(*) FROM assistant_grant_effective", [], |row| row.get(0)).unwrap();
    let revoked: u64 = connection.query_row("SELECT COUNT(*) FROM assistant_grant_effective WHERE revoked=1", [], |row| row.get(0)).unwrap();
    let authority: u64 = connection.query_row("SELECT COUNT(*) FROM assistant_grant_authority", [], |row| row.get(0)).unwrap();
    let unauthenticated: u64 = connection.query_row("SELECT COUNT(*) FROM assistant_grant_pending WHERE authenticated=0", [], |row| row.get(0)).unwrap();
    let pending_unauthenticated_grants: u64 = connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_pending WHERE authenticated=0 AND kind=?1",
        [assistant_grant_authority::GRANTED], |row| row.get(0),
    ).unwrap();
    assert_eq!(effective, expected_effective);
    assert!(effective > 0, "sample must include authenticated valid groups");
    assert_eq!(revoked, expected_revoked);
    assert_eq!(authority, expected_effective * EVENTS);
    assert_eq!(unauthenticated, expected_unauthenticated_rows);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM assistant_grant_dirty", [], |row| row.get::<_, u64>(0)).unwrap(), 0);
    assert_eq!(connection.query_row("SELECT value FROM meta WHERE key='assistant_grant_authority_version'", [], |row| row.get::<_, String>(0)).unwrap(), "1");
    // A second maintenance pass must not duplicate resumed rows or alter readiness.
    let transaction = connection.transaction().unwrap();
    assert!(assistant_grant_authority::backfill_slice(&transaction, Duration::from_millis(config.slice_ms)).unwrap());
    transaction.commit().unwrap();
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM assistant_grant_authority", [], |row| row.get::<_, u64>(0)).unwrap(), authority);

    println!("authority_backfill_compatibility pending_unauthenticated_grant_claims={pending_unauthenticated_grants}");
    let scale = config.target_bytes as f64 / fixture_db_bytes as f64;
    println!("authority_backfill_fixture raw_claims={raw_rows} grant_kind_claims={raw_authority_rows} granted={} policies={} revocations={raw_revocations} malformed_rows={expected_malformed_rows} unauthenticated_rows={expected_unauthenticated_rows} effective_grants={effective} authority_rows={authority} authority_bps={} malformed_group_bps={} unauthenticated_group_bps={} revoked_group_bps={} unrelated_json_bytes={unrelated_body_bytes} authority_json_bytes={authority_body_bytes} fixture_db_bytes={fixture_db_bytes} fixture_wal_bytes={fixture_wal_bytes} final_db_bytes={final_db_bytes} final_wal_bytes={final_wal_bytes} transaction_rows={}",
        raw_authority_rows / EVENTS, raw_authority_rows - raw_authority_rows / EVENTS - raw_revocations,
        config.authority_bps, config.malformed_bps, config.unauthenticated_bps, config.revoked_bps, config.transaction_rows);
    println!("authority_backfill_timing install_ms={:.3} startup_slice_ms={:.3} startup_total_ms={:.3} startup_budget_ms={} continuation_budget_ms={} slices={slices} max_slice_ms={:.3} slice_sum_ms={:.3} elapsed_with_restart_ms={:.3} resume_probe_ms={:.3} restart_count=1 readiness_checked_each_slice=true",
        install_elapsed.as_secs_f64() * 1_000.0, startup_slice_elapsed.as_secs_f64() * 1_000.0,
        (install_elapsed + startup_slice_elapsed).as_secs_f64() * 1_000.0, config.startup_ms, config.slice_ms,
        max_slice.as_secs_f64() * 1_000.0, slice_elapsed.as_secs_f64() * 1_000.0, backfill_elapsed.as_secs_f64() * 1_000.0,
        resume_probe_elapsed.as_secs_f64() * 1_000.0);
    println!("authority_backfill_memory baseline_rss_kib={baseline_rss_kib} baseline_process_peak_kib={baseline_peak_kib} final_rss_kib={final_rss_kib} peak_rss_kib={peak_rss_kib} high_water_increase_kib={} sqlite_cache_kib={}", peak_rss_kib.saturating_sub(baseline_peak_kib), crate::sqlite::WRITE_CACHE_KIB);
    println!("authority_backfill_extrapolation target_bytes={} scale={scale:.6} estimated_raw_claims={:.0} estimated_grant_kind_claims={:.0} linear_elapsed_seconds={:.3} fixed_working_set_peak_kib={peak_rss_kib} pessimistic_linear_peak_kib={:.0} methodology=target_divided_by_checkpointed_fixture_db_bytes_times_observed_work warning=synthetic_mix_not_production_distribution;time_assumes_constant_throughput_and_same_indexes_cache_storage;memory_fixed_cache_hypothesis_not_proven_at_50GB;linear_peak_is_sensitivity_not_prediction;VmHWM_includes_fixture_and_test_process;WAL_is_physical_recycled_bytes;startup_excludes_Store_open_and_includes_one_prior_resume_probe_row;no_cold_cache_claim",
        config.target_bytes, raw_rows as f64 * scale, raw_authority_rows as f64 * scale,
        backfill_elapsed.as_secs_f64() * scale, peak_rss_kib as f64 * scale);
}
