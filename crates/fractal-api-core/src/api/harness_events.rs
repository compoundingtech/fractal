use crate::AttentionSnapshotStore;
use crate::EvidenceSeatsStore;
use super::*;
use crate::harness_events::Publication;

pub async fn publish(
    State(state): State<AppState>,
    peer: Option<Extension<NativeDeliveryPeer>>,
    Json(request): Json<Publication>,
) -> Result<Json<ClaimRecord>, ApiError> {
    let Some(peer) = peer else {
        return Err(ApiError::bad(St3Error::new(
            "unbound-harness-event",
            "harness events require a local native driver",
        )));
    };
    if peer.agent != request.claim.subject
        || request.claim.actor.as_deref() != Some(&peer.agent)
        || !peer.archives_inbox && peer.transport != "pi-channel" && peer.transport != "omp-channel"
    {
        return Err(ApiError::bad(St3Error::new(
            "foreign-harness-event",
            "only this seat's native driver can publish its observations",
        )));
    }
    let store = state.store.clone();
    let role_state = state.clone();
    let kind = request.claim.kind.clone();
    let peer = peer.0;
    let (record, changed, transition) = blocking_action(move || {
        // Preserve the canonical stale-runtime refusal before resolving any role evidence
        // for the requested generation. The append transaction checks this fence again.
        crate::store::check_harness_event_runtime(
            &store.readers.get(), &request.claim.subject, Some(&request.runtime_incarnation),
        )?;
        let fence = crate::mailbox::Fence::new(
            &request.claim.subject, &request.runtime_incarnation, "events",
        );
        let proof = super::native_role::capture(&role_state, &fence, &peer)?;
        proof.require_publication(&request)?;
        store.append_harness_event_publication_checked(&request, &|| {
            proof.validate()?;
            proof.require_publication(&request)
        })
    }).await?;
    finish_claim_publication(&state, &kind, record, changed, Some(transition)).await
}

#[derive(Debug, Deserialize)]
pub struct PromptStateQuery {
    agent: String,
    /// The hook's own observation: its state record's ownership and transition sequences.
    ownership: u64,
    transition: u64,
}

/// Where a waiting prompt hook's prompt stands: open, answered by a person (and how), or gone.
/// The hook asks its own host's daemon, which holds the person's answer once it replicates.
pub async fn prompt_state(
    State(state): State<AppState>,
    Query(query): Query<PromptStateQuery>,
) -> Result<Json<crate::store::NativePromptState>, ApiError> {
    let store = state.store.clone();
    let prompt = blocking_action(move || {
        store
            .native_prompt_state(&query.agent, query.ownership, query.transition)
            .map_err(|error| St3Error::new("internal", format!("{error:#}")))
    })
    .await?;
    Ok(Json(prompt))
}

#[derive(Debug, Deserialize)]
pub struct PromptAnswerQuery {
    agent: String,
    /// The claim that raised the prompt or denial.
    episode: String,
}

