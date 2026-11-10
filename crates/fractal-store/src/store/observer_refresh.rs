//! `observer.refresh-requested`: a declared `refresh` or a verified GitHub webhook delivery asks
//! an observer to poll now. The claim replicates, so the member that polls the observer sees it
//! wherever it was written.
use super::*;
use crate::github_webhook;
use crate::model::ObserverSpec;

/// The active observers this store declares, with their selected revision and spec.
pub(super) fn active_observers_tx(
    transaction: &Transaction<'_>,
) -> Result<Vec<(String, String, ObserverSpec)>, FractalError> {
    let mut statement = transaction
        .prepare("SELECT subject, body, revision FROM desired WHERE kind='observer'")
        .map_err(internal)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(internal)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)?;
    Ok(rows
        .into_iter()
        .filter_map(|(subject, body, revision)| {
            serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|desired| crate::graph::observer_spec(&desired))
                .filter(|spec| !spec.stopped)
                .map(|spec| (subject, revision, spec))
        })
        .collect())
}

/// Ask `observer` at `revision` to poll once for `attempt`.
pub(super) fn append_observer_refresh_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    (observer, revision): (&str, &str),
    attempt: &str,
    batch_id: Option<&str>,
) -> Result<ClaimRecord, FractalError> {
    let fields = json!({"revision": revision, "attempt": attempt});
    append_claim_tx(
        transaction,
        origin,
        observer,
        "observer.refresh-requested",
        None,
        &json!({"fields": fields}),
        &[],
        batch_id,
    )
    .map_err(internal)
}

/// The oldest refresh attempt `observer` has not yet answered with an observation or a state.
pub fn pending_attempt(connection: &Connection, observer: &str) -> rusqlite::Result<Option<String>> {
    Ok(pending_attempts(connection, observer, 1)?.pop())
}

