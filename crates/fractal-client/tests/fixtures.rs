#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

use fractal_client::*;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(test_env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/st3/client-v0/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn decode<T: serde::de::DeserializeOwned>(name: &str) -> T {
    serde_json::from_slice(&fixture(name)).unwrap_or_else(|error| panic!("decode {name}: {error}"))
}
#[test]
fn availability_fixtures_decode_coarse_and_full_views_and_encode_a_status_clear_as_null() {
    let read: Envelope<PersonAvailabilities> = decode("person-availability.json");
    let [ada, avery, blair] = &read.value.people[..] else { panic!("three people") };
    assert_eq!((ada.status.as_str(), ada.status_source.as_str(), ada.interruptions.as_str()), ("away", "override", "urgent"));
    assert!(ada.presence.is_none() && ada.presence_observed_at.is_none());
    assert_eq!((avery.deadline_clock, avery.deadline_clock_until.as_deref()), (AvailabilityDeadlineClock::HoldRoutine, Some("2026-11-12T17:00:00Z")));
    assert_eq!((blair.status, blair.status_source), (AvailabilityStatus::Available, AvailabilityStatusSource::Unreported));
    let own: Envelope<PersonAvailability> = decode("availability-set-result.json");
    assert_eq!(own.value.presence, Some(AvailabilityPresence::Present));
    let set: AvailabilitySet = decode("availability-set.json");
    assert_eq!(serde_json::to_value(&set).unwrap(), serde_json::from_slice::<serde_json::Value>(&fixture("availability-set.json")).unwrap());
    let clear = AvailabilitySet { status: Some(AvailabilityStatusChange { value: None, until: None }), ..AvailabilitySet::default() };
    assert_eq!(serde_json::to_value(&clear).unwrap(), serde_json::json!({"status": {"value": null}}));
    let report: PresenceReport = decode("presence-report.json");
    assert_eq!((report.state.as_str(), report.last_input_at.as_deref(), report.observed_at.as_str()), ("active", Some("2026-11-12T09:57:30Z"), "2026-11-12T09:58:00Z"));
    let reported: Envelope<PresenceReportResult> = decode("presence-report-result.json");
    assert_eq!((reported.value.availability.presence, reported.value.report_again_at.as_deref()), (Some(AvailabilityPresence::Present), Some("2026-11-12T10:07:30Z")));
    let bare = PresenceReport { state: "locked".into(), last_input_at: None, observed_at: "2026-11-12T09:58:00Z".into() };
    assert_eq!(serde_json::to_value(&bare).unwrap(), serde_json::json!({"state": "locked", "observed_at": "2026-11-12T09:58:00Z"}));
}

#[test]
fn availability_words_a_newer_daemon_adds_read_as_unknown() {
    let mut row = serde_json::from_slice::<serde_json::Value>(&fixture("person-availability.json")).unwrap()["value"]["people"][0].clone();
    for (field, word) in [("status", "busy"), ("status_source", "calendar"), ("presence", "in-call"), ("interruptions", "priority"), ("deadline_clock", "hold-weekend")] {
        row[field] = serde_json::json!(word);
    }
    let future: PersonAvailability = serde_json::from_value(row).unwrap();
    assert_eq!((future.status, future.status_source), (AvailabilityStatus::Unknown, AvailabilityStatusSource::Unknown));
    assert_eq!(future.presence, Some(AvailabilityPresence::Unknown));
    assert_eq!((future.interruptions, future.deadline_clock), (AvailabilityInterruptions::Unknown, AvailabilityDeadlineClock::Unknown));
    assert_eq!((AvailabilityDeadlineClock::HoldAll.as_str(), AvailabilityStatus::Unknown.to_string()), ("hold-all", "unknown".to_owned()));
}

#[test]
fn availability_history_fixture_round_trips_and_every_axis_covers_its_bucket() {
    let read: Envelope<AvailabilityHistory> = decode("availability-history.json");
    let wire = serde_json::from_slice::<serde_json::Value>(&fixture("availability-history.json")).unwrap();
    assert_eq!(serde_json::to_value(&read.value).unwrap(), wire["value"]);
    let [first, last] = &read.value.buckets[..] else { panic!("two buckets") };
    assert_eq!((first.status_source.r#override, last.status_source.r#override), (0, 1_200_000));
    assert_eq!((last.covered_ms, last.deadline_clock.hold_routine), (1_800_000, 1_200_000));
    for bucket in &read.value.buckets {
        let presence = bucket.presence.unwrap();
        for total in [
            bucket.status.available + bucket.status.away,
            bucket.status_source.r#override + bucket.status_source.presence + bucket.status_source.unreported,
            presence.present + presence.idle + presence.locked + presence.unreported,
            bucket.interruptions.all + bucket.interruptions.urgent + bucket.interruptions.none,
            bucket.deadline_clock.run + bucket.deadline_clock.hold_routine + bucket.deadline_clock.hold_all,
        ] {
            assert_eq!(total, bucket.covered_ms, "{bucket:?}");
        }
    }
}

#[test]
fn directive_note_set_fixtures_decode_and_encode_a_clear_as_null() {
    let set: DirectiveNoteSet = decode("directive-note-set.json");
    assert_eq!(serde_json::to_value(&set).unwrap(), serde_json::from_slice::<serde_json::Value>(&fixture("directive-note-set.json")).unwrap());
    let written: Envelope<DirectiveNoteSetResult> = decode("directive-note-set-result.json");
    assert_eq!(written.value.note.as_ref().map(|note| note.text.as_str()), set.text.as_deref());
    let clear = DirectiveNoteSet { text: None, expires_at: None };
    assert_eq!(serde_json::to_value(&clear).unwrap(), serde_json::json!({"text": null}));
}


#[test]
fn response_evidence_schema_and_typed_models_preserve_observations() {
    let bodies: Vec<serde_json::Value> = decode("response-evidence.json");
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/st3/client-v0/schemas/client-v0.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&serde_json::json!({
        "$defs":schema["$defs"], "$ref":"#/$defs/TimelineUsageBody"
    })).unwrap();
    for raw in bodies {
        assert!(validator.is_valid(&raw));
        let body: TimelineUsageBody = serde_json::from_value(raw.clone()).unwrap();
        let encoded = serde_json::to_value(&body).unwrap();
        for (key, value) in raw.as_object().unwrap() {
            assert_eq!(&encoded[key], value, "{key}");
        }
        assert_eq!(body.response_id.as_deref(), raw["response_id"].as_str());
    }
    let unknown = serde_json::json!({
        "semantics":"response", "driver":"omp", "attribution":{"agent_id":"agent/example"}
    });
    assert!(validator.is_valid(&unknown));
    let body: TimelineUsageBody = serde_json::from_value(unknown.clone()).unwrap();
    assert!(body.total_tokens.is_none());
    assert!(body.cost.is_none());
    assert!(body.provider_quota.is_none());
    assert!(body.reasoning_tokens.is_none());
    assert!(body.cache_write_1h_tokens.is_none());
    let mut invalid = unknown;
    invalid["provider_quota"] = serde_json::json!({"unit":"tokens","quantity":1});
    assert!(!validator.is_valid(&invalid));
    invalid["provider_quota"] = serde_json::json!({"unit":"premium_requests","quantity":-1});
    assert!(!validator.is_valid(&invalid));
}

#[test]
fn sender_outcomes_keep_expired_delivered_and_read_as_open_string_states() {
    let cases: serde_json::Value = decode("message-sender-outcomes.json");
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/st3/client-v0/schemas/client-v0.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&serde_json::json!({
        "$defs":schema["$defs"], "$ref":"#/$defs/Page"
    })).unwrap();
    assert!(validator.is_valid(&cases));
    for raw in cases["items"].as_array().unwrap() {
        let message: Message = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(message.state, raw["state"].as_str().unwrap());
        let encoded = serde_json::to_value(message).unwrap();
        assert_eq!(encoded["state"], raw["state"]);
    }
}

#[test]
fn person_option_notes_round_trip_and_empty_omission() {
    let legacy = serde_json::json!({"id": "land", "text": "Ship it."});
    let input: PersonAnswerInput = serde_json::from_value(legacy.clone()).unwrap();
    assert!(input.option_notes.is_empty());
    assert_eq!(serde_json::to_value(&input).unwrap(), legacy);
    let notes = serde_json::json!([{"id": "hold", "text": "Keep as a fallback."}]);
    let wire = serde_json::json!({"id": "land", "text": "Ship it.", "option_notes": notes});
    let input: PersonAnswerInput = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(input.option_notes[0].id, "hold");
    assert_eq!(serde_json::to_value(input).unwrap(), wire);
    let stored = serde_json::json!({"type": "decision", "outcome": "accept",
        "id": "land", "text": "Ship it.", "option_notes": notes});
    let answer: PersonAnswer = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(serde_json::to_value(answer).unwrap(), stored);
    let mut empty = stored;
    empty["option_notes"] = serde_json::json!([]);
    let answer: PersonAnswer = serde_json::from_value(empty.clone()).unwrap();
    empty.as_object_mut().unwrap().remove("option_notes");
    assert_eq!(serde_json::to_value(answer).unwrap(), empty);
}

#[test]
fn person_option_notes_schema_is_nested_and_strict() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/st3/client-v0/schemas/client-v0.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&serde_json::json!({
        "$defs": schema["$defs"], "$ref": "#/$defs/PersonAnswerInput"
    })).unwrap();
    assert!(validator.is_valid(&serde_json::json!({"id": "land",
        "option_notes": [{"id": "hold", "text": "Fallback."}]})));
    for note in [
        serde_json::json!({"id": "hold"}),
        serde_json::json!({"id": "hold", "text": ""}),
        serde_json::json!({"id": "hold", "text": "x".repeat(8001)}),
        serde_json::json!({"id": "hold", "text": "Fallback.", "label": "extra"}),
    ] {
        assert!(!validator.is_valid(&serde_json::json!({"id": "land", "option_notes": [note]})));
    }
}

