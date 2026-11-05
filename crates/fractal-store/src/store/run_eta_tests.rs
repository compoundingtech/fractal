use super::run_eta::{person_adjusted_duration, step_timestamps};
use rusqlite::Connection;
use serde_json::{Value, json};

#[test]
fn eta_person_holds_are_clipped_and_union_counted_without_removing_other_waits() {
    let mut holds = [(12, 18), (15, 22), (0, 11), (29, 40), (50, 60)];
    assert_eq!(person_adjusted_duration(10, 30, &mut holds), 8);
    assert_eq!(person_adjusted_duration(10, 30, &mut []), 20);
}

#[test]
fn eta_duration_boundaries_use_selected_attempt_for_workers_and_agentless_steps() {
    let event = |kind: &str, fields, at| (kind.to_owned(), json!({"fields": fields}), at);
    let events = [
        event("work.claimed", json!({"attempt":1}), 10),
        event("work.failed", json!({"attempt":1,"status":"failed"}), 20),
        event("step-run.retried", json!({"attempt":2}), 30),
        event("work.claimed", json!({"attempt":2}), 40),
        event(
            "work.progress",
            json!({"attempt":2,"clock":"paused","wait_until_unix_ms":80}),
            50,
        ),
        event("work.released", json!({"attempt":2}), 70),
        event("work.claimed", json!({"attempt":2}), 90),
        event("step-run.state", json!({"status":"completed"}), 100),
    ];
    let (start, end) = step_timestamps(&events, 2);
    assert_eq!((start, end), (Some(40), Some(100)));
    assert_eq!(
        person_adjusted_duration(start.unwrap(), end.unwrap(), &mut []),
        60
    );
    let automatic = [
        event(
            "step-run.state",
            json!({"attempt":1,"status":"working"}),
            200,
        ),
        event("step-run.state", json!({"status":"completed"}), 275),
    ];
    assert_eq!(step_timestamps(&automatic, 1), (Some(200), Some(275)));
}

