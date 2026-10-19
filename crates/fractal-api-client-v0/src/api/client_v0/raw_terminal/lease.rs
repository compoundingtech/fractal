//! The part of `api::client_v0::raw_terminal::lease` that moved up from st3-api.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::MissionsStore;
use super::*;
use rusqlite::{Connection, OptionalExtension as _, params};
use std::sync::{LazyLock, Mutex};
use tokio::time::Instant;
pub use crate::peer::{RAW_ORIGIN_HEADER as ORIGIN_HEADER, RawLeaseBinding as Binding};
pub use fractal_api_core::api::client_v0::raw_terminal::lease::*;
pub use fractal_api_core::api::client_v0::raw_terminal::lease::{ABSOLUTE, Clock, Control, IDLE, Lease, MembershipAuthority, WATCH, authorization_epoch};

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, AppState, ClientSession) {
        let root = tempfile::tempdir().unwrap();
        let state = AppState {
            owning_person: None,
            store: Arc::new(Store::open_memory("lease-owner").unwrap()),
            notify: Arc::new(Notify::new()),
            event_notify: watch::channel(0).0,
            node: "lease-owner".into(),
            state_dir: root.path().into(),
            pty_root: root.path().join("pty"),
            pty_binary: root.path().join("unused-pty"),
            fleet_id: None,
            configured_peers: Vec::new(),
            client_relay: None,
            native_session_home: None,
            planner_default: crate::model::PlannerSpec::default(),
        };
        claim(
            &state,
            "agent/shell",
            "runtime.observed",
            json!({"status":"running","runtime_id":"runtime-shell","incarnation_id":"incarnation-one","terminal":true}),
        );
        (
            root,
            state,
            ClientSession::local(Some("person/alex")).unwrap(),
        )
    }

    fn claim(state: &AppState, subject: &str, kind: &str, fields: Value) {
        state
            .store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: kind.into(),
                actor: None,
                fields: fields
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }

    fn pairing(state: &AppState, subject: &str, credential: &str, scopes: Value) {
        claim(state, subject, "custom.client.pairing-completed", json!({
            "person_id":"person/alex",
            "session_actor":"person/alex/session/same-key",
            "credential_hash":credential_digest(credential),
            "scopes":scopes,
            "expires_at_unix_ms":client_now_ms()+600_000
        }));
    }

    fn authenticated(state: &AppState, credential: &str) -> ClientSession {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/client/terminals/agent/shell/attachments")
            .header(AUTHORIZATION, format!("Bearer {credential}"))
            .body(Body::empty()).unwrap();
        authenticate(state, &request, "fabric-loopback").unwrap()
    }

    #[tokio::test]
    async fn same_key_pairings_keep_the_authenticated_grant() {
        let (_root, state, _) = fixture();
        pairing(&state, "custom/client/first", "first", json!(["terminal.read"]));
        let first = authenticated(&state, "first");
        let epoch = authorization_epoch(&state, &first).unwrap();
        pairing(&state, "custom/client/second", "second", json!(["read.projections"]));
        let first = authenticated(&state, "first");
        assert_eq!(authorization_epoch(&state, &first).unwrap(), epoch);
        assert!(revalidate_session(&state, &first).unwrap().allows("terminal.read"));
        let response = super::super::attachment(
            State(state.clone()), Extension(first.clone()),
            AxumPath("agent/shell".into()),
            Json(super::super::AttachmentRequest {
                runtime_incarnation: "incarnation-one".into(),
                mode: fractal_client::RawTerminalMode::Peek,
            }),
        ).await.unwrap();
        assert_eq!(response.0["mode"], "peek");
        claim(&state, "custom/client/second", "custom.client.pairing-revoked", json!({}));
        assert_eq!(authorization_epoch(&state, &first).unwrap(), epoch);
        claim(&state, "custom/client/first", "custom.client.pairing-revoked", json!({}));
        assert_eq!(authorization_epoch(&state, &first).unwrap_err().code, "forbidden");
        assert!(revalidate_session(&state, &first).is_err());
    }

    #[test]
    fn same_key_pairings_do_not_borrow_another_issuer() {
        let (_root, issuer, _) = fixture();
        pairing(&issuer, "custom/client/issuer-first", "first", json!(["terminal.read"]));
        let (_other_root, mut gateway, _) = fixture();
        gateway.store = Arc::new(Store::open_memory("other-member").unwrap());
        gateway.node = "other-member".into();
        gateway.store.import_replication(
            "lease-owner", &issuer.store.export_replication(0).unwrap(),
        ).unwrap();
        pairing(&gateway, "custom/client/gateway-second", "second", json!(["terminal.read"]));
        let first = authenticated(&gateway, "first");
        assert_eq!(authorization_epoch(&gateway, &first).unwrap_err().code, "forbidden");
        let second = authenticated(&gateway, "second");
        authorization_epoch(&gateway, &second).unwrap();
    }

    fn acquire(state: &AppState, session: &ClientSession) -> Arc<Lease> {
        Lease::register(
            state,
            session,
            "terminal/agent/shell",
            "host/lease-owner",
            "incarnation-one",
            None,
            &authorization_epoch(state, session).unwrap(),
        )
        .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn lease_expiry_wakes_without_a_reader_when_admission_is_saturated() {
        let (_root, state, session) = smallclaims::sqlite::with_read_limit_for_test(1, fixture);
        // Both leases are registered before the holder saturates the only reader slot:
        // registration itself runs authority queries.
        let lease = acquire(&state, &session);
        let sibling = acquire(&state, &session);
        let store = state.store.clone();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (entered, started) = std::sync::mpsc::channel::<()>();
        let holder_store = store.clone();
        let holder = std::thread::spawn(move || {
            holder_store.readers.request_read(|| {
                entered.send(()).unwrap();
                held.recv_timeout(std::time::Duration::from_secs(30)).ok();
            })
        });
        started.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(
            store.readers.try_admit_read().is_none(),
            "the holder owns the only admission slot"
        );
        let held_usage = store.readers.usage();
        // Clock expiry and cancellation must both complete while every reader slot is
        // busy: the lease timer is pure local state and never queues for admission.
        let woken_by_clock = lease.expired();
        tokio::time::advance(Duration::from_secs(61)).await;
        let Ok(()) = tokio::time::timeout(Duration::from_secs(5), woken_by_clock).await else {
            panic!("lease expiry must not wait for reader admission");
        };
        assert_eq!(store.readers.usage().open, held_usage.open, "no reader was opened");
        assert_eq!(store.readers.usage().peak, held_usage.peak, "no transient reader was opened");
        // A second lease proves cancellation wakes the same pure timer path.
        let woken_by_cancel = sibling.expired();
        sibling.cancel("superseded");
        let Ok(()) = tokio::time::timeout(Duration::from_secs(5), woken_by_cancel).await else {
            panic!("lease cancellation must not wait for reader admission");
        };
        assert_eq!(store.readers.usage().open, held_usage.open, "no reader was opened");
        assert_eq!(store.readers.usage().peak, held_usage.peak, "no transient reader was opened");
        drop(release);
        holder.join().unwrap().unwrap();
    }

    #[test]
    fn partial_runtime_reports_preserve_leases_but_explicit_nulls_revoke() {
        for null_fields in [
            json!({"status":null}),
            json!({"incarnation_id":null}),
        ] {
            let (_root, state, session) = fixture();
            let existing = acquire(&state, &session);
            claim(&state, "agent/shell", "runtime.observed", json!({"reason":"partial report"}));
            assert!(existing.check().is_ok(), "omitted runtime fields must retain authority");
            let replacement = acquire(&state, &session);
            claim(&state, "agent/shell", "runtime.observed", null_fields);
            assert!(existing.check().is_err(), "explicit null must revoke the old lease");
            assert!(replacement.check().is_err(), "explicit null must revoke the new lease");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn idle_expires_at_sixty_seconds_despite_authority_proofs() {
        let (_root, state, session) = fixture();
        let lease = acquire(&state, &session);
        for sequence in 1..60 {
            tokio::time::advance(Duration::from_secs(1)).await;
            lease.proof(&lease.binding.gateway_epoch, sequence).unwrap();
        }
        assert!(lease.check().is_ok());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(
            lease
                .check()
                .unwrap_err()
                .to_string()
                .contains("lease-idle-expired")
        );
        assert!(
            lease.selected_use(1).is_err(),
            "expired streams cannot be resurrected"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn foreground_renewal_never_extends_absolute_and_fresh_rotation_is_independent() {
        let (_root, state, session) = fixture();
        let old = acquire(&state, &session);
        let mut replacement = None;
        for second in 1..300 {
            tokio::time::advance(Duration::from_secs(1)).await;
            old.proof(&old.binding.gateway_epoch, second).unwrap();
            if second % 20 == 0 {
                old.selected_use(second / 20).unwrap();
            }
            if second == 290 {
                replacement = Some(acquire(&state, &session));
            }
            if let Some(new) = &replacement {
                new.proof(&new.binding.gateway_epoch, second).unwrap();
            }
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(
            old.check()
                .unwrap_err()
                .to_string()
                .contains("lease-absolute-expired")
        );
        assert!(replacement.as_ref().unwrap().check().is_ok());
        assert_ne!(
            old.binding.lease_id,
            replacement.as_ref().unwrap().binding.lease_id
        );
        assert!(old.selected_use(99).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn watch_loss_closes_at_five_seconds_and_replays_do_not_refresh() {
        let (_root, state, session) = fixture();
        let lease = acquire(&state, &session);
        lease.proof(&lease.binding.gateway_epoch, 2).unwrap();
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(lease.proof(&lease.binding.gateway_epoch, 2).is_err());
        assert!(lease.proof(&lease.binding.gateway_epoch, 1).is_err());
        assert!(lease.proof("restarted-watch", 3).is_err());
        lease.selected_use(1).unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(
            lease
                .check()
                .unwrap_err()
                .to_string()
                .contains("lease-watch-lost")
        );
    }

    #[tokio::test]
    async fn principal_and_incarnation_mutations_revoke_before_acknowledgement() {
        let (_root, state, session) = fixture();
        let old = acquire(&state, &session);
        claim(
            &state,
            "person/alex",
            "principal.key-revoked",
            json!({"key":"old-key","reason":"test"}),
        );
        assert!(
            old.check().is_err(),
            "mutation returned before lease invalidation"
        );
        let replacement = acquire(&state, &session);
        claim(
            &state,
            "agent/shell",
            "runtime.observed",
            json!({"status":"running","runtime_id":"runtime-shell","incarnation_id":"incarnation-two","terminal":true}),
        );
        assert!(
            replacement.check().is_err(),
            "incarnation mutation returned before invalidation"
        );
        assert!(old.check().is_err());
    }

    #[tokio::test]
    async fn paired_actor_restriction_and_revocation_invalidate_all_generations() {
        let (_root, state, mut session) = fixture();
        session.actor = "client/device-one".into();
        session.pairing_grant = Some("custom/client/device-one".into());
        let grant = json!({"session_actor":session.actor,"person_id":session.authority_actor,"scopes":["terminal.read"],"expires_at_unix_ms":client_now_ms()+600_000,"credential_hash":"digest"});
        claim(
            &state,
            "custom/client/device-one",
            "custom.client.pairing-completed",
            grant.clone(),
        );
        let old = acquire(&state, &session);
        let replacement = acquire(&state, &session);
        claim(
            &state,
            "custom/client/device-one",
            "custom.client.pairing-revoked",
            json!({"device_id":"device-one"}),
        );
        assert!(old.check().is_err());
        assert!(replacement.check().is_err());
        assert!(
            Lease::register(
                &state,
                &session,
                "terminal/agent/shell",
                "host/lease-owner",
                "incarnation-one",
                None,
                &old.acquisition_epoch
            )
            .is_err()
        );
        session.actor = "client/device-two".into();
        session.pairing_grant = Some("custom/client/device-two".into());
        let mut restricted = grant;
        restricted["session_actor"] = json!(session.actor);
        restricted["scopes"] = json!([]);
        claim(
            &state,
            "custom/client/device-two",
            "custom.client.pairing-completed",
            restricted,
        );
        assert!(
            Lease::register(
                &state,
                &session,
                "terminal/agent/shell",
                "host/lease-owner",
                "incarnation-one",
                None,
                &old.acquisition_epoch
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn owner_restriction_of_original_grant_revokes_forwarded_actor() {
        let (_root, state, session) = fixture();
        let local = acquire(&state, &session);
        let mut binding = local.binding.clone();
        binding.actor = "client/original".into();
        binding.grant_subject = Some("custom/client/original".into());
        let grant = json!({"fields":{"session_actor":"client/original","person_id":"person/alex","scopes":["terminal.read"],"expires_at_unix_ms":client_now_ms()+600_000}});
        binding.grant_digest = Some(credential_digest(&serde_json::to_string(&grant).unwrap()));
        let forwarded = Lease::register(
            &state,
            &session,
            "terminal/agent/shell",
            "host/lease-owner",
            "incarnation-one",
            Some(binding),
            &authorization_epoch(&state, &session).unwrap(),
        )
        .unwrap();
        assert!(forwarded.check().is_ok());
        claim(
            &state,
            "custom/client/original",
            "custom.client.pairing-completed",
            json!({
                "session_actor":"client/original","person_id":"person/alex","scopes":[],
                "expires_at_unix_ms":client_now_ms()+600_000
            }),
        );
        assert!(
            forwarded.check().is_err(),
            "owner restriction must invalidate the original subject/digest"
        );
    }

    #[tokio::test]
    async fn non_issuer_pairing_revoke_fails_without_committing() {
        let (_root, mut state, session) = fixture();
        let subject = "custom/client/issuer-only";
        claim(
            &state,
            subject,
            "custom.client.pairing-completed",
            json!({
                "device_id":"device/issuer-only",
                "person_id":"person/alex",
                "session_actor":"client/issuer-only",
                "scopes":["terminal.read"],
                "expires_at_unix_ms":client_now_ms()+600_000
            }),
        );
        // The claim keeps its issuer origin while the API runs on another member.
        state.node = "other-member".into();
        let snapshot = new_client_snapshot(&state);
        let request = ActionRequest {
            api_version: CLIENT_API_VERSION.into(),
            id: "action/issuer-only".into(),
            action_type: "pairing.revoke".into(),
            idempotency_key: "issuer-only".into(),
            fence: Fence {
                snapshot_id: snapshot.id.clone(),
                ..Default::default()
            },
            parameters: json!({"target_id":"device/issuer-only"}),
        };
        let error = dispatch_action(&state, &snapshot, &session, &request)
            .await
            .unwrap_err();
        assert_eq!(error.code, "issuer-required");
        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(
            error.details.get("issuer_host_id"),
            Some(&json!("host/lease-owner"))
        );
        assert!(
            state
                .store
                .claims_for_subject_kind_at(subject, "custom.client.pairing-revoked", None, true, 1)
                .unwrap()
                .claims
                .is_empty()
        );
        state.node = "lease-owner".into();
        dispatch_action(&state, &new_client_snapshot(&state), &session, &request)
            .await
            .unwrap();
        assert_eq!(
            state
                .store
                .claims_for_subject_kind_at(subject, "custom.client.pairing-revoked", None, true, 1)
                .unwrap()
                .claims
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn principal_key_changes_move_the_authorization_epoch() {
        let (_root, state, session) = fixture();
        let acquired = authorization_epoch(&state, &session).unwrap();
        claim(
            &state,
            "person/alex",
            "principal.key-revoked",
            json!({"key":"acquired-key","reason":"race"}),
        );
        assert_ne!(authorization_epoch(&state, &session).unwrap(), acquired);
    }

    #[tokio::test]
    async fn an_acquired_capability_cannot_adopt_a_later_principal_epoch() {
        let (_root, state, session) = fixture();
        let acquired = authorization_epoch(&state, &session).unwrap();
        // No lease is registered yet: invalidate between consumption and registration.
        claim(
            &state,
            "person/alex",
            "principal.key-revoked",
            json!({"key":"acquired-key","reason":"race"}),
        );
        assert!(
            Lease::register(
                &state,
                &session,
                "terminal/agent/shell",
                "host/lease-owner",
                "incarnation-one",
                None,
                &acquired
            )
            .is_err()
        );
        let fresh = acquire(&state, &session);
        assert!(
            fresh.check().is_ok(),
            "only a fresh authorization may acquire the new epoch"
        );
    }
}