#[test]
fn related_run_rows_decode_and_validate_server_semantics() {
    let value: serde_json::Value = serde_json::from_slice(&fixture("agent-related-runs.json")).unwrap();
    let decoded: AgentRelatedRuns = serde_json::from_value(value.clone()).unwrap();
    assert!(decoded.rows[0].waiting_on_you);
    assert!(decoded.rows[0].eta.is_none());
    assert_eq!(decoded.rows[0].current_step.as_ref().unwrap().who.as_deref(), Some("person/reviewer"));
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/st3/client-v0/schemas/client-v0.schema.json")).unwrap();
    let schema = serde_json::json!({"$defs":schema["$defs"],"$ref":"#/$defs/AgentRelatedRuns"});
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&value));
    let mut invalid = value;
    invalid["rows"][0]["urgency_rank"] = serde_json::json!(5);
    assert!(!validator.is_valid(&invalid));
}

#[test]
fn revisioned_agents_frames_keep_typed_publication_and_snapshot_provenance() {
    let CollectionEvent::AgentsSnapshotChunk {
        id, collection, snapshot, publication, chunk_index, chunk_count, items, order, has_more,
    } = decode("agents-publication-chunk.json") else { panic!("expected typed chunk"); };
    assert_eq!(id, "roster");
    assert_eq!(collection, "agents");
    assert_eq!(snapshot.published_at.as_deref(), Some("2026-10-10T12:00:00.000Z"));
    assert_eq!(publication.revision, 42);
    assert_eq!(publication.status_watermark.store_index, snapshot.store_index);
    assert_eq!(publication.status_watermark.local_frontier, 17);
    assert_eq!(publication.materialized_at_ms, 1791633600000);
    assert_eq!((chunk_index, chunk_count), (0, 2));
    assert_eq!(items[0].header.id, "agent/a");
    assert_eq!(order, ["agent/a"]);
    assert!(!has_more);
    let CollectionEvent::AgentsChanges {
        id, collection, snapshot, publication, base_revision, upserts, removes, order, has_more,
    } = decode("agents-publication-changes.json") else { panic!("expected typed delta"); };
    assert_eq!(id, "roster");
    assert_eq!(collection, "agents");
    assert_eq!(snapshot.published_at.as_deref(), Some("2026-10-10T12:00:01.000Z"));
    assert_eq!(publication.revision, 43);
    assert_eq!(base_revision, 42);
    assert!(upserts.is_empty());
    assert_eq!(removes, ["agent/b"]);
    assert_eq!(order, ["agent/a"]);
    assert!(!has_more);
}

#[test]
fn collection_errors_keep_distinct_permanent_code_and_optional_legacy_metadata() {
    let CollectionEvent::Error { id, collection, code, retryable, .. } =
        decode("agents-publication-row-too-large.json") else { panic!("expected error"); };
    assert_eq!(id, "roster");
    assert_eq!(collection.as_deref(), Some("agents"));
    assert_eq!(code, Some(ErrorCode::AgentsPublicationRowTooLarge));
    assert_eq!(retryable, Some(false));
    let legacy: CollectionEvent = serde_json::from_value(serde_json::json!({
        "kind":"resync", "id":"roster"
    })).unwrap();
    assert!(matches!(legacy, CollectionEvent::Resync { collection: None, retryable: None, .. }));
    let notice: CollectionEvent = serde_json::from_value(serde_json::json!({
        "kind":"resync", "id":"talk", "collection":"conversation", "retryable":true, "code":"remote-unavailable"
    })).unwrap();
    assert!(matches!(notice, CollectionEvent::Resync { collection: Some(collection), retryable: Some(true), .. } if collection == "conversation"));
    let error: CollectionEvent = serde_json::from_value(serde_json::json!({
        "kind":"error", "message":"legacy refusal"
    })).unwrap();
    assert!(matches!(error, CollectionEvent::Error { collection: None, retryable: None, .. }));
}

#[test]
fn collection_publication_grammar_cannot_leak_into_legacy_events() {
    let chunk: serde_json::Value = serde_json::from_slice(&fixture("agents-publication-chunk.json")).unwrap();
    let mut legacy = chunk.clone();
    for key in ["publication", "chunk_index", "chunk_count"] { legacy.as_object_mut().unwrap().remove(key); }
    let event: CollectionEvent = serde_json::from_value(legacy.clone()).unwrap();
    assert!(matches!(event, CollectionEvent::Snapshot { collection, snapshot, .. } if collection == "agents" && snapshot.published_at.is_some()));
    for key in ["chunk_index", "chunk_count", "base_revision"] {
        let mut invalid = legacy.clone();
        invalid[key] = serde_json::json!(1);
        assert!(serde_json::from_value::<CollectionEvent>(invalid).is_err(), "{key}");
    }
    for key in ["publication", "chunk_index", "chunk_count"] {
        let mut invalid = chunk.clone();
        invalid.as_object_mut().unwrap().remove(key);
        assert!(serde_json::from_value::<CollectionEvent>(invalid).is_err(), "{key}");
    }
    for (key, value) in [
        ("chunk_count", serde_json::json!(0)),
        ("chunk_index", serde_json::json!(2)),
        ("collection", serde_json::json!("work")),
        ("has_more", serde_json::json!(true)),
    ] {
        let mut invalid = chunk.clone();
        invalid[key] = value;
        assert!(serde_json::from_value::<CollectionEvent>(invalid).is_err(), "{key}");
    }
    let mut unpublished = chunk.clone();
    unpublished["snapshot"].as_object_mut().unwrap().remove("published_at");
    assert!(serde_json::from_value::<CollectionEvent>(unpublished).is_err());
    for base_revision in [0, 9_007_199_254_740_992_u64] {
        let mut invalid: serde_json::Value = serde_json::from_slice(&fixture("agents-publication-changes.json")).unwrap();
        invalid["base_revision"] = serde_json::json!(base_revision);
        assert!(serde_json::from_value::<CollectionEvent>(invalid).is_err());
    }
    let mut delta: serde_json::Value = serde_json::from_slice(&fixture("agents-publication-changes.json")).unwrap();
    delta["chunk_index"] = serde_json::json!(0);
    assert!(serde_json::from_value::<CollectionEvent>(delta).is_err());
    let legacy_snapshot: Snapshot = serde_json::from_value(serde_json::json!({
        "id":"snapshot/host-a/1/fence", "host_id":"host/host-a", "store_index":1,
        "projection_version":"client-projection.v0", "created_at":"2026-10-10T12:00:00.000Z"
    })).unwrap();
    assert!(legacy_snapshot.published_at.is_none());
    assert!(serde_json::to_value(legacy_snapshot).unwrap().get("published_at").is_none());
}

#[test]
fn collection_frames_ignore_unknown_extension_keys() {
    for (name, publication) in [
        ("agents-publication-chunk.json", true),
        ("agents-publication-changes.json", true),
        ("agents-publication-chunk.json", false),
    ] {
        let mut frame: serde_json::Value = serde_json::from_slice(&fixture(name)).unwrap();
        if !publication {
            for key in ["publication", "chunk_index", "chunk_count"] {
                frame.as_object_mut().unwrap().remove(key);
            }
        }
        frame["future_extension"] = serde_json::json!({"value": true});
        let event: CollectionEvent = serde_json::from_value(frame).unwrap();
        assert!(matches!((publication, event),
            (true, CollectionEvent::AgentsSnapshotChunk { .. } | CollectionEvent::AgentsChanges { .. })
            | (false, CollectionEvent::Snapshot { .. })));
    }
}

