//! A seat whose run ended may still be running past its stop: the reconciler holds it while it
//! has mail delivered, or work assigned, before the stop was declared. Such a seat keeps the
//! mailbox of the declaration its stop ended, so it can read that mail and the hold drains. The
//! reconciler alone decides how long: once nothing from before the stop holds the seat it stops
//! the runtime, and the mailbox lease ends with the incarnation. Mail that arrives after the stop
//! is not offered to it and waits for the seat's next incarnation.
use super::*;
use crate::model::MemberSpec;

/// The stop declared for `seat`, if its declaration is one: the run whose end declared it
/// (`None` for a person's or an agent's own stop) and the stop claim's local store index.
pub fn stop_cut(connection: &Connection, seat: &str) -> Result<Option<(Option<String>, u64)>> {
    Ok(connection
        .query_row(
            "SELECT desired.owner_run, claims.store_index
             FROM desired JOIN claims ON claims.id=desired.claim_id
             WHERE desired.subject=?1 AND desired.kind='stop'",
            [seat],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)? as u64,
                ))
            },
        )
        .optional()?)
}

/// The member whose mailbox a stop a run declared still carries: the one it ended. A person's or
/// an agent's own stop carries none, and neither does a stop whose ended declaration is
/// ambiguous or gone.
pub fn stopping_member(connection: &Connection, seat: &str) -> Result<Option<MemberSpec>> {
    Ok(super::seats::declaration_ended_by_stop_tx(connection, seat)?
        .filter(|ended| ended.stop.owner_run.is_some())
        .and_then(|ended| ended.declaration.member))
}

impl Store {
    /// The store index of the stop a run declared for `seat`, while its declaration is one. Mail
    /// created at or after it is not offered to the stopping seat.
    pub fn stopping_seat_cut(&self, seat: &str) -> Result<Option<u64>> {
        smallclaims::touched::note_read(|| seat.to_owned());
        Ok(stop_cut(&self.readers.get(), seat)?
            .and_then(|(owner_run, cut)| owner_run.map(|_| cut)))
    }
}

impl Store {
    /// The member `seat`'s selected declaration authorizes its mailbox for, as
    /// `mailbox_lease::mailbox_member` decides it, with the declaration and the declaration its
    /// stop ended read in one snapshot: `None` when the seat is no longer declared, `Some(None)`
    /// when it is intentionally stopped.
    pub fn mailbox_member_named(
        &self,
        seat: &str,
    ) -> Result<Option<Option<MemberSpec>>, FractalError> {
        smallclaims::touched::note_read(|| seat.to_owned());
        self.read_snapshot_unindexed(|| {
            let connection = self.readers.get();
            let Some((kind, member)) = connection
                .query_row(
                    "SELECT kind, member FROM desired WHERE subject=?1",
                    [seat],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()?
            else {
                return Ok(None);
            };
            Ok(Some(super::mailbox_lease::mailbox_member(
                &connection,
                seat,
                &kind,
                member.as_deref(),
            )?))
        })
        .map_err(claim_append_error)
    }
}
