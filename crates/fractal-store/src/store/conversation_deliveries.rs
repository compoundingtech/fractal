//! Bounded lookups of newly authored, fenced native receipt identities. No historical
//! fold, read-path initialization or transcript content. Native identities are stored
//! only beside a newly appended, authenticated owner-local consumption receipt.
use super::*;
use crate::conversation_delivery::NativeReceipt;

const IDS: usize = 200;
const BODY_BYTES: usize = 4096;

pub const QUERY: &str = "WITH candidates AS (
    SELECT requested.value AS native_id, proof.claim_id, proof.claim_hash
    FROM json_each(?1) AS requested
    CROSS JOIN main.local_native_delivery_receipts_v2 AS proof
        INDEXED BY sqlite_autoindex_local_native_delivery_receipts_v2_1
    WHERE proof.claim_id IN (
        SELECT claim_id FROM main.local_native_delivery_receipts_v2
            INDEXED BY local_native_delivery_identity_v2
        WHERE session_id=?2 AND native_id=requested.value AND actor=?3 AND origin=?4
        ORDER BY store_index DESC LIMIT 3
    )
)
SELECT candidates.native_id, substr(candidates.claim_id,1,513), substr(claims.subject,1,513),
       substr(CAST(claims.body AS BLOB),1,4097),
       EXISTS(SELECT 1 FROM main.replica_records INDEXED BY replica_records_repaired_claim
              WHERE claim_id=claims.id AND state='repaired'),
       substr(operations.request_digest,1,65), substr(operations.canonical_claim_id,1,513),
       substr(operations.state,1,33), substr(candidates.claim_hash,1,65),
       COALESCE(claims.kind='message.read' AND claims.actor=?3 AND claims.origin=?4 AND claims.store_index<=?5,0)
FROM candidates LEFT JOIN main.claims AS claims ON claims.id=candidates.claim_id
LEFT JOIN main.operations AS operations
ON operations.id=CASE WHEN json_valid(substr(CAST(claims.body AS BLOB),1,4097))
                     THEN json_extract(substr(CAST(claims.body AS BLOB),1,4097),'$._operation.id') END";

impl Store {
    /// At most three physical receipts per selected native id, including missing
    /// claim rows and above-cut claims. Hitting that limit
    /// cannot prove uniqueness, so the optional link stays absent. Two qualified
    /// receipts are ambiguous even if their graph subjects happen to be identical.
    pub fn conversation_delivery_receipts(
        &self,
        session: &str,
        owner: &str,
        ids: &BTreeSet<String>,
        through: u64,
    ) -> Result<Vec<(String, NativeReceipt)>> {
        if ids.is_empty() || ids.len() > IDS || ids.iter().any(|id| id.len() > 256) {
            return Ok(Vec::new());
        }
        let connection = self.readers.get();
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h09cab06daf63 owner=fractal_store::store::conversation_deliveries TODO=trace-statement-and-value-representation */ QUERY)?;
        let requested = serde_json::to_string(ids)?;
        let rows = statement.query_map(
            params![requested, session, owner, self.origin(), through],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, bool>(9)?,
                ))
            },
        )?;
        let mut counts = BTreeMap::<String, usize>::new();
        let mut qualified = Vec::new();
        for row in rows {
            let (
                native,
                id,
                subject,
                bytes,
                repaired,
                digest,
                canonical,
                state,
                fingerprint,
                eligible,
            ) = row?;
            *counts.entry(native.clone()).or_default() += 1;
            let (Some(subject), Some(bytes)) = (subject, bytes) else {
                continue;
            };
            if !eligible || repaired || bytes.len() > BODY_BYTES {
                continue;
            }
            let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if body.get("_operation").is_some() {
                let Some((_, request_digest)) = operation_parts(&body) else {
                    continue;
                };
                if digest.as_deref() != Some(request_digest)
                    || canonical.as_deref() != Some(id.as_str())
                    || state.as_deref() != Some("active")
                {
                    continue;
                }
            }
            if body.pointer("/fields/status").and_then(Value::as_str) != Some("read") {
                continue;
            }
            if crate::conversation_delivery::receipt_fingerprint(
                &subject,
                owner,
                self.origin(),
                &body,
            )? != fingerprint
            {
                continue;
            }
            let receipt = NativeReceipt {
                session_id: session.into(),
                message_id: native,
            };
            if receipt.valid() && subject.len() <= 512 && id.len() <= 512 {
                qualified.push((subject, receipt));
            }
        }
        qualified.retain(|(_, receipt)| {
            counts
                .get(&receipt.message_id)
                .is_some_and(|count| *count < 3)
        });
        Ok(qualified)
    }

    /// A new receipt for this owner can alter a previously delivered native turn.
    /// Conservatively resync its native conversation, with one owner/cut index seek.
    pub fn conversation_delivery_changed(
        &self,
        owner: &str,
        after: u64,
        through: u64,
    ) -> Result<bool> {
        Ok(self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM main.local_native_delivery_receipts_v2
             INDEXED BY local_native_delivery_owner_cut_v2
             WHERE actor=?1 AND store_index>?2 AND store_index<=?3 LIMIT 1)",
            params![owner, after, through],
            |row| row.get(0),
        )?)
    }
}