#[test]
fn terminal_conversation_and_legacy_changes_keep_collection_names() {
    let envelope: serde_json::Value = serde_json::from_slice(&fixture("terminal-screen.json")).unwrap();
    let screen: CollectionEvent = serde_json::from_value(serde_json::json!({
        "kind":"screen", "id":"term", "collection":"terminal",
        "snapshot":envelope["snapshot"], "value":envelope["value"],
    })).unwrap();
    assert!(matches!(screen, CollectionEvent::Screen { collection, .. } if collection == "terminal"));
    let conversation: CollectionEvent = serde_json::from_value(serde_json::json!({
        "kind":"conversation", "id":"talk", "collection":"conversation",
        "session_id":"session/a", "replace":false, "items":[],
    })).unwrap();
    assert!(matches!(conversation, CollectionEvent::Conversation { collection, has_more: None, .. } if collection == "conversation"));
    let changes: CollectionEvent = serde_json::from_value(serde_json::json!({
        "kind":"changes", "id":"window", "collection":"agents", "snapshot":envelope["snapshot"],
        "upserts":[], "removes":[], "order":[], "has_more":false,
    })).unwrap();
    assert!(matches!(changes, CollectionEvent::Changes { collection, .. } if collection == "agents"));
}

#[test]
fn shared_error_sentences_preserve_machine_values_and_match_human_clients() {
    #[derive(serde::Deserialize)]
    struct Case {
        kind: String,
        code: Option<String>,
        message: String,
        plain: String,
        human: String,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../fixtures/clients/plain-errors-v1.json"
    )).unwrap();
    for case in cases {
        let code: Option<ErrorCode> = case.code.as_ref()
            .map(|code| serde_json::from_value(serde_json::json!(code)).unwrap());
        if case.kind == "api" {
            assert_eq!(plain_message(code.as_ref(), &case.message), case.plain);
            assert_eq!(human_message(code.as_ref(), &case.message), case.human);
        }
        let error = if case.kind == "transport" {
            ClientError::Transport(case.message.clone())
        } else if let Some(code) = code {
            let envelope: ErrorEnvelope = serde_json::from_value(serde_json::json!({
                "api_version": "st3.client.v0", "error_version": "st3.client.error.v0",
                "request_id": "request/parity", "code": case.code,
                "message": case.message, "retryable": false,
                "details": if case.kind == "applied-unknown" {
                    serde_json::json!({"applied": "unknown"})
                } else { serde_json::json!({}) },
            })).unwrap();
            ClientError::Api(code, case.message.clone(), Box::new(envelope))
        } else {
            continue;
        };
        let before = match &error {
            ClientError::Api(_, _, envelope) => Some(serde_json::to_value(envelope).unwrap()),
            _ => None,
        };
        assert_eq!(error.plain(), case.plain, "{} {:?}", case.kind, case.code);
        assert_eq!(error.human(), case.human, "{} {:?}", case.kind, case.code);
        if let ClientError::Api(_, message, envelope) = &error {
            assert_eq!(message, &case.message);
            assert_eq!(serde_json::to_value(envelope).unwrap(), before.unwrap());
        }
    }
}

#[test]
fn assistant_timeline_round_trip_preserves_read_side_wire_contract() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/st3/client-v0/schemas/client-v0.schema.json"
    ))
    .unwrap();
    let definitions = &schema["$defs"];
    let properties = &definitions["TimelineMessageBody"]["properties"];
    assert_eq!(properties["from"]["$ref"], "#/$defs/SenderRef");
    assert_eq!(properties["to"]["$ref"], "#/$defs/TimelineRecipientRef");
    for reference in ["SenderRef", "TimelineRecipientRef"] {
        assert_eq!(
            definitions[reference]["x-st-ref"],
            serde_json::json!(["agent", "assistant", "person", "step"])
        );
    }
    for narrow in ["ParticipantRef", "ActorRef"] {
        assert!(!definitions[narrow]["x-st-ref"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("assistant")));
    }
    let wire = serde_json::json!({
        "id": "timeline-entry/assistant-message", "sequence": 1, "revision": 1,
        "timestamp": "2026-10-17T10:00:00Z", "role": "user", "type": "message",
        "final": true,
        "body": {
            "message_id": "message/assistant", "from": "assistant/chatgpt",
            "to": "agent/example", "sender_class": "external-assistant",
            "assistant_grant_id": "assistant-grant/example"
        }
    });
    let entry: TimelineEntry = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(&entry).unwrap();
    for field in ["message_id", "from", "to", "sender_class", "assistant_grant_id"] {
        assert_eq!(encoded["body"][field], wire["body"][field]);
    }
    let decoded: TimelineEntry = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, entry);
}

#[test]
fn directive_notes_preserve_author_time_expiry_and_clear_as_empty() {
    let wire: Envelope<DirectiveNotes> = decode("directive-notes.json");
    let note = &wire.value.notes[0];
    assert_eq!(note.person, "person/ada");
    assert_eq!(note.author, note.person);
    assert_eq!(note.time, "2026-10-10T09:00:00Z");
    assert_eq!(note.expires_at.as_deref(), Some("2026-10-12T09:00:00Z"));
    assert_eq!(note.revision, "claim/example");
    let empty: DirectiveNotes = serde_json::from_value(serde_json::json!({"notes":[]})).unwrap();
    assert!(empty.notes.is_empty());
    let mut without_expiry = serde_json::to_value(note).unwrap();
    without_expiry.as_object_mut().unwrap().remove("expires_at");
    assert!(serde_json::from_value::<DirectiveNote>(without_expiry).unwrap().expires_at.is_none());
}

#[test]
fn agent_lifecycle_future_value_keeps_the_roster_row() {
    let row = serde_json::json!({
        "kind": "agent", "id": "agent/example", "revision": "r1",
        "updated_at": "2026-10-04T08:00:00Z", "name": "Example",
        "state": "running", "reachability": "local", "runtime_ids": [],
    });
    for (value, expected) in [
        ("standing", AgentLifecycle::Standing),
        ("owner", AgentLifecycle::Owner),
        ("bounded", AgentLifecycle::Bounded),
        ("future-kind", AgentLifecycle::Unknown),
    ] {
        let mut declared = row.clone();
        declared["lifecycle"] = serde_json::json!(value);
        let page: Page = serde_json::from_value(serde_json::json!({
            "kind": "page", "collection": "agents", "filters": {},
            "items": [declared], "page": {"limit": 50, "has_more": false},
        })).unwrap();
        let [Resource::Agent(agent)] = page.items.as_slice() else {
            panic!("lifecycle must not discard the agent row: {:?}", page.items);
        };
        assert_eq!(agent.header.id, "agent/example");
        assert_eq!(agent.name, "Example");
        assert_eq!(agent.lifecycle, Some(expected));
    }
    let absent: Agent = serde_json::from_value(row).unwrap();
    assert_eq!(absent.lifecycle, None);
    assert!(serde_json::to_value(absent).unwrap().get("lifecycle").is_none());
}


#[test]
fn person_ask_mission_preserves_person_assignees_in_both_step_views_and_wake() {
    let mission: Mission = decode("person-ask-mission.json");
    let run = &mission.run_details[0];
    assert_eq!(run.current_steps[0]["assignee"], "person/avery");
    assert!(run.current_steps[0]["claimant"].is_null());
    let step = &run.steps.as_ref().unwrap()[0];
    assert_eq!(step.assignee.as_deref(), Some("person/avery"));
    assert!(step.claimant.is_none());
    assert_eq!(step.wake.as_ref().unwrap().assignee, "person/avery");
}

#[test]
fn search_fixture_preserves_result_targets_and_incomplete_history() {
    let search: Envelope<ConversationSearch> = decode("conversation-search.json");
    assert_eq!(search.value.items[0].entry_id, "timeline-entry/note");
    assert_eq!(search.value.items[0].agent_id.as_deref(), Some("agent/scribe"));
    assert_eq!(search.value.incomplete_sources, ["session/older: truncation"]);
    assert!(!search.value.page.has_more);
}

#[test]
fn glass_groups_preserve_nested_tabs_and_empty_groups_on_round_trip() {
    let put: GlassPut = decode("glass-put.json");
    let GlassLayout::Split {
        split: GlassSplit::Right,
        children,
        ratio,
    } = &put.body.layout
    else {
        panic!("expected right split");
    };
    assert_eq!(*ratio, Some(0.3));
    let GlassLayout::Group { tabs } = &*children[0] else {
        panic!("expected tab group");
    };
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].title.as_deref(), Some("Work"));
    assert_eq!(tabs[0].pane, "agent:agent/example/worker");
    let GlassLayout::Split {
        split: GlassSplit::Below,
        children,
        ratio: None,
    } = &*children[1]
    else {
        panic!("expected nested below split");
    };
    assert!(matches!(&*children[1], GlassLayout::Group { tabs } if tabs.is_empty()));
    assert_eq!(
        serde_json::to_value(&put).unwrap(),
        serde_json::from_slice::<serde_json::Value>(&fixture("glass-put.json")).unwrap()
    );
    let empty: GlassPut = decode("glass-put-empty.json");
    assert!(matches!(empty.body.layout, GlassLayout::Group { tabs } if tabs.is_empty()));
    assert!(
        serde_json::from_value::<GlassBody>(serde_json::json!({"name":"Old", "tabs":[]})).is_err()
    );
}

