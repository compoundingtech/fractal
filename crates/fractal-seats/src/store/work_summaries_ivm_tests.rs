#![allow(unused_imports)]
use crate::MissionsStore;
use super::*;
use smallclaims::ivm::{SourceCut, events};
use st3_missions::store::tests_shared::{exchange_from, receive_and_project};
use st3_missions::store::work_summaries_ivm::tests_shared_seats::*;
use st3_missions::store::work_summaries_ivm::tests_shared_seats::{ResetClock, advance, append, append_at, capture, fixture, freeze, parity, register};


fn mission(store: &Store) -> MissionRunView {
    let intent=crate::parse_intent("version 2\nmission \"orchard\" state=\"ready\" { goal \"Prepare samples.\"; step \"build\" { goal \"Build a sample.\"; }; step \"review\" { goal \"Review a sample.\"; } }\n",store.origin()).unwrap();
    store.apply_internal(&intent, "publish").unwrap();
    store
        .create_mission_run(&MissionRunRequest {
            mission: "orchard".into(),
            revision: None,
            workspace: "/example/project".into(),
            requester: Some("person/avery".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "start".into(),
        })
        .unwrap()
}

#[test]
fn real_store_summary_parity_attempts_and_batched_reads() {
    let _clock = clock_snapshot();
    let (store, views) = fixture();
    let run = mission(&store);
    let subject = &run.steps[0].subject;
    for (attempt, kind, summary) in [
        (1, "work.progress", "first"),
        (1, "work.progress", "  latest progress \n"),
        (1, "work.submitted", " complete "),
        (2, "work.progress", "next attempt"),
        (1, "work.progress", " \t "),
    ] {
        append(&store, subject, kind, attempt, summary);
    }
    let mut steps = run.steps.clone();
    let mut second = steps[0].clone();
    second.attempt = 2;
    steps.push(second);
    parity(&store, &views, &steps, now_ms() + 10_000);
    let connection = store.readers.get();
    let value = row(&connection, &views, subject, 1, now_ms() + 10_000).unwrap();
    assert_eq!(value.progress_summary.as_deref(), Some("latest progress"));
    assert_eq!(value.completion_summary.as_deref(), Some("complete"));
    assert!(
        rows(
            &connection,
            &views,
            &vec![(subject.clone(), 1); 502],
            now_ms() + 10_000
        )
        .is_err()
    );
    assert_eq!(
        row(&connection, &views, subject, 999, now_ms() + 10_000).unwrap(),
        Summaries::default()
    );
}

#[test]
fn future_claims_require_bounded_captured_time_pages() {
    let at = 1_800_000_000_000;
    let _reset = freeze(at);
    let (store, views) = fixture();
    let run = mission(&store);
    for step in &run.steps {
        append_at(&store, &step.subject, at, "current");
    }
    parity(&store, &views, &run.steps, at);
    for step in &run.steps {
        append_at(&store, &step.subject, at + 100, "future");
    }
    capture(&store, &views);
    assert_eq!(
        next_deadline(&store.readers.get(), &views, at).unwrap(),
        Some(at + 100)
    );
    parity(&store, &views, &run.steps, at + 99);
    assert!(
        row(
            &store.readers.get(),
            &views,
            &run.steps[0].subject,
            1,
            at + 100
        )
        .is_err()
    );
    assert_eq!(advance(&store, &views, at + 100, 1), 1);
    assert!(
        row(
            &store.readers.get(),
            &views,
            &run.steps[0].subject,
            1,
            at + 100
        )
        .is_err(),
        "partial time coverage never ready"
    );
    assert_eq!(advance(&store, &views, at + 100, 1), 1);
    parity(&store, &views, &run.steps, at + 100);
    assert_eq!(
        row(
            &store.readers.get(),
            &views,
            &run.steps[0].subject,
            1,
            at + 100
        )
        .unwrap()
        .progress_summary
        .as_deref(),
        Some("future")
    );
    assert!(
        row(
            &store.readers.get(),
            &views,
            &run.steps[0].subject,
            1,
            at + 99
        )
        .is_err(),
        "no historical fallback"
    );
    assert_eq!(
        next_deadline(&store.readers.get(), &views, at + 100).unwrap(),
        None
    );
}

#[test]
fn signed_replication_permutations_and_duplicates_match_canonical_reader() {
    use crate::fleet::{MemberKey, envelope_signature_message, verify_signature};
    pub const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let at = 1_800_000_000_000;
    let _reset = freeze(at);
    let source = Store::open_memory("cedar").unwrap();
    source.bind_fleet(FLEET).unwrap();
    let member = Arc::new(MemberKey::generate().unwrap().0);
    source.set_member_key(Some(member.clone())).unwrap();
    let run = mission(&source);
    for step in &run.steps {
        for (kind, text) in [
            ("work.progress", "first"),
            ("work.progress", "latest"),
            ("work.submitted", "complete"),
        ] {
            append(&source, &step.subject, kind, 1, text);
        }
    }
    let exchange = exchange_from(&source, &ReplicationInventory::default());
    assert!(exchange.envelopes.len() > 3);
    for envelope in &exchange.envelopes {
        assert_eq!(envelope.member_key.as_deref(), Some(member.public()));
        assert!(verify_signature(
            member.public(),
            &envelope_signature_message(FLEET, "cedar", envelope.sequence, &envelope.hash),
            envelope.signature.as_deref().unwrap()
        ));
    }
    for reverse in [false, true] {
        let (target, views) = fixture();
        let mut permuted = exchange.clone();
        if reverse {
            permuted.envelopes.reverse();
        }
        receive_and_project(&target, "cedar", &permuted);
        parity(&target, &views, &run.steps, at);
        receive_and_project(&target, "cedar", &permuted);
        parity(&target, &views, &run.steps, at);
        assert_eq!(
            row(&target.readers.get(), &views, &run.steps[0].subject, 1, at)
                .unwrap()
                .completion_summary
                .as_deref(),
            Some("complete")
        );
    }
    // Split: reverse order in batches of one and two, each batch's predecessor delivered again
    // with it, with parity against the canonical reader after every batch.
    for width in [1, 2] {
        let (target, views) = fixture();
        let order = exchange.envelopes.iter().rev().cloned().collect::<Vec<_>>();
        let mut previous = Vec::new();
        for chunk in order.chunks(width) {
            let mut batch = chunk.to_vec();
            batch.extend(previous.iter().cloned());
            receive_and_project(&target, "cedar", &st3_store::store::tests_shared_seats::exchange_of("cedar", batch));
            parity(&target, &views, &run.steps, at);
            previous = chunk.to_vec();
        }
        assert_eq!(
            row(&target.readers.get(), &views, &run.steps[0].subject, 1, at)
                .unwrap()
                .completion_summary
                .as_deref(),
            Some("complete")
        );
    }
}

#[test]
fn reopen_preserves_rows_pending_appends_and_source_edits_fence() {
    let _clock = clock_snapshot();
    let at = now_ms() + 10_000;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sample.sqlite3");
    let store = Store::open(&path, "birch").unwrap();
    let views = register(&store);
    let run = mission(&store);
    let subject = &run.steps[0].subject;
    append(&store, subject, "work.progress", 1, "before reopen");
    parity(&store, &views, &run.steps, at);
    let before = row(&store.readers.get(), &views, subject, 1, at).unwrap();
    let token = views.readiness(&store.readers.get(), VIEW, 1).unwrap();
    drop(store);
    let reopened = Store::open(&path, "birch").unwrap();
    assert_eq!(
        row(&reopened.readers.get(), &views, subject, 1, at).unwrap(),
        before
    );
    assert_eq!(
        views.readiness(&reopened.readers.get(), VIEW, 1).unwrap(),
        token
    );
    let claim = append(&reopened, subject, "work.progress", 1, "after reopen");
    assert!(
        row(&reopened.readers.get(), &views, subject, 1, at).is_err(),
        "uncaptured write stays pending"
    );
    parity(&reopened, &views, &run.steps, at);
    reopened
        .connection
        .write()
        .execute(/* sqlite-large-value: allow-for-now lv-hf03433679f75 owner=st3_seats::store::work_summaries_ivm_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body='{}' WHERE id=?1", [claim.id])
        .unwrap();
    assert_eq!(
        views.readiness(&reopened.readers.get(), VIEW, 1).unwrap(),
        Readiness::Fenced
    );
    assert!(row(&reopened.readers.get(), &views, subject, 1, at).is_err());
    let mut writer = reopened.connection.write();
    let tx = writer.transaction().unwrap();
    assert!(clock_page(&tx, &views, at, 1).is_err());
}
