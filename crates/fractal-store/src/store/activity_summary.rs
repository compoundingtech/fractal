//! Read-only ownership evidence: reuse a valid roster fold, otherwise inspect a bounded prefix.
//! Never populate the roster cache here: cold and reordered histories must not trigger a replay.
use super::*;

const FIELDS: [&str; 3] = ["status", "workspace", "host"];
pub(super) const CANDIDATES: usize = 64;

// Unlike canonical::position_sql, this must never infer legacy position by counting a batch.
// Both stored-position arms are point/index seeks; absence is an ordering uncertainty.
const STORED_KEY_SQL: &str = "SELECT length(claims.accepted_at_unix_ms),claims.accepted_at_unix_ms,
    batches.origin,batches.replica_sequence,claims.batch_id,
    COALESCE((SELECT position FROM claim_positions WHERE claim_id=claims.id LIMIT 1),
        (SELECT position FROM replica_records INDEXED BY replica_records_claim
         WHERE claim_id=claims.id ORDER BY position LIMIT 1))
    FROM claims JOIN batches ON batches.id=claims.batch_id WHERE claims.id=?1 LIMIT 1";

fn candidate_sql() -> String {
    format!("SELECT id,kind,accepted_at_unix_ms,store_index FROM claims INDEXED BY claims_actual_accepted_index
        WHERE subject=?1 AND {ACTUAL_STATE_CLAIM}
        ORDER BY length(accepted_at_unix_ms) DESC,accepted_at_unix_ms DESC LIMIT ?2")
}

impl Store {
    /// Exact fields when complete; unresolved fields are null with an explicit `unresolved` list.
    /// This path takes only a reader snapshot and briefly reads the existing roster cache.
    pub fn status_activity_summary_at(&self, selected: &str, at_index: Option<u64>) -> Result<Value> {
        self.read_snapshot(|_| {
            let connection = self.readers.get();
            let subject = identities::resolve_reference_conn(&connection, selected)?;
            anyhow::ensure!(subject.starts_with("agent/"), "activity summary requires an agent subject");
            let cut = selected_index(current_index(&connection)?, at_index).map_err(anyhow::Error::new)?;
            if !self.generic_status_subject_visible_at(&subject, cut)? {
                return Err(anyhow::Error::new(FractalError::new(
                    "notices-unavailable", "reserved notice facts are unavailable through generic status",
                ).with_detail("reason", "notices-unavailable")));
            }
            let revision: u64 = connection.query_row(
                "SELECT COALESCE((SELECT revision FROM card_actual_revisions WHERE subject=?1),0)",
                [&subject], |row| row.get(0))?;
            let head: u64 = connection.query_row(
                "SELECT COALESCE(MAX(store_index),0) FROM claims WHERE subject=?1 AND store_index<=?2",
                params![subject, cut], |row| row.get(0))?;
            let share = connection.is_autocommit() || connection.is_readonly(rusqlite::DatabaseName::Main)?;
            let cached = if share {
                let cache = self.fractal.card_actuals.lock().unwrap_or_else(PoisonError::into_inner);
                cache.get(&(subject.clone(), true))
                    .filter(|entry| entry.revision == revision && entry.through <= cut && head <= entry.through)
                    .map(|entry| entry.activity_fields())
            } else { None };
            let (actual, complete, unresolved) = match cached {
                Some(actual) => (actual, true, Vec::new()),
                None => bounded_actual(&connection, &subject, cut)?,
            };
            Ok(json!({"store_index":cut, "complete":complete, "unresolved":unresolved,
                "subjects":[{"subject":subject, "actual":actual, "complete":complete, "unresolved":unresolved}]}))
        })
    }
}

fn bounded_actual(connection: &Connection, subject: &str, cut: u64) -> Result<(Option<Value>, bool, Vec<&'static str>)> {
    // Bound index entries BEFORE filtering repairs, historical cuts, or canonical tie sorting.
    // This keeps an old cut or a long repaired prefix from turning into an unbounded scan.
    let mut rows = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hf1ddc10f46d2 owner=fractal_store::store::activity_summary TODO=trace-statement-and-value-representation */ &candidate_sql())?
        .query_map(params![subject, CANDIDATES + 1], |row| Ok((row.get::<_,String>(0)?,
            row.get::<_,String>(1)?, row.get::<_,String>(2)?, row.get::<_,u64>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut truncated = rows.len() > CANDIDATES;
    if truncated {
        let boundary = rows.pop().expect("lookahead row").2;
        // The omitted row can precede every retained row of its accepted-time group in
        // canonical order. Discard that WHOLE group rather than report a possibly wrong value.
        rows.retain(|row| row.2 != boundary);
    }
    let mut candidates = Vec::with_capacity(rows.len());
    let mut keys = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h2f93cc0d062b owner=fractal_store::store::activity_summary TODO=trace-statement-and-value-representation */ STORED_KEY_SQL)?;
    for (id, kind, _, index) in rows {
        if index > cut || kind.as_bytes().get(..8).is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"harness.")) { continue; }
        let repaired: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=?1)",
            [&id], |row| row.get(0))?;
        if !repaired {
            let (prefix, position) = keys.query_row(/* sqlite-large-value: allow-for-now lv-h58276482e453 owner=fractal_store::store::activity_summary TODO=trace-statement-and-value-representation */ [&id], |row| Ok((
                (row.get::<_,usize>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?,
                 row.get::<_,Option<i64>>(3)?, row.get::<_,String>(4)?),
                row.get::<_,Option<i64>>(5)?,
            )))?;
            candidates.push(((prefix, position, id), kind));
        }
    }
    candidates.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    if let Some(index) = candidates.iter().position(|candidate| candidate.0.1.is_none()) {
        // The latest group without a stored position is an ordering barrier. Only groups
        // strictly newer than it are safe: sorting its known positions first would guess.
        let before = candidates[..index].partition_point(|candidate|
            candidate.0.0 > candidates[index].0.0);
        candidates.truncate(before);
        truncated = true;
    }
    let mut actual = serde_json::Map::new();
    let mut resolved = [false; FIELDS.len()];
    let mut seen = false;
    let registry = fractal_schema::registry();
    let mut body_query = connection.prepare_cached("SELECT body FROM claims WHERE id=?1")?;
    for ((_, _, id), kind) in candidates {
        let body: String = body_query.query_row(/* sqlite-large-value: allow-for-now lv-hbfc641a2239f owner=fractal_store::store::activity_summary TODO=trace-statement-and-value-representation */ [id], |row| row.get(0))?;
        #[cfg(any(test, feature = "test-support"))]
        ACTUAL_HISTORY_BODIES_DECODED.with(|count| count.set(count.get() + 1));
        seen = true;
        let body: Value = serde_json::from_str(&body)?;
        let mut incoming = match body {
            Value::Object(mut body) => match body.remove("fields") {
                Some(Value::Object(fields)) => fields,
                None => body,
                _ => continue,
            },
            _ => continue,
        };
        if kind == "resource.observed"
            && incoming.get("kind").and_then(Value::as_str).is_some_and(carries_opener)
            && incoming.get("attribution_only") == Some(&Value::Bool(true)) { continue; }
        if kind == fractal_schema::github_plugin::ITEM_STATE { continue; }
        let transition = registry.claim(&kind)
            .filter(|spec| spec.cardinality == fractal_schema::Cardinality::StateTransition);
        for (index, field) in FIELDS.iter().enumerate() {
            if resolved[index] { continue; }
            if let Some(value) = incoming.remove(*field) {
                actual.insert((*field).into(), value);
                resolved[index] = true;
            } else if transition.is_some_and(|spec| spec.fields.contains_key(*field)) {
                resolved[index] = true;
            }
        }
        if resolved.into_iter().all(|field| field) { break; }
    }
    let complete = !truncated || resolved.into_iter().all(|field| field);
    let mut unresolved = Vec::new();
    if !complete {
        for (index, field) in FIELDS.iter().enumerate() {
            if !resolved[index] {
                actual.insert((*field).into(), Value::Null);
                unresolved.push(*field);
            }
        }
    }
    Ok(((seen || !complete).then_some(Value::Object(actual)), complete, unresolved))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_summary_candidates_use_the_existing_ordered_index_without_a_sorter() {
        let store = Store::open_memory("node").unwrap();
        let connection = store.readers.get();
        let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {}", candidate_sql())).unwrap()
            .query_map(params!["agent/example", CANDIDATES + 1], |row| row.get::<_,String>(3)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert!(plan.iter().any(|line| line.contains("claims_actual_accepted_index")), "{plan:?}");
        assert!(!plan.iter().any(|line| line.contains("SCAN ") || line.contains("TEMP B-TREE")), "{plan:?}");
        let positions = connection.prepare(&format!("EXPLAIN QUERY PLAN {STORED_KEY_SQL}")).unwrap()
            .query_map(["candidate-id"], |row| row.get::<_,String>(3)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert!(!positions.iter().any(|line| line.contains("SCAN ") || line.contains("TEMP B-TREE")), "{positions:?}");
    }

    #[test]
    fn activity_summary_never_counts_the_prefix_of_a_large_legacy_batch() {
        fn legacy(rows: usize) -> Store {
            let store = Store::open_memory("node").unwrap();
            {
                let connection = store.connection.write();
                connection.execute(
                    "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                     VALUES ('legacy','legacy-origin',1,'fixture','1')", [],
                ).unwrap();
                connection.execute(
                    "WITH RECURSIVE ordinal(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ordinal WHERE n<?1)
                     INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                     SELECT printf('legacy-%d',n),'legacy','agent/example/legacy','runtime.observed',
                        'legacy-origin','{\"fields\":{\"status\":\"running\",\"host\":\"observed\"}}','[]',
                        CAST(1000+n AS TEXT) FROM ordinal", [rows],
                ).unwrap();
                assert_eq!(connection.query_row(
                    "SELECT COUNT(*) FROM claim_positions JOIN claims ON claims.id=claim_positions.claim_id
                     WHERE claims.batch_id='legacy'", [], |row| row.get::<_,usize>(0),
                ).unwrap(), 0);
                assert_eq!(connection.query_row(
                    "SELECT COUNT(*) FROM replica_records WHERE claim_id LIKE 'legacy-%'",
                    [], |row| row.get::<_,usize>(0),
                ).unwrap(), 0);
            }
            store
        }
        let (few, many) = (legacy(128), legacy(12_000));
        let read = |store: &Store| {
            let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
            let result = store.status_activity_summary_at("agent/example/legacy", None).unwrap();
            (scope.finish(), result)
        };
        let (few_work, _) = read(&few);
        let (many_work, result) = read(&many);
        assert!(many_work.vm_steps <= few_work.vm_steps + 2_000,
            "canonical positions scanned legacy history: 128={few_work:?}, 12000={many_work:?}");
        assert_eq!(result["complete"], false);
        assert_eq!(result["unresolved"], json!(["status", "workspace", "host"]));
        assert_eq!(result["subjects"][0]["actual"], json!({"status":null,"workspace":null,"host":null}));
    }
}
