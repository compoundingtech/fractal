#![allow(unused_imports)]
use crate::OwnedSetsAttentionStore;
use crate::OwnedSetsStore;
use st3_seats::store::owned_sets_tests_shared::*;
use super::*;
use st3_seats::store::owned_sets_tests_shared::*;
use st3_seats::store::owned_sets_tests_shared::{bundle, direct, share, signed_fleet, signed_share};


#[test]
fn empty_effect_authority_cost_is_one_current_statement_per_invocation() {
    let store = Store::open_memory("amber").unwrap();
    let source = format!(
        "version 2\n{}",
        (0..64)
            .map(|index| format!(
                "agent \"remote/{index}\" {{ host \"cobalt\"; command \"true\" }}\n"
            ))
            .collect::<String>()
    );
    direct(
        &store,
        &parse_intent(&source, "amber").unwrap(),
        "remote-roster",
    )
    .unwrap();
    let desired = store.desired_subjects().unwrap();
    store.owned_desired_guard(&desired[0]).unwrap();
    for roster in [&desired[..1], &desired[..8], &desired[..64]] {
        let before = smallclaims::sqlite::work::total();
        for declaration in roster {
            let (result, reads) =
                smallclaims::touched::record(|| store.owned_desired_guard(declaration));
            result.unwrap();
            assert!(reads.contains("kind:owned-set.revised"));
            assert!(reads.contains(&declaration.subject));
        }
        let spent = smallclaims::sqlite::work::total() - before;
        assert_eq!(spent.statements, roster.len() as u64, "{spent:?}");
        assert_eq!(spent.fullscan_steps, 0, "{spent:?}");
        assert!(spent.vm_steps <= roster.len() as u64 * 100, "{spent:?}");
        println!("empty-authority roster={} cost={spent:?}", roster.len());
    }
    assert_pass_candidates_match_effect_guards(&store, &desired);
}

#[test]
fn first_receipt_after_empty_proof_fences_the_captured_old_member() {
    let store = Store::open_memory("amber").unwrap();
    direct(&store, &bundle("unowned", false), "legacy").unwrap();
    let captured = store.desired_subjects().unwrap().remove(0);
    store.owned_desired_guard(&captured).unwrap();
    let input = bundle("owned", false);
    let mut opts = options(&store, 10);
    opts.adopt.insert(captured.subject.clone());
    opts.expected_subjects = store
        .owned_set_preview(&input, &opts)
        .unwrap()
        .expected_subjects;
    store
        .apply_owned_set(&input, &opts, "first-receipt", "person/operator")
        .unwrap();
    assert_eq!(
        store.owned_desired_guard(&captured).unwrap_err().code,
        "stale-set-member"
    );
    let fresh = store.desired_subjects().unwrap().remove(0);
    let before = smallclaims::sqlite::work::total();
    store.owned_desired_guard(&fresh).unwrap();
    let spent = smallclaims::sqlite::work::total() - before;
    assert!(spent.statements > 1 && spent.statements <= 20, "{spent:?}");
    println!("nonempty-authority one-member fallback cost={spent:?}");
    assert_pass_candidates_match_effect_guards(&store, &[captured, fresh]);
}

#[test]
fn staged_only_proof_uses_any_claim_and_the_exact_non_null_marker() {
    for marker in [
        json!(""),
        json!(false),
        json!(0),
        json!({}),
        json!([]),
        Value::Null,
    ] {
        let store = Store::open_memory("amber").unwrap();
        direct(&store, &bundle("unowned", false), "legacy").unwrap();
        let captured = store.desired_subjects().unwrap().remove(0);
        store.owned_desired_guard(&captured).unwrap();
        {
            // Reader-contract fixture, not admission of an arbitrary claim kind/marker.
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            let id = "reader-staged-fixture";
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h19ac7b405a95 owner=st3_evidence::store::owned_sets_tests::empty_authority_guard TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                 VALUES(?1,'amber',1,?1,'1')",
                [id],
            )
            .unwrap();
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h1f02c743efc1 owner=st3_evidence::store::owned_sets_tests::empty_authority_guard TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
                 VALUES(?1,?1,?2,'fixture.staged','amber','person/operator',?3,'[]','1')",
                params![id, captured.subject, json!({"owned_set": marker}).to_string()],
            ).unwrap();
            tx.commit().unwrap();
        }
        assert!(store.owned_sets().unwrap().is_empty());
        if marker.is_null() {
            store.owned_desired_guard(&captured).unwrap();
        } else {
            assert_eq!(
                store.owned_desired_guard(&captured).unwrap_err().code,
                "owned-set-pending"
            );
        }
    }
}
