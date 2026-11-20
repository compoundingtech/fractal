//! Check the version 24 readers on a compacted copy of a real member's database: every key grant
//! and revocation, plus a sample of other signed claims, is judged afresh through the compacted
//! facade, and each verdict and served signature must equal what the member had before.
//! Usage: signature_compaction_check --copy COPY ORIGIN [SAMPLE]. It writes, so it refuses to
//! run without `--copy`, which says the path is a copy and not a member's live store.
use smallclaims::Store;
use smallclaims::principal::{KEY_GRANTED, KEY_REVOKED};
use smallclaims::store::runtime::Plain;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    assert_eq!(args.next().as_deref(), Some("--copy"), "usage: signature_compaction_check --copy COPY ORIGIN [SAMPLE]");
    let path = args.next().expect("COPY");
    let origin = args.next().expect("ORIGIN");
    let sample: i64 = args.next().map_or(20_000, |value| value.parse().unwrap());
    let opened = Instant::now();
    let store = Store::open(std::path::Path::new(&path), &origin, Arc::new(Plain)).unwrap();
    println!("open_ms={}", opened.elapsed().as_millis());
    let count = |sql: &str| -> i64 { store.readers.get().query_row(sql, [], |row| row.get(0)).unwrap() };
    let compacted = count(
        "SELECT COUNT(*) FROM claim_signatures s JOIN claim_signature_counts USING(claim_id)
          WHERE NOT EXISTS(SELECT 1 FROM claim_signature_candidates d WHERE d.claim_id=s.claim_id)",
    );
    println!("signed={} compacted={compacted}", count("SELECT COUNT(*) FROM claim_signatures"));

    // Authority claims first, then an even spread of the rest by claim id.
    let chosen: BTreeMap<String, (String, String)> = {
        let connection = store.readers.get();
        let mut statement = connection
            .prepare(&format!(
                "SELECT s.claim_id, v.verdict, s.signature FROM claim_signatures s
                   JOIN claim_verdicts v USING(claim_id) JOIN claims c ON c.id=s.claim_id
                  WHERE c.kind IN ('{KEY_GRANTED}','{KEY_REVOKED}')
                 UNION
                 SELECT s.claim_id, v.verdict, s.signature FROM claim_signatures s
                   JOIN claim_verdicts v USING(claim_id)
                  WHERE s.rowid % MAX(1, (SELECT COUNT(*) FROM claim_signatures) / ?1) = 0"
            ))
            .unwrap();
        statement
            .query_map([sample], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let authority = count(&format!("SELECT COUNT(*) FROM claims WHERE kind IN ('{KEY_GRANTED}','{KEY_REVOKED}')"));
    println!("rejudging={} (authority claims={authority})", chosen.len());
    {
        let connection = store.connection.write();
        let mut insert = connection
            .prepare("INSERT OR IGNORE INTO claim_verdict_fresh(claim_id) VALUES (?1)")
            .unwrap();
        for claim in chosen.keys() {
            insert.execute([claim]).unwrap();
        }
    }
    let judged = Instant::now();
    let mut loans = 0;
    while store.verdicts_pending().unwrap() {
        store.advance_signature_judgment().unwrap();
        loans += 1;
    }
    println!("judge_ms={} loans={loans}", judged.elapsed().as_millis());

    let (mut same, mut changed) = (0, Vec::new());
    for (claim, (verdict, signature)) in &chosen {
        let (now, served): (String, String) = store
            .readers
            .get()
            .query_row(
                "SELECT v.verdict, s.signature FROM claim_verdicts v JOIN claim_signatures s USING(claim_id) WHERE claim_id=?1",
                [claim],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        if &now == verdict && &served == signature {
            same += 1;
        } else {
            changed.push(format!("{claim}: {verdict}->{now}"));
        }
    }
    println!("unchanged={same} changed={}", changed.len());
    for line in changed.iter().take(20) {
        println!("  {line}");
    }
    let still = count(
        "SELECT COUNT(*) FROM claim_signatures s JOIN claim_signature_counts USING(claim_id)
          WHERE NOT EXISTS(SELECT 1 FROM claim_signature_candidates d WHERE d.claim_id=s.claim_id)",
    );
    println!("compacted_after={still} (judging never expands a compacted claim)");
}
