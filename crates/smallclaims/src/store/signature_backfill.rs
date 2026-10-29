//! Signatures this member lacks for envelopes it has already admitted.
//!
//! Admission is final, but which window an envelope falls in depends on the membership this
//! node had folded when it admitted it. A writer migrated without a writer floor is keyed from
//! sequence 1 and signs its whole retained history when it joins. A member that admitted that
//! history unsigned, before the migration claim counted, never asks for those signatures, so it
//! cannot answer a fresh member that receives the claim first and holds the same envelopes as
//! unsigned. Each exchange therefore also asks for the window-key signatures of admitted
//! envelopes that lack one, and stores and serves what arrives. Nothing is admitted or revoked
//! here; only signatures are added.
//!
//! The scan walks keyed writers' envelopes in `(writer, sequence, hash)` order from a cursor,
//! a bounded number of rows per call, so every missing signature is asked for in turn even when
//! some peers cannot answer. A pass that finds nothing missing stops the scan, and so do passes
//! that find no fewer than the pass before them for an hour: signatures no reachable peer holds.
//! A membership change or a restart starts the scan again.

use super::*;

/// Envelope rows one call examines at most.
const SCAN_ROWS: usize = 4096;

/// How long passes may find no fewer missing signatures before the scan stops.
const STALL_MS: u128 = 60 * 60 * 1000;

#[derive(Debug, Default)]
pub struct SignatureBackfill {
    /// The `fleet_generation` this scan started at. A new generation starts a new scan.
    generation: Option<i64>,
    /// The last envelope examined.
    after: Option<(String, u64, String)>,
    /// Missing signatures seen so far in this pass.
    found: u64,
    /// Missing signatures seen in the last complete pass.
    previous: Option<u64>,
    /// When a complete pass first found no fewer than the one before it.
    stalled_since: Option<u128>,
    idle: bool,
}

impl Store {
    /// Up to `room` admitted envelopes in a keyed window without a signature by a window key.
    pub(super) fn signature_backfill_requests(&self, room: usize) -> Result<Vec<ReplicaEnvelopeId>> {
        if room == 0 {
            return Ok(Vec::new());
        }
        let mut state = self
            .signature_backfill
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let generation = fleet_generation(&self.readers.get())?;
        if state.generation != Some(generation) {
            *state = SignatureBackfill {
                generation: Some(generation),
                ..SignatureBackfill::default()
            };
        }
        if state.idle {
            return Ok(Vec::new());
        }
        let membership = self.fleet_membership()?;
        // Each keyed writer, from its lowest window start to its highest end.
        let mut windows = BTreeMap::<String, (u64, u64)>::new();
        for incarnation in membership.incarnations() {
            let end = incarnation.end.unwrap_or(i64::MAX as u64);
            windows
                .entry(incarnation.name.clone())
                .and_modify(|(start, last)| {
                    *start = (*start).min(incarnation.start);
                    *last = (*last).max(end);
                })
                .or_insert((incarnation.start, end));
        }
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT sequence, envelope_hash, receipt_state,
                    (SELECT json_group_array(member_key) FROM replica_envelope_signatures AS signatures
                     WHERE signatures.writer=envelopes.writer AND signatures.sequence=envelopes.sequence
                       AND signatures.envelope_hash=envelopes.envelope_hash)
             FROM replica_envelopes AS envelopes
             WHERE writer=?1 AND (sequence, envelope_hash)>(?2, ?3) AND sequence<=?4
             ORDER BY sequence, envelope_hash LIMIT ?5",
        )?;
        let mut wanted = Vec::new();
        let mut budget = SCAN_ROWS;
        let first = state.after.as_ref().map(|(writer, _, _)| writer.clone());
        let range = match &first {
            Some(writer) => windows.range(writer.clone()..),
            None => windows.range::<String, _>(..),
        };
        let mut finished = true;
        for (writer, (start, end)) in range {
            let (sequence, hash) = match &state.after {
                Some((after, sequence, hash)) if after == writer => (*sequence, hash.clone()),
                // `(start, '')` sorts before every envelope at `start`.
                _ => (*start, String::new()),
            };
            let rows = statement
                .query_map(
                    params![writer, sequence, hash, end, budget as i64],
                    |row| {
                        Ok((
                            row.get::<_, u64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            let exhausted = rows.len() < budget;
            for (sequence, hash, receipt_state, signers) in rows {
                budget -= 1;
                state.after = Some((writer.clone(), sequence, hash.clone()));
                // A held envelope already asks for its signature as unsigned.
                if receipt_state != "validated" {
                    continue;
                }
                let keys = membership.keys_for(writer, sequence);
                let signers = serde_json::from_str::<BTreeSet<String>>(&signers)?;
                if keys.is_empty() || signers.iter().any(|signer| keys.contains(signer)) {
                    continue;
                }
                state.found += 1;
                wanted.push(ReplicaEnvelopeId {
                    writer: writer.clone(),
                    sequence,
                    hash,
                });
                if wanted.len() == room {
                    break;
                }
            }
            if wanted.len() == room || budget == 0 || !exhausted {
                finished = false;
                break;
            }
            // Past every envelope of this writer: the next call starts after it.
            state.after = Some((writer.clone(), i64::MAX as u64, String::new()));
        }
        if finished {
            let found = std::mem::take(&mut state.found);
            if state.previous.is_some_and(|previous| found >= previous) {
                let now = now_ms();
                let since = *state.stalled_since.get_or_insert(now);
                state.idle = now.saturating_sub(since) >= STALL_MS;
            } else {
                state.stalled_since = None;
            }
            state.idle |= found == 0;
            state.previous = Some(found);
            state.after = None;
        }
        Ok(wanted)
    }
}
