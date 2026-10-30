//! Exact sealed history for portable, independently verified rename proofs.
use super::*;

impl Store {
    /// Seal outstanding local batches and export the complete retained envelope history.
    ///
    /// Keeping the writer prefixes includes membership removals and key revocations, not
    /// merely the positive facts an exporter would like a verifier to see. Missing or
    /// checkpoint-pruned prefixes fail closed in the independent proof verifier.
    pub fn rename_proof_envelopes(&self) -> Result<Vec<ReplicaEnvelope>> {
        self.seal_local_batches()?;
        let connection = self.readers.get();
        let membership = fleet_membership_tx(&connection)?;
        let anchor = fleet_meta(&connection, "fleet_anchor_key")?;
        let fleet_id = fleet_meta(&connection, "fleet_id")?
            .ok_or_else(|| anyhow::anyhow!("rename proofs require a bound fleet"))?;
        let mut statement = connection.prepare(
            "SELECT writer,sequence,envelope_hash FROM replica_envelopes ORDER BY writer,sequence,envelope_hash",
        )?;
        let identities = statement.query_map([], |row| Ok(ReplicaEnvelopeId {
            writer: row.get(0)?, sequence: row.get(1)?, hash: row.get(2)?,
        }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        drop(connection);
        // Replica serving omits retired payloads. The verifier detects any resulting gap.
        let mut envelopes = self.replica_envelopes(identities)?;
        let connection = self.readers.get();
        let mut signatures = connection.prepare(
            "SELECT member_key,signature FROM replica_envelope_signatures WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 ORDER BY member_key",
        )?;
        for envelope in &mut envelopes {
            let window = membership.window(&envelope.writer, envelope.sequence);
            let message = crate::fleet::envelope_signature_message(
                &fleet_id, &envelope.writer, envelope.sequence, &envelope.hash,
            );
            let mut selected = None;
            for candidate in signatures.query_map(
                params![envelope.writer, envelope.sequence, envelope.hash],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )? {
                let (key, signature) = candidate?;
                let authorized = match &window {
                    crate::fleet::Window::Keyed(keys) => keys.contains(&key),
                    crate::fleet::Window::Legacy | crate::fleet::Window::Fenced => false,
                } || anchor.as_deref() == Some(key.as_str());
                if authorized && crate::fleet::verify_signature(&key, &message, &signature) {
                    selected = Some((key, signature));
                    break;
                }
            }
            let (key, signature) = selected.ok_or_else(|| anyhow::anyhow!(
                "no authorized sealed signature for {}:{}", envelope.writer, envelope.sequence,
            ))?;
            envelope.member_key = Some(key);
            envelope.signature = Some(signature);
        }
        Ok(envelopes)
    }
}