pub(super) fn fixture(history: i64) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h12bb55457f44 owner=fractal_store::store::run_eta_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);
        CREATE TABLE mission_runs(id TEXT PRIMARY KEY,current_generation_id TEXT,
            phase TEXT,status TEXT,mission_id TEXT DEFAULT 'mission');
        CREATE TABLE step_runs(subject TEXT PRIMARY KEY,run_id TEXT,generation_id TEXT,
            definition_hash TEXT,step_path TEXT,attempt INTEGER,status TEXT);
        CREATE INDEX step_runs_run_index ON step_runs(run_id,generation_id,step_path);
        CREATE INDEX step_runs_definition_index ON step_runs(definition_hash,run_id,generation_id);
        CREATE TABLE claims(store_index INTEGER PRIMARY KEY,id TEXT,subject TEXT,kind TEXT,
            body TEXT,accepted_at_unix_ms TEXT,batch_id TEXT DEFAULT 'batch');
        CREATE INDEX claim_subject ON claims(subject,kind,accepted_at_unix_ms);
        CREATE TABLE batches(id TEXT,origin TEXT,replica_sequence INTEGER);
        INSERT INTO batches VALUES('batch','origin',1);
        CREATE TABLE claim_positions(claim_id TEXT,position INTEGER);
        CREATE TABLE replica_records(claim_id TEXT,position INTEGER);
        CREATE TABLE run_generations(id TEXT,revision TEXT);
        INSERT INTO run_generations VALUES('generation','revision');
        CREATE TABLE mission_revisions(mission_id TEXT,revision TEXT,body TEXT);",
    )
    .unwrap();
    for n in 0..history {
        seed(&c, n, n + 10);
    }
    c
}
pub(super) fn claim(c: &Connection, subject: &str, kind: &str, fields: Value, at: i64) {
    c.execute(
        /* sqlite-large-value: allow-for-now lv-he744e35e69c5 owner=fractal_store::store::run_eta_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,subject,kind,body,accepted_at_unix_ms)
        VALUES(?1||'-'||?2||'-'||?3,?1,?2,?4,?3)",
        rusqlite::params![subject, kind, at, json!({"fields":fields}).to_string()],
    )
    .unwrap();
}

pub(super) fn seed(c: &rusqlite::Connection, n: i64, end: i64) {
    c.execute(
        /* sqlite-large-value: allow-for-now lv-he59d5e1a74a6 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs(id,current_generation_id,phase,status)
        VALUES(?1,'generation','terminal','completed')",
        [format!("r-{n}")],
    )
    .unwrap();
    c.execute(
        /* sqlite-large-value: allow-for-now lv-hcec5f6ecc0a6 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs VALUES(?1,?2,'generation','hash','build',1,'completed')",
        rusqlite::params![format!("s-{n}"), format!("r-{n}")],
    )
    .unwrap();
    let step = format!("s-{n}");
    let run = format!("mission-run/r-{n}");
    let completed = json!({"attempt":1,"status":"completed"});
    for (subject, kind, fields, at) in [
        (&step, "work.claimed", json!({"attempt":1}), 1),
        (&step, "step-run.state", completed, end),
        (&run, "mission-run.state", json!({"phase":"terminal"}), end),
    ] {
        claim(c, subject, kind, fields, at);
    }
}

fn retained(c: &rusqlite::Connection) -> (i64, i64) {
    c.query_row(
        "SELECT COUNT(*),COALESCE(MIN(ended),0) FROM local_run_eta_samples",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

#[test]
fn eta_populated_open_is_bounded_with_unusable_and_long_history_candidates() {
    for unusable in [false, true] {
        let mut c = fixture(1000);
        for n in 936..1000 {
            let subject = format!("s-{n}");
            for at in 2000..2100 {
                claim(&c, &subject, "work.claimed", json!({"attempt":2}), at);
            }
        }
        if unusable {
            c.execute(/* sqlite-large-value: allow-for-now lv-hecdc80d10d31 owner=fractal_store::store::run_eta_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE kind='work.claimed' AND json_extract(body,'$.fields.attempt')=1 AND CAST(substr(subject,3) AS INTEGER)>=936", []).unwrap();
        }
        super::run_eta::BODY_ROWS.with(|count| count.set(0));
        let tx = c.transaction().unwrap();
        super::run_eta::open(&tx).unwrap();
        assert_eq!(retained(&tx), if unusable { (0, 0) } else { (64, 946) });
        super::run_eta::BODY_ROWS
            .with(|count| assert_eq!(count.get(), if unusable { 64 } else { 128 }));
        if unusable {
            let statements = [
                /* sqlite-large-value: allow-for-now lv-h3c1e6e8ad66d owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */
                "DELETE FROM step_runs WHERE subject='s-999'",
                /* sqlite-large-value: allow-for-now lv-h85885f47ee57 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */
                "DELETE FROM mission_runs WHERE id='r-998'",
                /* sqlite-large-value: allow-for-now lv-h69dcdb7ef848 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */
                "UPDATE step_runs SET definition_hash='other' WHERE subject='s-997'",
            ];
            for (sql, expected) in statements.into_iter().zip([(1, 945), (2, 944), (3, 943)]) {
                tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h1acd02af02ee owner=fractal_store::store::run_eta_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
                super::run_eta::flush(&tx).unwrap();
                assert_eq!(retained(&tx), expected);
            }
        }
        tx.commit().unwrap();
    }
}

