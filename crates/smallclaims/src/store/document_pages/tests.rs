use super::*;
use crate::sqlite::work::{SqliteWork, SqliteWorkScope};

// Exact pre-specialization statement, with an independent copy of its row decoder.
const OLD: &str = "SELECT d.name, d.hash, b.size, d.created_index,
                     d.hash=(SELECT n.hash FROM documents n
                 WHERE n.name=d.name ORDER BY n.binding_key DESC LIMIT 1)
                     ,d.binding_claim_id, c.accepted_at_unix_ms, c.actor, c.origin
                     FROM documents d JOIN blobs b ON b.hash=d.hash
                     JOIN claims c ON c.id=d.binding_claim_id
                     WHERE (?1 IS NULL OR d.name=?1)
                       AND (?2 OR d.hash=(SELECT n.hash FROM documents n
                 WHERE n.name=d.name ORDER BY n.binding_key DESC LIMIT 1))
                       AND (?3 IS NULL OR substr(d.name,1,length(?3))=?3)
                       AND (?4 IS NULL OR d.name>?4 OR (d.name=?4 AND EXISTS (
                           SELECT 1 FROM documents cursor_document
                           WHERE cursor_document.name=?4 AND cursor_document.created_index=?5
                             AND cursor_document.binding_key>d.binding_key)))
                     ORDER BY d.name, d.binding_key DESC LIMIT ?6";

fn node() -> Store {
    Store::open_memory("documents-page", Arc::new(runtime::Plain)).unwrap()
}