#[test]
fn generated_models_decode_every_stream_fixture() {
    assert!(
        std::mem::size_of::<TimelineBody>() <= 128,
        "the discriminated timeline body must keep large variants boxed"
    );
    let capabilities: Envelope<Capabilities> = decode("capabilities.json");
    assert_eq!(capabilities.value.limits.max_page_items, 200);
    let events: Envelope<EventPage> = decode("events.json");
    assert_eq!(events.value.items.len(), 2);
    let timeline: Envelope<TimelinePage> = decode("timeline.json");
    assert_eq!(timeline.value.items.len(), 10);
    assert!(matches!(
        timeline.value.items[0].body,
        TimelineBody::Status(_)
    ));
    match &timeline.value.items[1].body {
        TimelineBody::Message(message) => assert_eq!(message.tags, ["dictated"]),
        other => panic!("message body lost: {other:?}"),
    }
    assert!(matches!(
        timeline.value.items[2].body,
        TimelineBody::Content(_)
    ));
    assert!(matches!(
        timeline.value.items[4].body,
        TimelineBody::ToolCall(_)
    ));
    assert!(matches!(
        timeline.value.items[5].body,
        TimelineBody::ToolResult(_)
    ));
    let TimelineBody::Usage(usage) = &timeline.value.items[6].body else {
        panic!("usage discriminator was not preserved");
    };
    assert_eq!(usage.total_tokens, Some(500));
    assert_eq!(usage.attribution.agent_id, "agent/release-agent");
    assert!(matches!(
        timeline.value.items[7].body,
        TimelineBody::Redaction(_)
    ));
    assert!(matches!(
        timeline.value.items[8].body,
        TimelineBody::Truncation(_)
    ));
    assert!(matches!(
        timeline.value.items[9].body,
        TimelineBody::Error(_)
    ));
    let screen: Envelope<TerminalScreen> = decode("terminal-screen.json");
    assert_eq!(screen.value.lines.len(), 3);
    assert_eq!(screen.value.cursor.style, TerminalCursorStyle::Block);
    assert!(screen.value.modes.bracketed_paste);
    for line in &screen.value.lines {
        let spelled = line
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(spelled.trim_end_matches(' '), line.text);
    }
    assert_eq!(
        screen.value.lines[0].runs[0],
        TerminalRun {
            text: "$ ".into(),
            cells: Some(2),
            fg: Some(TerminalColor::Palette(2)),
            bold: true,
            ..TerminalRun::default()
        }
    );
    assert_eq!(
        screen.value.lines[1].runs[0].fg,
        Some(TerminalColor::Rgb("#5fd75f".into()))
    );
    let facts = screen.value.facts.as_ref().expect("the fixture carries facts");
    assert_eq!(facts.clients.as_ref().unwrap().read_only, 1);
    assert_eq!(facts.uptime_s, Some(3120));
    let relay = screen.value.relay.as_ref().expect("the fixture carries provenance");
    assert_eq!(relay.owner_host_id, "host/host-b");
    assert!(!relay.direct);
    assert_eq!(relay.capability_ttl_s, 60);
    let changed: Envelope<TerminalScreen> = decode("terminal-screen-changed.json");
    assert_eq!(changed.value.terminal_id, screen.value.terminal_id);
    assert_ne!(changed.value.revision, screen.value.revision);
    let ended: ErrorEnvelope = decode("terminal-stale-fence-error.json");
    assert_eq!(ended.code, ErrorCode::StaleFence);
    let pairing: Envelope<PersonPairedSession> = decode("pairing.json");
    assert!(pairing.value.credential.len() >= 32);
}

#[test]
fn mcp_pairing_and_read_proofs_preserve_their_principal() {
    let paired: Envelope<PairedSession> = decode("assistant-mcp-pairing.json");
    let PairedSession::Assistant(session) = paired.value else {
        panic!("MCP session decoded as person");
    };
    assert_eq!(session.principal, AssistantPrincipal::Mcp);
    assert_eq!(session.grant.principal, AssistantPrincipal::Mcp);
    assert_eq!(session.grant.issuer, "person/robin");
    assert_eq!(
        session.session_actor,
        format!("assistant/mcp/session/{}", session.grant.id)
    );
    assert_eq!(session.session_actor, session.grant.session_actor);
    assert!(serde_json::from_slice::<Envelope<PersonPairedSession>>(
        &fixture("assistant-mcp-pairing.json")
    ).is_err());

    let mut proof: serde_json::Value = decode("assistant-read-authorization.json");
    proof["payload"]["issuer"] = serde_json::json!("person/robin");
    proof["payload"]["identity"] = serde_json::json!("mcp");
    proof["payload"]["assistant_id"] = serde_json::json!("assistant/mcp");
    let proof: ScgReadAuthorization = serde_json::from_value(proof).unwrap();
    assert_eq!(proof.payload.identity, ScgReadIdentity::Mcp);
    assert_eq!(proof.payload.assistant_id, AssistantPrincipal::Mcp);
}

#[test]
fn assistant_pairing_is_disjoint_from_person_delegation() {
    let begin: PairingBegin = decode("assistant-pairing-begin.json");
    let PairingBegin::Assistant(request) = begin else { panic!("assistant request decoded as person"); };
    assert_eq!(request.assistant_id, AssistantPrincipal::Chatgpt);
    let scopes = vec![
        AssistantScope::Summary,
        AssistantScope::Attention,
        AssistantScope::Missions,
        AssistantScope::Agents,
    ];
    assert_eq!(request.scopes, scopes);
    assert!(request.disclosure_consent);
    assert_eq!(request.consent_evidence, "axe decision ask recorded by the owning person");
    assert_eq!(request.expires_in_seconds, Some(3600));
    let policy: AssistantGrantPolicy = decode("assistant-grant-policy.json");
    assert_eq!(policy.minimal_read_mode, AssistantMinimalReadMode::ApprovalOnce);
    assert_eq!(policy.scopes, scopes);
    assert!(policy.disclosure_consent);
    assert_eq!(policy.consent_evidence, "axe decision ask recorded by the owning person");

    let challenge: Envelope<PairingChallenge> = decode("assistant-pairing-challenge.json");
    assert_eq!(challenge.value.grant_kind.as_deref(), Some("assistant"));
    assert_ne!(challenge.value.grant_expires_at.as_deref(), Some(challenge.value.expires_at.as_str()));
    assert!(challenge.value.person_root_fingerprint.is_none());

    let paired: Envelope<PairedSession> = decode("assistant-pairing.json");
    let PairedSession::Assistant(session) = paired.value else { panic!("assistant session decoded as person"); };
    assert_eq!(session.sender_class, AssistantSenderClass::ExternalAssistant);
    assert_eq!(session.grant.policy_revision, 1);
    assert_eq!(session.grant.issuer, "person/operator");
    assert_eq!(session.scopes, scopes);
    assert_eq!(session.grant.scopes, scopes);
    assert!(session.grant.disclosure_consent);
    assert_eq!(session.grant.consent_evidence, "axe decision ask recorded by the owning person");
    assert!(session.grant.recipient_allowlist.is_empty());
    let public = serde_json::to_value(&session.grant).unwrap();
    assert!(public.get("credential").is_none());
    assert!(public.get("credential_digest").is_none());
    assert!(public.get("person_id").is_none());
    assert!(serde_json::from_slice::<Envelope<PersonPairedSession>>(&fixture("assistant-pairing.json")).is_err());
    assert!(serde_json::from_slice::<Envelope<AssistantPairedSession>>(&fixture("pairing.json")).is_err());
    let person: Envelope<PairedSession> = decode("pairing.json");
    assert!(matches!(person.value, PairedSession::Person(_)));

    let mut mixed: serde_json::Value = decode("assistant-pairing-begin.json");
    mixed["person_id"] = serde_json::json!("person/operator");
    assert!(serde_json::from_value::<PairingBegin>(mixed).is_err());
    let mut broad: serde_json::Value = decode("assistant-pairing-begin.json");
    broad["scopes"] = serde_json::json!(["control.messages"]);
    assert!(serde_json::from_value::<PairingBegin>(broad).is_err());
}

