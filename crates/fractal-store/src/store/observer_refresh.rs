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

/// The oldest refresh attempt `observer` has not yet answered, or `None` when none is pending. A
/// request is pending until an observation or a state carries its attempt. It names the poll that
/// answers it, so it stays the same while that poll runs.
pub fn pending_attempt(connection: &Connection, observer: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            /* sqlite-large-value: allow-for-now lv-hf930998a9bf2 owner=fractal_store::store::observer_refresh TODO=trace-statement-and-value-representation */ &pending_requests_sql(true),
            params![observer, format!("{}%", github_webhook::ATTEMPT_PREFIX)],
            |row| row.get(0),
        )
        .optional()
}

/// The attempt a poll taken for pending `attempt` answers, read right before the poll's reads.
/// For a webhook refresh it is the newest pending webhook refresh from the member that wrote
/// `attempt`; answering it answers every earlier webhook refresh from that member, since each
/// was written after the change it announced and before those reads. Any other attempt answers
/// itself.
pub fn coalesced_attempt(connection: &Connection, observer: &str, attempt: &str) -> rusqlite::Result<String> {
    if !attempt.starts_with(github_webhook::ATTEMPT_PREFIX) {
        return Ok(attempt.to_owned());
    }
    let pending = pending_requests(connection, observer)?;
    let Some((_, origin)) = pending.iter().find(|(pending, _)| pending == attempt) else {
        return Ok(attempt.to_owned());
    };
    Ok(pending
        .iter()
        .rev()
        .find(|(newest, from)| from == origin && newest.starts_with(github_webhook::ATTEMPT_PREFIX))
        .map_or_else(|| attempt.to_owned(), |(newest, _)| newest.clone()))
}

/// The pending refresh attempts of `observer` with the member that wrote each, oldest first. A
/// webhook refresh is no longer pending once a later webhook refresh from the same member is
/// answered.
fn pending_requests(connection: &Connection, observer: &str) -> rusqlite::Result<Vec<(String, Option<String>)>> {
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h76ad4685e6b9 owner=fractal_store::store::observer_refresh TODO=trace-statement-and-value-representation */ &pending_requests_sql(false))?;
    statement
        .query_map(params![observer, format!("{}%", github_webhook::ATTEMPT_PREFIX)], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect()
}

/// Decode history once and reduce answered webhooks to one frontier per origin, rather than
/// searching all later requests for every outstanding request. The oldest-only reader returns
/// one SQLite row and never constructs the full pending vector.
/// A NULL batch origin makes the old canonical tuple comparison unknown at equal timestamps;
/// only a strictly newer timestamp can cover those requests. Keep that SQL NULL behavior,
/// including for requests without an attempt.
fn pending_requests_sql(oldest_only: bool) -> String {
    canonical_sql(&format!(
        "WITH answered(attempt) AS MATERIALIZED (
           SELECT json_extract(body, '$.fields.attempt') FROM claims
           WHERE subject=?1 AND kind IN ('observer.observed', 'observer.state')
             AND json_extract(body, '$.fields.attempt') IS NOT NULL
         ),
         requests AS MATERIALIZED (
           SELECT json_extract(request.body, '$.fields.attempt') AS attempt,
                  (SELECT origin FROM batches WHERE id=request.batch_id) AS origin,
                  request.accepted_at_unix_ms AS accepted,
                  ROW_NUMBER() OVER (ORDER BY CANONICAL_ASC(request)) AS position
           FROM claims request
           WHERE request.subject=?1 AND request.kind='observer.refresh-requested'
         ),
         latest_answered AS MATERIALIZED (
           SELECT origin, MAX(position) AS position
           FROM requests
           WHERE attempt LIKE ?2 AND attempt IN (SELECT attempt FROM answered)
           GROUP BY origin
         )
         SELECT request.attempt, request.origin
         FROM requests request
         LEFT JOIN latest_answered latest ON latest.origin IS request.origin
         LEFT JOIN requests frontier ON frontier.position=latest.position
         WHERE (request.attempt IS NULL OR request.attempt NOT IN (SELECT attempt FROM answered))
           AND NOT (request.attempt LIKE ?2 AND COALESCE(
             CASE WHEN request.origin IS NULL
               THEN (length(frontier.accepted), frontier.accepted) >
                    (length(request.accepted), request.accepted)
               ELSE latest.position > request.position END, 0))
         ORDER BY request.position{}",
        if oldest_only { " LIMIT 1" } else { "" },
    ))
}

/// Whether `observer` already has a refresh request for `attempt`.
fn requested(connection: &Connection, observer: &str, attempt: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims
         WHERE subject=?1 AND kind='observer.refresh-requested'
           AND json_extract(body, '$.fields.attempt')=?2)",
        params![observer, attempt],
        |row| row.get(0),
    )
}

