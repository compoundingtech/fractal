//! The part of `store::owned_sets` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::model::MemberSpec;
use serde::Deserialize;
use std::cell::RefCell;
pub use st3_store::store::owned_sets::*;
pub use st3_store::store::owned_sets::{Member, Options, Plan, Preview, Request, Revision, Source, View, claim, commit_tx, desired_at, effective_members};
pub use st3_store::store::owned_sets::{guard_member, guard_mission_start, manual_member, manual_member_at, one_shot_retirement_owner, owner, plan_tx, project_tx, refuse_unmanaged, rows, selected, subject};
pub use st3_store::store::owned_sets::{validate_apply, validate_receipt};

// One statement proves both absences in a current SQLite snapshot. Any receipt, including a
// repaired or malformed one, conservatively requires the existing full authority evaluation.
const EMPTY_DESIRED_AUTHORITY_QUERY: &str =
    "SELECT NOT EXISTS(SELECT 1 FROM claims WHERE kind='owned-set.revised')
        AND NOT EXISTS(SELECT 1 FROM claims WHERE subject=?1
                       AND json_extract(body,'$.owned_set') IS NOT NULL)
        AND NOT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='agent.retired'
                       AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired'))";

/// Reuse validated receipts only on the connection held by this fixed read snapshot. Other
/// connections and writer transactions still read authority directly. The guard restores an
/// enclosing scope on every exit, including errors and panics.
fn with_snapshot_rows<T>(connection: &Connection, read: impl FnOnce() -> T) -> T {
    debug_assert!(
        !connection.is_autocommit(),
        "receipt reuse requires a read snapshot"
    );
    struct Restore(Option<SnapshotRows>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SNAPSHOT_ROWS.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SNAPSHOT_ROWS.with(|slot| {
        slot.replace(Some(SnapshotRows {
            connection: connection as *const Connection as usize,
            rows: BTreeMap::new(),
        }))
    }));
    read()
}

/// The owning sets of every member at `at`, from one read of the set receipts. A member with
/// more than one is what [`owner`] refuses as a conflict.
pub fn owners_at(
    connection: &Connection,
    at: Option<u64>,
) -> Result<BTreeMap<String, BTreeSet<String>>, St3Error> {
    let mut owners = BTreeMap::<String, BTreeSet<String>>::new();
    for view in rows(connection, at)? {
        for member in view.receipt.members.keys().chain(view.receipt.retired.keys()) {
            owners.entry(member.clone()).or_default().insert(view.id.clone());
        }
    }
    Ok(owners)
}

/// A member's conflicts come from its owning set, rather than independent declaration leaves.
pub fn conflicts_at(
    connection: &Connection,
    member: &str,
    at: Option<u64>,
) -> Result<Option<Vec<String>>, St3Error> {
    let Some(set) = owner(connection, member, at)? else {
        return Ok(None);
    };
    let view = selected(connection, at)?
        .into_iter()
        .find(|v| v.id == set)
        .ok_or_else(|| St3Error::new("owned-set-pending", "missing owning set"))?;
    Ok(Some(if view.blockers.is_empty() {
        Vec::new()
    } else {
        vec![view.claim]
    }))
}

pub trait OwnedSetsAttentionStore {
    /// Called inside `read_snapshot`; holding this guard keeps the connection identity alive
    /// until the receipt cache is discarded, before the snapshot returns it to the pool.
    fn with_owned_set_snapshot_reads<T>(&self, read: impl FnOnce() -> T) -> T;
    /// Fence an external effect prepared from a declaration against the selected set.
    /// Effect callers must enter outside an enclosing read snapshot and wait for this
    /// method to return before writing or performing the effect.
    fn owned_desired_guard(&self, desired: &DesiredSubject) -> Result<(), St3Error>;
}

impl OwnedSetsAttentionStore for Store {

    fn with_owned_set_snapshot_reads<T>(&self, read: impl FnOnce() -> T) -> T {
        let connection = self.readers.get();
        with_snapshot_rows(&connection, read)
    }

    fn owned_desired_guard(&self, desired: &DesiredSubject) -> Result<(), St3Error> {
        smallclaims::sqlite::debug_assert_no_pinned_read();
        smallclaims::touched::note_read(|| "kind:owned-set.revised".into());
        smallclaims::touched::note_read(|| desired.subject.clone());
        let empty: bool = {
            let connection = self.readers.get();
            connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-h76635e22dc3f owner=st3_attention::store::owned_sets TODO=trace-statement-and-value-representation */ EMPTY_DESIRED_AUTHORITY_QUERY)
                .map_err(internal)?
                .query_row(/* sqlite-large-value: allow-for-now lv-h7b7ed61f4fe3 owner=st3_attention::store::owned_sets TODO=trace-statement-and-value-representation */ [&desired.subject], |row| row.get(0))
                .map_err(internal)?
        };
        if empty {
            return Ok(());
        }
        if let Some(current) = retirements::terminal_selection(&self.readers.get(), desired)? {
            return if current { Ok(()) } else {
                Err(St3Error::new("stale-set-member", "terminal retirement replaced this declaration"))
            };
        }
        // A negative probe is not authorization. Start another current snapshot for the full
        // fence and reuse receipt rows only within it, never across invocations or effects.
        self.read_snapshot(|_| {
            Ok(self.with_owned_set_snapshot_reads(|| {
                let connection = self.readers.get();
                if let Some(set) = owner(&connection, &desired.subject, None)? {
                    let view = selected(&connection, None)?
                        .into_iter()
                        .find(|v| v.id == set)
                        .ok_or_else(|| St3Error::new("owned-set-pending", "missing owning set"))?;
                    if !view.blockers.is_empty() {
                        return Err(St3Error::new("owned-set-pending", view.blockers.join("; ")));
                    }
                    let members = effective_members(&connection, &view, None)?;
                    let member = members.get(&desired.subject);
                    if member.is_none_or(|(m, _, _)| m.revision != desired_revision(desired)) {
                        return Err(St3Error::new(
                            "stale-set-member",
                            "declaration changed since this effect was prepared",
                        ));
                    }
                } else {
                    let staged: bool = connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND json_extract(body,'$.owned_set') IS NOT NULL)",
                        [&desired.subject], |row| row.get(0),
                    ).map_err(internal)?;
                    if staged {
                        return Err(St3Error::new("owned-set-pending", "staged member awaits its owning set revision"));
                    }
                }
                Ok(())
            }))
        }).map_err(internal)?
    }
}