#[test]
fn generated_resource_union_decodes_all_kinds() {
    let resources: Vec<Resource> = serde_json::from_slice(&fixture("resources.json")).unwrap();
    assert_eq!(resources.len(), 19);
    let question = resources
        .iter()
        .find_map(|resource| match resource {
            Resource::HarnessQuestion(question) => Some(question),
            _ => None,
        })
        .expect("the fixture must decode a typed harness question, not an unknown resource");
    assert_eq!(question.header.id, "harness-question/00ca25ee5d0da6af852c81a5ee4949df");
    assert_eq!(question.header.revision, "claim/ada-observed");
    assert_eq!(question.agent, "agent/ada");
    assert_eq!(question.session, "ada-session");
    assert_eq!(question.runtime_incarnation, "ada-incarnation-1");
    assert_eq!(question.observed_at, 1793008800000);
    assert_eq!(question.questions, vec![HarnessQuestionItem {
        id: "q0".into(),
        options: 2,
        multi: false,
        required: true,
    }]);
    assert!(question.resolution.is_none());
    assert_eq!(question.answer.state, HarnessQuestionAnswerabilityState::Unavailable);
    assert_eq!(question.answer.reason, Some(HarnessQuestionAnswerabilityReason::AdapterMissing));
    let typed = Resource::HarnessQuestion(question.clone());
    let encoded = serde_json::to_value(&typed).unwrap();
    assert_eq!(encoded["kind"], "harness-question");
    let round_trip: Resource = serde_json::from_value(encoded).unwrap();
    assert_eq!(round_trip, typed);
    let lane = resources
        .iter()
        .find_map(|resource| match resource {
            Resource::Lane(lane) => Some(lane),
            _ => None,
        })
        .unwrap();
    assert_eq!(lane.entries[0].label, "42");
    assert_eq!(lane.recent[0].placement, Some(AgentQueuePlacement::Bottom));
    assert!(
        resources
            .iter()
            .all(|resource| resource.header().id.contains('/'))
    );
    let launch = resources
        .iter()
        .find_map(|resource| match resource {
            Resource::Launch(launch) => Some(launch),
            _ => None,
        })
        .unwrap();
    let visualization = launch.visualization.as_ref().unwrap();
    assert_eq!(visualization.nodes[0].goals, ["Build artifacts"]);
    assert_eq!(
        visualization.decisions[0].decision_type,
        DecisionType::SingleChoice
    );
    assert_eq!(visualization.diffs[0].changes.len(), 1);
    assert_eq!(visualization.swimlanes[0].nodes, ["step/build"]);
    let work = resources
        .iter()
        .find_map(|resource| match resource {
            Resource::Work(work) => Some(work),
            _ => None,
        })
        .unwrap();
    assert_eq!(work.blocked_reason, None);
    assert!(work.blockers.is_empty());
    let agent = resources
        .iter()
        .find_map(|resource| match resource {
            Resource::Agent(agent) => Some(agent),
            _ => None,
        })
        .unwrap();
    assert_eq!(agent.workspace.as_deref(), Some("/srv/example/release"));
    assert_eq!(agent.checkout.as_ref().unwrap().branch, "release");
    assert_eq!(agent.current_work_ids, ["step-run/release/build"]);
    assert_eq!(
        agent.next_work_id.as_deref(),
        Some("step-run/release/deploy")
    );
    assert_eq!(agent.queued_work_count, 1);
    let [subagent] = agent.subagents.as_slice() else {
        panic!("the release agent runs one subagent: {:?}", agent.subagents);
    };
    assert_eq!(
        subagent.description.as_deref(),
        Some("review the changelog")
    );
    assert_eq!(subagent.work_id.as_deref(), Some("step-run/release/build"));
    assert_eq!(subagent.lease_expires_at, "2026-09-20T11:17:10Z");
    assert!(!agent.extra.contains_key("subagents"));
    assert_eq!(agent.background_jobs, None, "an older card without the field is unknown");
    let machine = resources
        .iter()
        .find_map(|resource| match resource {
            Resource::Machine(machine) => Some(machine),
            _ => None,
        })
        .unwrap();
    assert_eq!(machine.host_id, "host/host-a");
    assert_eq!(machine.capacity.state, "unknown");
    assert_eq!(machine.occupancy.running_runtimes, 1);
    assert!(machine.projects.is_empty());
    let device = resources
        .iter()
        .find_map(|resource| match resource {
            Resource::Device(device) => Some(device),
            _ => None,
        })
        .unwrap();
    assert_eq!(device.person_id.as_deref(), Some("person/alex"));
    assert_eq!(device.state, "active");
}

#[test]
fn agent_background_jobs_decode_as_known_count_or_unknown() {
    let resources: Vec<serde_json::Value> =
        serde_json::from_slice(&fixture("resources.json")).unwrap();
    let card = resources
        .into_iter()
        .find(|resource| resource["kind"] == "agent")
        .unwrap();
    for (wire, expected) in [
        (serde_json::json!(2), Some(2)),
        (serde_json::json!(0), Some(0)),
        (serde_json::Value::Null, None),
    ] {
        let mut card = card.clone();
        card["background_jobs"] = wire;
        let Resource::Agent(agent) = serde_json::from_value(card).unwrap() else {
            panic!("agent discriminator was not preserved");
        };
        assert_eq!(agent.background_jobs, expected);
        assert!(!agent.extra.contains_key("background_jobs"));
    }
}

#[test]
fn usage_aggregation_schema_literals_keep_agent_rows() {
    let schema: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            PathBuf::from(test_env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/st3/client-v0/schemas/client-v0.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let literals = schema["$defs"]["UsageSummary"]["properties"]["aggregation"]["enum"]
        .as_array()
        .unwrap();
    assert_eq!(
        literals,
        serde_json::json!([
            "cumulative-per-incarnation-else-response-deltas",
            "rollup-per-slot-else-cumulative-per-incarnation-else-response-deltas"
        ])
        .as_array()
        .unwrap()
    );
    for aggregation in literals {
        let page: Page = serde_json::from_value(serde_json::json!({
            "kind": "page", "collection": "agents", "filters": {},
            "items": [{
                "kind": "agent", "id": "agent/example", "revision": "r1",
                "updated_at": "2026-10-04T08:00:00Z", "name": "Example",
                "state": "running", "reachability": "local", "runtime_ids": [],
                "usage": {
                    "total_tokens": 1200, "input_tokens": 900, "output_tokens": 300,
                    "cached_tokens": 100, "incarnation_count": 2,
                    "aggregation": aggregation
                }
            }],
            "page": {"limit": 50, "has_more": false}
        }))
        .unwrap();
        let [Resource::Agent(agent)] = page.items.as_slice() else {
            panic!("usage aggregation must not discard the agent row: {:?}", page.items);
        };
        assert_eq!(
            agent.usage.as_ref().unwrap().aggregation,
            aggregation.as_str().unwrap()
        );
    }
}

#[test]
fn session_usage_context_decodes_through_flattened_resource() {
    let resource: Resource = serde_json::from_value(serde_json::json!({
        "id": "session/compatibility-test",
        "kind": "session",
        "revision": "s1",
        "updated_at": "2026-09-23T12:00:00Z",
        "owner_id": "agent/test",
        "state": "running",
        "started_at": "2026-09-23T11:00:00Z",
        "ended_at": null,
        "timeline_cursor": "timeline-cursor/compatibility-test/latest",
        "usage": {
            "total_tokens": 1200,
            "input_tokens": 900,
            "output_tokens": 300,
            "cached_tokens": 100,
            "cost": null,
            "currency": null,
            "incarnation_count": 2,
            "aggregation": "rollup-per-slot-else-cumulative-per-incarnation-else-response-deltas",
            "context": {
                "used_tokens": 800,
                "window_tokens": 200000,
                "used_percent": 0.4,
                "model": "future-model",
                "observed_at_unix_ms": 1_790_163_492_730_u64,
                "new_harness_field": true
            }
        },
        "new_session_field": {"preserved": true}
    }))
    .expect("session usage emitted by a newer harness must decode");

    let Resource::Session(session) = resource else {
        panic!("resource discriminator was not preserved");
    };
    assert_eq!(
        session
            .usage
            .and_then(|usage| usage.context)
            .map(|context| context.observed_at_unix_ms),
        Some(1_790_163_492_730)
    );
    assert_eq!(session.extra["new_session_field"]["preserved"], true);
}

#[test]
fn timeline_tool_result_preserves_timing_metadata_and_decodes_legacy_results() {
    let mut wire = serde_json::json!({
        "id": "timeline-entry/tool-timing",
        "sequence": 1,
        "revision": 1,
        "timestamp": "2026-10-05T12:00:00Z",
        "role": "tool",
        "type": "tool_result",
        "final": true,
        "body": {
            "call_id": "call/timing",
            "status": "success",
            "media_type": "text/plain",
            "content": "finished",
            "blocks": [{
                "id": "tool-timing/0",
                "kind": "tool_output",
                "source_type": "tool_result",
                "payload": {"body_ref": true},
                "metadata": {
                    "wallTimeMs": 1250.5,
                    "timeoutSeconds": 0,
                    "future": {"unit": "ticks", "value": 0.125}
                }
            }]
        }
    });
    let entry: TimelineEntry = serde_json::from_value(wire.clone()).unwrap();
    let TimelineBody::ToolResult(body) = entry.body else {
        panic!("tool result discriminator was not preserved");
    };
    assert_eq!(
        body.blocks[0].metadata.as_ref(),
        Some(&wire["body"]["blocks"][0]["metadata"])
    );
    assert_eq!(
        serde_json::to_value(&body).unwrap()["blocks"][0]["metadata"],
        wire["body"]["blocks"][0]["metadata"]
    );

    wire["body"]["blocks"][0].as_object_mut().unwrap().remove("metadata");
    let untimed: TimelineEntry = serde_json::from_value(wire.clone()).unwrap();
    assert!(matches!(
        untimed.body,
        TimelineBody::ToolResult(body) if body.blocks[0].metadata.is_none()
    ));
    wire["body"].as_object_mut().unwrap().remove("blocks");
    let legacy: TimelineEntry = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        legacy.body,
        TimelineBody::ToolResult(body) if body.blocks.is_empty()
    ));
}

