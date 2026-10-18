use super::*;

fn publish_identifier_missions(state: &AppState, ids: &[String]) {
    let intent = crate::graph::parse_internal_intent(
        "version 2\nmission \"identifier-template\" state=\"ready\" { goal \"identifier fixture\"; }",
        "reads-fixture",
    ).unwrap();
    let mut connection = state.store.connection.write();
    let transaction = connection.transaction().unwrap();
    for id in ids {
        let mut mission = intent.missions["identifier-template"].clone();
        mission.id.clone_from(id);
        mission.subject = format!("mission/{id}");
        mission.revision.clear();
        mission.revision = hex::encode(Sha256::digest(serde_json::to_vec(&mission).unwrap()));
        // Historical/replicated claims can contain keys outside today's declaration
        // grammar. Exercise real claim storage and the production mission projector.
        let claim = smallclaims::store::append_claim_record_tx(
            &transaction, &state.store.origin, &mission.subject, "mission.published",
            None, &serde_json::to_value(&mission).unwrap(), &[], None,
        ).unwrap();
        crate::store::select_replicated_mission(&transaction, &claim, claim.store_index)
            .unwrap();
    }
    transaction.commit().unwrap();
}

fn identifier_missions(state: &AppState, only_filtered: bool) {
    let mut ids = vec!["café".to_owned(), "release#2".to_owned(), "z".repeat(250)];
    if !only_filtered {
        ids.extend((0..12).map(|index| format!("visible-{index:02}-{}", "bounded-".repeat(8))));
        ids.push("mission/foo".into());
    }
    publish_identifier_missions(state, &ids);
}

#[tokio::test]
async fn identifier_mission_pages_filter_before_pagination() {
    for only_filtered in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        identifier_missions(&state, only_filtered);
        let admitted = new_client_snapshot(&state);
        let now = client_snapshot_time(&admitted);
        let native = state.store.mission_collection_page_at(false, 0, 100, None, now).unwrap();
        assert_eq!(native.len(), if only_filtered { 3 } else { 16 });
        assert!(native.iter().any(|(id, _)| id == "mission/café"));
        assert!(native.iter().any(|(id, _)| id == "mission/release#2"));
        let ids = native.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let cards = assistant_mission_list_cards_with(&state.store, &ids, now).unwrap();
        for projection in ["missions", "missions-rich"] {
            let expected = mission_rows(&cards, projection.ends_with("-rich")).unwrap();
            assert_eq!(expected.len(), if only_filtered { 0 } else { 13 });
            assert_eq!(expected.iter().filter(|row| row["id"] == "mission/mission/foo").count(),
                usize::from(!only_filtered));
            let context = response_body::Context {
                state: state.clone(), client_request: true, observation_report: false,
                request_id: "request/00000000-0000-0000-0000-000000000000".into(),
                client_snapshot: Some(admitted.clone()), server: None, roster: false,
            };
            let (full_snapshot, full) = project_with_context(&state, &admitted, projection,
                &params(&[("limit", "100")]), MAX_RESPONSE_BYTES, "identifiers:1", &context)
                .await.unwrap();
            assert_eq!(full["items"], json!(expected));
            let full_bytes = serde_json::to_vec(&json!({
                "api_version": CLIENT_API_VERSION, "request_id": context.request_id,
                "snapshot": full_snapshot, "value": full
            })).unwrap().len();
            for (limit, budget) in [(1, MAX_RESPONSE_BYTES), (100, full_bytes.saturating_sub(1))] {
                if only_filtered && limit != 1 { continue; }
                let limit_string = limit.to_string();
                let (mut snapshot, mut page) = project_with_context(&state, &admitted, projection,
                    &params(&[("limit", &limit_string)]), budget, "identifiers:1", &context)
                    .await.unwrap();
                if !only_filtered && limit > 1 {
                    assert!(page["items"].as_array().unwrap().len() < expected.len());
                }
                let mut delivered = Vec::new();
                loop {
                    let items = page["items"].as_array().unwrap();
                    assert!(items.len() <= limit);
                    assert!(!items.is_empty() || only_filtered, "empty visible page: {page}");
                    delivered.extend(items.iter().cloned());
                    assert!(delivered.len() <= expected.len(), "duplicate or looping pages");
                    let Some(cursor) = page["page"]["next_cursor"].as_str() else {
                        assert_eq!(page["page"]["has_more"], false);
                        break;
                    };
                    assert!(!items.is_empty(), "filtered-only source must not continue");
                    let decoded = decode_client_cursor(cursor).unwrap();
                    let (_, id) = decoded.after_key.as_ref().unwrap();
                    assert_eq!(ident(Some(id)).as_deref(), Some(id.as_str()));
                    assert_eq!(items.last().unwrap()["id"], id.as_str());
                    assert_eq!(decoded.offset, delivered.len());
                    (snapshot, page) = project_with_context(&state, &snapshot, projection,
                        &params(&[("cursor", cursor)]), budget, "identifiers:1", &context)
                        .await.unwrap();
                }
                assert_eq!(delivered, expected, "{projection}; limit={limit}");
            }
        }
    }
}

