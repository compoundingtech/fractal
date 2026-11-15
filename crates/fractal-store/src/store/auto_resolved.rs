//! Person-scoped deadline resolutions, sharing the digest's live override predicate.
use super::*;

const CANDIDATES: usize = 512;
const ROWS: usize = 128;
const ASKS: &str = "SELECT id,subject,body FROM claims INDEXED BY claims_person_ask_recipient_index
    WHERE kind='work.person-asked' AND json_extract(body, '$.fields.person')=?1
    ORDER BY store_index DESC LIMIT ?2";
const RESOLUTIONS: &str = "SELECT id,CAST(accepted_at_unix_ms AS INTEGER) FROM claims INDEXED BY claims_subject_kind_index
    WHERE subject=?1 AND kind='work.person-done' AND actor='daemon/runtime'
    AND json_extract(body,'$.fields.resolved_by')='deadline'
    AND json_extract(body,'$.fields.episode')=?2 AND CAST(accepted_at_unix_ms AS INTEGER)>=?3
    ORDER BY CAST(accepted_at_unix_ms AS INTEGER) DESC,id DESC LIMIT 1";

impl Store {
    pub fn auto_resolved_asks(&self, person: &str, now: u128) -> Result<Value> {
        self.read_snapshot_unindexed(|| read(&self.readers.get(), person, now))
    }
}

fn read(connection: &Connection, person: &str, now: u128) -> Result<Value> {
    let asks = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-ha37e39b50670 owner=fractal_store::store::auto_resolved TODO=trace-statement-and-value-representation */ ASKS)?.query_map(params![person, CANDIDATES], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut complete = asks.len() < CANDIDATES;
    let since = i64::try_from(now.saturating_sub(ask_digest::OVERRIDE_WINDOW_MS))?;
    let mut items = Vec::new();
    let mut resolutions = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h5d36580c6dcb owner=fractal_store::store::auto_resolved TODO=trace-statement-and-value-representation */ RESOLUTIONS)?;
    for (episode, subject, body) in asks {
        let resolution = resolutions.query_row(
            /* sqlite-large-value: allow-for-now lv-h1e9da3eec7a8 owner=fractal_store::store::auto_resolved TODO=trace-statement-and-value-representation */
            params![subject, episode, since], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        ).optional()?;
        let Some((resolution, resolved_at)) = resolution else { continue };
        if u128::try_from(resolved_at)? > now { continue; }
        if let Some(mut row) = ask_digest::resolution_row(connection, &resolution, &episode, now)? {
            let body: Value = serde_json::from_str(&body)?;
            if let Some(request) = body["fields"].get("request").filter(|request| !request.is_null()) {
                row["request"] = crate::person_request::client_view(request);
            }
            items.push((resolved_at, resolution, row));
        }
    }
    items.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    if items.len() > ROWS { complete = false; items.truncate(ROWS); }
    Ok(json!({"items": items.into_iter().map(|(_, _, row)| row).collect::<Vec<_>>(), "complete": complete}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_use_person_and_subject_indexes() {
        let store = Store::open_memory("example").unwrap();
        let connection = store.readers.get();

        for (query, index, parameters) in [
            (ASKS, "claims_person_ask_recipient_index", vec![Value::from("person/avery"), Value::from(512)]),
            (RESOLUTIONS, "claims_subject_kind_index", vec![Value::from("step-run/example"), Value::from("episode"), Value::from(0)]),
        ] {
            let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap()
                .query_map(rusqlite::params_from_iter(parameters.iter().map(|value| match value {
                    Value::String(value) => rusqlite::types::Value::Text(value.clone()),
                    Value::Number(value) => rusqlite::types::Value::Integer(value.as_i64().unwrap()),
                    _ => unreachable!(),
                })), |row| row.get::<_, String>(3)).unwrap()
                .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
            assert!(plan.contains(index), "{plan}");
        }
        assert_eq!(store.auto_resolved_asks("person/avery", now_ms()).unwrap(), json!({"items": [], "complete": true}));
    }
    #[test]
    fn person_scope_window_override_and_omitted_fields() {
        let store = Store::open_memory("example").unwrap();
        let (ask, applied) = store.connection.batched(|tx| -> Result<_> {
            let ask = append_claim_tx(tx, &store.origin, "step-run/example", "work.person-asked",
                Some("agent/example"), &json!({"fields": {"person": "person/avery", "title": "Choose"}}), &[], None)?;
            let applied = append_claim_tx(tx, &store.origin, &ask.subject, "work.person-done",
                Some("daemon/runtime"), &json!({"fields": {"episode": ask.id, "resolved_by": "deadline"}}),
                std::slice::from_ref(&ask.id), None)?;
            Ok((ask, applied))
        }).unwrap().unwrap();
        let now = applied.accepted_at_unix_ms;
        let value = store.auto_resolved_asks("person/avery", now).unwrap();
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        let row = &value["items"][0];
        for optional in ["default_answer", "request", "action_parameters"] {
            assert!(row.get(optional).is_none(), "{row}");
        }
        assert_eq!(store.auto_resolved_asks("person/blair", now).unwrap()["items"], json!([]));
        assert_eq!(store.auto_resolved_asks("person/avery", now + ask_digest::OVERRIDE_WINDOW_MS + 1).unwrap()["items"], json!([]));
        store.connection.batched(|tx| -> Result<_> {
            append_claim_tx(tx, &store.origin, &ask.subject, "work.person-override", Some("person/avery"),
                &json!({"fields": {"episode": ask.id}}), &[], None)
        }).unwrap().unwrap();
        assert_eq!(store.auto_resolved_asks("person/avery", now_ms()).unwrap()["items"], json!([]));
    }

    #[test]
    fn candidate_and_result_caps_report_incomplete_newest_first() {
        let store = Store::open_memory("example").unwrap();
        store.connection.batched(|tx| -> Result<()> {
            for n in 0..CANDIDATES {
                let ask = append_claim_tx(tx, &store.origin, &format!("step-run/example-{n}"), "work.person-asked",
                    Some("agent/example"), &json!({"fields": {"person": "person/avery", "title": n.to_string()}}), &[], None)?;
                append_claim_tx(tx, &store.origin, &ask.subject, "work.person-done", Some("daemon/runtime"),
                    &json!({"fields": {"episode": ask.id, "resolved_by": "deadline", "answer": {"id":"hold","label":"Hold"}}}),
                    std::slice::from_ref(&ask.id), None)?;
            }
            Ok(())
        }).unwrap().unwrap();
        let value = store.auto_resolved_asks("person/avery", now_ms()).unwrap();
        assert_eq!(value["complete"], false);
        let items = value["items"].as_array().unwrap();
        assert_eq!(items.len(), ROWS);
        for rows in items.windows(2) {
            assert!(rows[0]["resolved_at"].as_str().unwrap() >= rows[1]["resolved_at"].as_str().unwrap());
        }
        assert!(items[0]["action_parameters"]["work.override"].is_object());
    }
}
