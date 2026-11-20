//! A settled verdict keeps no judging row: the fold drops it once the verdict is published, and
//! a paced prune drops the rows earlier releases kept. Verification, serving and invalidation
//! read only candidates, the facade, verdicts and dependency links, which stay.

use std::sync::Arc;

use serde_json::json;
use smallclaims::claim::{ClaimInput, ClaimRecord};
use smallclaims::fleet::MemberKey;
use smallclaims::principal::Verdict;
use smallclaims::store::Store;
use smallclaims::store::runtime::Plain;

const FLEET: &str = "6f1e8a52-3c4d-4b7e-9a10-2d5c8e7f9b31";

fn key() -> Arc<MemberKey> {
    Arc::new(MemberKey::generate().unwrap().0)
}

fn node(name: &str, member: &Arc<MemberKey>, anchor: &Arc<MemberKey>) -> Store {
    let store = Store::open_memory(name, Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(anchor.public()).unwrap();
    store.set_member_key(Some(member.clone())).unwrap();
    store
}

fn append(store: &Store, kind: &str, subject: &str, actor: Option<&str>, fields: serde_json::Value) -> ClaimRecord {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: actor.map(str::to_owned),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

fn admit(by: &Store, name: &str, member: &MemberKey, via: &str) {
    let mut fields =
        json!({"fleet_id": FLEET, "member_key": member.public(), "via": via, "mode": "listening"});
    if via != "anchor" {
        fields["sponsor"] = json!(format!("host/{}", by.origin));
    }
    append(by, "fleet.member-admitted", &format!("host/{name}"), None, fields);
}

fn fleet(names: &[&str]) -> Vec<Store> {
    let anchor = key();
    let a = node("a", &anchor, &anchor);
    admit(&a, "a", &anchor, "anchor");
    let mut all = vec![a];
    for name in names {
        let member = key();
        admit(&all[0], name, &member, "invite");
        all.push(node(name, &member, &anchor));
    }
    for store in &all[1..] {
        sync(&all[0], store);
    }
    all
}

fn sync(from: &Store, to: &Store) {
    let exchange = from
        .export_replication_exchange_answering(
            FLEET,
            &to.replication_inventory().unwrap(),
            &to.replication_signature_requests().unwrap(),
        )
        .unwrap();
    to.receive_replication_exchange(&from.origin, FLEET, &exchange).unwrap();
    to.validate_replication_backlog().unwrap();
    to.project_replication_backlog().unwrap();
}

fn drain(store: &Store) {
    for _ in 0..10_000 {
        store.advance_signature_judgment().unwrap();
        if !store.verdicts_pending().unwrap() {
            return;
        }
    }
    panic!("the paced signature worker must converge");
}

fn note(store: &Store, text: &str) -> ClaimRecord {
    append(store, "example.note", "note/plans", Some("person/ada"), json!({"text": text}))
}

fn count(store: &Store, sql: &str) -> i64 {
    store.readers.get().query_row(sql, [], |row| row.get(0)).unwrap()
}

fn judging_rows(store: &Store) -> i64 {
    count(store, "SELECT COUNT(*) FROM claim_signature_judging")
}

#[test]
fn a_settled_verdict_drops_its_judging_row_and_a_late_revocation_still_invalidates_it() {
    let stores = fleet(&["b", "c"]);
    let (a, b, c) = (&stores[0], &stores[1], &stores[2]);
    let earlier = note(b, "before the revocation is known");
    b.replication_snapshot().unwrap();
    let signature = b.claim_signature(&earlier.id).unwrap().unwrap();
    sync(b, a);
    drain(a);
    assert_eq!(a.claim_verdict(&earlier.id).unwrap(), Verdict::Verified);
    assert_eq!(judging_rows(a), 0, "a settled small fold keeps no resume row");
    // Serving and verification read the facade and candidates, which stay.
    assert_eq!(a.claim_signature(&earlier.id).unwrap().unwrap(), signature);
    assert_eq!(count(a, "SELECT COUNT(*) FROM claim_signature_candidates"), count(a, "SELECT COUNT(*) FROM claim_signatures"));
    assert!(a.recheck_claim_verdicts().unwrap().is_empty());

    // c withdrew the key earlier in canonical order; a learns of it only now. The dependency
    // links requeue the settled claim, which is judged afresh without its old row.
    c.set_write_clock_at(earlier.accepted_at_unix_ms - 60_000).unwrap();
    c.revoke_device_key("person/ada", &signature.key, "example withdrawal").unwrap();
    sync(c, a);
    drain(a);
    assert!(
        matches!(a.claim_verdict(&earlier.id).unwrap(), Verdict::Invalid(_)),
        "{:?}",
        a.claim_verdict(&earlier.id).unwrap()
    );
    assert!(a.recheck_claim_verdicts().unwrap().is_empty());
}

#[test]
fn the_prune_drops_only_settled_rows_an_earlier_release_kept() {
    let stores = fleet(&["b"]);
    let (a, b) = (&stores[0], &stores[1]);
    let claims = (0..40).map(|index| note(b, &format!("note {index}"))).collect::<Vec<_>>();
    sync(b, a);
    drain(a);
    assert_eq!(judging_rows(a), 0);
    // Rows as a release before this one left them: one per settled claim, plus one dirty and
    // one whose candidate count moved on, which a later pass still needs.
    {
        let connection = a.connection.write();
        connection
            .execute_batch(
                "INSERT INTO claim_signature_judging(claim_id,candidate_count,roots_digest,content_digest,last_signature,best_signature,best_verdict,complete,dirty,notified,blocked)
                 SELECT claim_id,1,'','content','',signature,'verified',1,0,1,0 FROM claim_signatures;",
            )
            .unwrap();
        connection
            .execute("UPDATE claim_signature_judging SET dirty=1 WHERE claim_id=?1", [&claims[3].id])
            .unwrap();
        connection
            .execute("UPDATE claim_signature_judging SET candidate_count=2 WHERE claim_id=?1", [&claims[7].id])
            .unwrap();
    }
    let signed = count(a, "SELECT COUNT(*) FROM claim_signatures");
    assert_eq!(judging_rows(a), signed);
    let mut pages = 0;
    let mut deleted = 0;
    loop {
        let page = a.prune_settled_signature_judgments().unwrap();
        deleted += page.deleted;
        pages += 1;
        if page.finished {
            break;
        }
        assert!(pages < 100, "the prune must finish");
    }
    assert_eq!(deleted as i64, signed - 2);
    let kept: Vec<String> = a
        .readers
        .get()
        .prepare("SELECT claim_id FROM claim_signature_judging ORDER BY claim_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut expected = vec![claims[3].id.clone(), claims[7].id.clone()];
    expected.sort();
    assert_eq!(kept, expected);
    // Finished is remembered: later pages read nothing.
    let again = a.prune_settled_signature_judgments().unwrap();
    assert!(again.finished && again.examined == 0);
    // A release rolled back to keeps a row again; the next upgrade's prune drops it.
    a.connection
        .write()
        .execute(
            "INSERT INTO claim_signature_judging(claim_id,candidate_count,roots_digest,content_digest,last_signature,complete,dirty,notified,blocked)
             VALUES (?1,1,'','content','',1,0,1,0)",
            [&claims[11].id],
        )
        .unwrap();
    let restarted = a.prune_settled_signature_judgments().unwrap();
    assert_eq!(restarted.deleted, 1);
    assert!(restarted.finished);
    for (index, claim) in claims.iter().enumerate() {
        let verdict = a.claim_verdict(&claim.id).unwrap();
        if index == 3 {
            // A dirty fold is pending until judged again, as before.
            assert!(matches!(&verdict, Verdict::Held(reason) if reason.starts_with("signature-judgment-incomplete:")));
        } else {
            assert_eq!(verdict, Verdict::Verified);
        }
    }
}