impl Store {
    /// Ask every active observer that a verified delivery `delivery` concerns to poll now, and
    /// return how many it asked. A redelivery asks nothing more. Deliveries that arrive while a
    /// poll runs each add a request, which the next poll answers together (see
    /// [`coalesced_attempt`]).
    pub fn request_github_webhook_refresh(
        &self,
        delivery: &str,
        scope: &github_webhook::Scope,
    ) -> Result<usize, FractalError> {
        let mut connection = self.connection.write();
        let transaction = connection.transaction().map_err(internal)?;
        let mut refreshed = 0;
        for (observer, revision, spec) in active_observers_tx(&transaction)? {
            let attempt = github_webhook::attempt(delivery, &observer);
            if !github_webhook::refreshes(&spec, scope)
                || requested(&transaction, &observer, &attempt).map_err(internal)?
            {
                continue;
            }
            append_observer_refresh_tx(&transaction, &self.origin, (&observer, &revision), &attempt, None)?;
            refreshed += 1;
        }
        transaction.commit().map_err(internal)?;
        Ok(refreshed)
    }

    /// [`coalesced_attempt`] on this store's readers.
    pub fn coalesced_refresh_attempt(&self, observer: &str, attempt: &str) -> Result<String> {
        coalesced_attempt(&self.readers.get(), observer, attempt).map_err(Into::into)
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

    fn history() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
             CREATE TABLE claims(store_index INTEGER PRIMARY KEY, id TEXT UNIQUE, batch_id TEXT,
               subject TEXT, kind TEXT, body TEXT, accepted_at_unix_ms TEXT);
             CREATE INDEX claims_subject_kind_index ON claims(subject,kind,store_index);
             CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY, position INTEGER);
             CREATE TABLE replica_records(claim_id TEXT, position INTEGER);",
        ).unwrap();
        connection
    }

    fn history_claim(
        connection: &Connection,
        id: &str,
        observer: &str,
        kind: &str,
        attempt: Option<&str>,
        origin: Option<&str>,
        time: &str,
    ) {
        connection.execute("INSERT INTO batches VALUES(?1,?2,1)", params![id, origin]).unwrap();
        connection.execute("INSERT INTO claim_positions VALUES(?1,0)", [id]).unwrap();
        connection.execute(
            "INSERT INTO claims(id,batch_id,subject,kind,body,accepted_at_unix_ms)
             VALUES(?1,?1,?2,?3,?4,?5)",
            params![id, observer, kind, json!({"fields":{"attempt":attempt}}).to_string(), time],
        ).unwrap();
    }

    fn old_pending_sql() -> String {
        canonical_sql(&format!(
            "WITH answered(attempt) AS (
               SELECT json_extract(result.body, '$.fields.attempt') FROM claims result
               WHERE result.subject=?1 AND result.kind IN ('observer.observed', 'observer.state')
                 AND json_extract(result.body, '$.fields.attempt') IS NOT NULL
             )
             SELECT json_extract(request.body, '$.fields.attempt'),
                    (SELECT origin FROM batches WHERE id=request.batch_id)
             FROM claims request
             WHERE request.subject=?1 AND request.kind='observer.refresh-requested'
               AND (json_extract(request.body, '$.fields.attempt') IS NULL
                 OR json_extract(request.body, '$.fields.attempt') NOT IN (SELECT attempt FROM answered))
               AND NOT (json_extract(request.body, '$.fields.attempt') LIKE ?2 AND EXISTS (
                 SELECT 1 FROM claims later
                 WHERE later.subject=?1 AND later.kind='observer.refresh-requested'
                   AND json_extract(later.body, '$.fields.attempt') LIKE ?2
                   AND json_extract(later.body, '$.fields.attempt') IN (SELECT attempt FROM answered)
                   AND (SELECT origin FROM batches WHERE id=later.batch_id)
                     IS (SELECT origin FROM batches WHERE id=request.batch_id)
                   AND {}
               ))
             ORDER BY CANONICAL_ASC(request)",
            canonical::after_sql("later", "request"),
        ))
    }

    #[test]
    fn pending_query_matches_legacy_history_semantics() {
        let connection = history();
        let webhook = |name: &str| format!("{}{name}", github_webhook::ATTEMPT_PREFIX);
        let first = webhook("first");
        let last = webhook("last");
        let other = webhook("other");
        let null_first = webhook("null-first");
        let null_last = webhook("null-last");
        let fixtures = [
            ("z", "observer/test", "observer.refresh-requested", Some(first.as_str()), Some("a"), "9"),
            ("a", "observer/test", "observer.refresh-requested", Some(last.as_str()), Some("a"), "10"),
            ("b", "observer/test", "observer.refresh-requested", Some(other.as_str()), Some("b"), "9"),
            ("c", "observer/test", "observer.refresh-requested", Some("declared"), Some("a"), "9"),
            ("d", "observer/test", "observer.state", Some(last.as_str()), Some("a"), "11"),
            ("e", "observer/test", "observer.refresh-requested", Some(null_first.as_str()), None, "10"),
            ("f", "observer/test", "observer.refresh-requested", Some(null_last.as_str()), None, "10"),
            ("g", "observer/test", "observer.observed", Some(null_last.as_str()), None, "11"),
            ("h", "observer/elsewhere", "observer.state", Some(other.as_str()), Some("b"), "12"),
            ("i", "observer/test", "observer.state", None, Some("a"), "13"),
            ("j", "observer/test", "observer.refresh-requested", None, None, "9"),
            ("k", "observer/test", "observer.refresh-requested", None, Some("a"), "8"),
            ("l", "observer/test", "observer.refresh-requested", Some("declared-new"), Some("a"), "20"),
            ("m", "observer/test", "observer.refresh-requested", None, Some("b"), "30"),
            ("n", "observer/test", "observer.refresh-requested", Some(last.as_str()), Some("b"), "31"),
        ];
        type Pending = Vec<(Option<String>, Option<String>)>;
        let read = |sql: &str| -> Pending {
            connection.prepare(sql).unwrap()
                .query_map(params!["observer/test", format!("{}%", github_webhook::ATTEMPT_PREFIX)],
                    |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap().collect::<rusqlite::Result<_>>().unwrap()
        };
        for (id, observer, kind, attempt, origin, time) in fixtures {
            history_claim(&connection, id, observer, kind, attempt, origin, time);
            let expected = read(&old_pending_sql());
            assert_eq!(read(&pending_requests_sql(false)), expected, "{id}");
            assert_eq!(read(&pending_requests_sql(true)), expected.into_iter().take(1).collect::<Pending>(), "{id}");
        }
        // Answers carry attempts, not origins. The final request repeats an answered attempt
        // on b, covering b's earlier webhook and making its missing-attempt predicate NULL.
        assert_eq!(
            read(&pending_requests_sql(false)),
            vec![
                (Some("declared".into()), Some("a".into())),
                (Some(null_first), None),
                (Some("declared-new".into()), Some("a".into())),
            ],
        );
    }

    #[test]
    fn oldest_reader_does_not_decode_the_pending_tail() {
        let connection = history();
        history_claim(&connection, "first", "observer/test", "observer.refresh-requested", Some("first"), Some("a"), "1");
        history_claim(&connection, "missing", "observer/test", "observer.refresh-requested", None, Some("a"), "2");
        assert_eq!(pending_attempt(&connection, "observer/test").unwrap().as_deref(), Some("first"));
        assert!(pending_requests(&connection, "observer/test").is_err());
        connection.execute("DELETE FROM claims WHERE id='first'", []).unwrap();
        assert!(pending_attempt(&connection, "observer/test").is_err());
    }

    #[test]
    fn pending_history_vm_steps_grow_subquadratically() {
        fn cost(count: usize, answered: bool) -> i32 {
            let connection = history();
            for index in 0..count {
                let id = format!("request-{index}");
                let attempt = format!("{}{index}", github_webhook::ATTEMPT_PREFIX);
                let origin = format!("origin-{}", index % 16);
                history_claim(&connection, &id, "observer/test", "observer.refresh-requested",
                    Some(&attempt), Some(&origin), &index.to_string());
                if answered && index % 2 == 0 {
                    history_claim(&connection, &format!("answer-{index}"), "observer/test", "observer.observed",
                        Some(&attempt), Some(&origin), &index.to_string());
                }
            }
            let mut statement = connection.prepare(&pending_requests_sql(true)).unwrap();
            let _: Option<String> = statement.query_row(
                params!["observer/test", format!("{}%", github_webhook::ATTEMPT_PREFIX)],
                |row| row.get(0),
            ).optional().unwrap();
            statement.get_status(rusqlite::StatementStatus::VmStep)
        }
        for answered in [false, true] {
            let small = cost(128, answered);
            let large = cost(512, answered);
            assert!(large < small * 8, "answered={answered}: {small} -> {large}");
        }
    }

    #[test]
    fn coalescing_keeps_member_boundaries_and_canonical_wire_order() {
        let connection = history();
        let first = format!("{}first", github_webhook::ATTEMPT_PREFIX);
        let newest = format!("{}newest", github_webhook::ATTEMPT_PREFIX);
        let elsewhere = format!("{}elsewhere", github_webhook::ATTEMPT_PREFIX);
        for (id, attempt, origin) in [
            ("z-first", first.as_str(), "a"),
            ("a-newest", newest.as_str(), "a"),
            ("other", elsewhere.as_str(), "b"),
            ("declared", "declared", "a"),
        ] {
            history_claim(&connection, id, "observer/test", "observer.refresh-requested",
                Some(attempt), Some(origin), "10");
        }
        // Arrival and lexical IDs disagree with the wire order within the first batch.
        connection.execute("UPDATE claims SET batch_id='z-first' WHERE id='a-newest'", []).unwrap();
        connection.execute("UPDATE claim_positions SET position=1 WHERE claim_id='a-newest'", []).unwrap();
        assert_eq!(pending_requests(&connection, "observer/test").unwrap(), vec![
            ("declared".into(), Some("a".into())),
            (first.clone(), Some("a".into())),
            (newest.clone(), Some("a".into())),
            (elsewhere.clone(), Some("b".into())),
        ]);
        assert_eq!(coalesced_attempt(&connection, "observer/test", &first).unwrap(), newest);
        assert_eq!(coalesced_attempt(&connection, "observer/test", &elsewhere).unwrap(), elsewhere);
        assert_eq!(coalesced_attempt(&connection, "observer/test", "declared").unwrap(), "declared");
        let absent = format!("{}absent", github_webhook::ATTEMPT_PREFIX);
        assert_eq!(coalesced_attempt(&connection, "observer/test", &absent).unwrap(), absent);
        history_claim(&connection, "answer", "observer/test", "observer.observed",
            Some(&newest), Some("a"), "11");
        assert_eq!(coalesced_attempt(&connection, "observer/test", &first).unwrap(), first);
        assert_eq!(pending_attempt(&connection, "observer/test").unwrap().as_deref(), Some("declared"));
        let legacy: Vec<(String, Option<String>)> = connection
            .prepare(&old_pending_sql())
            .unwrap()
            .query_map(
                params!["observer/test", format!("{}%", github_webhook::ATTEMPT_PREFIX)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(pending_requests(&connection, "observer/test").unwrap(), legacy);
    }

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
    fn deliveries_during_a_poll_are_answered_together_by_the_next_one() {
        let store = store();
        let scope = github_webhook::Scope { repository: "acme/garden".into(), branch: None };
        let attempt = |delivery: &str| github_webhook::attempt(delivery, "observer/garden");
        let pending = || pending_attempt(&store.readers.get(), "observer/garden").unwrap();
        let revision = store.selected_desired_revision("observer/garden").unwrap().unwrap();
        let answer = |attempt: String| {
            store
                .append_claim(&ClaimInput {
                    subject: "observer/garden".into(),
                    kind: "observer.state".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("attempt".into(), json!(attempt)),
                        ("revision".into(), json!(revision)),
                        ("state".into(), json!("healthy")),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        };
        let coalesced = |pending: String| store.coalesced_refresh_attempt("observer/garden", &pending).unwrap();
        assert_eq!(store.request_github_webhook_refresh("delivery-1", &scope).unwrap(), 1);
        assert_eq!(pending(), Some(attempt("delivery-1")));
        assert_eq!(coalesced(attempt("delivery-1")), attempt("delivery-1"));
        // The poller has taken delivery-1's refresh and may have read GitHub already, so each
        // delivery while it polls adds a request; a redelivery adds none. The running poll keeps
        // its name.
        assert_eq!(store.request_github_webhook_refresh("delivery-2", &scope).unwrap(), 1);
        assert_eq!(store.request_github_webhook_refresh("delivery-3", &scope).unwrap(), 1);
        assert_eq!(store.request_github_webhook_refresh("delivery-1", &scope).unwrap(), 0);
        assert_eq!(requests(&store, "observer/garden").len(), 3);
        assert_eq!(pending(), Some(attempt("delivery-1")));
        answer(attempt("delivery-1"));
        // The next poll is named for the oldest and answers the newest, which answers both.
        assert_eq!(pending(), Some(attempt("delivery-2")));
        assert_eq!(coalesced(attempt("delivery-2")), attempt("delivery-3"));
        answer(attempt("delivery-3"));
        assert_eq!(pending(), None);
        assert_eq!(store.request_github_webhook_refresh("delivery-3", &scope).unwrap(), 0);
        assert_eq!(pending(), None);

        // A declared refresh keeps its place and answers only itself.
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        append_observer_refresh_tx(&transaction, "node", ("observer/garden", &revision), "declared", None).unwrap();
        transaction.commit().unwrap();
        drop(connection);
        assert_eq!(store.request_github_webhook_refresh("delivery-4", &scope).unwrap(), 1);
        assert_eq!(pending(), Some("declared".into()));
        assert_eq!(coalesced("declared".into()), "declared");
        answer("declared".into());
        assert_eq!(pending(), Some(attempt("delivery-4")));
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