#[test]
fn a_mailbox_content_entry_names_its_message_and_older_content_decodes_without() {
    let mut wire = serde_json::json!({
        "id": "timeline-entry/mailbox-content", "sequence": 3, "revision": 1,
        "timestamp": "2026-09-20T12:00:00Z", "role": "user", "type": "content", "final": true,
        "body": {"media_type": "text/plain", "text": "hello", "message_id": "message/first"}
    });
    let entry: TimelineEntry = serde_json::from_value(wire.clone()).unwrap();
    let TimelineBody::Content(body) = &entry.body else {
        panic!("content discriminator was not preserved");
    };
    assert_eq!(body.message_id.as_deref(), Some("message/first"));
    assert_eq!(serde_json::to_value(&entry).unwrap(), wire);

    wire["body"].as_object_mut().unwrap().remove("message_id");
    let older: TimelineEntry = serde_json::from_value(wire.clone()).unwrap();
    assert!(matches!(&older.body, TimelineBody::Content(body) if body.message_id.is_none()));
    assert_eq!(serde_json::to_value(&older).unwrap(), wire);
}

#[test]
fn timeline_models_tolerate_future_discriminators() {
    fn entry(entry_type: &str, body: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": format!("timeline-entry/{entry_type}"),
            "sequence": 1,
            "revision": 1,
            "timestamp": "2026-09-23T12:00:00Z",
            "role": "future_role",
            "type": entry_type,
            "final": true,
            "body": body
        })
    }

    let unknown: TimelineEntry = serde_json::from_value(entry(
        "future_event",
        serde_json::json!({"future": "payload"}),
    ))
    .expect("a newer harness timeline type must not break an older client");
    assert_eq!(unknown.role, TimelineRole::Unknown);
    assert!(matches!(
        &unknown.body,
        TimelineBody::Unknown { entry_type, body }
            if entry_type == "future_event" && body["future"] == "payload"
    ));
    assert_eq!(unknown.body.entry_type(), TimelineType::Unknown);
    assert_eq!(
        serde_json::to_value(&unknown).unwrap()["type"],
        "future_event",
        "JSON output must preserve a future timeline discriminator"
    );

    let status: TimelineEntry = serde_json::from_value(entry(
        "status",
        serde_json::json!({"status": "future_status", "detail": null}),
    ))
    .expect("a newer harness status must not break an older client");
    assert!(matches!(
        status.body,
        TimelineBody::Status(TimelineStatusBody {
            status: TimelineStatus::Unknown,
            ..
        })
    ));

    let tool: TimelineEntry = serde_json::from_value(entry(
        "tool_result",
        serde_json::json!({
            "call_id": "call/future",
            "status": "future_status",
            "media_type": "application/json",
            "content": null
        }),
    ))
    .expect("a newer tool status must not break an older client");
    assert!(matches!(
        tool.body,
        TimelineBody::ToolResult(result) if result.status == TimelineToolStatus::Unknown
    ));

    let usage: TimelineEntry = serde_json::from_value(entry(
        "usage",
        serde_json::json!({
            "semantics": "future_semantics",
            "driver": "future-driver",
            "attribution": {
                "agent_id": "agent/test",
                "mission_run_id": null,
                "generation_id": null,
                "step_id": null
            }
        }),
    ))
    .expect("newer usage semantics must not break an older client");
    assert!(matches!(
        usage.body,
        TimelineBody::Usage(body) if body.semantics == TimelineUsageSemantics::Unknown
    ));
}

#[test]
fn generated_action_union_and_machine_manifest_stay_in_lockstep() {
    let action: ActionRequest = decode("action.json");
    let operations: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/st3/client-v0/schemas/operations.json"
    ))
    .unwrap();
    assert_eq!(action.action_type(), &ActionType::WorkComplete);
    assert_eq!(
        ACTION_NAMES.len(),
        operations["actions"].as_object().unwrap().len()
    );
    assert_eq!(
        READ_OPERATIONS.len(),
        operations["reads"].as_array().unwrap().len()
    );
}

#[test]
fn crate_exposes_both_transport_constructors() {
    let _ = Client::unix(Path::new("/tmp/st3.sock"));
    let _ = Client::fabric_loopback("http://127.0.0.1:8787", "credential");
}

#[test]
fn authority_error_codes_are_typed_and_round_trip() {
    for (raw, expected) in [
        ("runtime-not-local", ErrorCode::RuntimeNotLocal),
        (
            "runtime-authority-indeterminate",
            ErrorCode::RuntimeAuthorityIndeterminate,
        ),
    ] {
        let decoded: ErrorCode = serde_json::from_str(&format!("\"{raw}\"")).unwrap();
        assert_eq!(decoded, expected);
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            format!("\"{raw}\"")
        );
    }
}

#[test]
fn message_selector_readiness_is_typed_and_preserves_its_plain_message() {
    let code: ErrorCode = serde_json::from_str("\"message-selectors-not-ready\"").unwrap();
    assert_eq!(code, ErrorCode::MessageSelectorsNotReady);
    assert_eq!(
        serde_json::to_string(&code).unwrap(),
        "\"message-selectors-not-ready\""
    );
    let message = "message selector initialization is still in progress";
    assert_eq!(plain_message(Some(&code), message), message);
}

#[test]
fn runtime_publication_errors_are_typed_and_round_trip() {
    for (raw, expected) in [
        ("runtime-list-not-ready", ErrorCode::RuntimeListNotReady),
        ("runtime-list-ended", ErrorCode::RuntimeListEnded),
    ] {
        let decoded: ErrorCode = serde_json::from_str(&format!("\"{raw}\"")).unwrap();
        assert_eq!(decoded, expected);
        assert_eq!(serde_json::to_string(&decoded).unwrap(), format!("\"{raw}\""));
    }
}

#[test]
fn runtime_publication_error_messages_preserve_retryability() {
    for (code, retryable, expected) in [
        (ErrorCode::RuntimeListNotReady, true,
            "st is still preparing the runtime list; try again shortly"),
        (ErrorCode::RuntimeListEnded, false,
            "the runtime list stopped updating; restart fractal"),
    ] {
        let envelope = serde_json::from_value(serde_json::json!({
            "api_version": "st3.client.v0", "error_version": "st3.client.error.v0",
            "request_id": "request/runtime-publication", "code": code,
            "message": "raw server message", "retryable": retryable, "details": {},
        })).unwrap();
        let error = ClientError::Api(code, "raw server message".into(), Box::new(envelope));
        assert_eq!(error.plain(), expected);
        assert_eq!(error.is_transient(), retryable);
    }
}

#[test]
fn creation_actions_round_trip_typed_parameters() {
    for name in ["agent-create.json", "terminal-create.json", "terminal-end.json"] {
        let action: ActionRequest = decode(name);
        let mut expected: serde_json::Value = decode(name);
        let fence: Fence = serde_json::from_value(expected["fence"].clone()).unwrap();
        expected["fence"] = serde_json::to_value(fence).unwrap();
        assert_eq!(serde_json::to_value(action).unwrap(), expected);
    }
    let action: ActionRequest = decode("agent-create.json");
    assert_eq!(action.action_type(), &ActionType::AgentCreate);
    let value: serde_json::Value = decode("agent-create.json");
    let parameters: AgentCreateParameters = serde_json::from_value(value["parameters"].clone()).unwrap();
    assert_eq!(parameters.name, "worker"); assert!(parameters.message.as_deref().unwrap().starts_with("--First"));
    let generated = ActionRequest::agent_create("action/agent-create", "fixture-agent-create-0001",
        serde_json::from_value(value["fence"].clone()).unwrap(), parameters).unwrap();
    assert_eq!(serde_json::to_value(generated).unwrap(), serde_json::to_value(action).unwrap());
}

