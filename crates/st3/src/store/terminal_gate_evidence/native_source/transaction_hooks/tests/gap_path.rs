//! Private gap persistence controls; no production installation or execution path.
use super::*;

type Fence = (
    (i64, bool, Option<String>, bool),
    (i64, i64, i64),
    Vec<(String, i64, Option<String>, i64, i64)>,
);
fn fenced() -> Fixture {
    let f = Fixture::new(true);
    f.raw(|c| update_exit_raw(c, 4));
    f.commit(|_| {});
    assert_eq!(f.line()["status"], "unknown");
    assert!(f.read(|tx| f.source().installer.root(tx, VIEW).is_err()));
    f.reset_counts();
    f
}
fn snapshot(f: &Fixture) -> Fence {
    f.read(|tx| {
        let state = f.source().scoped_state(tx).unwrap();
        let source = tx
            .query_row(
                "SELECT available,epoch,revision FROM ivm_install_sources WHERE name=?1",
                [SOURCE],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let mut s=tx.prepare("SELECT view,ready,error,generation,status_revision FROM ivm_install_roots WHERE source=?1 ORDER BY view LIMIT 3").unwrap();
        let roots = s
            .query_map([SOURCE], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (state, source, roots)
    })
}
fn rank_change(c: &rusqlite::Connection) {
    assert_eq!(c.execute("UPDATE batches SET hash=hash WHERE id=(SELECT batch_id FROM claims WHERE subject='exec/one/probe' AND kind='runtime.observed' LIMIT 1)", []).unwrap(), 1);
}
fn stable_commit(f: &Fixture, unrelated: bool) -> (SqliteWork, u64) {
    f.reset_counts();
    let mut writer = f.store.connection.write();
    let before = writer.total_changes();
    let scope = SqliteWorkScope::start();
    let tx = writer.transaction().unwrap();
    if unrelated {
        assert_eq!(
            tx.execute(
                "UPDATE meta SET value=value WHERE key='private-gap-unrelated'",
                [],
            )
            .unwrap(),
            1
        );
    }
    tx.commit().unwrap();
    let work = scope.finish();
    let changes = writer.total_changes() - before;
    drop(writer);
    assert_eq!(changes, if unrelated { 3 } else { 2 });
    assert_eq!(f.counts(), (1, 1));
    (work, changes)
}
fn query_program(f: &Fixture) -> u64 {
    f.read(|tx| {
        let reason = f.source().state(tx).unwrap().2.unwrap();
        let mut s = tx.prepare(&format!("EXPLAIN QUERY PLAN {GAP_FENCE_SQL}")).unwrap();
        let details = s
            .query_map(params![SOURCE, reason], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        eprintln!("private gap query plan: {details:?}");
        assert!(details.iter().any(|v| v.contains("ivm_install_source_jobs")));
        assert!(details.iter().any(|v| v.contains("ivm_install_source_roots")));
        assert!(!details.iter().any(|v| v.starts_with("SCAN ")));
        drop(s);
        let mut s = tx.prepare(&format!("EXPLAIN {GAP_FENCE_SQL}")).unwrap();
        let program = s
            .query_map(params![SOURCE, reason], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(program.iter().any(|v| v.1 == "Column" && v.5 & 0xc0 == 0xc0));
        // Closed benchmark layout: one root, one published (not active) job.
        // Reserve16 steps for the two IN probes and final cursor-next branches.
        // Both this bound and actual VM ceilings are asserted; neither may expand
        // after a failure. This is not a bound for same-source metadata fanout.
        let bound = program.len() as u64 + 16;
        eprintln!("private gap bytecode={program:?}; helper VM bound={bound}; prepare bound={}; finish bound={}; whole idle bound={}", 57+bound, 23+bound, 127+2*bound);
        assert!(23 + bound <= 160);
        assert!(57 + bound <= 240);
        assert!(127 + 2 * bound <= 500);
        bound
    })
}

#[test]
fn managed_gap_stable_mutations_and_work_at_zero_1024_and_100000_rows() {
    for n in [0, 1024, 100_000] {
        let f = fenced();
        f.raw(|c| {
            c.execute("INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES('foreign','node',999,'fixture-foreign','1')",[]).unwrap();
            if n>0 { c.execute("WITH RECURSIVE rows(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM rows WHERE n<?1) INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms) SELECT 'foreign/'||n,'foreign','exec/unrelated/'||n,'runtime.observed','node','{}','[]','1' FROM rows",[n]).unwrap(); }
            c.execute("INSERT INTO meta(key,value) VALUES('private-gap-unrelated','value')", []).unwrap();
        });
        let before = snapshot(&f);
        let bound = query_program(&f);
        for repeat in 0..3 {
            for unrelated in [false, true] {
                let (whole, changes) = stable_commit(&f, unrelated);
                eprintln!(
                    "private gap rows={n} repeat={repeat} unrelated={unrelated} whole={whole:?} total_changes={changes}"
                );
                assert_eq!(whole.fullscan_steps, 0);
                assert_eq!(whole.sorts, 0);
                assert_eq!(whole.autoindex_rows, 0);
                assert_eq!(whole.statements, if unrelated { 10 } else { 9 });
                if !unrelated {
                    assert!(whole.vm_steps <= 500 && whole.vm_steps <= 127 + 2 * bound);
                }
                for (phase, work) in f.adapter.work.lock().unwrap().iter() {
                    eprintln!("private gap rows={n} repeat={repeat} phase={phase} work={work:?}");
                    assert_eq!(work.fullscan_steps, 0);
                    assert_eq!(work.sorts, 0);
                    assert_eq!(work.autoindex_rows, 0);
                    match *phase {
                        "prepare" => {
                            assert_eq!(work.statements, 3);
                            assert!(work.vm_steps <= 240 && work.vm_steps <= 57 + bound);
                        }
                        "source-finish" => {
                            assert_eq!(work.statements, 2);
                            assert!(work.vm_steps <= 160 && work.vm_steps <= 23 + bound);
                        }
                        "finalize-marker-read" | "marker-clear" => {
                            assert_eq!(work.statements, 1);
                            assert!(work.vm_steps <= 40);
                        }
                        _ => panic!("unexpected phase {phase}"),
                    }
                }
                assert_eq!(snapshot(&f), before);
                assert_eq!(f.line()["status"], "unknown");
            }
        }
    }
}

#[test]
fn managed_gap_later_capture_reason_changes_once_and_stays_unknown() {
    let f = fenced();
    let before = snapshot(&f);
    f.commit(|tx| rank_change(tx));
    let after = snapshot(&f);
    assert_ne!(after.0.2, before.0.2);
    assert_eq!(
        after.0.2.as_deref(),
        Some("unsupported native rank/repair/checkpoint mutation")
    );
    assert_eq!(after.2[0].4, before.2[0].4 + 1);
    assert_eq!(after.2[0].2, after.0.2);
    assert_eq!(after.2[0].3, before.2[0].3);
    stable_commit(&f, false);
    assert_eq!(snapshot(&f), after);
    assert_eq!(f.line()["status"], "unknown");
}

#[test]
fn managed_gap_reasserts_partial_fences_and_every_source_root_job() {
    for variant in 0..7 {
        let f = fenced();
        let reason = snapshot(&f).0.2.unwrap();
        f.raw(|c| match variant {
            0 => { c.execute("UPDATE ivm_install_sources SET available=1 WHERE name=?1", [SOURCE]).unwrap(); }
            1 => { c.execute("UPDATE ivm_install_roots SET ready=1 WHERE source=?1", [SOURCE]).unwrap(); }
            2 => { c.execute("UPDATE ivm_install_roots SET error=NULL WHERE source=?1", [SOURCE]).unwrap(); }
            3 => { c.execute("UPDATE ivm_install_roots SET error=x'00' WHERE source=?1", [SOURCE]).unwrap(); }
            4 => { c.execute("UPDATE ivm_install_roots SET error=?2 WHERE source=?1", params![SOURCE,"界".repeat(400)]).unwrap(); }
            5 => { c.execute("UPDATE ivm_install_jobs SET phase='scan' WHERE source=?1", [SOURCE]).unwrap(); }
            6 => { c.execute("INSERT INTO ivm_install_roots SELECT 'private/other-root',fingerprint,namespace,source,source_fingerprint,epoch,revision,1,generation,status_revision,'other reason' FROM ivm_install_roots WHERE view=?1", [VIEW]).unwrap(); }
            _ => unreachable!(),
        });
        f.commit(|_| {});
        let after = snapshot(&f);
        assert_eq!(after.1.0, 0);
        for root in &after.2 {
            assert_eq!(root.1, 0);
            assert_eq!(root.2.as_deref(), Some(reason.as_str()));
        }
        assert!(!f.read(|tx| tx.query_row("SELECT EXISTS(SELECT 1 FROM ivm_install_jobs WHERE source=?1 AND phase IN ('scan','catchup'))", [SOURCE], |r| r.get::<_,bool>(0)).unwrap()));
        stable_commit(&f, false);
        assert_eq!(snapshot(&f), after);
        assert_eq!(f.line()["status"], "unknown");
    }
}

#[test]
fn managed_gap_rollback_diagnostic_commit_failure_and_writer_recovery() {
    let f = fenced();
    let before = snapshot(&f);
    for drop_only in [false, true] {
        let mut writer = f.store.connection.write();
        let mut tx = writer.transaction().unwrap();
        let capture = f.source().scoped_state(&tx).unwrap();
        {
            let mut sp = tx.savepoint().unwrap();
            rank_change(&sp);
            sp.rollback().unwrap();
        }
        assert_eq!(f.source().scoped_state(&tx).unwrap(), capture);
        rank_change(&tx);
        f.source().finish(&tx).unwrap();
        if drop_only {
            drop(tx);
        } else {
            tx.rollback().unwrap();
        }
        drop(writer);
        assert_eq!(snapshot(&f), before);
    }
    f.raw(|c| c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE test_gap_parent(id INTEGER PRIMARY KEY); CREATE TABLE test_gap_child(parent INTEGER REFERENCES test_gap_parent DEFERRABLE INITIALLY DEFERRED);").unwrap());
    {
        let mut writer = f.store.connection.write();
        let tx = writer.transaction().unwrap();
        rank_change(&tx);
        tx.execute("INSERT INTO test_gap_child VALUES(404)", [])
            .unwrap();
        assert!(tx.commit().is_err());
    }
    assert_eq!(snapshot(&f), before);
    for fault in [2, 3] {
        f.adapter.fault.store(fault, Ordering::Relaxed);
        let mut writer = f.store.connection.write();
        let tx = writer.transaction().unwrap();
        rank_change(&tx);
        assert!(tx.commit().is_err());
        drop(writer);
        assert_eq!(snapshot(&f), before);
    }
    f.adapter.fault.store(0, Ordering::Relaxed);
    let mut rollback_layout = false;
    for setup in 0..4 {
        let case = fenced();
        let saved = snapshot(&case);
        case.reset_counts();
        let writer = case.store.connection.write();
        let rolled_back = case.enqueue(|tx| {
            rank_change(tx);
            false // WriterJob rolls this caller's savepoint back; outer ACK is still Ok.
        });
        let survivor = case.enqueue(|_| true);
        drop(writer);
        let outcomes = [answer(rolled_back), answer(survivor)];
        let counts = case.counts();
        eprintln!(
            "private gap savepoint setup={setup} layout={counts:?} acknowledgements={outcomes:?}"
        );
        if counts != (1, 1) {
            continue;
        }
        assert!(outcomes.iter().all(Result::is_ok));
        assert_eq!(snapshot(&case), saved);
        assert_eq!(case.line()["status"], "unknown");
        stable_commit(&case, false);
        rollback_layout = true;
        break;
    }
    assert!(
        rollback_layout,
        "four fresh setups did not establish the savepoint batch"
    );
    let mut established = false;
    for setup in 0..4 {
        let case = fenced();
        let saved = snapshot(&case);
        // Capture's revision UPDATE must succeed; only diagnostic persistence fails.
        case.raw(|c| c.execute_batch("CREATE TEMP TRIGGER test_gap_persist_failure BEFORE UPDATE OF available ON main.ivm_install_sources BEGIN SELECT RAISE(ABORT,'private gap persistence failed'); END;").unwrap());
        case.reset_counts();
        let writer = case.store.connection.write();
        let a = case.enqueue(|tx| {
            rank_change(tx);
            true
        });
        let b = case.enqueue(|tx| {
            rank_change(tx);
            true
        });
        drop(writer);
        let outcomes = [answer(a), answer(b)];
        let counts = case.counts();
        eprintln!(
            "private gap failure setup={setup} layout={counts:?} acknowledgements={outcomes:?}"
        );
        case.raw(|c| {
            c.execute_batch("DROP TRIGGER test_gap_persist_failure")
                .unwrap()
        });
        if counts != (1, 1) {
            continue;
        }
        assert!(outcomes.iter().all(Result::is_err));
        assert!(outcomes.iter().all(|r| {
            r.as_ref()
                .unwrap_err()
                .contains("private gap persistence failed")
        }));
        assert_eq!(snapshot(&case), saved);
        case.commit(|tx| rank_change(tx));
        assert_eq!(snapshot(&case).2[0].4, saved.2[0].4 + 1);
        stable_commit(&case, false);
        established = true;
        break;
    }
    assert!(
        established,
        "four fresh layout setups did not establish one batch"
    );
    f.commit(|tx| rank_change(tx));
    assert_eq!(f.line()["status"], "unknown");
    assert_eq!(snapshot(&f).2[0].4, before.2[0].4 + 1);
    stable_commit(&f, false);
}

#[test]
fn managed_gap_multibyte_logical_refusal_is_bounded_and_next_write_commits() {
    let f = Fixture::new(true);
    let wrong_kind = "界".repeat(300);
    f.commit(|tx| {
        tx.execute(
            "UPDATE desired SET member=json_set(member,'$.kind',?1) WHERE subject='exec/one/probe'",
            [&wrong_kind],
        )
        .unwrap();
    });
    let state = snapshot(&f);
    let reason = state.0.2.as_ref().unwrap();
    assert!(reason.starts_with("unknown variant"), "{reason}");
    assert!(reason.contains('界'));
    assert!(reason.len() <= 512 && reason.len() >= 500);
    assert_eq!(gap_reason(reason), *reason);
    stable_commit(&f, false);
    assert_eq!(snapshot(&f), state);
    assert_eq!(f.line()["status"], "unknown");
}

#[test]
fn managed_gap_clean_path_and_unavailable_layout_refuse_without_repair() {
    let f = Fixture::new(true);
    f.commit(|_| {});
    for (phase, work) in f.adapter.work.lock().unwrap().iter() {
        if *phase == "prepare" {
            assert_eq!(work.statements, 2);
            assert!(work.vm_steps <= 120);
        }
        if *phase == "source-finish" {
            assert_eq!(work.statements, 1);
            assert!(work.vm_steps <= 40);
        }
    }
    let f = fenced();
    let before = snapshot(&f);
    f.raw(|c| c.execute_batch("CREATE TEMP TABLE test_gap_saved_source AS SELECT * FROM ivm_install_sources; DELETE FROM ivm_install_sources;").unwrap());
    {
        let mut writer = f.store.connection.write();
        assert!(writer.transaction().is_err());
    }
    assert_eq!(f.line()["status"], "unknown");
    f.raw(|c| c.execute_batch("INSERT INTO ivm_install_sources SELECT * FROM test_gap_saved_source; DROP TABLE test_gap_saved_source;").unwrap());
    assert_eq!(snapshot(&f), before);
    stable_commit(&f, false);
    f.raw(|c| {
        c.execute(
            "UPDATE ivm_install_sources SET available=2 WHERE name=?1",
            [SOURCE],
        )
        .unwrap();
    });
    {
        let mut writer = f.store.connection.write();
        assert!(writer.transaction().is_err());
    }
    assert_eq!(f.line()["status"], "unknown");
    f.raw(|c| {
        c.execute(
            "UPDATE ivm_install_sources SET available=0 WHERE name=?1",
            [SOURCE],
        )
        .unwrap();
    });
    stable_commit(&f, false);
    assert_eq!(snapshot(&f), before);
    let reason = before.0.2.as_ref().unwrap();
    for bad in [None, Some("界".repeat(300))] {
        f.raw(|c| {
            if let Some(bad) = &bad {
                c.execute(
                    "UPDATE test_native_terminal_guard SET gap=?1 WHERE id=1",
                    [bad],
                )
                .unwrap();
            } else {
                c.execute(
                    "UPDATE test_native_terminal_guard SET gap=x'00' WHERE id=1",
                    [],
                )
                .unwrap();
            }
        });
        {
            let mut writer = f.store.connection.write();
            assert!(writer.transaction().is_err());
        }
        assert_eq!(f.line()["status"], "unknown");
        f.raw(|c| {
            c.execute(
                "UPDATE test_native_terminal_guard SET gap=?1 WHERE id=1",
                [reason],
            )
            .unwrap();
        });
        stable_commit(&f, false);
        assert_eq!(snapshot(&f), before);
    }
}
