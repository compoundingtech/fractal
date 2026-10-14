use super::*;

fn fixture() -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let mut state = state(root.path());
    state.store = Arc::new(roster_followup_store());
    (root, state)
}

fn unrelated(store: &Store, key: &str) {
    store
        .append_claim(&ClaimInput {
            subject: format!("custom/fresh-proof/{key}"),
            kind: "custom.test.marker".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn update_card(store: &Store) -> ClaimRecord {
    store
        .append_claim(&roster_local_observation(
            "runtime.observed",
            json!({"status":"running", "runtime_id":"fresh-new", "incarnation_id":"amber-1"}),
        ))
        .unwrap()
}

#[tokio::test]
async fn unrelated_claims_verify_the_exact_older_response_and_later_wait_target() {
    for history in [false, true] {
        let (_root, state) = fixture();
        let store = &state.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        refresh_agent_roster(store, history).unwrap();
        let query = ClientListQuery {
            history,
            fresh: true,
            ..ClientListQuery::default()
        };
        let (cut, cards, published_at) = store
            .published_agent_roster(store.index().unwrap(), history)
            .unwrap();
        let (Extension(before_snapshot), Json(before)) =
            client_agents_published_page(&state, &query, cut, None)
                .unwrap()
                .unwrap();
        unrelated(store, "request");
        let requested = store.index().unwrap();
        unrelated(store, "wait-start");
        let wanted = store.index().unwrap();
        assert!(cut < requested && requested < wanted);
        let refolds = store.agent_resources_refolded_cards_for_test();
        let writes = smallclaims::touched::writes();
        let waited = wait_for_agent_roster(store, history).await;
        assert_eq!(waited.wanted, Some(wanted));
        assert_eq!(waited.outcome, RosterFreshWaitOutcome::Unchanged);
        assert_eq!(
            roster_fresh_served_proof(store, history, waited, Some((cut, published_at, &cards))),
            RosterUnchangedProof::Verified
        );
        let (Extension(snapshot), Json(page)) =
            client_agents_published_page(&state, &query, requested, Some(waited))
                .unwrap()
                .unwrap();
        assert_eq!(snapshot.store_index, before_snapshot.store_index);
        assert_eq!(snapshot.store_index, cut);
        assert_eq!(snapshot.published_at, before_snapshot.published_at);
        assert_eq!(page.items, before.items);
        assert_eq!(
            store.agent_resources_refolded_cards_for_test(),
            refolds,
            "fresh reads never fold"
        );
        assert_eq!(
            smallclaims::touched::writes(),
            writes,
            "proof/page never write"
        );
    }
}

#[tokio::test]
async fn a_card_write_requires_a_new_publication_with_the_new_card() {
    for history in [false, true] {
        let (_root, state) = fixture();
        let store = &state.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        refresh_agent_roster(store, history).unwrap();
        let old = store
            .published_agent_roster(store.index().unwrap(), history)
            .unwrap();
        let changed = update_card(store);
        let requested = store.index().unwrap();
        assert!(changed.store_index > old.0);
        assert!(
            !store
                .served_agent_roster_unchanged_through(requested, history, old.0, old.2, &old.1)
                .unwrap()
        );
        let reader = Arc::clone(store);
        let waiter = tokio::spawn(async move { wait_for_agent_roster(&reader, history).await });
        store.fresh_agent_roster_wanted().await;
        assert!(
            !waiter.is_finished(),
            "changed cards cannot finish as unchanged"
        );
        refresh_agent_roster(store, history).unwrap();
        let waited = waiter.await.unwrap();
        // A publication racing the unchanged probe can satisfy that probe at cut==wanted.
        assert!(matches!(
            waited.outcome,
            RosterFreshWaitOutcome::Published | RosterFreshWaitOutcome::Unchanged
        ));
        let query = ClientListQuery {
            history,
            fresh: true,
            ..ClientListQuery::default()
        };
        let (Extension(snapshot), Json(page)) =
            client_agents_published_page(&state, &query, requested, Some(waited))
                .unwrap()
                .unwrap();
        assert!(snapshot.store_index >= changed.store_index);
        let card = page
            .items
            .iter()
            .find(|card| card["id"] == "agent/node.amber")
            .unwrap();
        assert_eq!(card["runtime_ids"], json!(["runtime/fresh-new"]));
    }
}

#[tokio::test]
async fn failed_refresh_remains_distinct_from_unchanged_and_keeps_the_old_response() {
    for history in [false, true] {
        let (_root, state) = fixture();
        let store = &state.store;
        let _wake = store.start_agent_roster_refresher().unwrap();
        refresh_agent_roster(store, history).unwrap();
        let query = ClientListQuery {
            history,
            fresh: true,
            ..ClientListQuery::default()
        };
        let old = store
            .published_agent_roster(store.index().unwrap(), history)
            .unwrap();
        let (Extension(before_snapshot), Json(before)) =
            client_agents_published_page(&state, &query, old.0, None)
                .unwrap()
                .unwrap();
        update_card(store);
        assert!(
            store
                .answer_agent_roster_requests::<()>(|| anyhow::bail!("private refresh failure"))
                .is_err()
        );
        let waited = wait_for_agent_roster(store, history).await;
        assert_eq!(waited.outcome, RosterFreshWaitOutcome::RefreshFailed);
        assert_ne!(
            roster_fresh_served_proof(store, history, waited, Some((old.0, old.2, &old.1))),
            RosterUnchangedProof::Verified
        );
        let (Extension(snapshot), Json(page)) =
            client_agents_published_page(&state, &query, store.index().unwrap(), Some(waited))
                .unwrap()
                .unwrap();
        assert_eq!(snapshot.store_index, before_snapshot.store_index);
        assert_eq!(snapshot.published_at, before_snapshot.published_at);
        assert_eq!(
            page.items, before.items,
            "observing failure must not change fallback policy"
        );
    }
}

#[tokio::test]
async fn closed_fixture_publication_channel_is_a_disconnection_not_unchanged() {
    let (_root, state) = fixture();
    let store = &state.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    refresh_agent_roster(store, true).unwrap();
    update_card(store);
    let (sender, receiver) = tokio::sync::watch::channel(0);
    drop(sender);
    let result =
        wait_for_agent_roster_publications(store, true, store.index().unwrap(), receiver).await;
    assert_eq!(result, RosterFreshWaitOutcome::Disconnected);
    // The closed receiver is a private branch control, not a production Store lifetime claim.
}

#[tokio::test(start_paused = true)]
async fn unsatisfied_fresh_wait_times_out_at_the_unchanged_virtual_deadline() {
    let (_root, state) = fixture();
    let store = &state.store;
    let _wake = store.start_agent_roster_refresher().unwrap();
    refresh_agent_roster(store, true).unwrap();
    let old = store
        .published_agent_roster(store.index().unwrap(), true)
        .unwrap();
    update_card(store);
    let started = tokio::time::Instant::now();
    let waited = wait_for_agent_roster(store, true).await;
    assert_eq!(waited.outcome, RosterFreshWaitOutcome::Timeout);
    assert_eq!(started.elapsed(), AGENT_ROSTER_READ_WAIT);
    assert_ne!(
        roster_fresh_served_proof(store, true, waited, Some((old.0, old.2, &old.1))),
        RosterUnchangedProof::Verified
    );
}
