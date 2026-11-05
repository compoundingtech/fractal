use super::run_eta::{person_adjusted_duration, step_timestamps};
use serde_json::json;

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

pub(super) fn fixture() -> rusqlite::Connection {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hfbd29c4eba3b owner=fractal_store::store::run_eta_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);
        CREATE TABLE mission_runs(id TEXT PRIMARY KEY,current_generation_id TEXT,
            phase TEXT,status TEXT,mission_id TEXT DEFAULT 'mission');
        CREATE TABLE step_runs(subject TEXT PRIMARY KEY,run_id TEXT,generation_id TEXT,
            definition_hash TEXT,step_path TEXT,attempt INTEGER,status TEXT);
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
    c
}

pub(super) fn claim(
    c: &rusqlite::Connection,
    subject: &str,
    kind: &str,
    fields: serde_json::Value,
    at: i64,
) {
    let at = at.to_string();
    let body = json!({"fields":fields}).to_string();
    c.execute(
        /* sqlite-large-value: allow-for-now lv-he744e35e69c5 owner=fractal_store::store::run_eta_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,subject,kind,body,accepted_at_unix_ms)
        VALUES(?1||'-'||?2||'-'||?3,?1,?2,?4,?3)",
        rusqlite::params![subject, kind, at, body],
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
        let mut c = fixture();
        for n in 0..1000 {
            seed(&c, n, n + 10);
        }
        for n in 936..1000 {
            for at in 2000..2100 {
                claim(
                    &c,
                    &format!("s-{n}"),
                    "work.claimed",
                    json!({"attempt":2}),
                    at,
                );
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
            for (sql, expected) in [
                (/* sqlite-large-value: allow-for-now lv-h3c1e6e8ad66d owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM step_runs WHERE subject='s-999'", (1, 945)),
                (/* sqlite-large-value: allow-for-now lv-h85885f47ee57 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mission_runs WHERE id='r-998'", (2, 944)),
                (
                    /* sqlite-large-value: allow-for-now lv-h69dcdb7ef848 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET definition_hash='other' WHERE subject='s-997'",
                    (3, 943),
                ),
            ] {
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
    let mut c = fixture();
    for n in 0..70 {
        seed(&c, n, n + 10);
    }
    let tx = c.transaction().unwrap();
    super::run_eta::open(&tx).unwrap();
    assert_eq!(retained(&tx), (64, 16));
    for (sql, expected) in [
        (
            "UPDATE mission_runs SET status='failed' WHERE id='r-69'",
            (63, 16),
        ),
        ("DELETE FROM mission_runs WHERE id='r-68'", (63, 15)),
        (
            /* sqlite-large-value: allow-for-now lv-hc89abf19ea11 owner=fractal_store::store::run_eta_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='2' WHERE subject='s-67' AND kind='step-run.state'",
            (63, 2),
        ),
        (
            /* sqlite-large-value: allow-for-now lv-hac6412f7b9a9 owner=fractal_store::store::run_eta_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms='100' WHERE subject IN('s-0','mission-run/r-0') AND kind IN('step-run.state','mission-run.state')",
            (63, 2),
        ),
    ] {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-haa75b699e8f2 owner=fractal_store::store::run_eta_tests TODO=trace-statement-and-value-representation */ sql).unwrap();
        super::run_eta::flush(&tx).unwrap();
        assert_eq!(retained(&tx), expected);
    }
    assert_eq!(
        tx.query_row(
            "SELECT duration FROM local_run_eta_samples WHERE subject='s-0'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        99
    );
    tx.commit().unwrap();
}

#[test]
fn eta_nonterminal_changes_do_not_dirty_or_refold_samples() {
    let mut c = fixture();
    seed(&c, 0, 10);
    let tx = c.transaction().unwrap();
    super::run_eta::open(&tx).unwrap();
    super::run_eta::BODY_ROWS.with(|count| count.set(0));
    tx.execute(/* sqlite-large-value: allow-for-now lv-he81e44f26b12 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO mission_runs(id,current_generation_id,phase,status) VALUES('active','generation','working','running')", []).unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-h1a5e4594acca owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO step_runs VALUES('active-step','active','generation','hash','build',1,'working')", []).unwrap();
    claim(&tx, "active-step", "work.claimed", json!({"attempt":1}), 20);
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h2b6859136770 owner=fractal_store::store::run_eta_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='claimed' WHERE subject='active-step'",
        [],
    )
    .unwrap();
    let pending: i64 = tx.query_row("SELECT (SELECT COUNT(*) FROM local_run_eta_pending)+(SELECT COUNT(*) FROM local_run_eta_dirty)", [], |row| row.get(0)).unwrap();
    assert_eq!(pending, 0);
    super::run_eta::flush(&tx).unwrap();
    super::run_eta::BODY_ROWS.with(|count| assert_eq!(count.get(), 0));
}
