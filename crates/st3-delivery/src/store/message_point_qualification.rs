#![allow(unused_imports)]
use crate::message_desired_recipient_match;
use crate::message_sent_recipient_match;
use super::*;
use std::fmt;
use std::io::{self, Write};
use st3_store::store::message_point_qualification_shared_delivery::*;
use st3_store::store::message_point_qualification_shared_delivery::{AUX_PER_KEY, AUX_ROWS, Accounting, CLAIM_FIELDS, CORE_BYTES, Cause, ClaimEdge, CoreSize, DESIRED_FIELDS, Domains};
use st3_store::store::message_point_qualification_shared_delivery::{Header, KEY_BYTES, METADATA_BYTES, OWN_ROWS, Point, PointCore, Qualified, REMINDER_BYTES, SOURCE_BYTES, Stage};
use st3_store::store::message_point_qualification_shared_delivery::{admit_batch, admit_desired, admit_fields, admit_global, admit_replica_and_repair, admitted_core_bytes, charge, check, domains, metadata_sql};
use st3_store::store::message_point_qualification_shared_delivery::{own_headers_sql, wide};
#[cfg(test)]
mod tests;

fn capture_point(store: &Store, subject: &str) -> Qualified<Option<Point>> {
    capture_with(store, subject, |_| {})
}

fn capture_with(
    store: &Store,
    subject: &str,
    mut hook: impl FnMut(Stage),
) -> Qualified<Option<Point>> {
    check()?;
    if subject.is_empty() || subject.len() > KEY_BYTES {
        return Err(wide("subject", KEY_BYTES));
    }
    if subject.starts_with("arrangement/") {
        return Err(Cause::Unsupported(
            "arrangement actual-state dependency graph",
        ));
    }
    // Preserve typed point failures inside the one snapshot, rather than flattening them
    // into a generic snapshot error. Reentry correctly joins the actual outer physical cut.
    store
        .read_snapshot(|cut| Ok(capture_pinned(store, subject, cut, &mut hook)))
        .map_err(Cause::Snapshot)?
}

fn capture_pinned(
    store: &Store,
    subject: &str,
    cut: u64,
    hook: &mut impl FnMut(Stage),
) -> Qualified<Option<Point>> {
    check()?;
    if !store.readers.has_pinned_reader() {
        return Err(Cause::InvalidSource("missing registered reader pin"));
    }
    let db = store.readers.get();
    if db.is_autocommit() || current_index(&db).map_err(Cause::Canonical)? != cut {
        return Err(Cause::InvalidSource("physical snapshot mismatch"));
    }
    let encoding: String = db.query_row("PRAGMA encoding", [], |row| row.get(0))?;
    if encoding != "UTF-8" {
        return Err(Cause::Unsupported("database encoding"));
    }
    let mut accounting = Accounting::default();
    accounting.source(subject.len())?;
    let query = own_headers_sql();
    let mut statement = db.prepare_cached(/* sqlite-large-value: allow-for-now lv-h92f839eeee37 owner=st3_store::store::message_point_qualification TODO=trace-statement-and-value-representation */ &query)?;
    let mut rows = statement.query(params![subject, cut])?;
    let mut headers = Vec::new();
    while let Some(row) = rows.next()? {
        check()?;
        if headers.len() == OWN_ROWS {
            return Err(wide("own claims", OWN_ROWS));
        }
        let index: u64 = row.get(0)?;
        accounting.metadata(CLAIM_FIELDS.len() * 32)?;
        let widths = admit_fields(row, 1, &CLAIM_FIELDS, false, &mut accounting)?;
        // IDs/batch IDs in preflight, kind/body in latest_actual, all fields in claims_for.
        let bytes = widths
            .iter()
            .try_fold(0usize, |n, b| n.checked_add(*b))
            .ok_or_else(|| wide("source copies", SOURCE_BYTES))?;
        accounting.source(bytes)?;
        // Also admit a conservative JSON-source copy in the sent-recipient SQL.
        for ordinal in [0, 1, 3, 6, 6] {
            accounting.source(widths[ordinal])?;
        }
        headers.push(Header { index, widths });
    }
    drop(rows);
    drop(statement);
    accounting.own_rows = headers.len();
    hook(Stage::AfterOwnHeaders);
    check()?;
    if headers.is_empty() {
        return Ok(None);
    }

    let mut batches = BTreeSet::new();
    let mut ids = Vec::with_capacity(headers.len());
    for header in &headers {
        check()?;
        let (id, batch): (String, String) = db.query_row(
            "SELECT id,batch_id FROM claims WHERE store_index=?1",
            [header.index],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if id.len() != header.widths[0] || batch.len() != header.widths[1] {
            return Err(Cause::InvalidSource("admitted identity changed"));
        }
        ids.push(id);
        batches.insert(batch);
    }
    for batch in batches {
        admit_batch(&db, &batch, &mut accounting)?;
    }
    for id in &ids {
        admit_replica_and_repair(&db, id, &mut accounting)?;
    }
    admit_desired(&db, subject, &mut accounting)?;
    let global = admit_global(&db, subject, headers[0].index, &mut accounting)?;
    hook(Stage::BeforeCanonical);
    check()?;

    let created = headers[0].index;
    let message = message_view_tx(&db, subject, created).map_err(Cause::Canonical)?;
    check()?;
    if message.to.len() > KEY_BYTES {
        return Err(wide("canonical recipient", KEY_BYTES));
    }
    let reminder = message_reminder_rank(&message.tags)
        .map(|(key, version)| {
            if key.len() > REMINDER_BYTES {
                return Err(wide("reminder", REMINDER_BYTES));
            }
            Ok((key.to_owned(), version))
        })
        .transpose()?;
    let (person_candidate, any_closed) = domains(&db, subject, &message.to)?;
    if global.is_some_and(|closed| closed != any_closed) {
        return Err(Cause::InvalidSource("global message closed state"));
    }
    let global_exists = global.is_some();
    let open = !any_closed && message.status != "closed";
    // Literal existing reader; the pin and preflight cover every joined source field.
    let history = store.claims_for(subject, None).map_err(Cause::Canonical)?;
    check()?;
    let first = history
        .first()
        .ok_or(Cause::InvalidSource("canonical history missing"))?;
    let last = history
        .last()
        .ok_or(Cause::InvalidSource("canonical history missing"))?;
    let session_id = first
        .body
        .get("fields")
        .unwrap_or(&first.body)
        .get("session_id")
        .cloned()
        .unwrap_or(Value::Null);
    let core = PointCore {
        message,
        first: ClaimEdge {
            id: first.id.clone(),
            accepted_at_unix_ms: first.accepted_at_unix_ms,
        },
        last: ClaimEdge {
            id: last.id.clone(),
            accepted_at_unix_ms: last.accepted_at_unix_ms,
        },
        session_id,
    };
    hook(Stage::AfterCanonical);
    check()?;
    accounting.core_bytes = admitted_core_bytes(&core)?;
    hook(Stage::BeforeReturn);
    check()?;
    Ok(Some(Point {
        physical_cut: cut,
        core,
        domains: Domains {
            global_exists,
            person_candidate,
            any_closed,
            open,
        },
        reminder,
        accounting,
    }))
}
