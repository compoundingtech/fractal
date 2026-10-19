//! The part of `store::attention_ivm` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use smallclaims::ivm::{Definition, LocalChange, View, Views, source_cut};
pub use st3_store::store::attention_ivm::*;
pub use st3_store::store::attention_ivm::{CUSTOM_VIEW, Family, PERSON_VIEW, clean, flush};
#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_seats;

pub fn definitions() -> Vec<Box<dyn View>> {
    vec![Box::new(Family::Person), Box::new(Family::Custom)]
}

/// Bounded explicit extraction for a fresh installation, outside GET/startup paths. The
/// installer supplies the source snapshot/cursor, owns an unpublished lifetime, and fences
/// all writes until catch-up and dependency certification finish. This is not publication.
pub fn seed_page(
    tx: &Transaction<'_>,
    view: &str,
    after: &str,
    limit: usize,
) -> Result<Vec<String>> {
    anyhow::ensure!(
        (1..=128).contains(&limit),
        "attention installation page exceeds bound"
    );
    let family = family(view)?;
    let query = match family {
        Family::Person => {
            "SELECT subject FROM step_runs WHERE assignee>='person/' AND assignee<'person0' AND subject>?1 ORDER BY subject LIMIT ?2"
        }
        Family::Custom => {
            "SELECT subject FROM custom_sources WHERE subject>?1 ORDER BY subject LIMIT ?2"
        }
    };
    let keys = tx
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h646426b92507 owner=st3_attention::store::attention_ivm TODO=trace-statement-and-value-representation */ query)?
        .query_map(params![after, limit], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for source in &keys {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h1a1c768b0ff4 owner=st3_attention::store::attention_ivm TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_attention_open_dirty VALUES(?1,?2)",
            params![family.name(), source],
        )?;
    }
    Ok(keys)
}

/// Build a bounded page in the installer's unpublished lifetime. The owner must reject
/// concurrent serving, finish all pages, verify complete source capture and then publish.
pub fn backfill_page(
    tx: &Transaction<'_>,
    captured_time: u128,
    limit: usize,
) -> Result<bool> {
    anyhow::ensure!(
        (1..=128).contains(&limit),
        "attention backfill page exceeds bound"
    );
    for (name, source) in dirty_page(tx, limit)? {
        let operator = match name.as_str() {
            "person" => Family::Person,
            "custom" => Family::Custom,
            _ => anyhow::bail!("unknown attention family"),
        };
        let ready: bool = tx.query_row(
            "SELECT COALESCE((SELECT ready FROM ivm_views WHERE name=?1),0)",
            [operator.view()],
            |r| r.get(0),
        )?;
        anyhow::ensure!(!ready, "cannot backfill a published attention family");
        let key = serde_json::to_string(&(name.as_str(), source.as_str()))?;
        operator.maintain_local_key(
            tx,
            &key,
            &LocalChange {
                kind: source_kind(&name)?.into(),
                old_keys: BTreeSet::new(),
                new_keys: BTreeSet::from([key.clone()]),
                evaluation_time_unix_ms: captured_time,
            },
        )?;
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-hf7bc2fdcc529 owner=st3_attention::store::attention_ivm TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_attention_open_dirty WHERE family=?1 AND source=?2",
            params![name, source],
        )?;
    }
    clean(tx, None)
}