/// The person's answer to a prompt or denial the seat's hook raised, once recorded. A hook asks
/// its own host's daemon, which holds the answer once it replicates.
pub async fn prompt_answer(
    State(state): State<AppState>,
    Query(query): Query<PromptAnswerQuery>,
) -> Result<Json<crate::store::PromptAnswer>, ApiError> {
    let store = state.store.clone();
    let answer = blocking_action(move || {
        // Only an episode of this seat answers its hook.
        let claim = store
            .claim_by_id(&query.episode)
            .map_err(|error| St3Error::new("internal", format!("{error:#}")))?;
        if claim.is_none_or(|claim| claim.subject != query.agent) {
            return Ok(crate::store::PromptAnswer { answer: None });
        }
        store
            .prompt_answer(&query.episode)
            .map_err(|error| St3Error::new("internal", format!("{error:#}")))
    })
    .await?;
    Ok(Json(answer))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };

    pub const SEAT: &str = "agent/example/native-wake";

    pub fn peer(agent: &str) -> NativeDeliveryPeer {
        let (socket, _connector) = tokio::net::UnixStream::pair().unwrap();
        let identity = local_peer_process(&socket).unwrap();
        NativeDeliveryPeer {
            agent: agent.into(),
            transport: "claude-channel",
            pid: identity.pid(),
            archives_inbox: true,
            peer_process: Some(identity),
        }
    }

    /// These publication tests start with an already admitted live connector, not a new launch.
    pub fn admitted_connector(state: &AppState, subject: &str, incarnation: &str) {
        use crate::DeliveryStore as _;
        let source = format!(
            "version 2\nagent {:?} {{ host {:?}; workspace \"/tmp\"; harness \"claude\" {{}} }}",
            subject.strip_prefix("agent/").unwrap(), state.node,
        );
        let intent = crate::graph::parse_test_intent(&source, state.store.origin()).unwrap();
        state.store.apply_internal(&intent, "event-connector-fixture").unwrap();
        let fence = crate::mailbox::Fence::new(subject, incarnation, "delivery");
        let dir = super::super::native_role::source_agent_dir(state, &fence);
        fractal_drivers::harness_events::enable(&dir, incarnation).unwrap();
        let now = client_now_ms() as u64;
        fractal_drivers::harness_events::write_snapshot(&dir, "harness-state",
            &serde_json::to_vec(&json!({
                "schema":"st2.harness-state.v1", "agent":subject, "harness":"claude",
                "state":"active", "blockedOn":"none", "inputBuffer":"empty",
                "incarnation":"event-provider", "seq":1,
                "sinceMs":now, "writtenAtMs":now, "transitions":1
            })).unwrap(),
        ).unwrap();
        let owner = crate::mailbox::Authority {
            provider: "claude".into(), session: "event-provider".into(), sequence: 1,
            pid: std::process::id(),
            process_token: fractal_runtime::process_start_token(std::process::id()).unwrap(),
        };
        state.store.bind_mailbox_with_lease(&fence, Some(&owner)).unwrap();
    }

    pub fn runtime(state: &AppState, incarnation: &str) {
        state
            .store
            .append_claim(&ClaimInput {
                subject: SEAT.into(),
                kind: "runtime.observed".into(),
                actor: Some(SEAT.into()),
                fields: BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("incarnation_id".into(), json!(incarnation)),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        admitted_connector(state, SEAT, incarnation);
    }

    pub fn event(sequence: u64, state: &str) -> Publication {
        Publication {
            runtime_incarnation: "native-one".into(),
            sequence,
            claim: ClaimInput {
                subject: SEAT.into(),
                kind: "harness.observed".into(),
                actor: Some(SEAT.into()),
                fields: BTreeMap::from([
                    ("state".into(), json!(state)),
                    ("driver".into(), json!("claude")),
                    ("incarnation_id".into(), json!("native-one")),
                    ("evidence_incarnation".into(), json!("event-provider")),
                    ("ownership_sequence".into(), json!(1)),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(format!("native-{sequence}")),
            },
        }
    }

    pub async fn post(
        state: &AppState,
        event: &Publication,
        agent: &str,
    ) -> (StatusCode, Value) {
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/harness-events")
                    .header("content-type", "application/json")
                    .extension(peer(agent))
                    .body(Body::from(serde_json::to_vec(event).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let envelope: Value = serde_json::from_slice(&body).unwrap();
        let value = if status.is_success() {
            envelope["value"].clone()
        } else {
            envelope
        };
        (status, value)
    }

    pub async fn notified(state: &AppState) -> bool {
        tokio::time::timeout(Duration::from_millis(20), state.notify.notified())
            .await
            .is_ok()
    }

    /// The member CPU target's seat check reads what a native driver's events write.
    #[tokio::test]
    async fn the_cpu_seat_check_sees_a_native_drivers_events() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        // A minute after now, so every event below is before it.
        let busy = |state: &AppState| {
            let from = (client_now_ms() as u64 / 60_000 + 2) * 60_000;
            state.store.local_seats_busy(from, from + 60_000, None).unwrap().0
        };
        runtime(&state, "native-one");
        assert!(busy(&state), "a running seat with no observation is not quiet");
        for (sequence, status, working) in [(1, "ready", false), (2, "working", true), (3, "idle", false), (4, "blocked", false)] {
            let (code, admitted) = post(&state, &event(sequence, status), SEAT).await;
            assert_eq!(code, StatusCode::OK, "{admitted}");
            assert_eq!(busy(&state), working, "{status}");
        }
    }

    #[tokio::test]
    async fn queued_observation_disposition_does_not_relax_publication_session_freshness() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        runtime(&state, "native-one");
        for (session, sequence) in [("predecessor", 1), ("event-provider", 2)] {
            let mut input = event(1, "working");
            input.claim.fields.insert("evidence_incarnation".into(), json!(session));
            input.claim.fields.insert("ownership_sequence".into(), json!(sequence));
            let (code, rejected) = post(&state, &input, SEAT).await;
            assert!(!code.is_success(), "{rejected}");
            assert!(rejected.to_string().contains("native-role-refused"), "{rejected}");
            assert!(state.store.local_observations_tail(100).unwrap().is_empty());
        }
        let (code, accepted) = post(&state, &event(1, "idle"), SEAT).await;
        assert_eq!(code, StatusCode::OK, "{accepted}");
        assert_eq!(state.store.local_observations_tail(100).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn native_readiness_and_idle_events_wake_reconcile_but_replay_does_not() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        runtime(&state, "native-one");
        for (sequence, status) in [(1, "ready"), (2, "working"), (3, "idle"), (4, "blocked")] {
            let input = event(sequence, status);
            let (code, admitted) = post(&state, &input, SEAT).await;
            assert_eq!(code, StatusCode::OK, "{admitted}");
            assert!(
                admitted["id"]
                    .as_str()
                    .unwrap()
                    .starts_with("local-observation/")
            );
            assert!(
                notified(&state).await,
                "native {status} must wake reconcile"
            );
            assert_eq!(
                state.store.current_harness(SEAT).unwrap().unwrap().state,
                status
            );
            let (code, replay) = post(&state, &input, SEAT).await;
            assert_eq!(code, StatusCode::OK, "{replay}");
            assert_eq!(replay, admitted);
            assert!(!notified(&state).await, "exact replay stays quiet");
        }
    }

    #[tokio::test]
    async fn native_usage_and_rejected_or_stale_events_do_not_wake_reconcile() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        runtime(&state, "native-one");
        let mut usage = event(1, "working");
        usage.claim.kind = "harness.usage".into();
        usage.claim.fields.remove("state");
        usage.claim.fields.remove("evidence_incarnation");
        usage.claim.fields.remove("ownership_sequence");
        usage
            .claim
            .fields
            .insert("semantics".into(), json!("context_occupancy"));
        usage
            .claim
            .fields
            .insert("context_used_tokens".into(), json!(10));
        let (code, body) = post(&state, &usage, SEAT).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert!(!notified(&state).await);
        let (code, body) = post(&state, &event(2, "idle"), "agent/example/other").await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(!notified(&state).await);
        runtime(&state, "native-two");
        let (code, body) = post(&state, &event(3, "ready"), SEAT).await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["code"], "stale-harness-event-session");
        assert!(!notified(&state).await);
    }
}

#[cfg(test)]
mod heartbeat_tests {
    use super::tests::{SEAT, event, notified, post, runtime};
    use super::*;

    #[tokio::test]
    async fn native_same_state_heartbeats_do_not_wake_even_without_echoed_transition_metadata() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        runtime(&state, "native-one");
        let (code, first) = post(&state, &event(1, "working"), SEAT).await;
        assert_eq!(code, StatusCode::OK, "{first}");
        assert!(notified(&state).await);
        for sequence in [2, 3] {
            let mut heartbeat = event(sequence, "working");
            if sequence == 3 {
                // A changed auxiliary reading publishes history, still with status_transition:false.
                heartbeat
                    .claim
                    .fields
                    .insert("background_jobs".into(), json!(1));
            }
            let (code, admitted) = post(&state, &heartbeat, SEAT).await;
            assert_eq!(code, StatusCode::OK, "{admitted}");
            assert!(
                admitted["body"]["fields"]
                    .get("status_transition")
                    .is_none(),
                "native admission echoes the producer fields unchanged"
            );
            assert!(
                !notified(&state).await,
                "same-state native heartbeat stays quiet"
            );
            let history = state
                .store
                .latest_claim(SEAT, Some("harness.observed"))
                .unwrap()
                .unwrap();
            assert_eq!(history.body["fields"]["status_transition"], sequence == 2);
        }
        let (code, idle) = post(&state, &event(4, "idle"), SEAT).await;
        assert_eq!(code, StatusCode::OK, "{idle}");
        assert!(notified(&state).await);
    }
}

#[cfg(test)]
mod acknowledgement_budget_tests {
    use super::*;
    use std::future::Future;
    use std::task::Poll;

    #[test]
    fn admitted_non_timeline_events_need_zero_more_blocking_tasks_to_acknowledge() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let root = tempfile::tempdir().unwrap();
            let state = super::super::tests::state(root.path());
            super::tests::runtime(&state, "native-one");
            let input = super::tests::event(1, "ready");
            let (record, appended, transition) = state
                .store
                .append_harness_event_publication(&input)
                .unwrap();
            let (ready, waiting) = std::sync::mpsc::channel();
            let (release, held) = std::sync::mpsc::channel();
            struct Release(Option<std::sync::mpsc::Sender<()>>);
            impl Drop for Release {
                fn drop(&mut self) {
                    if let Some(sender) = self.0.take() {
                        let _ = sender.send(());
                    }
                }
            }
            let guard = Release(Some(release));
            let blocker = tokio::task::spawn_blocking(move || {
                ready.send(()).unwrap();
                let _ = held.recv();
            });
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            // Count scheduling hops, not noisy wall time: the sole blocking worker is occupied.
            // A second task would leave this admitted write waiting after its durable commit.
            for is_retry in [false, true] {
                let future = finish_claim_publication(
                    &state,
                    &input.claim.kind,
                    record.clone(),
                    appended && !is_retry,
                    Some(transition),
                );
                tokio::pin!(future);
                let response =
                    std::future::poll_fn(|context| match future.as_mut().poll(context) {
                        Poll::Ready(value) => Poll::Ready(value),
                        Poll::Pending => panic!(
                            "non-timeline acknowledgement exceeded its zero blocking-hop CI budget"
                        ),
                    })
                    .await
                    .unwrap()
                    .0;
                assert_eq!(response.id, record.id);
            }
            drop(guard);
            blocker.await.unwrap();
        });
    }
}