#[test]
fn eta_window_refills_after_failure_deletion_and_completion_reordering() {
    let mut c = fixture(70);
    let tx = c.transaction().unwrap();
    super::run_eta::open(&tx).unwrap();
    assert_eq!(retained(&tx), (64, 16));
    let statements = [
        /* sqlite-large-value: allow-for-now lv-h287bdf68b661 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */
        "UPDATE mission_runs SET status='failed' WHERE id='r-69'",
        /* sqlite-large-value: allow-for-now lv-hf5ac5368b717 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */
        "DELETE FROM mission_runs WHERE id='r-68'",
        /* sqlite-large-value: allow-for-now lv-hc89abf19ea11 owner=fractal_store::store::run_eta_tests TODO=prepare-body-and-predecessor-values */
        "UPDATE claims SET accepted_at_unix_ms='2' WHERE subject='s-67' AND kind='step-run.state'",
        /* sqlite-large-value: allow-for-now lv-hac6412f7b9a9 owner=fractal_store::store::run_eta_tests TODO=prepare-body-and-predecessor-values */
        "UPDATE claims SET accepted_at_unix_ms='100' WHERE subject IN('s-0','mission-run/r-0') AND kind IN('step-run.state','mission-run.state')",
    ];
    for (sql, expected) in statements
        .into_iter()
        .zip([(63, 16), (63, 15), (63, 2), (63, 2)])
    {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-haa75b699e8f2 owner=fractal_store::store::run_eta_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
        super::run_eta::flush(&tx).unwrap();
        assert_eq!(retained(&tx), expected);
    }
    let duration: i64 = tx
        .query_row(
            "SELECT duration FROM local_run_eta_samples WHERE subject='s-0'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(duration, 99);
    tx.commit().unwrap();
}

#[test]
fn eta_nonterminal_changes_do_not_dirty_or_refold_samples() {
    let mut c = fixture(1);
    let tx = c.transaction().unwrap();
    super::run_eta::open(&tx).unwrap();
    super::run_eta::BODY_ROWS.with(|count| count.set(0));
    tx.execute(/* sqlite-large-value: allow-for-now lv-he81e44f26b12 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs(id,current_generation_id,phase,status) VALUES('active','generation','working','running')", []).unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-h1a5e4594acca owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs VALUES('active-step','active','generation','hash','build',1,'working')", []).unwrap();
    claim(&tx, "active-step", "work.claimed", json!({"attempt":1}), 20);
    claim(&tx, "s-0", "telemetry", json!({}), 21);
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h2b6859136770 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='claimed' WHERE subject='active-step'",
        [],
    )
    .unwrap();
    let pending: i64 = tx.query_row("SELECT (SELECT COUNT(*) FROM local_run_eta_pending)+(SELECT COUNT(*) FROM local_run_eta_dirty)", [], |row| row.get(0)).unwrap();
    assert_eq!(pending, 0);
    super::run_eta::flush(&tx).unwrap();
    super::run_eta::BODY_ROWS.with(|count| assert_eq!(count.get(), 0));
    for sql in [
        /* sqlite-large-value: allow-for-now lv-he41924f3834b owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */
        "UPDATE step_runs SET definition_hash='a' WHERE subject='s-0'",
        /* sqlite-large-value: allow-for-now lv-h21519d510b2d owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */
        "UPDATE step_runs SET step_path='a' WHERE subject='s-0'",
    ] {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h3595dd1da8bb owner=fractal_store::store::run_eta_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
        super::run_eta::flush(&tx).unwrap();
        assert_eq!(retained(&tx), (1, 10));
    }
}

#[test]
fn eta_rebuild_candidates_search_terminal_order_index_before_loading_claims() {
    let mut c = fixture(1000);
    let tx = c.transaction().unwrap();
    super::run_eta::open(&tx).unwrap();
    let sql = format!("EXPLAIN QUERY PLAN {}", super::run_eta::CANDIDATES_SQL);
    let plan = tx.prepare(/* sqlite-large-value: allow-for-now lv-h2cecf29641ee owner=fractal_store::store::run_eta_tests TODO=trace-statement-and-value-representation */ &sql).unwrap()
        .query_map(rusqlite::params!["hash","build",64,true], |row| row.get::<_,String>(3)).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
    assert!(
        plan.contains(
            "SEARCH local_run_eta_terminal USING COVERING INDEX local_run_eta_terminal_order"
        ),
        "{plan}"
    );
    assert!(!plan.contains("SCAN mission_runs"), "{plan}");
    let query = format!(
        "EXPLAIN QUERY PLAN {}",
        super::run_eta::INVALIDATE_MISSIONS_SQL
    );
    let plan = tx.prepare(/* sqlite-large-value: allow-for-now lv-hbd1d816360bf owner=fractal_store::store::run_eta_tests TODO=trace-statement-and-value-representation */ &query).unwrap()
        .query_map(["hash","build"], |row| row.get::<_,String>(3)).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
    assert!(
        plan.contains("SEARCH s USING INDEX step_runs_eta_remaining"),
        "{plan}"
    );
    super::run_eta::BODY_ROWS.with(|count| count.set(0));
    claim(&tx, "s-999", "work.claimed", json!({"attempt":1}), 2);
    super::run_eta::flush(&tx).unwrap();
    super::run_eta::BODY_ROWS.with(|count| assert_eq!(count.get(), 129));
}

#[test]
fn eta_new_terminal_transition_folds_only_its_run_at_large_history() {
    for history in [1000, 10000] {
        let mut c = fixture(history);
        seed(&c, history, history + 10);
        let run = format!("r-{history}");
        c.execute(/* sqlite-large-value: allow-for-now lv-h265fd785cf17 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET phase='working',status='running' WHERE id=?1", [&run]).unwrap();
        let tx = c.transaction().unwrap();
        super::run_eta::open(&tx).unwrap();
        super::run_eta::BODY_ROWS.with(|count| count.set(0));
        // Exercise working -> terminal, not just insertion of an already-terminal run.
        tx.execute(/* sqlite-large-value: allow-for-now lv-h1e30d65a948a owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET phase='terminal',status='completed' WHERE id=?1", [&run]).unwrap();
        super::run_eta::flush(&tx).unwrap();
        super::run_eta::BODY_ROWS.with(|count| assert_eq!(count.get(), 2, "history={history}"));
        assert_eq!(retained(&tx), (64, history - 53));
        super::run_eta::BODY_ROWS.with(|count| count.set(0));
        tx.execute(/* sqlite-large-value: allow-for-now lv-ha9356a96ddd9 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mission_runs WHERE id=?1", [&run]).unwrap();
        super::run_eta::flush(&tx).unwrap();
        super::run_eta::BODY_ROWS.with(|count| assert_eq!(count.get(), 128));
        assert_eq!(retained(&tx), (64, history - 54));
    }
}

#[test]
fn eta_v3_trigger_layout_upgrades_once_and_reopen_preserves_schema_cookie() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("eta-upgrade.db");
    drop(super::Store::open(&path, "node").unwrap());
    let triggers = |c: &Connection| {
        c.prepare("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND name LIKE 'run_eta_%' ORDER BY name").unwrap()
            .query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    let c = Connection::open(&path).unwrap();
    let current = triggers(&c);
    c.execute_batch(/* sqlite-large-value: allow-for-now lv-hd414c559eb6c owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key IN('run_eta_triggers_v4','run_eta_samples_v4');
        INSERT OR IGNORE INTO meta VALUES('run_eta_samples_v3','1');
        DROP TABLE local_run_eta_terminal;
        DROP TRIGGER run_eta_run_insert;
        CREATE TRIGGER run_eta_run_insert AFTER INSERT ON mission_runs BEGIN
        INSERT OR IGNORE INTO local_run_eta_pending VALUES(NEW.id); END;").unwrap();
    assert_ne!(triggers(&c), current);
    drop(c);
    drop(super::Store::open(&path, "node").unwrap());
    let c = Connection::open(&path).unwrap();
    assert_eq!(triggers(&c), current);
    let cookie: i64 = c
        .query_row("PRAGMA schema_version", [], |row| row.get(0))
        .unwrap();
    let marker: String = c
        .query_row(
            "SELECT value FROM meta WHERE key='run_eta_triggers_v4'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(marker, "1");
    drop(c);
    drop(super::Store::open(&path, "node").unwrap());
    let c = Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row::<i64, _, _>("PRAGMA schema_version", [], |row| row.get(0))
            .unwrap(),
        cookie
    );
    assert_eq!(triggers(&c), current);
}

#[test]
fn eta_fresh_v4_rollback_and_reupgrade_preserve_v4_triggers() {
    let mut c = fixture(1);
    let tx = c.transaction().unwrap();
    super::run_eta::open(&tx).unwrap();
    let trigger = |tx: &rusqlite::Transaction<'_>| {
        tx.query_row::<String, _, _>(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='run_eta_run_update'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    };
    let current = trigger(&tx);
    // The parent's installer runs only when its own initialization marker is absent.
    let initialized: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='run_eta_samples_v3')",
        [],
        |row| row.get(0),
    ).unwrap();
    if !initialized {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-ha4a9dd3c5d26 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER run_eta_run_update;
            CREATE TRIGGER run_eta_run_update AFTER UPDATE ON mission_runs
            WHEN NEW.phase='terminal' OR OLD.phase='terminal' BEGIN
            INSERT OR IGNORE INTO local_run_eta_dirty
            SELECT definition_hash,step_path FROM step_runs WHERE run_id=OLD.id;
            INSERT OR IGNORE INTO local_run_eta_pending VALUES(NEW.id); END;
            INSERT OR IGNORE INTO meta VALUES('run_eta_samples_v3','1');").unwrap();
    }
    super::run_eta::open(&tx).unwrap();
    assert_eq!(trigger(&tx), current);
}

#[test]
fn eta_terminal_metadata_survives_v3_writer_draining_shared_queues() {
    let mut c = fixture(70);
    let tx = c.transaction().unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-hd14808d55e0c owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta VALUES('run_eta_samples_v3','1')", [])
        .unwrap();
    super::run_eta::open(&tx).unwrap();
    seed(&tx, 70, 80);
    tx.execute(/* sqlite-large-value: allow-for-now lv-hc3dfb33e117b owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mission_runs WHERE id='r-69'", [])
        .unwrap();
    // The rolled-back v3 writer updates duration samples but never terminal metadata.
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h189746109b84 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_run_eta_samples WHERE run_id IN(SELECT run_id FROM local_run_eta_pending);
        INSERT INTO local_run_eta_samples VALUES('s-70','r-70','hash','build',80,79);
        DELETE FROM local_run_eta_pending; DELETE FROM local_run_eta_dirty;").unwrap();
    assert_eq!(
        tx.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM local_run_eta_terminal_pending",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        2
    );
    super::run_eta::open(&tx).unwrap();
    assert_eq!(retained(&tx), (64, 16));
    assert_eq!(
        tx.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM local_run_eta_terminal WHERE run_id='r-70'",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        tx.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM local_run_eta_terminal WHERE run_id='r-69'",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
    seed(&tx, 71, 81);
    super::run_eta::flush(&tx).unwrap();
    assert_eq!(retained(&tx), (64, 17));
}

/// The window's candidates as #5154 read them: every terminal run, then whether it ran the
/// definition. Kept only to prove the indexed read answers the same.
fn candidates_by_scanning_every_run() -> String {
    format!("WITH recent AS MATERIALIZED (
        SELECT r.id,r.current_generation_id,r.status,
        (SELECT CAST(claims.accepted_at_unix_ms AS INTEGER) FROM claims JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject='mission-run/'||r.id AND claims.kind='mission-run.state'
         AND json_extract(claims.body,'$.fields.phase')='terminal' ORDER BY {} LIMIT 1) AS ended
        FROM mission_runs r WHERE r.phase='terminal'
        AND EXISTS(SELECT 1 FROM step_runs WHERE run_id=r.id AND generation_id=r.current_generation_id AND definition_hash=?1)
        ORDER BY ended DESC,r.id DESC LIMIT ?3)
        SELECT s.subject,s.run_id,s.attempt FROM recent r JOIN step_runs s ON s.run_id=r.id
        AND s.generation_id=r.current_generation_id WHERE s.definition_hash=?1 AND s.step_path=?2
        AND r.status='completed' AND s.status='completed'", super::CANONICAL_ORDER)
}

const ASKS_BY_ONE_OR: &str = "SELECT subject,body,accepted_at_unix_ms FROM claims
    WHERE kind='work.person-asked' AND (subject=?1 OR json_extract(body,'$.fields.origin_step')=?1)";

/// `runs` terminal and working runs over shared and single-run definitions, two paths, stale
/// generations, failed runs, tied end times and person asks by subject and by origin.
fn history(runs: i64) -> Connection {
    let c = fixture(0);
    for n in 0..runs {
        let (run, phase) = (format!("r-{n}"), if n % 7 == 3 { "working" } else { "terminal" });
        let status = if n % 5 == 4 { "failed" } else { "completed" };
        c.execute(
            /* sqlite-large-value: allow-for-now lv-h32544824cbd4 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs(id,current_generation_id,phase,status) VALUES(?1,'current',?2,?3)",
            rusqlite::params![run, phase, status],
        )
        .unwrap();
        // One definition shared by every tenth run, overflowing the window; the rest by a few.
        let hash = if n % 10 == 0 { "shared".to_owned() } else { format!("h-{}", n % (runs / 3)) };
        let end = (n * 37) % 1000 + 10;
        for (path, generation) in [("build", "current"), ("test", "current"), ("build", "stale")] {
            if path == "test" && n % 2 == 0 {
                continue;
            }
            let step = format!("s-{n}-{path}-{generation}");
            c.execute(
                /* sqlite-large-value: allow-for-now lv-h4e61643fbe6f owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs VALUES(?1,?2,?3,?4,?5,1,?6)",
                rusqlite::params![step, run, generation, hash, path, if n % 11 == 6 { "failed" } else { "completed" }],
            )
            .unwrap();
            claim(&c, &step, "work.claimed", json!({"attempt":1}), 1);
            claim(&c, &step, "step-run.state", json!({"attempt":1,"status":"completed"}), end);
            if n % 13 == 0 {
                claim(&c, &format!("ask-{step}"), "work.person-asked",
                    json!({"origin_step":step,"origin_attempt":1,"request":{"type":"decision"}}), end - 5);
            }
            if n % 17 == 0 {
                claim(&c, &step, "work.person-asked", json!({"attempt":1,"origin_step":step}), end - 3);
            }
        }
        claim(&c, &format!("mission-run/{run}"), "mission-run.state", json!({"phase":"terminal"}), end);
    }
    // The real store's claim indexes and batch positions. Without positions the canonical order
    // counts the batch's earlier claims for every row it sorts.
    c.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h4d94de7d92a4 owner=fractal_store::store::run_eta_tests TODO=chunk-or-reference-variable-columns */ "CREATE INDEX claims_kind_index ON claims(kind,store_index);
         CREATE INDEX claims_person_ask_origin_step_index ON claims(json_extract(body,'$.fields.origin_step'))
             WHERE kind='work.person-asked';
         CREATE UNIQUE INDEX claim_positions_claim ON claim_positions(claim_id);
         INSERT OR IGNORE INTO claim_positions SELECT id,store_index FROM claims;",
    )
    .unwrap();
    c
}

fn rows(c: &Connection, sql: &str, params: impl rusqlite::Params) -> Vec<(String, String, String)> {
    let mut rows = c
        .prepare(/* sqlite-large-value: allow-for-now lv-h81ecd44328e3 owner=fractal_store::store::run_eta_tests TODO=trace-statement-and-value-representation */ sql)
        .unwrap()
        .query_map(params, |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, rusqlite::types::Value>(2).map(|v| format!("{v:?}"))?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows.sort();
    rows
}

#[test]
fn eta_indexed_candidates_and_asks_answer_as_the_scanning_reads_for_every_definition() {
    let mut c = history(1500);
    let tx = c.transaction().unwrap();
    super::run_eta::open(&tx).unwrap();
    tx.commit().unwrap();
    let keys = rows(&c, "SELECT DISTINCT definition_hash,step_path,'' FROM step_runs", []);
    assert!(keys.len() > 600, "{}", keys.len());
    let (mut candidates, mut full) = (0, 0);
    for (hash, path, _) in &keys {
        let args = rusqlite::params![hash, path, super::run_eta::SAMPLE_LIMIT];
        let expected = rows(&c, &candidates_by_scanning_every_run(), args);
        assert_eq!(rows(&c, super::run_eta::CANDIDATES_SQL, rusqlite::params![hash,path,super::run_eta::SAMPLE_LIMIT,true]), expected, "{hash} {path}");
        candidates += expected.len();
        full += usize::from(expected.len() >= 40);
    }
    assert!(candidates > 1000 && full > 0, "{candidates} {full}");
    let steps = rows(&c, "SELECT subject,'','' FROM step_runs", []);
    let mut asks = 0;
    for (step, _, _) in &steps {
        let expected = rows(&c, ASKS_BY_ONE_OR, [step]);
        assert_eq!(rows(&c, super::run_eta::ASKS_SQL, [step]), expected, "{step}");
        asks += expected.len();
    }
    assert!(asks > 100, "{asks}");
}

/// The first open on a long history read every terminal run once per definition, and every
/// person ask once per sampled step (#5239). Both must seek their own rows on the real schema.
#[test]
fn eta_backfill_seeks_each_definitions_runs_and_each_steps_asks() {
    let store = super::Store::open_memory("node").unwrap();
    let connection = store.readers.get();
    let plan = |sql: &str| {
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hbd7e2e22ab73 owner=fractal_store::store::run_eta_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let unbound = std::iter::repeat_n(rusqlite::types::Null, statement.parameter_count());
        statement
            .query_map(rusqlite::params_from_iter(unbound), |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("; ")
    };
    let candidates = plan(super::run_eta::CANDIDATES_SQL);
    assert!(
        candidates.contains("SEARCH local_run_eta_terminal USING COVERING INDEX local_run_eta_terminal_order (definition_hash=?)")
            && candidates.contains("SEARCH r USING INDEX sqlite_autoindex_mission_runs_1 (id=?)"),
        "{candidates}"
    );
    let asks = plan(super::run_eta::ASKS_SQL);
    assert!(
        asks.contains("claims_person_ask_origin_step_index") && !asks.contains("claims_kind_index"),
        "{asks}"
    );
}

/// Before and after for #5239: `ETA_COST_RUNS=4000 cargo test -p fractal-store --lib eta_backfill_cost
/// -- --ignored --nocapture`. Prints the first open, a rebuild and the flush after deleting a quarter
/// of the runs.
#[test]
#[ignore]
fn eta_backfill_cost_at_scale() {
    let runs = std::env::var("ETA_COST_RUNS").map_or(4000, |runs| runs.parse().unwrap());
    let mut c = history(runs);
    let timed = |what: &str, c: &mut Connection, work: &dyn Fn(&rusqlite::Transaction<'_>)| {
        let tx = c.transaction().unwrap();
        let started = std::time::Instant::now();
        work(&tx);
        println!("{what}: {:.2}s", started.elapsed().as_secs_f64());
        tx.commit().unwrap();
    };
    timed("first open", &mut c, &|tx| super::run_eta::open(tx).unwrap());
    timed("rebuild", &mut c, &|tx| super::run_eta::rebuild(tx).unwrap());
    timed("flush after deleting 1000 terminal runs", &mut c, &|tx| {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h83e8e20c3907 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mission_runs WHERE phase='terminal' AND CAST(substr(id,3) AS INTEGER)%4=1", []).unwrap();
        super::run_eta::flush(tx).unwrap();
    });
}