/// Scheduled captured-time membership transitions. The owner passes its prior clock cut;
/// backwards clocks require a replacement snapshot. Each page is bounded and advances only
/// after commit/delivery. No timer, wall-clock read or polling fallback lives in this operator.
pub fn clock_page(
    tx: &Transaction<'_>,
    views: &Views,
    from: u128,
    to: u128,
    after: Option<(u128, String, String)>,
    limit: usize,
) -> Result<Vec<(u128, String, String)>> {
    anyhow::ensure!(
        to >= from,
        "attention clock moved backwards; recapture snapshot"
    );
    anyhow::ensure!(
        (1..=128).contains(&limit),
        "attention clock page exceeds bound"
    );
    let (cursor, cf, cs) = after.unwrap_or((from, String::new(), String::new()));
    let lo = from.to_string();
    let hi = to.to_string();
    let cursor = cursor.to_string();
    let keys = tx
        .prepare_cached(
            "SELECT eligible,family,source FROM local_attention_open
      WHERE (length(eligible)>length(?1) OR (length(eligible)=length(?1) AND eligible>?1))
      AND (length(eligible)<length(?2) OR (length(eligible)=length(?2) AND eligible<=?2))
      AND (length(eligible),eligible,family,source)>(length(?3),?3,?4,?5)
      ORDER BY length(eligible),eligible,family,source LIMIT ?6",
        )?
        .query_map(params![lo, hi, cursor, cf, cs, limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut output = Vec::new();
    for (at, name, source) in keys {
        let previous = source_cut(tx)?.context("attention clock source is unavailable")?;
        let key = serde_json::to_string(&(name.as_str(), source.as_str()))?;
        let changes = views.local_change(
            tx,
            &LocalChange {
                kind: match name.as_str() {
                    "person" => PERSON_CLOCK,
                    "custom" => CUSTOM_CLOCK,
                    _ => anyhow::bail!("unknown attention family"),
                }
                .into(),
                old_keys: BTreeSet::new(),
                new_keys: BTreeSet::from([key]),
                evaluation_time_unix_ms: to,
            },
            smallclaims::ivm::SourceCut {
                local_generation: previous
                    .local_generation
                    .checked_add(1)
                    .context("local generation exhausted")?,
                ..previous
            },
        )?;
        anyhow::ensure!(
            changes.deferred.is_empty(),
            "attention clock operator is unavailable"
        );
        output.push((at.parse()?, name, source));
    }
    Ok(output)
}

fn require_ready(connection: &Connection, views: &Views, view: &str) -> Result<()> {
    let source = source_cut(connection)?.context("attention source unavailable")?;
    anyhow::ensure!(
        matches!(
            views.readiness(connection, view, source.epoch)?,
            smallclaims::ivm::Readiness::Ready(_)
        ),
        "attention family unavailable"
    );
    anyhow::ensure!(
        clean(connection, Some(view))?,
        "attention family has unflushed sources"
    );
    Ok(())
}

/// This is a family window, not the whole attention collection. Call inside the authorized
/// snapshot, after checking that this family's availability is Ready at the same boundary.
pub fn window(
    connection: &Connection,
    views: &Views,
    view: &str,
    person: &str,
    at: u128,
    limit: usize,
) -> Result<Vec<Value>> {
    require_ready(connection, views, view)?;
    let family = match view {
        PERSON_VIEW => "person",
        CUSTOM_VIEW => "custom",
        _ => anyhow::bail!("unknown attention family"),
    };
    anyhow::ensure!(limit <= 501, "attention window exceeds bound");
    let at = at.to_string();
    let rows = connection
        .prepare_cached(
            "SELECT body FROM local_attention_open WHERE family=?1 AND person=?2
      AND (length(eligible)<length(?3) OR (length(eligible)=length(?3) AND eligible<=?3))
      ORDER BY priority,length(requested) DESC,requested DESC,source,person,episode LIMIT ?4",
        )?
        .query_map(params![family, person, at, limit], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|text| serde_json::from_str(&text).map_err(Into::into))
        .collect()
}

pub fn row(
    connection: &Connection,
    views: &Views,
    view: &str,
    person: &str,
    id: &str,
    at: u128,
) -> Result<Option<Value>> {
    require_ready(connection, views, view)?;
    let family = match view {
        PERSON_VIEW => "person",
        CUSTOM_VIEW => "custom",
        _ => anyhow::bail!("unknown attention family"),
    };
    let at = at.to_string();
    let text: Option<String> = connection
        .query_row(
            "SELECT body FROM local_attention_open WHERE family=?1 AND person=?2 AND id=?3
      AND (length(eligible)<length(?4) OR (length(eligible)=length(?4) AND eligible<=?4))",
            params![family, person, id, at],
            |r| r.get(0),
        )
        .optional()?;
    text.map(|text| serde_json::from_str(&text).map_err(Into::into))
        .transpose()
}