fn old(
    store: &Store,
    name: Option<&str>,
    prefix: Option<&str>,
    history: bool,
    after: Option<(&str, u64)>,
    limit: usize,
) -> Result<Vec<DocumentVersion>> {
    store
        .readers
        .get()
        .prepare(OLD)?
        .query_map(
            params![
                name,
                history,
                prefix,
                after.map(|v| v.0),
                after.map(|v| v.1),
                limit
            ],
            |row| {
                Ok(DocumentVersion {
                    name: row.get(0)?,
                    hash: row.get(1)?,
                    size: row.get(2)?,
                    created_index: row.get(3)?,
                    latest: row.get::<_, i64>(4)? != 0,
                    binding_claim_id: row.get(5)?,
                    created_at_unix_ms: row.get::<_, String>(6)?.parse().map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    owner: row.get::<_, Option<String>>(7)?.or(row.get(8)?),
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn parity(
    store: &Store,
    name: Option<&str>,
    prefix: Option<&str>,
    history: bool,
    after: Option<(&str, u64)>,
    limit: usize,
) {
    let before = old(store, name, prefix, history, after, limit);
    let after_read = read(store, name, prefix, history, after, limit);
    let context =
        format!("name={name:?} prefix={prefix:?} history={history} cursor={after:?} limit={limit}");
    match (before, after_read) {
        (Ok(before), Ok(after)) => assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap(),
            "{context}"
        ),
        (Err(before), Err(after)) => {
            assert_eq!(before.to_string(), after.to_string(), "{context}");
        }
        (before, after) => panic!("{context}: old={before:?}, new={after:?}"),
    }
}

// Normal physical Store schema, graph claim append, canonical projection and fixed writer
// clocks. Distinct origins allow an older canonical binding to arrive after a newer one.
fn bind(
    store: &Store,
    name: &str,
    content: &str,
    time: u128,
    origin: &str,
    actor: Option<&str>,
) -> ClaimRecord {
    store.set_write_clock_at(time).unwrap();
    let hash = hex::encode(Sha256::digest(content.as_bytes()));
    store
        .connection
        .batched(|tx| {
            tx.execute(
                "INSERT OR IGNORE INTO blobs(hash,bytes,size) VALUES(?1,?2,?3)",
                params![hash, content.as_bytes(), content.len()],
            )?;
            let claim = store.runtime.append_claim_tx(
                tx,
                origin,
                name,
                "doc.bound",
                actor,
                &json!({"name":name,"hash":hash,"size":content.len()}),
                &[],
                None,
            )?;
            select_replicated_document(tx, &claim, claim.store_index)?;
            store.runtime.after_projection(tx)?;
            Ok::<_, anyhow::Error>(claim)
        })
        .unwrap()
        .unwrap()
}

#[test]
fn scalar_prefix_bounds_keep_binary_order_and_skip_the_surrogate_gap() {
    for (prefix, end) in [
        ("", None),
        ("\u{10ffff}", None),
        ("\u{10ffff}\u{10ffff}", None),
        ("a\u{10ffff}", Some("b")),
        ("\u{d7ff}", Some("\u{e000}")),
        ("😀", Some("😁")),
        ("doc/é", Some("doc/ê")),
        ("doc/\0", Some("doc/\u{1}")),
    ] {
        assert_eq!(prefix_end(prefix).as_deref(), end);
    }
}

#[test]
fn specialized_pages_match_old_full_metadata_filters_cursors_and_randomized_names() {
    let store = node();
    let mut names = vec![
        "doc/A",
        "doc/a",
        "doc/%_\\",
        "doc/é",
        "doc/中",
        "doc/😀",
        "doc/\u{d7ff}",
        "doc/\u{e000}",
        "doc/\u{10ffff}",
        "doc/x\u{10ffff}",
        "doc/nul\0tail",
        "doc/nul",
        "doc/aa",
        "doc/aaa",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let alphabet = [
        'A',
        'a',
        'Z',
        'z',
        '%',
        '_',
        '\\',
        'é',
        '中',
        '😀',
        '\0',
        '\u{10ffff}',
    ];
    let mut seed = 0x719_aa6_u64;
    for n in 0..48 {
        let mut name = format!("doc/r{n:02}-");
        for _ in 0..4 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            name.push(alphabet[(seed >> 32) as usize % alphabet.len()]);
        }
        names.push(name);
    }
    let mut cursors = vec![
        (String::new(), 0),
        ("doc/missing".into(), 9),
        ("doc/zzzz".into(), 9),
        ("doc/A".into(), u64::MAX),
    ];
    for (n, name) in names.iter().enumerate() {
        let first = bind(
            &store,
            name,
            &format!("first-{n}"),
            2000 + n as u128,
            &format!("late-{n}"),
            (n % 2 == 0).then_some("agent/author"),
        );
        let second = bind(
            &store,
            name,
            &format!("second-{n}"),
            4000 + n as u128,
            &format!("later-{n}"),
            None,
        );
        let earlier = bind(
            &store,
            name,
            &format!("first-{n}"),
            1000 + n as u128,
            &format!("early-{n}"),
            Some("agent/earlier"),
        );
        assert_eq!(earlier.accepted_at_unix_ms, 1000 + n as u128);
        let versions = read(&store, Some(name), None, true, None, 20).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].binding_claim_id, second.id);
        assert!(versions[0].latest);
        assert_eq!(versions[1].binding_claim_id, earlier.id);
        assert!(!versions[1].latest);
        assert_eq!(versions[1].owner.as_deref(), Some("agent/earlier"));
        assert_eq!(versions[1].created_index, earlier.store_index);
        assert_ne!(first.store_index, earlier.store_index);
        cursors.push((name.clone(), second.store_index));
        cursors.push((name.clone(), earlier.store_index));
    }
    let fixed_prefixes = [
        None,
        Some(""),
        Some("doc/"),
        Some("doc/A"),
        Some("doc/a"),
        Some("doc/%_\\"),
        Some("doc/é"),
        Some("doc/中"),
        Some("doc/😀"),
        Some("doc/nul"),
        Some("doc/nul\0"),
        Some("doc/\u{d7ff}"),
        Some("doc/\u{10ffff}"),
        Some("\u{10ffff}"),
        Some("doc/x\u{10ffff}"),
    ];
    for prefix in fixed_prefixes {
        for history in [false, true] {
            for limit in [0, 1, 2, 200] {
                parity(&store, None, prefix, history, None, limit);
                for cursor in &cursors[..8] {
                    parity(
                        &store,
                        None,
                        prefix,
                        history,
                        Some((&cursor.0, cursor.1)),
                        limit,
                    );
                }
            }
        }
    }
    for _ in 0..512 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let name = &names[(seed >> 32) as usize % names.len()];
        let boundaries = name
            .char_indices()
            .map(|(i, _)| i)
            .chain([name.len()])
            .collect::<Vec<_>>();
        let prefix = &name[..boundaries[seed as usize % boundaries.len()]];
        let cursor = &cursors[(seed >> 16) as usize % cursors.len()];
        parity(
            &store,
            (seed & 1 == 0).then_some(name.as_str()),
            Some(prefix),
            seed & 2 == 0,
            (seed & 4 == 0).then_some((cursor.0.as_str(), cursor.1)),
            [0, 1, 3, 1000][seed as usize % 4],
        );
    }
    parity(&store, Some("doc/missing"), None, true, None, 5);
}

#[test]
fn corrupt_document_metadata_stays_an_error_without_a_partial_page() {
    let store = node();
    bind(&store, "doc/a", "good", 1000, "good", None);
    let bad = bind(&store, "doc/b", "bad", 1001, "bad", None);
    store
        .connection
        .write()
        .execute(
            "UPDATE claims SET accepted_at_unix_ms='bad-time' WHERE id=?1",
            [&bad.id],
        )
        .unwrap();
    parity(&store, None, Some("doc/"), true, None, 10);
    assert!(read(&store, None, None, true, None, 10).is_err());
    // Cursor values retain rusqlite's checked u64 binding, not a wrapping cast.
    parity(&store, None, None, true, Some(("doc/a", u64::MAX)), 10);
}

fn plans(
    store: &Store,
    name: Option<&str>,
    prefix: Option<&str>,
    history: bool,
    after: Option<(&str, u64)>,
) -> (Vec<String>, Vec<String>) {
    let lower = match (prefix.filter(|s| !s.is_empty()), after.map(|v| v.0)) {
        (Some(p), Some(c)) => Some(p.max(c)),
        (p, c) => p.or(c),
    };
    let upper = prefix.and_then(prefix_end);
    let sql = query(
        name.is_some(),
        prefix.is_some(),
        lower.is_some(),
        upper.is_some(),
        history,
        after.is_some(),
    );
    let conn = store.readers.get();
    let old = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {OLD}"))
        .unwrap()
        .query_map(
            params![
                name,
                history,
                prefix,
                after.map(|v| v.0),
                after.map(|v| v.1),
                10
            ],
            |row| row.get(3),
        )
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap();
    let new = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(
            params![
                name,
                prefix,
                after.map(|v| v.0),
                after.map(|v| v.1),
                lower,
                upper,
                10
            ],
            |row| row.get(3),
        )
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap();
    println!(
        "document-page OLD={old:?} NEW={new:?} name={name:?} prefix={prefix:?} history={history} cursor={after:?}"
    );
    (old, new)
}

fn assert_plans(store: &Store) {
    let sql: String = store
        .readers
        .get()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='documents'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !sql.to_ascii_uppercase().contains("COLLATE"),
        "core name uses default BINARY: {sql}"
    );
    for name in [None, Some("doc/target/a")] {
        for prefix in [None, Some(""), Some("doc/target/")] {
            for history in [false, true] {
                for after in [None, Some(("doc/target/a", 0))] {
                    let (old, new) = plans(store, name, prefix, history, after);
                    let context = format!("OLD={old:?} NEW={new:?}");
                    let driven = new
                        .iter()
                        .find(|s| s.starts_with("SEARCH d ") || s.starts_with("SCAN d "))
                        .expect(&context);
                    assert!(driven.contains("document_canonical_latest"), "{context}");
                    if name.is_some() || prefix.is_some_and(|s| !s.is_empty()) || after.is_some() {
                        assert!(driven.starts_with("SEARCH d "), "{context}");
                    }
                    for table in ["b", "c"] {
                        assert!(
                            new.iter()
                                .any(|s| s.starts_with(&format!("SEARCH {table} "))),
                            "{context}"
                        );
                    }
                    assert!(
                        !new.iter().any(|s| s.starts_with("SCAN n ")
                            || s.starts_with("SCAN cursor_document ")
                            || s.contains("TEMP B-TREE")),
                        "{context}"
                    );
                }
            }
        }
    }
}