#[test]
fn a_kind_this_client_does_not_know_reads_as_unknown_and_the_page_still_reads() {
    // A newer member sends a kind this build has never heard of, beside one it knows.
    let page: Page = serde_json::from_value(serde_json::json!({
        "kind": "page", "collection": "resources", "filters": {},
        "items": [
            {"kind": "example-arrangement", "id": "example-arrangement/person/avery/1",
             "revision": "r1", "updated_at": "2026-10-04T08:00:00Z",
             "name": "Pinned", "folders": {"inbox": {"name": "Inbox"}}},
            {"kind": "device", "id": "device/0011", "revision": "r2",
             "updated_at": "2026-10-04T08:00:00Z", "person_id": "person/avery",
             "name": "Pocket", "session_actor": "person/avery/session/0011", "state": "active",
             "scopes": ["read.projections"], "expires_at": "2026-11-04T08:00:00Z"}
        ],
        "page": {"limit": 50, "has_more": false}
    }))
    .unwrap();
    let Resource::Unknown(unknown) = &page.items[0] else {
        panic!("an unknown kind is kept as unknown: {:?}", page.items[0])
    };
    assert_eq!(unknown.kind, "example-arrangement");
    assert_eq!(page.items[0].header().id, "example-arrangement/person/avery/1");
    assert_eq!(unknown.fields["name"], "Pinned");
    assert!(matches!(page.items[1], Resource::Device(_)));
    // It goes back out as it came.
    let again = serde_json::to_value(&page.items[0]).unwrap();
    assert_eq!(again["kind"], "example-arrangement");
    assert_eq!(again["folders"]["inbox"]["name"], "Inbox");
    assert_eq!(again["revision"], "r1");
    // A known kind that does not match its model is still an error, never unknown.
    assert!(
        serde_json::from_value::<Resource>(serde_json::json!({
            "kind": "device", "id": "device/0011", "revision": "r2",
            "updated_at": "2026-10-04T08:00:00Z"
        }))
        .is_err()
    );
}

#[test]
fn ordered_membership_contract_preserves_root_remove_and_versioned_bodies() {
    let value: serde_json::Value = serde_json::from_str(include_str!("../../../fixtures/clients/ordered-memberships-v2.json")).unwrap();
    let parameters: ArrangementMembershipEditParameters = serde_json::from_value(value["edit"]["parameters"].clone()).unwrap();
    assert!(matches!(&parameters.operations[0], OrderedMembershipOperation::Place { bucket: None, .. }));
    assert!(matches!(&parameters.operations[1], OrderedMembershipOperation::Remove { .. }));
    let fence: Fence = serde_json::from_value(value["edit"]["fence"].clone()).unwrap();
    let serialized_fence = serde_json::to_value(&fence).unwrap();
    let request = ActionRequest::arrangement_membership_edit(
        "action/membership-edit", "membership-edit-ada-0001",
        fence, parameters,
    ).unwrap();
    let mut expected = value["edit"].clone();
    expected["fence"] = serialized_fence;
    assert_eq!(serde_json::to_value(request).unwrap(), expected);
    let page: OrderedMembershipPage = serde_json::from_value(value["page"].clone()).unwrap();
    assert_eq!(page.items[0].id, page.items[0].member);
    assert!(page.items[0].position.bucket.is_none());
    let mut expected_page = value["page"].clone();
    expected_page["page"]["next_cursor"] = serde_json::Value::Null;
    expected_page["page"]["cursor_expires_at"] = serde_json::Value::Null;
    assert_eq!(serde_json::to_value(page).unwrap(), expected_page);
    let body: ArrangementBody = serde_json::from_value(value["body"].clone()).unwrap();
    assert_eq!(serde_json::to_value(body).unwrap(), value["body"]);
    let legacy: serde_json::Value = serde_json::from_str(include_str!("../../../fixtures/clients/arrangements-v1.json")).unwrap();
    let body: ArrangementBody = serde_json::from_value(legacy["resource"]["body"].clone()).unwrap();
    assert_eq!(serde_json::to_value(body).unwrap(), legacy["resource"]["body"]);
    let mut invalid = value["body"].clone();
    invalid["version"] = serde_json::json!(3);
    assert!(serde_json::from_value::<ArrangementBody>(invalid).is_err());
    let mut invalid = value["body"].clone();
    invalid["placements"] = serde_json::json!({});
    assert!(serde_json::from_value::<ArrangementBody>(invalid).is_err());
    let parameters: ArrangementEditParameters = serde_json::from_value(legacy["edit"]["parameters"].clone()).unwrap();
    assert_eq!(parameters.version, None);
    assert_eq!(serde_json::to_value(parameters).unwrap(), legacy["edit"]["parameters"]);
    for (raw, code) in [
        ("unsupported-membership-container", ErrorCode::UnsupportedMembershipContainer),
        ("membership-owner-forbidden", ErrorCode::MembershipOwnerForbidden),
        ("invalid-membership-member", ErrorCode::InvalidMembershipMember),
        ("invalid-membership-bucket", ErrorCode::InvalidMembershipBucket),
        ("invalid-membership-key", ErrorCode::InvalidMembershipKey),
        ("invalid-membership-operations", ErrorCode::InvalidMembershipOperations),
        ("invalid-arrangement-version", ErrorCode::InvalidArrangementVersion),
        ("invalid-arrangement-body", ErrorCode::InvalidArrangementBody),
        ("membership-edit-too-large", ErrorCode::MembershipEditTooLarge),
    ] {
        assert_eq!(serde_json::from_value::<ErrorCode>(serde_json::json!(raw)).unwrap(), code);
        assert_eq!(serde_json::to_value(code).unwrap(), raw);
    }
}

#[test]
fn ordered_membership_envelopes_decode_and_missing_buckets_fail_before_a_request_exists() {
    let value: serde_json::Value = serde_json::from_str(include_str!("../../../fixtures/clients/ordered-memberships-v2.json")).unwrap();
    let envelope: Envelope<OrderedMembershipPage> = serde_json::from_value(value["envelope"].clone()).unwrap();
    assert_eq!(envelope.value.collection, "ordered-memberships");
    assert!(envelope.value.items.is_empty());
    let mut expected_page = value["envelope"]["value"].clone();
    expected_page["page"]["next_cursor"] = serde_json::Value::Null;
    expected_page["page"]["cursor_expires_at"] = serde_json::Value::Null;
    assert_eq!(serde_json::to_value(&envelope.value).unwrap(), expected_page);
    // A missing bucket is not the root: neither the edit nor a read row decodes without it.
    let mut missing = value["edit"]["parameters"].clone();
    missing["operations"][0].as_object_mut().unwrap().remove("bucket");
    let error = serde_json::from_value::<ArrangementMembershipEditParameters>(missing).unwrap_err();
    assert!(error.to_string().contains("missing field `bucket`"), "{error}");
    let mut row = value["page"]["items"][0].clone();
    row["position"].as_object_mut().unwrap().remove("bucket");
    assert!(serde_json::from_value::<OrderedMembership>(row).is_err());
    let mut explicit_root = value["page"]["items"][0].clone();
    explicit_root["position"]["bucket"] = serde_json::Value::Null;
    let decoded: OrderedMembership = serde_json::from_value(explicit_root).unwrap();
    assert_eq!(decoded.position.bucket, None);
}

#[test]
fn invocation_attribution_preserves_each_optional_evidence_field() {
    for mask in 0..8 {
        let mut raw = serde_json::json!({"agent_id":"agent/example",
            "mission_run_id":null,"generation_id":null,"step_id":null});
        for (bit, key, value) in [
            (1, "execution_id", serde_json::json!("claim/exact")),
            (2, "invocation_id", serde_json::json!("invocation/exact")),
            (4, "attempt", serde_json::json!(2)),
        ] {
            if mask & bit != 0 { raw[key] = value; }
        }
        let typed: TimelineAttribution = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), raw);
    }
}

#[test]
fn relay_omission_fixture_round_trips_without_inventing_nonnullable_fields() {
    let bytes = fixture("timeline-relay-omission.json");
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let typed: Envelope<TimelinePage> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_value(&typed).unwrap(), raw);
    let changes = serde_json::json!({"kind":"conversation-changes", "session_id":raw["value"]["session_id"],
        "items":raw["value"]["items"], "next_cursor":"conversation-cursor/relay/next"});
    let typed: ConversationChanges = serde_json::from_value(changes.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), changes);
}

#[path = "support/legacy_delivery_entry.rs"]
mod legacy_delivery_entry;

