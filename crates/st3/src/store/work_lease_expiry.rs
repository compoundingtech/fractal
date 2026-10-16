//! Lease-loss provenance comes from the accepted renewal, including local quiet renewals.
use super::*;

const PREFIX: &str = "the worker lease expired";

pub(super) fn is_reason(reason: Option<&str>) -> bool {
    reason.is_some_and(|reason| reason.starts_with(PREFIX))
}

/// Existing local renewals retain their request time. Their acceptance time cannot
/// be reconstructed; nullable metadata makes that absence explicit after upgrade.
pub(super) fn ensure_acceptance_columns(connection: &Connection) -> Result<()> {
    let columns = connection
        .prepare("PRAGMA table_info(local_work_lease_renewals)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    for column in ["accepted_at_unix_ms", "accepted_request_at_unix_ms"] {
        if !columns.contains(column) {
            connection.execute_batch(&format!(
                "ALTER TABLE local_work_lease_renewals ADD COLUMN {column} TEXT"
            ))?;
        }
    }
    Ok(())
}

fn parse_timestamp(at: &str) -> rusqlite::Result<u128> {
    at.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn timestamp(at: u128) -> String {
    i64::try_from(at)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
        .unwrap_or_else(|| format!("{at} ms since Unix epoch"))
}

/// One subject/kind seek, then canonical ordering only within the newest accepted millisecond.
fn latest_event(
    connection: &Connection,
    subject: &str,
    kind: &str,
) -> rusqlite::Result<Option<(u128, Value)>> {
    connection
        .prepare_cached(&canonical_sql(
            "WITH newest AS (
                 SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_subject_kind_accepted_index
                 WHERE subject=?1 AND kind=?2
                 ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1
             )
             SELECT claims.accepted_at_unix_ms, claims.body
             FROM newest CROSS JOIN claims INDEXED BY claims_subject_kind_accepted_index
             WHERE claims.subject=?1 AND claims.kind=?2
               AND length(claims.accepted_at_unix_ms)=length(newest.accepted_at_unix_ms)
               AND claims.accepted_at_unix_ms=newest.accepted_at_unix_ms
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
        ))?
        .query_row(params![subject, kind], |row| {
            let at: String = row.get(0)?;
            let body: String = row.get(1)?;
            let at = at.parse::<u128>().map_err(|error| rusqlite::Error::FromSqlConversionFailure(
                0, rusqlite::types::Type::Text, Box::new(error),
            ))?;
            let body = serde_json::from_str::<Value>(&body).map_err(|error| rusqlite::Error::FromSqlConversionFailure(
                1, rusqlite::types::Type::Text, Box::new(error),
            ))?;
            Ok((at, body))
        })
        .optional()
}