#[tokio::test]
async fn identifier_mission_sql_membership_matches_projection_policy() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let mut ids = (0_u8..=127).map(|byte| format!("ascii-{byte:03}-{}", char::from(byte)))
        .collect::<Vec<_>>();
    ids.extend([
        String::new(), "café".into(), "release#2".into(), "mission/foo".into(),
        "nested/._:-@+=~".into(), "決定".into(), "😀".into(), "\u{80}".into(),
        "é".repeat(124), "é".repeat(125),
        "a".repeat(247), "b".repeat(248), "c".repeat(249),
    ]);
    // Deterministic generated strings exercise combinations as well as individual
    // ASCII characters, without a second allowlist implementing the expected policy.
    let mut seed = 0x1234_5678_u32;
    for index in 0..64 {
        let mut id = format!("generated-{index:02}-");
        for _ in 0..(index * 37 % 260) {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            id.push(char::from(if index % 2 == 0 {
                b'a' + u8::try_from(seed % 26).unwrap()
            } else {
                u8::try_from(seed % 128).unwrap()
            }));
        }
        ids.push(id);
    }
    publish_identifier_missions(&state, &ids);
    let now = client_now_ms();
    let native = state.store.mission_collection_page_at(false, 0, ids.len() + 1, None, now)
        .unwrap();
    assert_eq!(native.len(), ids.len(), "native membership must retain every raw claim ID");
    let expected = native.iter().filter(|(id, _)| ident(Some(id)).is_some())
        .cloned().collect::<Vec<_>>();
    // An empty token itself is invalid, but an empty historical raw key has the
    // nonempty, policy-valid full identifier mission/. Filter the returned full ID.
    assert_eq!(ident(Some("")), None);
    assert!(expected.iter().any(|(id, _)| id == "mission/"));
    for (raw, allowed) in [("a".repeat(247), true), ("b".repeat(248), true), ("c".repeat(249), false)] {
        assert_eq!(expected.iter().any(|(id, _)| id == &format!("mission/{raw}")), allowed,
            "full identifier byte length {}", "mission/".len() + raw.len());
    }
    let selected = state.store.assistant_mission_collection_page_at(
        false, 0, ids.len() + 1, None, now,
    ).unwrap();
    assert_eq!(selected, expected, "actual SQL membership must match API ident");
    for limit in [1, 3, 11] {
        let mut delivered = Vec::new();
        let mut after = None;
        loop {
            let page = state.store.assistant_mission_collection_page_at(
                false, 0, limit, after.as_ref(), now,
            ).unwrap();
            assert!(page.len() <= limit);
            if page.is_empty() { break; }
            let (id, time) = page.last().unwrap();
            assert_eq!(ident(Some(id)).as_deref(), Some(id.as_str()));
            after = Some((*time, id.clone()));
            delivered.extend(page);
            assert!(delivered.len() <= expected.len(), "keyset duplicated an identifier");
        }
        assert_eq!(delivered, expected, "keyset limit={limit}");
        for offset in (0..=expected.len()).step_by(limit) {
            let page = state.store.assistant_mission_collection_page_at(
                false, offset, limit, None, now,
            ).unwrap();
            assert_eq!(page, expected[offset..expected.len().min(offset + limit)],
                "offset={offset}; limit={limit}");
        }
    }
    assert_eq!(state.store.mission_collection_page_at(
        false, 0, ids.len() + 1, None, now,
    ).unwrap(), native, "assistant reads must not rewrite native membership");
}