#[test]
fn delivery_link_round_trip_survives_the_typed_owner_relay_and_old_codec_ignores_it() {
    let bytes = fixture("timeline-delivery-links.json");
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let typed: Envelope<TimelinePage> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(typed.value.items[0].delivers.as_deref(), Some("message/fixture-delivery"));
    assert!(typed.value.items[1].delivers.is_none());
    assert_eq!(serde_json::to_value(&typed).unwrap(), raw);
    let changes = serde_json::json!({"kind":"conversation-changes", "session_id":raw["value"]["session_id"],
        "items":raw["value"]["items"], "next_cursor":"conversation-cursor/delivery/next"});
    let typed: ConversationChanges = serde_json::from_value(changes.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), changes);
    for item in raw["value"]["items"].as_array().unwrap() {
        let old: legacy_delivery_entry::TimelineEntry = serde_json::from_value(item.clone()).unwrap();
        let mut expected = item.clone(); expected.as_object_mut().unwrap().remove("delivers");
        assert_eq!(serde_json::to_value(old).unwrap(), expected);
    }
}

/// Contract shape only: these fixtures carry placeholder pins and an intentionally invalid
/// signature. Ed25519 verification, nonce replay and pin checks are exercised by the daemon tests.
#[test]
fn assistant_read_contract_decodes_fixtures_and_rejects_malformed_shapes() {
    use sha2::Digest as _;

    let trust: AssistantReadTrustRequest = decode("assistant-read-trust.json");
    assert_eq!(trust.issuer, "person/operator");
    assert_ne!(trust.attestor_public_key, trust.shim_public_key);
    assert!(trust.default_response_bytes.unwrap() <= trust.max_response_bytes.unwrap());
    let mut extra: serde_json::Value = decode("assistant-read-trust.json");
    extra["shim_private_key"] = serde_json::json!("never");
    assert!(serde_json::from_value::<AssistantReadTrustRequest>(extra).is_err());

    let proof: ScgReadAuthorization = decode("assistant-read-authorization.json");
    assert_eq!(proof.payload.identity, ScgReadIdentity::Chatgpt);
    assert_eq!(proof.payload.assistant_id, AssistantPrincipal::Chatgpt);
    assert_eq!(proof.payload.projection, AssistantProjection::ConversationContent);
    assert_eq!(
        proof.payload.parameters.get("max_response_bytes").map(String::as_str),
        Some(proof.payload.max_response_bytes.to_string().as_str())
    );
    // The request digest is SHA-256 of canonical {method, parameters, path} over the decoded query.
    let request = serde_json::json!({
        "method": "GET",
        "path": "/v1/client/assistant/conversation-content",
        "parameters": proof.payload.parameters,
    });
    let canonical = smallclaims::hash::canonical_json_text(&request).unwrap();
    assert_eq!(proof.payload.request_digest, hex::encode(sha2::Sha256::digest(canonical.as_bytes())));
    // The query model a proof binds is the same disjoint request the SDK sends.
    let query: AssistantConversationContentRequest = serde_json::from_value(serde_json::json!({
        "agent_id": "agent/example", "session_id": "session/example", "reference": "content/example-0",
        "revision": "r1", "start": 0, "end": 2048, "max_response_bytes": 8192,
    }))
    .unwrap();
    assert_eq!(query.max_response_bytes, Some(proof.payload.max_response_bytes));

    for (field, value) in [
        ("identity", serde_json::json!("claude")),
        ("assistant_id", serde_json::json!("assistant/other")),
        ("projection", serde_json::json!("status")),
        ("person_id", serde_json::json!("person/operator")),
        ("parameters", serde_json::json!({"start": 0})),
    ] {
        let mut malformed: serde_json::Value = decode("assistant-read-authorization.json");
        malformed["payload"][field] = value;
        assert!(
            serde_json::from_value::<ScgReadAuthorization>(malformed).is_err(),
            "payload `{field}` must not decode"
        );
    }
    let mut unsigned: serde_json::Value = decode("assistant-read-authorization.json");
    unsigned.as_object_mut().unwrap().remove("signature");
    assert!(serde_json::from_value::<ScgReadAuthorization>(unsigned).is_err());

    let summary: Envelope<AssistantSummary> = decode("assistant-summary.json");
    assert_eq!(summary.value.kind, "assistant-summary");
    let mut owner_leak: serde_json::Value = decode("assistant-summary.json");
    owner_leak["value"]["person_id"] = serde_json::json!("person/operator");
    assert!(serde_json::from_value::<Envelope<AssistantSummary>>(owner_leak).is_err());
    let attention: Envelope<AssistantAttentionPage> = decode("assistant-attention-page.json");
    assert_eq!(attention.value.collection, "assistant.attention");
    let missions: Envelope<AssistantMissionPage> = decode("assistant-mission-page.json");
    assert_eq!(missions.value.items[0].step_counts.total, 3);
    let agents: Envelope<AssistantAgentPage> = decode("assistant-agent-page.json");
    assert!(agents.value.items[1].updated_at.is_none());
    let conversation: Envelope<AssistantContentPage> = decode("assistant-conversation-content.json");
    let AssistantContentPage::Conversation(text) = conversation.value else { panic!("conversation content decoded as terminal") };
    assert_eq!(text.returned_bytes, text.text.len() as u64);
    let terminal: Envelope<AssistantContentPage> = decode("assistant-terminal-content.json");
    assert!(matches!(terminal.value, AssistantContentPage::Terminal(_)));

    let mut mixed: serde_json::Value = decode("assistant-conversation-content.json");
    mixed["value"]["terminal_id"] = serde_json::json!("terminal/example");
    assert!(serde_json::from_value::<Envelope<AssistantContentPage>>(mixed).is_err());
    let mut widened: serde_json::Value = decode("assistant-agent-page.json");
    widened["value"]["items"][0]["runtime_ids"] = serde_json::json!([]);
    assert!(serde_json::from_value::<Envelope<AssistantAgentPage>>(widened).is_err());
    assert!(
        serde_json::from_value::<AssistantContentRequest>(serde_json::json!({
            "agent_id": "agent/example", "session_id": "session/example", "reference": "content/example-0",
            "revision": "r1", "start": 0, "end": 1, "terminal_id": "terminal/example",
        }))
        .is_err()
    );
    assert!(serde_json::from_value::<AssistantPageRequest>(serde_json::json!({"person": "person/operator"})).is_err());
    assert!(serde_json::from_value::<AssistantSummaryRequest>(serde_json::json!({"cursor": "c"})).is_err());
}

#[test]
fn queued_mission_deadlines_decode_for_tui() {
    let mut wire: serde_json::Value =
        serde_json::from_slice(&fixture("person-ask-mission.json")).unwrap();
    wire["run_details"][0]["after"] = serde_json::json!("mission-run/predecessor");
    wire["run_details"][0]["deadline"] = serde_json::Value::Null;
    let queued: Mission = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(queued.run_details[0].deadline, None);
    wire["run_details"][0]["deadline"] = serde_json::json!("2026-09-20T11:05:00Z");
    let admitted: Mission = serde_json::from_value(wire).unwrap();
    assert_eq!(
        admitted.run_details[0].deadline.as_deref(),
        Some("2026-09-20T11:05:00Z")
    );
}

#[path = "support/legacy_message_body.rs"]
mod legacy_message_body;

#[test]
fn optional_message_participants_preserve_legacy_scalar_types_and_typed_relay() {
    let bytes = fixture("timeline-message-participants.json");
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let page: Envelope<TimelinePage> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_value(&page).unwrap(),raw);
    for entry in raw["value"]["items"].as_array().unwrap() {
        let old: legacy_message_body::TimelineMessageBody = serde_json::from_value(entry["body"].clone()).unwrap();
        let mut expected = entry["body"].clone(); expected.as_object_mut().unwrap().remove("author");
        expected.as_object_mut().unwrap().remove("recipients");
        assert_eq!(serde_json::to_value(old).unwrap(),expected);
    }
    let changes = serde_json::json!({"kind":"conversation-changes", "session_id":raw["value"]["session_id"],
        "items":raw["value"]["items"], "next_cursor":"conversation-cursor/participants/next"});
    let typed: ConversationChanges = serde_json::from_value(changes.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(),changes);
}

#[tokio::test]
async fn a_relayed_plugin_read_refuses_paths_outside_the_plugin_routes() {
    let client = Client::unix(Path::new("/nonexistent/fractal-plugin-read.sock"));
    let refused = |result: &Result<serde_json::Value, ClientError>| {
        matches!(result, Err(ClientError::Protocol(message)) if message.contains("not a GitHub plugin read"))
    };
    for path in [
        "/v1/plugins/github/repos/acme/../garden/pulls",
        "/v1/plugins/github/repos/./acme/garden/pulls",
        "/v1/client/agents",
        "/v1/plugins/github/repos/acme/garden/pulls#12",
    ] {
        assert!(refused(&client.github_plugin_read(path).await), "{path}");
    }
    // A dotted value in the query names no segment: the read is sent, and finds no daemon here.
    let sent = client.github_plugin_read("/v1/plugins/github/repos/acme/garden/events?after=a..b").await;
    assert!(sent.is_err() && !refused(&sent), "{sent:?}");
}