#[test]
fn actual_store_document_plans_seek_before_and_after_population_and_analyze() {
    let store = node();
    bind(&store, "doc/target/a", "target", 1000, "target", None);
    assert_plans(&store);
    for n in 0..256 {
        bind(
            &store,
            &format!("doc/before/{n:04}"),
            "unrelated",
            1001 + n,
            "population",
            None,
        );
    }
    assert_plans(&store);
    store.connection.write().execute_batch("ANALYZE").unwrap();
    assert_plans(&store);
}

fn measured(
    store: &Store,
    use_old: bool,
    name: Option<&str>,
    prefix: Option<&str>,
    after: Option<(&str, u64)>,
) -> SqliteWork {
    let scope = SqliteWorkScope::start();
    let result = if use_old {
        old(store, name, prefix, true, after, 5)
    } else {
        read(store, name, prefix, true, after, 5)
    };
    let work = scope.finish();
    assert!(!result.unwrap().is_empty());
    work
}

#[test]
fn filtered_document_pages_keep_warmed_work_flat_with_unrelated_earlier_names() {
    let store = node();
    for n in 0..4 {
        bind(
            &store,
            &format!("doc/target/{n}"),
            &format!("target-{n}"),
            1000 + n,
            "targets",
            None,
        );
    }
    let shapes = [
        (Some("doc/target/1"), None, None),
        (None, Some("doc/target/"), None),
        (None, None, Some(("doc/target/0", 0))),
        (None, Some("doc/target/"), Some(("doc/target/0", 0))),
    ];
    let before = shapes.map(|(name, prefix, after)| {
        for use_old in [true, false] {
            measured(&store, use_old, name, prefix, after);
        }
        (
            measured(&store, true, name, prefix, after),
            measured(&store, false, name, prefix, after),
        )
    });
    for n in 0..512 {
        bind(
            &store,
            &format!("doc/before/{n:04}"),
            "unrelated",
            2000 + n,
            "population",
            None,
        );
    }
    let after_work = shapes.map(|(name, prefix, after)| {
        for use_old in [true, false] {
            measured(&store, use_old, name, prefix, after);
        }
        (
            measured(&store, true, name, prefix, after),
            measured(&store, false, name, prefix, after),
        )
    });
    println!("document-page before={before:?} after={after_work:?}");
    for ((old_before, new_before), (old_after, new_after)) in before.into_iter().zip(after_work) {
        assert!(new_before.statements > 0 && new_before.vm_steps > 0);
        assert_eq!(new_before.statements, new_after.statements);
        assert_eq!(new_before.vm_steps, new_after.vm_steps);
        assert_eq!(new_before.fullscan_steps, 0);
        assert_eq!(new_after.fullscan_steps, 0);
        assert_eq!(new_before.sorts, 0);
        assert_eq!(new_after.sorts, 0);
        assert!(
            old_after.vm_steps > old_before.vm_steps,
            "old={old_before:?}->{old_after:?}"
        );
    }
}
