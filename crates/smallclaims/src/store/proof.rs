//! Exact sealed history for portable, independently verified rename proofs.
use super::*;

const PROOF_INVENTORY_PAGE: i64 = 64;

fn proof_root(kind: &str) -> bool {
    kind.starts_with("fleet.") || matches!(kind,
        "agent.renamed" | "mission.renamed" | "intent.desired" | "mission.published"
        | "principal.key-granted" | "principal.key-revoked" | "record.repaired")
}

impl Store {
    /// Export complete retained prefixes for naming, membership and authorization evidence.
    /// Exact co-batched payloads are fleet-internal; callers must obtain disclosure consent.
    /// Unadmitted transport receipts are not evidence. Missing required prefixes fail closed.
    pub fn rename_proof_envelopes(&self) -> Result<Vec<ReplicaEnvelope>> {
        self.collect_rename_proof_envelopes(&mut |_| Ok(()))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn rename_proof_envelopes_with_page_observer(
        &self, mut observer: impl FnMut(usize) -> Result<()>,
    ) -> Result<Vec<ReplicaEnvelope>> {
        self.collect_rename_proof_envelopes(&mut observer)
    }

    fn collect_rename_proof_envelopes(
        &self, observer: &mut impl FnMut(usize) -> Result<()>,
    ) -> Result<Vec<ReplicaEnvelope>> {
        self.seal_local_batches()?;
        let (membership, fleet_id, high) = {
            let connection = self.readers.get();
            (fleet_membership_tx(&connection)?,
             fleet_meta(&connection, "fleet_id")?
                .ok_or_else(|| anyhow::anyhow!("rename proofs require a bound fleet"))?,
             max_envelope_rowid(&connection)?)
        };
        let mut writers = BTreeSet::new();
        let mut nonces = BTreeMap::<String, BTreeSet<String>>::new();
        let mut operations = BTreeSet::new();
        // Naming reservations and negative security facts can belong to other identities.
        // Root them globally, not merely at the selected storage subject.
        self.visit_proof_inventory(high, &membership, observer, &mut |envelope| {
            let payload: ReplicaEnvelopePayload = ciborium::from_reader(envelope.payload.bytes()?)?;
            for claim in &payload.batch.claims {
                if proof_root(&claim.kind) {
                    writers.insert(envelope.writer.clone());
                    if let Some((operation, _)) = operation_parts(&claim.body) {
                        operations.insert(operation.to_owned());
                    }
                    if let Some(signature) = payload.claim_signatures.get(&claim.id) {
                        nonces.entry(signature.key.clone()).or_default().insert(signature.nonce.clone());
                    }
                }
            }
            Ok(())
        })?;
        // Operation conflicts and nonce replays may originate on otherwise unrelated
        // writers. Include their prefixes and close over their authentication nonces.
        loop {
            let before = nonces.values().map(BTreeSet::len).sum::<usize>();
            self.visit_proof_inventory(high, &membership, observer, &mut |envelope| {
                let payload: ReplicaEnvelopePayload = ciborium::from_reader(envelope.payload.bytes()?)?;
                for claim in &payload.batch.claims {
                    let signature = payload.claim_signatures.get(&claim.id);
                    let operation = operation_parts(&claim.body)
                        .is_some_and(|(id, _)| operations.contains(id));
                    let replay = signature.is_some_and(|signature| {
                        nonces.get(&signature.key).is_some_and(|used| used.contains(&signature.nonce))
                    });
                    if operation || replay {
                        writers.insert(envelope.writer.clone());
                        if let Some(signature) = signature {
                            nonces.entry(signature.key.clone()).or_default().insert(signature.nonce.clone());
                        }
                    }
                }
                Ok(())
            })?;
            if before == nonces.values().map(BTreeSet::len).sum::<usize>() { break; }
        }
        let mut envelopes = Vec::new();
        self.visit_proof_inventory(high, &membership, observer, &mut |mut envelope| {
            if !writers.contains(&envelope.writer) { return Ok(()); }
            // Use the runtime membership rule, not a second proof-only window reducer.
            let keys = membership.keys_for(&envelope.writer, envelope.sequence);
            let message = crate::fleet::envelope_signature_message(
                &fleet_id, &envelope.writer, envelope.sequence, &envelope.hash,
            );
            let selected = {
                let connection = self.readers.get();
                let mut signatures = connection.prepare_cached(
                    "SELECT member_key,signature FROM replica_envelope_signatures WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 ORDER BY member_key",
                )?;
                let mut selected = None;
                for candidate in signatures.query_map(
                    params![envelope.writer, envelope.sequence, envelope.hash],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )? {
                    let (key, signature) = candidate?;
                    if keys.contains(&key) && crate::fleet::verify_signature(&key, &message, &signature) {
                        selected = Some((key, signature));
                        break;
                    }
                }
                selected
            };
            let (key, signature) = selected.ok_or_else(|| anyhow::anyhow!(
                "no authorized sealed signature for {}:{}", envelope.writer, envelope.sequence,
            ))?;
            envelope.member_key = Some(key);
            envelope.signature = Some(signature);
            envelopes.push(envelope);
            Ok(())
        })?;
        envelopes.sort_by(|a, b| (&a.writer, a.sequence, &a.hash).cmp(&(&b.writer, b.sequence, &b.hash)));
        Ok(envelopes)
    }

    fn visit_proof_inventory(
        &self, high: i64, membership: &crate::fleet::Membership,
        observer: &mut impl FnMut(usize) -> Result<()>,
        visit: &mut impl FnMut(ReplicaEnvelope) -> Result<()>,
    ) -> Result<()> {
        let mut after = 0;
        loop {
            let page = {
                let connection = self.readers.get();
                let mut statement = connection.prepare_cached(
                    "SELECT rowid,writer,sequence,envelope_hash FROM replica_envelopes
                     WHERE rowid>?1 AND rowid<=?2 AND batch_id IS NOT NULL
                     ORDER BY rowid LIMIT ?3",
                )?;
                statement.query_map(params![after, high, PROOF_INVENTORY_PAGE], |row| {
                    Ok((row.get::<_, i64>(0)?, ReplicaEnvelopeId {
                        writer: row.get(1)?, sequence: row.get(2)?, hash: row.get(3)?,
                    }))
                })?.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let Some((last, _)) = page.last() else { break; };
            after = *last;
            let count = page.len();
            // Runtime serving owns retirement filtering. Both the inventory statement and
            // its lease are gone before serving this bounded page, decoding, or callbacks.
            let envelopes = self.replica_envelopes(page.into_iter().map(|(_, id)| id).collect())?;
            observer(count)?;
            for envelope in envelopes {
                let held = {
                    let connection = self.readers.get();
                    fleet_admission_hold(&connection, membership, &envelope)?.is_some()
                };
                if !held { visit(envelope)?; }
            }
        }
        Ok(())
    }
}
