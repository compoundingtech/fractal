//! The part of `store::seat_status` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_store::store::seat_status::*;
pub use st3_store::store::seat_status::{MAX_TRANSITIONS, WINDOW_MS, enrich_harness, history_at, history_at_with_sources, history_source, observation_time, permission_blocked, state_run_since, transition_positions};

pub const STALE_MS: u128 = 90_000;

pub trait SeatStatusStore {
    /// Freshness is a read-time fact, never part of the replicated projection digest.
    fn seat_observation_at(
            &self,
            subject: &str,
            harness: Option<&crate::model::CurrentHarnessView>,
            index: u64,
            now: u128,
        ) -> Result<&'static str>;
    fn seat_status_history(&self, subject: &str, now: u128) -> Result<Value>;
    fn seat_status_history_at(
            &self,
            subject: &str,
            now: u128,
            index: u64,
        ) -> Result<Value>;
}

impl SeatStatusStore for Store {

    fn seat_observation_at(
            &self,
            subject: &str,
            harness: Option<&crate::model::CurrentHarnessView>,
            index: u64,
            now: u128,
        ) -> Result<&'static str> {
        let Some(harness) = harness else {
            return Ok("missing");
        };
        let connection = self.readers.get();
        let claim = claim_by_id_tx(&connection, &harness.claim)?;
        let mut observed = claim
            .as_ref()
            .map_or(harness.observed_at_unix_ms, observation_time);
        let prompt_query = newest_claims_of_kind_query("claims.accepted_at_unix_ms", "harness.diagnostic")
            .replace(" ORDER BY", " AND json_extract(claims.body,'$.fields.incarnation_id')=?3 AND json_extract(claims.body,'$.fields.code') IN ('provider-auth-expired','provider-trust-prompt','provider-auth-restored','provider-update-prompt','provider-update-restored') ORDER BY") + " LIMIT 1";
        let prompt: Option<String> = connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-hb50a9603a7cb owner=st3_seats::store::seat_status TODO=trace-statement-and-value-representation */ &prompt_query)?
            .query_row(/* sqlite-large-value: allow-for-now lv-he12dd6e66d38 owner=st3_seats::store::seat_status TODO=trace-statement-and-value-representation */ params![subject, index, harness.incarnation_id], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(prompt) = prompt {
            observed = observed.max(prompt.parse()?);
        }
        let local: Option<(String, u128)> = connection
            .prepare_cached(
                "SELECT body, observed_at_unix_ms FROM local_observations
             WHERE subject=?1 AND kind='harness.observed' AND after_store_index<=?2
               AND json_extract(body, '$.fields.incarnation_id')=?3
             ORDER BY id DESC LIMIT 1",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-he4836e305e93 owner=st3_seats::store::seat_status TODO=trace-statement-and-value-representation */ params![subject, index, harness.incarnation_id], |row| {
                Ok((row.get(0)?, row.get::<_, i64>(1)? as u128))
            })
            .optional()?;
        if let Some((body, accepted)) = local {
            let body: Value = serde_json::from_str(&body)?;
            let at = body
                .pointer("/fields/observed_at_ms")
                .and_then(Value::as_u64)
                .map_or(accepted, u128::from)
                .min(accepted);
            observed = observed.max(at);
        }
        Ok(if now.saturating_sub(observed) > STALE_MS {
            "stale"
        } else {
            "current"
        })
    }

    fn seat_status_history(&self, subject: &str, now: u128) -> Result<Value> {
        self.seat_status_history_at(subject, now, i64::MAX as u64)
    }

    fn seat_status_history_at(
            &self,
            subject: &str,
            now: u128,
            index: u64,
        ) -> Result<Value> {
        let connection = self.readers.get();
        history_at(&connection, subject, now, index)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use st3_store::store::seat_status_tests_shared_seats::*;
    use st3_store::store::seat_status_tests_shared_seats::{checked_harness, input, observe, runtime};

    #[test]
    fn approval_observation_is_blocked_in_current_then_clears() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms();
        observe(&store, "one", "working", at, "working");
        store
            .append_latest_observation(
                &input(
                    "harness.observed",
                    json!({
                        "state":"working", "incarnation_id":"one", "observed_at_ms":(at + 1) as u64,
                        "blocked_on":"human", "ask":"permission", "reason":"waitingOnApproval"
                    }),
                ),
                at + 1,
            )
            .unwrap();
        let blocked = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(blocked.state, "blocked");
        assert_eq!(blocked.blocked_on.as_deref(), Some("human"));
        assert_eq!(blocked.ask.as_deref(), Some("permission"));
        assert_eq!(blocked.reason.as_deref(), Some("waitingOnApproval"));

        let sparse = store
            .append_latest_observation(
                &input(
                    "harness.observed",
                    json!({
                        "state":"working", "incarnation_id":"one", "observed_at_ms":(at + 2) as u64,
                        "reason":"stillWaiting"
                    }),
                ),
                at + 2,
            )
            .unwrap()
            .0;
        assert_eq!(sparse.body["fields"]["status_transition"], false);
        assert_eq!(checked_harness(&store, "agent/cedar").unwrap().unwrap().state, "blocked");

        store
            .append_latest_observation(
                &input(
                    "harness.observed",
                    json!({
                        "state":"working", "incarnation_id":"one", "observed_at_ms":(at + 3) as u64,
                        "blocked_on":null, "ask":null, "reason":null
                    }),
                ),
                at + 3,
            )
            .unwrap();
        let resumed = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(resumed.state, "working");
        assert!(resumed.blocked_on.is_none());

        store
            .append_latest_observation(
                &input(
                    "harness.observed",
                    json!({
                        "state":"working", "incarnation_id":"one", "observed_at_ms":(at + 4) as u64,
                        "blocked_on":"human", "ask":"question", "reason":"waitingOnUserInput"
                    }),
                ),
                at + 4,
            )
            .unwrap();
        assert_eq!(
            checked_harness(&store, "agent/cedar").unwrap().unwrap().state,
            "working"
        );
        let history = store.seat_status_history("agent/cedar", at + 5).unwrap();
        let states = history["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["state"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(states, ["working", "blocked", "working"]);

        runtime(&store, "two");
        observe(&store, "two", "working", at + 5, "working");
        let current = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(current.incarnation_id, "two");
        assert_eq!(current.state, "working");
        let history = store.seat_status_history("agent/cedar", at + 6).unwrap();
        let last = history["items"].as_array().unwrap().last().unwrap();
        assert_eq!(last["state"], "working");
        assert_eq!(last["runtime_incarnation"], "two");
    }

    #[test]
    fn terminal_observation_fences_permission_before_sparse_work_resumes() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms() - 1_000;
        let publish = |state: &str, fields: Value, time: u128| {
            let mut fields = fields.as_object().unwrap().clone();
            fields.insert("state".into(), json!(state));
            fields.insert("incarnation_id".into(), json!("one"));
            fields.insert("observed_at_ms".into(), json!(time as u64));
            store.append_latest_observation(&input("harness.observed", json!(fields)), time)
                .unwrap().0
        };
        publish("working", json!({"blocked_on":"human", "ask":"permission"}), at);
        assert_eq!(checked_harness(&store, "agent/cedar").unwrap().unwrap().state, "blocked");
        let ended = publish("ended", json!({"blocked_on":"human", "ask":"permission"}), at + 1);
        assert!(ended.body["fields"]["blocked_on"].is_null());
        assert!(ended.body["fields"]["ask"].is_null());
        let resumed = publish("working", json!({"reason":"resumed"}), at + 2);
        assert!(resumed.body["fields"]["blocked_on"].is_null());
        assert!(resumed.body["fields"]["ask"].is_null());
        assert_eq!(resumed.body["fields"]["status_transition"], true);
        assert_eq!(resumed.body["fields"]["observed_since_ms"], json!((at + 2) as u64));
        let current = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(current.state, "working");
        assert_eq!(current.since_unix_ms, at + 2);
        assert!(current.blocked_on.is_none());
        assert!(current.ask.is_none());
        let history = store.seat_status_history("agent/cedar", at + 3).unwrap();
        let states = history["items"].as_array().unwrap().iter()
            .filter_map(|item| item["state"].as_str()).collect::<Vec<_>>();
        assert_eq!(states, ["blocked", "ended", "working"]);
    }

    #[test]
    fn unknown_provider_auth_does_not_clear_a_known_rejection() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms();
        let rejected = store
            .append_latest_observation(
                &input("harness.observed", json!({
                    "state":"working", "incarnation_id":"one", "observed_at_ms":at as u64,
                    "provider_auth":false, "reason":"providerAuth"
                })),
                at,
            )
            .unwrap()
            .0;
        let uncertain = store
            .append_latest_observation(
                &input("harness.observed", json!({
                    "state":"working", "incarnation_id":"one", "observed_at_ms":(at + 1) as u64,
                    "provider_auth":null, "reason":"authUnknown"
                })),
                at + 1,
            )
            .unwrap()
            .0;
        assert_eq!(uncertain.body["fields"]["provider_auth"], false);
        assert_eq!(uncertain.body["fields"]["status_transition"], false);
        assert_eq!(
            uncertain.body["fields"]["observed_since_ms"],
            rejected.body["fields"]["observed_since_ms"]
        );
        assert_eq!(checked_harness(&store, "agent/cedar").unwrap().unwrap().state, "needs-login");
        let history = store.seat_status_history("agent/cedar", at + 2).unwrap();
        let states = history["items"].as_array().unwrap().iter()
            .filter_map(|item| item["state"].as_str()).collect::<Vec<_>>();
        assert_eq!(states, ["unauthenticated"]);
    }

    #[test]
    fn repeated_observations_and_detail_changes_preserve_since_and_refresh_freshness() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms() - 1_000;
        let first = observe(&store, "one", "idle", at, "ready");
        let second = observe(&store, "one", "idle", at + 1, "ready");
        assert!(local_observation_position(&second).is_some());
        observe(&store, "one", "idle", at + 2, "waiting");
        let harness = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(
            harness.since_unix_ms,
            first.body["fields"]["observed_since_ms"].as_u64().unwrap() as u128
        );
        let index = store.index().unwrap();
        assert_eq!(
            store
                .seat_observation_at("agent/cedar", Some(&harness), index, at + 90_002)
                .unwrap(),
            "current"
        );
        assert_eq!(
            store
                .seat_observation_at("agent/cedar", Some(&harness), index, at + 90_003)
                .unwrap(),
            "stale"
        );
        assert_eq!(
            store
                .seat_observation_at("agent/cedar", None, index, at)
                .unwrap(),
            "missing"
        );
        let history = store.seat_status_history("agent/cedar", at + 3).unwrap();
        assert_eq!(
            history["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["state"] == "idle")
                .count(),
            1
        );
        observe(&store, "one", "working", at + 4, "turn");
        assert!(
            store
                .current_harness("agent/cedar")
                .unwrap()
                .unwrap()
                .since_unix_ms
                == at + 4
        );
    }

    #[test]
    fn history_reports_runtime_reset_even_before_a_new_observation() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms();
        observe(&store, "one", "idle", at, "ready");
        runtime(&store, "two");
        assert!(checked_harness(&store, "agent/cedar").unwrap().is_none());
        let history = store.seat_status_history("agent/cedar", now_ms()).unwrap();
        let items = history["items"].as_array().unwrap();
        let reset = items.last().unwrap();
        assert_eq!(reset["runtime_incarnation"], "two");
        assert_eq!(reset["reset"], true);
        assert_eq!(reset["state"], Value::Null);
        assert_eq!(history["complete"], true);
    }

    #[test]
    fn history_is_bounded_by_count_and_age_and_reports_its_retained_boundary() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms();
        for index in 0..205 {
            observe(
                &store,
                "one",
                if index % 2 == 0 { "working" } else { "idle" },
                at,
                &format!("turn-{index}"),
            );
        }
        let history = store.seat_status_history("agent/cedar", at).unwrap();
        assert_eq!(history["items"].as_array().unwrap().len(), MAX_TRANSITIONS);
        assert_eq!(history["complete"], false);
        assert_eq!(history["retained_from"], history["items"][0]["observed_at"]);
        let aged = store
            .seat_status_history("agent/cedar", now_ms() + WINDOW_MS + 1)
            .unwrap();
        assert!(aged["items"].as_array().unwrap().is_empty());
        assert_eq!(aged["complete"], false);
    }

    #[test]
    fn status_historical_observations_do_not_restart_the_current_runtime_since() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms() - 1_000;
        observe(&store, "one", "idle", at, "ready");
        runtime(&store, "two");
        observe(&store, "two", "working", at + 1, "turn");
        observe(&store, "one", "working", at + 2, "historical turn");
        observe(&store, "two", "working", at + 3, "turn");
        let current = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(current.since_unix_ms, at + 1);
        let history = store.seat_status_history("agent/cedar", now_ms()).unwrap();
        let resets = history["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["reset"] == true)
            .count();
        assert_eq!(resets, 2, "only runtime replacements reset history");
    }

    #[test]
    fn status_history_restores_the_auth_fence_that_predates_the_window() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        observe(&store, "one", "idle", now_ms(), "ready");
        let diagnostic = |code| {
            store.append_claim(&input("harness.diagnostic", json!({
            "incarnation_id":"one", "code":code, "driver":"codex", "reason":"fixture", "severity":"warning"
        }))).unwrap()
        };
        diagnostic("provider-auth-expired");
        diagnostic("provider-update-prompt");
        let restored = diagnostic("provider-update-restored");
        let now = restored.accepted_at_unix_ms + WINDOW_MS + 1_000;
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE claims SET accepted_at_unix_ms=?1 WHERE id=?2",
                params![(now - 1).to_string(), restored.id],
            )
            .unwrap();
        let history = store.seat_status_history("agent/cedar", now).unwrap();
        assert_eq!(history["items"].as_array().unwrap().len(), 1);
        assert_eq!(history["items"][0]["state"], "unauthenticated");
        assert_eq!(history["complete"], false);
    }

    #[test]
    fn status_update_prompt_has_priority_and_preserves_since_until_restored() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        observe(&store, "one", "idle", now_ms(), "ready");
        let diagnostic = |code| {
            store.append_claim(&input("harness.diagnostic", json!({
            "incarnation_id":"one", "code":code, "driver":"codex", "reason":"fixture", "severity":"warning"
        }))).unwrap()
        };
        diagnostic("provider-auth-expired");
        let prompt = diagnostic("provider-update-prompt");
        diagnostic("provider-update-prompt");
        diagnostic("provider-auth-restored");
        observe(&store, "one", "idle", now_ms(), "parallel channel");
        let blocked = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(blocked.state, "blocked");
        assert_eq!(blocked.since_unix_ms, prompt.accepted_at_unix_ms);
        let restored = diagnostic("provider-update-restored");
        let idle = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(idle.state, "idle");
        assert_eq!(idle.since_unix_ms, restored.accepted_at_unix_ms);
        let history = store.seat_status_history("agent/cedar", now_ms()).unwrap();
        let states = history["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["state"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(states, ["idle", "unauthenticated", "blocked", "idle"]);
    }

    #[test]
    fn a_null_auth_successor_through_publication_keeps_the_login_episode() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms() - 1_000;
        let publish = |auth: Value, sequence: u64, time: u128| {
            store
                .append_latest_observation(
                    &input(
                        "harness.observed",
                        json!({
                            "incarnation_id":"one", "driver":"claude", "state":"idle",
                            "provider_auth":auth, "provider_auth_sequence":sequence,
                            "ownership_sequence":1, "observed_at_ms":time as u64
                        }),
                    ),
                    time,
                )
                .unwrap()
                .0
        };
        publish(json!(false), 1, at);
        let successor = publish(Value::Null, 1, at + 1);
        assert!(local_observation_position(&successor).is_some());
        let blocked = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(blocked.state, "needs-login");
        assert_eq!(blocked.since_unix_ms, at);
        let history = store.seat_status_history("agent/cedar", now_ms()).unwrap();
        assert_eq!(
            history["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["state"] == "unauthenticated")
                .count(),
            1
        );
        publish(json!(true), 2, at + 2);
        let recovered = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(recovered.state, "idle");
        assert_eq!(recovered.since_unix_ms, at + 2);
    }

    #[test]
    fn native_login_since_survives_activity_until_successful_recovery() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let observed_at = std::cell::Cell::new(now_ms() as u64 - 1_000);
        let native = |state: &str, auth: bool| {
            observed_at.set(observed_at.get() + 1);
            store
                .append_claim(&input(
                    "harness.observed",
                    json!({
                        "incarnation_id":"one", "state":state, "provider_auth":auth,
                        "driver":"claude", "provider_auth_sequence": if auth { 2 } else { 1 },
                        "observed_at_ms": observed_at.get()
                    }),
                ))
                .unwrap()
        };
        let first = native("idle", false);
        native("working", false);
        native("idle", false);
        let blocked = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(blocked.state, "needs-login");
        assert_eq!(
            blocked.since_unix_ms,
            u128::from(first.body["fields"]["observed_at_ms"].as_u64().unwrap())
        );
        let restored = native("idle", true);
        let idle = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(idle.state, "idle");
        assert_eq!(
            idle.since_unix_ms,
            u128::from(restored.body["fields"]["observed_at_ms"].as_u64().unwrap())
        );
        let history = store.seat_status_history("agent/cedar", now_ms()).unwrap();
        let states = history["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["state"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(states, ["unauthenticated", "idle"]);
    }

    #[test]
    fn status_prompt_transitions_and_repeats_agree_with_current_since() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms();
        observe(&store, "one", "idle", at, "ready");
        let diagnostic = |code| {
            store.append_claim(&input("harness.diagnostic", json!({
            "incarnation_id":"one", "code":code, "reason":"fixture", "severity":"warning"
        }))).unwrap()
        };
        let prompt = diagnostic("provider-auth-expired");
        diagnostic("provider-auth-expired");
        let blocked = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(blocked.state, "needs-login");
        assert_eq!(blocked.since_unix_ms, prompt.accepted_at_unix_ms);
        let restored = diagnostic("provider-auth-restored");
        let idle = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(idle.state, "idle");
        assert_eq!(idle.since_unix_ms, restored.accepted_at_unix_ms);
        let history = store.seat_status_history("agent/cedar", now_ms()).unwrap();
        let states = history["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["state"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(states, ["idle", "unauthenticated", "idle"]);
    }
}
