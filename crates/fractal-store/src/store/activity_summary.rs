//! Read-only ownership evidence: reuse a valid roster fold, otherwise inspect a bounded prefix.
//! Never populate the roster cache here: cold and reordered histories must not trigger a replay.
use super::*;

const FIELDS: [&str; 3] = ["status", "workspace", "host"];
pub(super) const CANDIDATES: usize = 64;

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
    let mut rows = connection.prepare_cached(&candidate_sql())?
        .query_map(params![subject, CANDIDATES + 1], |row| Ok((row.get::<_,String>(0)?,
            row.get::<_,String>(1)?, row.get::<_,String>(2)?, row.get::<_,u64>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let truncated = rows.len() > CANDIDATES;
    if truncated {
        let boundary = rows.pop().expect("lookahead row").2;
        // The omitted row can precede every retained row of its accepted-time group in
        // canonical order. Discard that WHOLE group rather than report a possibly wrong value.
        rows.retain(|row| row.2 != boundary);
    }
    let mut candidates = Vec::with_capacity(rows.len());
    for (id, kind, _, index) in rows {
        if index > cut || kind.as_bytes().get(..8).is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"harness.")) { continue; }
        let repaired: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=?1)",
            [&id], |row| row.get(0))?;
        if !repaired { candidates.push((card_actual::key(connection, &id)?, id, kind)); }
    }
    candidates.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    let mut actual = serde_json::Map::new();
    let mut resolved = [false; FIELDS.len()];
    let mut seen = false;
    let registry = fractal_schema::registry();
    let mut body_query = connection.prepare_cached("SELECT body FROM claims WHERE id=?1")?;
    for (_, id, kind) in candidates {
        let body: String = body_query.query_row([id], |row| row.get(0))?;
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
    }
}
