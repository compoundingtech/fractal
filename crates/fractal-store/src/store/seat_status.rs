//! Observed status is separate from desired state and agent-declared work status.
//! Claims provide the replicated history; local receipts only improve current freshness.
use super::*;

pub const WINDOW_MS: u128 = 7 * 24 * 60 * 60 * 1000;
pub const MAX_TRANSITIONS: usize = 200;
pub fn permission_blocked(
    state: Option<&str>,
    blocked_on: Option<&str>,
    ask: Option<&str>,
) -> bool {
    state.is_some_and(|state| !matches!(state, "ended" | "indeterminate"))
        && blocked_on == Some("human")
        && ask == Some("permission")
}

pub fn observation_time(claim: &ClaimRecord) -> u128 {
    claim
        .body
        .pointer("/fields/observed_at_ms")
        .and_then(Value::as_u64)
        .map_or(claim.accepted_at_unix_ms, u128::from)
        .min(claim.accepted_at_unix_ms)
}

/// Match history_at's SQL `status_transition IS NOT 0` selection before reducing sources.
/// A suppressed heartbeat can still change reducer state; feeding it only to the drop planner
/// would hide a later legacy transition that the published history reader exposes.
pub fn history_source(claim: &ClaimRecord) -> bool {
    if claim.kind != "harness.observed" {
        return true;
    }
    !claim.body.pointer("/fields/status_transition").is_some_and(|stamp| {
        matches!(stamp, Value::Bool(false)) || stamp.as_f64() == Some(0.0)
    })
}

/// Canonical source positions of transitions and incarnation resets. Heartbeats and changes
/// to display details do not become transitions. Shared by the read and checkpoint rules.
pub fn transition_positions(claims: &[&ClaimRecord]) -> Vec<usize> {
    transitions(claims)
        .into_iter()
        .map(|transition| transition.0)
        .collect()
}

fn transitions(claims: &[&ClaimRecord]) -> Vec<(usize, Option<String>, bool)> {
    #[derive(Default)]
    struct Status<'a> {
        harness: Option<&'a str>,
        prompt: Option<&'a str>,
        update_prompt: bool,
        provider_auth: Option<bool>,
        blocked_on: Option<&'a str>,
        ask: Option<&'a str>,
        state: Option<&'a str>,
    }
    let mut runtime = None;
    let mut statuses = BTreeMap::<&str, Status<'_>>::new();
    let mut entries = Vec::new();
    for (position, claim) in claims.iter().enumerate() {
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        let Some(incarnation) = fields.get("incarnation_id").and_then(Value::as_str) else {
            continue;
        };
        if claim.kind == "runtime.observed" && fields["status"] == "running" {
            if runtime != Some(incarnation) {
                if runtime.is_some() || !statuses.contains_key(incarnation) {
                    entries.push((position, None, true));
                    statuses.insert(incarnation, Status::default());
                }
                runtime = Some(incarnation);
            }
            continue;
        }
        let reset = runtime.is_none() && !statuses.contains_key(incarnation);
        match claim.kind.as_str() {
            "harness.observed" => {
                let Some(state) = fields.get("state").and_then(Value::as_str) else {
                    continue;
                };
                let status = statuses.entry(incarnation).or_default();
                if matches!(state, "ended" | "indeterminate") {
                    status.blocked_on = None;
                    status.ask = None;
                } else {
                    if let Some(blocked_on) = fields.get("blocked_on") {
                        status.blocked_on = blocked_on.as_str();
                    }
                    if let Some(ask) = fields.get("ask") {
                        status.ask = ask.as_str();
                    }
                }
                status.harness = Some(if permission_blocked(Some(state), status.blocked_on, status.ask) {
                    "blocked"
                } else {
                    state
                });
                if let Some(auth) = fields.get("provider_auth").and_then(Value::as_bool) {
                    status.provider_auth = Some(auth);
                }
            }
            "harness.diagnostic" if statuses.contains_key(incarnation) => {
                let status = statuses.get_mut(incarnation).unwrap();
                match fields.get("code").and_then(Value::as_str) {
                    Some("provider-auth-expired") => status.prompt = Some("unauthenticated"),
                    Some("provider-trust-prompt") => status.prompt = Some("blocked"),
                    Some("provider-auth-restored") => status.prompt = None,
                    Some("provider-update-prompt") => status.update_prompt = true,
                    Some("provider-update-restored") => status.update_prompt = false,
                    _ => continue,
                }
            }
            _ => continue,
        }
        let status = statuses.get_mut(incarnation).unwrap();
        let next = if status.update_prompt {
            Some("blocked")
        } else if status.provider_auth == Some(false) {
            Some("unauthenticated")
        } else {
            status.prompt.or(status.harness)
        };
        // Heartbeats never become transitions when a retained prefix disappears. A recorded
        // transition remains one even if trimming removes an intervening state.
        let heartbeat = claim.kind == "harness.observed"
            && fields.get("status_transition").and_then(Value::as_bool) == Some(false);
        let recorded = claim.kind == "harness.observed"
            && status.prompt.is_none()
            && status.provider_auth != Some(false)
            && !status.update_prompt
            && fields.get("status_transition").and_then(Value::as_bool) == Some(true);
        if (reset || status.state != next || recorded) && !heartbeat {
            entries.push((position, next.map(str::to_owned), reset));
        }
        status.state = next;
    }
    entries
}