pub(super) fn reason(
    connection: &Connection,
    view: &StepRunView,
    expiry: u128,
) -> rusqlite::Result<String> {
    let matches = |body: &Value| {
        let fields = body.get("fields").unwrap_or(body);
        fields["attempt"].as_u64() == Some(u64::from(view.attempt))
            && fields["claimant"].as_str() == view.claimant.as_deref()
            && fields["claim_incarnation"].as_str() == view.claim_incarnation.as_deref()
    };
    let claimed = latest_event(connection, &view.subject, "work.claimed")?
        .filter(|(_, body)| matches(body))
        .map(|(at, _)| at);
    let mut published = None;
    // Progress and explicit extensions also renew the operational lease.
    for kind in ["work.renewed", "work.progress", "work.extended"] {
        let accepted = latest_event(connection, &view.subject, kind)?
            .filter(|(at, body)| matches(body) && claimed.is_none_or(|claim| *at >= claim))
            .map(|(at, _)| at);
        published = published.into_iter().chain(accepted).max();
    }
    // A quiet row is the latest unpublished renewal and is deleted by every
    // published work action. Match its lease and request stamp before trusting
    // acceptance metadata: an older binary can update the row without those fields.
    let quiet: Option<(String, Option<String>, Option<String>)> = connection
        .prepare_cached(
            "SELECT updated_at_unix_ms, accepted_at_unix_ms, accepted_request_at_unix_ms
             FROM local_work_lease_renewals
             WHERE subject=?1 AND attempt=?2 AND lease_owner IS ?3 AND lease_incarnation IS ?4
               AND lease_expires_at_unix_ms=?5",
        )?
        .query_row(
            params![
                view.subject,
                view.attempt,
                view.claimant,
                view.claim_incarnation,
                expiry.to_string()
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let detail = match quiet {
        Some((request, Some(accepted), Some(proof_request))) if request == proof_request => {
            format!(
                "last accepted renewal at {}",
                timestamp(parse_timestamp(&accepted)?)
            )
        }
        Some((request, _, _)) => format!(
            "last accepted renewal request at {}; its acceptance time was not recorded by that build",
            timestamp(parse_timestamp(&request)?),
        ),
        None => match published {
            Some(at) => format!("last accepted renewal at {}", timestamp(at)),
            None => match claimed {
                Some(at) => format!(
                    "no renewal was accepted for this claim; claim accepted at {}",
                    timestamp(at)
                ),
                None => "no accepted renewal is recorded for this claim".into(),
            },
        },
    };
    Ok(format!("{PREFIX} at {}; {detail}", timestamp(expiry)))
}

/// Called before the durable transition clears the lease, in the same transaction.
/// Recheck the lease here so a renewal that beat the writer is never released or notified.
pub(super) fn prepare_notice(
    transaction: &Transaction<'_>,
    origin: &str,
    subject: &str,
    now: u128,
) -> Result<Option<String>> {
    let view = transaction.query_row(
        "SELECT subject, run_id, step_path, definition_hash, status, attempt, assignee, available_to, agentless, title, goals, worker_reported,
                lease_owner, lease_incarnation, lease_expires_at_unix_ms, blocked_reason, not_before_unix_ms, created_at_unix_ms, updated_at_unix_ms, readiness_epoch, constraints
         FROM step_runs WHERE subject=?1", [subject], step_run_from_row)?;
    let Some(expiry) = view
        .claim_expires_at_unix_ms
        .filter(|expiry| *expiry <= now)
    else {
        return Ok(None);
    };
    if !matches!(
        view.status.as_str(),
        "claimed" | "working" | "verifying" | "blocked"
    ) {
        return Ok(None);
    }
    let detail = reason(transaction, &view, expiry)?;
    if let Some(claimant) = &view.claimant {
        let stable = canonical_hash(&(
            subject,
            view.attempt,
            &view.claim_incarnation,
            view.readiness_epoch,
            expiry,
        ))?;
        append_claim_tx(
            transaction,
            origin,
            &format!("message/work-lease-expired-{}", &stable[..20]),
            "message.sent",
            Some("daemon/runtime"),
            &json!({"fields": {
                "from": "daemon/runtime", "to": claimant, "title": "Work claim expired",
                "content": format!("Your claim on {subject} ended: {detail}. Claim incarnation: {}. At expiry the step became ready with attempt {} unchanged. Inspect current state with st work show {subject} before continuing; work requires a current claim.", view.claim_incarnation.as_deref().unwrap_or("not recorded"), view.attempt),
                "status": "sent", "tags": [format!("step-run:{subject}"), "st3-wake-source:lease-expired"],
            }}),
            &[],
            None,
        )?;
    }
    Ok(Some(detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: u128 = 1_700_000_000_000;
    struct Clock;
    impl Clock {
        fn at(store: &Store, at: u128) -> Self {
            smallclaims::store::set_thread_clock(Some(at));
            store.set_write_clock_at(at).unwrap();
            Self
        }
    }
    impl Drop for Clock {
        fn drop(&mut self) {
            smallclaims::store::set_thread_clock(None);
        }
    }
    fn request(key: &str, summary: Option<&str>) -> WorkRequest {
        WorkRequest {
            actor: Some("agent/node.worker".into()),
            incarnation: Some("current".into()),
            summary: summary.map(Into::into),
            reason: None,
            evidence: Vec::new(),
            wait_ms: None,
            idempotency_key: key.into(),
        }
    }
    fn fixture_at(start: u128) -> (Store, String, Clock) {
        fixture_with_store(Store::open_memory("node").unwrap(), start)
    }
    fn fixture_with_store(store: Store, start: u128) -> (Store, String, Clock) {
        let clock = Clock::at(&store, start);
        let source = "version 2\nmission \"lease\" state=\"ready\" { goal \"Keep the accepted renewal.\"; step \"work\" { assigned-to \"agent/node.worker\" } }";
        let intent = crate::graph::parse_test_intent(source, "node").unwrap();
        let plan = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store
            .apply(&intent, &plan.subject_tokens, "lease-publish")
            .unwrap();
        let run = store
            .create_mission_run(&MissionRunRequest {
                mission: "lease".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some("person/operator".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: "lease-run".into(),
            })
            .unwrap();
        let subject = run.steps[0].subject.clone();
        store.set_step_state(&subject, "ready", None).unwrap();
        store
            .work_action(&subject, "claim", &request("lease-claim", None))
            .unwrap();
        (store, subject, clock)
    }

    #[test]
    fn published_renewal_and_a_new_claim_have_distinct_expiry_provenance() {
        let (store, subject, _clock) = fixture_at(START);
        let _renew_clock = Clock::at(&store, START + 60_000);
        store
            .work_action(
                &subject,
                "renew",
                &request("lease-renew", Some("accepted anchor")),
            )
            .unwrap();
        let accepted = store
            .latest_claim(&subject, Some("work.renewed"))
            .unwrap()
            .unwrap()
            .accepted_at_unix_ms;
        let _expired_clock = Clock::at(&store, START + 660_001);
        let view = store.step_run(&subject).unwrap().unwrap();
        assert_eq!(view.status, "ready");
        assert!(view.blocked_reason.unwrap().contains(&timestamp(accepted)));
        assert!(
            store
                .set_step_state(&subject, "ready", Some(PREFIX))
                .unwrap()
        );
        assert_eq!(
            store
                .messages(Some("agent/node.worker"), true)
                .unwrap()
                .len(),
            1
        );
        let _reclaim_clock = Clock::at(&store, START + 700_000);
        store
            .work_action(&subject, "claim", &request("lease-reclaim", None))
            .unwrap();
        let _second_expiry = Clock::at(&store, START + 1_300_001);
        let view = store.step_run(&subject).unwrap().unwrap();
        assert_eq!(view.attempt, 1);
        let detail = view.blocked_reason.unwrap();
        assert!(
            detail.contains("no renewal was accepted for this claim"),
            "{detail}"
        );
        assert!(!detail.contains("last accepted renewal"), "{detail}");
        assert!(
            store
                .set_step_state(&subject, "ready", Some(PREFIX))
                .unwrap()
        );
        assert_eq!(
            store
                .messages(Some("agent/node.worker"), true)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn a_direct_reclaim_notifies_the_expired_incarnation_once() {
        let (store, subject, _clock) = fixture_at(START);
        let _renew_clock = Clock::at(&store, START + 60_000);
        store
            .work_action(&subject, "renew", &request("reclaim-quiet", None))
            .unwrap();
        let live_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let _expiry_clock = Clock::at(&store, live_now);
        // No durable ready repair runs before the new claim replaces the raw lease.
        let mut rejected = request("unavailable-reclaim", None);
        rejected.actor = Some("agent/other.worker".into());
        assert_eq!(
            store
                .work_action(&subject, "claim", &rejected)
                .unwrap_err()
                .code,
            "work-not-available"
        );
        assert!(
            store
                .messages(Some("agent/node.worker"), true)
                .unwrap()
                .is_empty()
        );
        let mut reclaim = request("direct-reclaim", None);
        reclaim.incarnation = Some("successor".into());
        let new_claim = store.work_action(&subject, "claim", &reclaim).unwrap();
        assert_eq!(new_claim.status, "claimed");
        assert_eq!(new_claim.attempt, 1);
        assert_eq!(new_claim.readiness_epoch, 2);
        assert_eq!(new_claim.claim_incarnation.as_deref(), Some("successor"));
        let messages = store.messages(Some("agent/node.worker"), true).unwrap();
        assert_eq!(messages.len(), 1);
        let content = &messages[0].content;
        assert!(content.contains("Claim incarnation: current"), "{content}");
        assert!(content.contains("attempt 1 unchanged"), "{content}");
        assert!(content.contains("last accepted renewal at"), "{content}");
        // Repeating the committed request cannot produce a second expiry notice.
        store.work_action(&subject, "claim", &reclaim).unwrap();
        assert_eq!(
            store
                .messages(Some("agent/node.worker"), true)
                .unwrap()
                .len(),
            1
        );
        assert!(
            !store
                .set_step_state(&subject, "ready", Some(PREFIX))
                .unwrap()
        );
        assert_eq!(
            store
                .step_run(&subject)
                .unwrap()
                .unwrap()
                .claim_incarnation
                .as_deref(),
            Some("successor")
        );
    }

    #[test]
    fn a_renewal_that_beats_the_expiry_writer_is_not_released_or_notified() {
        // This lease is live under both the caller and transaction clocks.
        let live_start = now_ms();
        let (store, subject, _clock) = fixture_at(live_start);
        // A stale evaluator asks to release a lease it previously saw expired. The
        // writer must validate the current row, which is now a live renewed lease.
        let _renew_clock = Clock::at(&store, live_start + 60_000);
        store
            .work_action(&subject, "renew", &request("lease-live-renew", None))
            .unwrap();
        assert!(
            !store
                .set_step_state(&subject, "ready", Some(PREFIX))
                .unwrap()
        );
        let view = store.step_run(&subject).unwrap().unwrap();
        assert_eq!(view.status, "claimed");
        assert_eq!(view.claimant.as_deref(), Some("agent/node.worker"));
        assert!(
            store
                .messages(Some("agent/node.worker"), true)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn progress_that_extends_the_lease_is_the_latest_accepted_renewal() {
        let (store, subject, _clock) = fixture_at(START);
        let _renew_clock = Clock::at(&store, START + 60_000);
        store
            .work_action(
                &subject,
                "renew",
                &request("earlier-renewal", Some("still here")),
            )
            .unwrap();
        let _progress_clock = Clock::at(&store, START + 120_000);
        store
            .work_action(
                &subject,
                "progress",
                &request("later-progress", Some("new progress")),
            )
            .unwrap();
        let accepted = store
            .latest_claim(&subject, Some("work.progress"))
            .unwrap()
            .unwrap()
            .accepted_at_unix_ms;
        let _expiry_clock = Clock::at(&store, START + 720_001);
        let view = store.step_run(&subject).unwrap().unwrap();
        let detail = view.blocked_reason.unwrap();
        assert!(
            detail.contains(&format!("last accepted renewal at {}", timestamp(accepted))),
            "{detail}"
        );
    }
    #[test]
    fn quiet_acceptance_metadata_upgrades_without_inventing_an_old_acceptance_time() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("legacy.sqlite3");
        let (store, subject, _clock) =
            fixture_with_store(Store::open(&path, "node").unwrap(), START);
        let _renew_clock = Clock::at(&store, START + 60_000);
        store
            .work_action(&subject, "renew", &request("legacy-quiet", None))
            .unwrap();
        drop(store);
        // Restore the actual older table shape, retaining its quiet renewal row.
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "ALTER TABLE local_work_lease_renewals DROP COLUMN accepted_at_unix_ms;
            ALTER TABLE local_work_lease_renewals DROP COLUMN accepted_request_at_unix_ms;",
            )
            .unwrap();
        drop(connection);
        let store = Store::open(&path, "node").unwrap();
        let _expiry_clock = Clock::at(&store, START + 660_001);
        let view = store.step_run(&subject).unwrap().unwrap();
        assert_eq!(view.status, "ready");
        assert_eq!(view.attempt, 1);
        let detail = view.blocked_reason.unwrap();
        assert!(
            detail.contains("last accepted renewal request at 2023-11-14T22:14:20Z"),
            "{detail}"
        );
        assert!(
            detail.contains("acceptance time was not recorded"),
            "{detail}"
        );
        let (request, accepted): (String, Option<String>) = store.connection.lock().unwrap().query_row(
            "SELECT updated_at_unix_ms, accepted_at_unix_ms FROM local_work_lease_renewals WHERE subject=?1",
            [&subject], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(request, (START + 60_000).to_string());
        assert!(accepted.is_none());
        drop(store);
        let store = Store::open(&path, "node").unwrap();
        assert_eq!(
            store
                .step_run(&subject)
                .unwrap()
                .unwrap()
                .blocked_reason
                .as_deref(),
            Some(detail.as_str())
        );
    }

    #[test]
    fn an_older_writer_cannot_leave_a_previous_acceptance_stamp_looking_current() {
        let (store, subject, _clock) = fixture_at(START);
        let _renew_clock = Clock::at(&store, START + 60_000);
        store
            .work_action(&subject, "renew", &request("modern-quiet", None))
            .unwrap();
        let original_accepted: String = store
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT accepted_at_unix_ms FROM local_work_lease_renewals WHERE subject=?1",
                [&subject],
                |row| row.get(0),
            )
            .unwrap();
        // An older renewal changes only the original columns of both local rows.
        {
            let mut connection = store.connection.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            transaction.execute("UPDATE local_work_lease_renewals SET updated_at_unix_ms=?2, lease_expires_at_unix_ms=?3 WHERE subject=?1",
                params![subject, (START + 120_000).to_string(), (START + 720_000).to_string()]).unwrap();
            transaction.execute("UPDATE step_runs SET updated_at_unix_ms=?2, lease_expires_at_unix_ms=?3 WHERE subject=?1",
                params![subject, (START + 120_000).to_string(), (START + 720_000).to_string()]).unwrap();
            transaction.commit().unwrap();
        }
        let _expiry_clock = Clock::at(&store, START + 720_001);
        let detail = store
            .step_run(&subject)
            .unwrap()
            .unwrap()
            .blocked_reason
            .unwrap();
        assert!(
            detail.contains("last accepted renewal request at 2023-11-14T22:15:20Z"),
            "{detail}"
        );
        assert!(
            detail.contains("acceptance time was not recorded"),
            "{detail}"
        );
        assert!(
            !detail.contains(&format!(
                "last accepted renewal at {}",
                timestamp(original_accepted.parse().unwrap())
            )),
            "{detail}"
        );
    }
}
