//! Outbound rejection alerts are repair evidence, never the exclusion authority.
//! Only explicit operational repair deletes the matching durable park episode.
use super::*;

const CONDITION: &str = "replication-outbound-parked";

impl Store {
    /// Record an actionable graph fault before durably parking the exact target store's page.
    /// A crash between these commits leaves a retryable alert, not a false terminal success.
    pub fn park_outbound_with_failure(
        &self,
        target: &smallclaims::store::Store,
        peer: &str,
        hashes: &[String],
        code: &str,
    ) -> Result<(), FractalError> {
        if hashes.is_empty() { return Ok(()); }
        if std::ptr::eq(target, &self.graph) {
            // Same-store failure and parks commit atomically through the runtime's existing
            // transaction hook. No second loan of this writer is taken.
            return target.park_outbound(peer, hashes, code, None).map_err(internal);
        }
        if !matches!(code,
            "replication-ingress/raw-oversize" | "replication-ingress/inflated-oversize"
            | "replication-ingress/malformed-deflate" | "replication-ingress/malformed-json") {
            return Err(FractalError::new("invalid-outbound-park", "only terminal ingress codes can park a page"));
        }
        let scope = target.bound_shard().map_err(internal)?.unwrap_or_default();
        let origin = target.origin();
        let mut hashes = hashes.to_vec();
        hashes.sort();
        hashes.dedup();
        // Reuse the current destination/code episode even for rejected alert envelopes.
        let signature = canonical_hash(&(origin, &scope, peer, code)).map_err(internal)?;
        let subject = format!("daemon/{}", self.origin());
        let reviewer = self.trim_fault_reviewer();
        {
            let mut writer = self.connection.write();
            let transaction = writer.transaction().map_err(internal)?;
            let open = transaction.query_row(
                "SELECT json_extract(f.body,'$.fields.episode') FROM claims f
                 WHERE f.subject=?1 AND f.kind='operational.failure'
                   AND json_extract(f.body,'$.fields.condition')=?2
                   AND json_extract(f.body,'$.fields.reproduction.signature')=?3
                   AND NOT EXISTS(SELECT 1 FROM claims r WHERE r.subject=f.subject
                     AND r.kind='operational.recovered'
                     AND json_extract(r.body,'$.fields.failure')=f.id)
                 ORDER BY f.store_index DESC LIMIT 1",
                params![subject, CONDITION, signature], |row| row.get::<_, String>(0),
            ).optional().map_err(internal)?;
            let episode = match open {
                Some(episode) => episode,
                None => {
                    let episode = format!("{CONDITION}/{signature}/{}", current_index(&transaction).map_err(internal)?);
                    append_claim_tx(&transaction, &self.origin, &subject, "operational.failure",
                        Some("daemon/runtime"), &json!({"fields": {
                            "episode": episode, "condition": CONDITION, "reviewer": reviewer,
                            "title": format!("outbound replication to {peer} parked after {code}"),
                            "reason": "The authenticated destination rejected the sent envelopes. Fix the rejection cause, then explicitly approve operational repair to retry this exact failure episode. Restart and wake do not unpark it.",
                            "severity": "error", "targets": [subject],
                            "reproduction": {"signature": signature, "origin": origin,
                                "scope": scope, "peer": peer, "hashes": hashes, "code": code}
                        }}), &[], None).map_err(internal)?;
                    episode
                }
            };
            transaction.commit().map_err(internal)?;
            // Keep the graph writer loan until shard persistence finishes, so manual repair
            // cannot recover the episode between the graph alert and its shard park.
            target.park_outbound(peer, &hashes, code, Some(&episode)).map_err(internal)
        }
    }
}