pub fn state_run_since(
    connection: &Connection,
    subject: &str,
    incarnation: &str,
    state: &str,
    index: u64,
) -> Result<Option<u128>> {
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hf429bed1b7cd owner=fractal_store::store::seat_status TODO=trace-statement-and-value-representation */ &harness_observations_of_incarnation_query())?;
    let mut rows = statement.query(params![subject, index, incarnation])?;
    let mut since = None;
    while let Some(row) = rows.next()? {
        smallclaims::read_budget::check()?;
        let body: Value = serde_json::from_str(&row.get::<_, String>(1)?)?;
        let fields = body.get("fields").unwrap_or(&body);
        if fields.get("state").and_then(Value::as_str) != Some(state) {
            break;
        }
        let accepted: u128 = row.get::<_, String>(2)?.parse()?;
        since = Some(
            fields
                .get("observed_at_ms")
                .and_then(Value::as_u64)
                .map_or(accepted, u128::from)
                .min(accepted),
        );
    }
    Ok(since)
}

pub fn enrich_harness(
    connection: &Connection,
    subject: &str,
    at_index: Option<u64>,
    view: &mut crate::model::CurrentHarnessView,
) -> Result<()> {
    let index = at_index.unwrap_or(i64::MAX as u64);
    let claim = claim_by_id_tx(connection, &view.claim)?;
    if claim
        .as_ref()
        .is_some_and(|claim| matches!(claim.kind.as_str(), "work.claimed" | "work.progress"))
    {
        return Ok(());
    }
    if let Some(claim) = claim.as_ref()
        && let Some(since) = claim
            .body
            .pointer("/fields/observed_since_ms")
            .and_then(Value::as_u64)
    {
        view.since_unix_ms = u128::from(since).min(claim.accepted_at_unix_ms);
    } else if claim
        .as_ref()
        .is_some_and(|claim| claim.kind == "harness.observed")
    {
        view.since_unix_ms = state_run_since(
            connection,
            subject,
            &view.incarnation_id,
            &view.state,
            index,
        )?
        .unwrap_or(view.since_unix_ms);
    }
    let has_prompt: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='harness.diagnostic'
         AND store_index<=?2 AND json_extract(body,'$.fields.incarnation_id')=?3
         AND json_extract(body,'$.fields.code') IN ('provider-auth-expired','provider-trust-prompt','provider-auth-restored','provider-update-prompt','provider-update-restored'))",
        params![subject, index, view.incarnation_id], |row| row.get(0),
    )?;
    let unstamped_auth = claim.as_ref().is_some_and(|claim| {
        claim
            .body
            .pointer("/fields/provider_auth")
            .and_then(Value::as_bool)
            .is_some()
            && claim.body.pointer("/fields/observed_since_ms").is_none()
    });
    if has_prompt || view.state == "needs-login" || unstamped_auth {
        // Rare positive prompt and credential fences can interrupt an otherwise unchanged harness state.
        // Replay transition sources only for this incarnation, excluding ordinary heartbeats.
        let query = format!("SELECT {CLAIM_COLUMNS} FROM claims INDEXED BY claims_incarnation_accepted_index
            JOIN batches ON batches.id=claims.batch_id
            WHERE claims.subject=?1 AND +claims.store_index<=?2 AND {INCARNATION_OF_CLAIM}=?3
            AND claims.kind IN ('harness.observed','harness.diagnostic','runtime.observed')
            AND (claims.kind!='harness.observed' OR json_extract(claims.body,'$.fields.status_transition') IS NOT 0)
            ORDER BY {CANONICAL_ORDER}");
        let claims = connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-had5b89d22ca7 owner=fractal_store::store::seat_status TODO=trace-statement-and-value-representation */ &query)?
            .query_map(params![subject, index, view.incarnation_id], claim_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let refs = claims.iter().collect::<Vec<_>>();
        if let Some(entry) = transitions(&refs).last()
            && entry.1.as_deref()
                == Some(if view.state == "needs-login" {
                    "unauthenticated"
                } else {
                    view.state.as_str()
                })
        {
            view.since_unix_ms = observation_time(&claims[entry.0]);
        }
    }
    attention_snapshot::resolve_native_harness_at(connection, subject, index, view)?;
    Ok(())
}