/// Up to `limit` refresh attempts `observer` has not yet answered with an observation or a
/// state, oldest first: the order its poller takes them in.
fn pending_attempts(connection: &Connection, observer: &str, limit: usize) -> rusqlite::Result<Vec<String>> {
    // The observations are listed once, not scanned for each request; see
    // `pending_subscription_mission_requests`.
    let mut statement = connection.prepare(
        /* sqlite-large-value: allow-for-now lv-h5b18a0f55300 owner=fractal_store::store::observer_refresh TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT json_extract(request.body, '$.fields.attempt')
             FROM claims request
             WHERE request.subject=?1
               AND request.kind='observer.refresh-requested'
               AND (
                 json_extract(request.body, '$.fields.attempt') IS NULL
                 OR json_extract(request.body, '$.fields.attempt') NOT IN (
                   SELECT json_extract(result.body, '$.fields.attempt') FROM claims result
                   WHERE result.subject=?1
                     AND result.kind IN ('observer.observed', 'observer.state')
                     AND json_extract(result.body, '$.fields.attempt') IS NOT NULL
                 )
               )
             ORDER BY CANONICAL_ASC(request)
             LIMIT ?2",
        ),
    )?;
    let attempts = statement.query_map(params![observer, limit], |row| row.get(0))?.collect();
    attempts
}

impl Store {
    /// Ask every active observer that a verified delivery `delivery` concerns to poll now, and
    /// return how many it asked. The poller answers refreshes oldest first, so the oldest pending
    /// one may already be polling, but a second has not started: an observer with two pending
    /// is not asked again, as its second poll reads what this delivery announced.
    pub fn request_github_webhook_refresh(
        &self,
        delivery: &str,
        scope: &github_webhook::Scope,
    ) -> Result<usize, FractalError> {
        let mut connection = self.connection.write();
        let transaction = connection.transaction().map_err(internal)?;
        let mut refreshed = 0;
        for (observer, revision, spec) in active_observers_tx(&transaction)? {
            if !github_webhook::refreshes(&spec, scope)
                || pending_attempts(&transaction, &observer, 2).map_err(internal)?.len() == 2
            {
                continue;
            }
            append_observer_refresh_tx(
                &transaction,
                &self.origin,
                (&observer, &revision),
                &github_webhook::attempt(delivery, &observer),
                None,
            )?;
            refreshed += 1;
        }
        transaction.commit().map_err(internal)?;
        Ok(refreshed)
    }

    /// Whether a webhook delivery asked `observer` to refresh within the coverage window, on
    /// whichever member it arrived; its attempt carries [`github_webhook::ATTEMPT_PREFIX`]. While
    /// it is covered, its polls are a slow backstop.
    pub fn github_webhook_covered(&self, observer: &str, now_unix_ms: u128) -> Result<bool> {
        let since = now_unix_ms.saturating_sub(github_webhook::COVERAGE_MS).to_string();
        self.readers
            .get()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM claims
                 WHERE subject=?1 AND kind='observer.refresh-requested'
                   AND (length(accepted_at_unix_ms) > length(?2)
                     OR (length(accepted_at_unix_ms) = length(?2) AND accepted_at_unix_ms >= ?2))
                   AND json_extract(body, '$.fields.attempt') LIKE ?3)",
                params![observer, since, format!("{}%", github_webhook::ATTEMPT_PREFIX)],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"version 2
resource "garden" { kind "vcs.repository" }
observer "garden" { resource "resource/garden"; provider "github.repository"; locator "Acme/Garden"; field "pull_requests" }
observer "greenhouse" { resource "resource/garden"; provider "github.repository"; locator "acme/greenhouse"; field "pull_requests" }
resource "garden-main" { kind "vcs.ref" }
observer "garden-main" { resource "resource/garden-main"; provider "github.ref"; locator "acme/garden@main"; field "head" }
observer "garden-release" { resource "resource/garden-main"; provider "github.ref"; locator "acme/garden@release"; field "head" }
"#;

    fn store() -> Store {
        let store = Store::open_memory("node").unwrap();
        let intent = crate::graph::parse_internal_intent(SOURCE, "node").unwrap();
        let planned = store
            .mission(&intent, IntentInput { kdl: SOURCE.into(), source_name: None })
            .unwrap();
        store.apply(&intent, &planned.subject_tokens, "webhook-observers").unwrap();
        store
    }

    fn requests(store: &Store, observer: &str) -> Vec<Value> {
        store
            .graph
            .claims_for(observer, Some("observer.refresh-requested"))
            .unwrap()
            .into_iter()
            .map(|claim| claim.body["fields"].clone())
            .collect()
    }

    #[test]
    fn a_delivery_refreshes_the_repository_and_moved_branch_observers_with_a_bound_attempt() {
        let store = store();
        let scope = github_webhook::Scope { repository: "acme/garden".into(), branch: Some("main".into()) };
        assert_eq!(store.request_github_webhook_refresh("delivery-1", &scope).unwrap(), 2);
        for observer in ["observer/garden", "observer/garden-main"] {
            let revision = store.selected_desired_revision(observer).unwrap().unwrap();
            assert_eq!(requests(&store, observer), [json!({
                "attempt": github_webhook::attempt("delivery-1", observer),
                "revision": revision,
            })]);
            assert_eq!(
                pending_attempt(&store.readers.get(), observer).unwrap(),
                Some(github_webhook::attempt("delivery-1", observer)),
            );
            assert!(store.github_webhook_covered(observer, now_ms()).unwrap());
        }
        for observer in ["observer/greenhouse", "observer/garden-release"] {
            assert!(requests(&store, observer).is_empty(), "{observer}");
            assert!(!store.github_webhook_covered(observer, now_ms()).unwrap());
        }
    }

    #[test]
    fn a_delivery_during_a_poll_queues_one_refresh_that_absorbs_later_deliveries() {
        let store = store();
        let scope = github_webhook::Scope { repository: "acme/garden".into(), branch: None };
        let attempt = |delivery: &str| github_webhook::attempt(delivery, "observer/garden");
        let answer = |delivery: &str| {
            let revision = store.selected_desired_revision("observer/garden").unwrap().unwrap();
            store
                .append_claim(&ClaimInput {
                    subject: "observer/garden".into(),
                    kind: "observer.state".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("attempt".into(), json!(attempt(delivery))),
                        ("revision".into(), json!(revision)),
                        ("state".into(), json!("healthy")),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        };
        // The poller has taken delivery-1's refresh, and may have read GitHub already, so a
        // delivery while it polls still asks for one more poll after it.
        assert_eq!(store.request_github_webhook_refresh("delivery-1", &scope).unwrap(), 1);
        assert_eq!(pending_attempt(&store.readers.get(), "observer/garden").unwrap(), Some(attempt("delivery-1")));
        assert_eq!(store.request_github_webhook_refresh("delivery-2", &scope).unwrap(), 1);
        // That queued poll has not started, so it reads what any later delivery announced.
        assert_eq!(store.request_github_webhook_refresh("delivery-3", &scope).unwrap(), 0);
        assert_eq!(requests(&store, "observer/garden").len(), 2);
        answer("delivery-1");
        assert_eq!(pending_attempt(&store.readers.get(), "observer/garden").unwrap(), Some(attempt("delivery-2")));
        assert_eq!(store.request_github_webhook_refresh("delivery-4", &scope).unwrap(), 1);
        assert_eq!(store.request_github_webhook_refresh("delivery-5", &scope).unwrap(), 0);
        answer("delivery-2");
        answer("delivery-4");
        assert_eq!(pending_attempt(&store.readers.get(), "observer/garden").unwrap(), None);
        assert_eq!(store.request_github_webhook_refresh("delivery-6", &scope).unwrap(), 1);
        assert_eq!(pending_attempt(&store.readers.get(), "observer/garden").unwrap(), Some(attempt("delivery-6")));
    }

    #[test]
    fn coverage_ends_after_the_window_and_ignores_declared_refreshes() {
        let store = store();
        let revision = store.selected_desired_revision("observer/greenhouse").unwrap().unwrap();
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        append_observer_refresh_tx(&transaction, "node", ("observer/greenhouse", &revision), "declared", None).unwrap();
        transaction.commit().unwrap();
        drop(connection);
        assert!(!store.github_webhook_covered("observer/greenhouse", now_ms()).unwrap());
        let scope = github_webhook::Scope { repository: "acme/garden".into(), branch: None };
        store.request_github_webhook_refresh("delivery-1", &scope).unwrap();
        let later = now_ms() + github_webhook::COVERAGE_MS + 1_000;
        assert!(store.github_webhook_covered("observer/garden", now_ms()).unwrap());
        assert!(!store.github_webhook_covered("observer/garden", later).unwrap());
    }
}