impl Store {
    /// Client status describes harness evidence. Work progress can prove delivery readiness
    /// internally, but never substitutes for an observation in this contract.
    pub fn observed_harness_at(
        &self,
        subject: &str,
        index: u64,
    ) -> Result<Option<crate::model::CurrentHarnessView>> {
        let connection = self.readers.get();
        let mut harness = current_harness_fold_at(&connection, subject, Some(index), false, false)?;
        if let Some(view) = harness.as_mut() {
            enrich_harness(&connection, subject, Some(index), view)?;
        }
        Ok(harness)
    }

}

pub fn history_at(
    connection: &Connection,
    subject: &str,
    now: u128,
    index: u64,
) -> Result<Value> {
    history_at_inner(connection, subject, now, index, None)
}

pub fn history_at_with_sources(connection: &Connection, subject: &str, now: u128, index: u64) -> Result<(Value, Vec<smallclaims::store::checkpoint::CheckpointItemSource>)> {
    let mut sources = Vec::new();
    let history = history_at_inner(connection, subject, now, index, Some(&mut sources))?;
    Ok((history, sources))
}

fn history_at_inner(connection: &Connection, subject: &str, now: u128, index: u64, mut sources: Option<&mut Vec<smallclaims::store::checkpoint::CheckpointItemSource>>) -> Result<Value> {
    let cutoff = now.saturating_sub(WINDOW_MS);
    // Seek each kind by acceptance time. Read a single older baseline per prompt channel, rather
    // than decoding a seat's lifetime observations on every history request.
    let mut keyed = Vec::new();
    let mut older = false;
    for (kind, codes) in [
        ("harness.observed", ""),
        ("runtime.observed", ""),
        (
            "harness.diagnostic",
            "'provider-auth-expired','provider-trust-prompt','provider-auth-restored'",
        ),
        (
            "harness.diagnostic",
            "'provider-update-prompt','provider-update-restored'",
        ),
    ] {
        let columns = format!(
            "{CLAIM_COLUMNS}, batches.origin, batches.replica_sequence, {}",
            canonical::position_sql("claims")
        );
        let filter = if codes.is_empty() {
            " AND (claims.kind!='harness.observed' OR json_extract(claims.body, '$.fields.status_transition') IS NOT 0)".into()
        } else {
            format!(" AND json_extract(claims.body, '$.fields.code') IN ({codes})")
        };
        let query = newest_claims_of_kind_query(&columns, kind)
            .replace(" ORDER BY", &format!("{filter} ORDER BY"));
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-he29cafed8c61 owner=fractal_store::store::seat_status TODO=trace-statement-and-value-representation */ &query)?;
        let mut rows = statement.query(params![subject, index])?;
        while let Some(row) = rows.next()? {
            smallclaims::read_budget::check()?;
            let claim = claim_from_row(row)?;
            let before_window = claim.accepted_at_unix_ms < cutoff;
            let key = canonical::key_from_record(&claim, row.get(10)?, row.get(11)?, row.get(12)?);
            keyed.push((key, claim));
            if before_window {
                older = true;
                break;
            }
        }
    }
    // Seed older baselines with their incarnation's first retained runtime observation, so
    // a later running receipt does not hide a prompt that already held that same runtime.
    let baseline_incarnations = keyed
        .iter()
        .filter(|(_, claim)| claim.accepted_at_unix_ms < cutoff)
        .filter_map(|(_, claim)| {
            claim
                .body
                .pointer("/fields/incarnation_id")
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let query = format!("SELECT {CLAIM_COLUMNS}, batches.origin, batches.replica_sequence, {}
        FROM claims INDEXED BY claims_incarnation_accepted_index JOIN batches ON batches.id=claims.batch_id
        WHERE claims.subject=?1 AND {INCARNATION_OF_CLAIM}=?2 AND +claims.store_index<=?3 AND claims.kind='runtime.observed'
        AND json_extract(claims.body,'$.fields.status')='running' ORDER BY {CANONICAL_ORDER} LIMIT 1", canonical::position_sql("claims"));
    for incarnation in baseline_incarnations {
        let source = connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-hdb2e592f95e7 owner=fractal_store::store::seat_status TODO=trace-statement-and-value-representation */ &query)?
            .query_row(/* sqlite-large-value: allow-for-now lv-haf8f634176f1 owner=fractal_store::store::seat_status TODO=trace-statement-and-value-representation */ params![subject, incarnation, index], |row| {
                let claim = claim_from_row(row)?;
                Ok((
                    canonical::key_from_record(&claim, row.get(10)?, row.get(11)?, row.get(12)?),
                    claim,
                ))
            })
            .optional()?;
        if let Some(source) = source {
            keyed.push(source);
        }
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.dedup_by(|a, b| a.1.id == b.1.id);
    let claims = keyed
        .iter()
        .map(|(_, claim)| claim)
        .collect::<Vec<_>>();
    let entries = transitions(&claims);
    let tombstoned: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM checkpoint_claims WHERE subject=?1 AND kind IN ('harness.observed','runtime.observed'))",
            [subject], |row| row.get(0)
        )?;
    let eligible = entries
        .iter()
        .filter(|entry| observation_time(claims[entry.0]) >= cutoff)
        .collect::<Vec<_>>();
    let legacy = claims.iter().any(|claim| {
        claim.kind == "harness.observed"
            && claim
                .body
                .pointer("/fields/incarnation_id")
                .and_then(Value::as_str)
                .is_none()
    });
    let trimmed = legacy
        || older
        || tombstoned
        || eligible.len() < entries.len()
        || eligible.len() > MAX_TRANSITIONS;
    let start = eligible.len().saturating_sub(MAX_TRANSITIONS);
    let items = eligible[start..].iter().map(|entry| {
        let claim = claims[entry.0];
        if let Some(sources) = sources.as_mut() {
            sources.push(smallclaims::store::checkpoint::CheckpointItemSource {
                claim: claim.id.clone(), order: keyed[entry.0].0.clone(),
            });
        }
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        json!({
            "seat": subject, "runtime_incarnation": fields["incarnation_id"],
            "state": entry.1, "observed_at": crate::client_values::client_timestamp(observation_time(claim)), "reset": entry.2,
        })
    }).collect::<Vec<_>>();
    Ok(json!({
        "kind": "status-history", "seat": subject, "items": items,
        "retained_from": items.first().map(|item| item["observed_at"].clone()).unwrap_or_else(|| json!(crate::client_values::client_timestamp(cutoff))),
        "complete": !trimmed,
    }))
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::seat_status_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::seat_status_tests_shared_seats::{checked_harness, input, observe, runtime};
use super::*;

    #[test]
    fn history_source_selection_matches_sql_for_legacy_transition_stamps() {
        let store = Store::open_memory("cedar").unwrap();
        let mut claim = observe(&store, "one", "idle", now_ms(), "ready");
        let connection = Connection::open_in_memory().unwrap();
        for stamp in [None, Some(json!(false)), Some(json!(true)), Some(Value::Null),
            Some(json!(0)), Some(json!(0.0)), Some(serde_json::from_str::<Value>("-0").unwrap()),
            Some(serde_json::from_str::<Value>("0e0").unwrap()), Some(serde_json::from_str::<Value>("1e-400").unwrap()),
            Some(json!("0")), Some(json!("false"))]
        {
            claim.body = json!({"fields":{"state":"idle", "incarnation_id":"one"}});
            if let Some(stamp) = stamp { claim.body["fields"]["status_transition"] = stamp; }
            let selected: bool = connection.query_row(
                "SELECT json_extract(?1, '$.fields.status_transition') IS NOT 0",
                [claim.body.to_string()], |row| row.get(0),
            ).unwrap();
            assert_eq!(history_source(&claim), selected, "{}", claim.body);
        }
        claim.body = json!({"status_transition":false});
        assert!(history_source(&claim), "a legacy raw body has no $.fields stamp");
        claim.kind = "runtime.observed".into();
        claim.body = json!({"fields":{"status_transition":false}});
        assert!(history_source(&claim));
    }

    #[test]
    fn status_upgrade_preserves_since_from_legacy_observations() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms() - 1_000;
        let first = observe(&store, "one", "idle", at, "ready");
        store.connection.lock().unwrap().execute(
            "UPDATE claims SET body=json_remove(body, '$.fields.observed_since_ms', '$.fields.status_transition') WHERE id=?1",
            [first.id],
        ).unwrap();
        let before = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        observe(&store, "one", "idle", at + 1, "waiting");
        let after = checked_harness(&store, "agent/cedar").unwrap().unwrap();
        assert_eq!(before.since_unix_ms, at);
        assert_eq!(after.since_unix_ms, before.since_unix_ms);
    }

    #[test]
    fn unchanged_observations_publish_a_bounded_heartbeat_without_a_transition() {
        let store = Store::open_memory("cedar").unwrap();
        runtime(&store, "one");
        let at = now_ms();
        let first = observe(&store, "one", "idle", at, "ready");
        // Move the previous publication's acceptance time to simulate the publish interval.
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE claims SET accepted_at_unix_ms=?1 WHERE id=?2",
                params![(at - crate::store::RESTAMP_FALLBACK_MS).to_string(), first.id],
            )
            .unwrap();
        let heartbeat = observe(&store, "one", "idle", at + 1, "ready");
        assert!(local_observation_position(&heartbeat).is_none());
        assert_eq!(heartbeat.body["fields"]["status_transition"], false);
        assert_eq!(
            heartbeat.body["fields"]["observed_since_ms"],
            first.body["fields"]["observed_since_ms"]
        );
    }
}
