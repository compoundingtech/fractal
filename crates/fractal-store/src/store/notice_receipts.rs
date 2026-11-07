//! Durable delivery evidence, independent of disposable notice-runtime materialization.
use super::*;
use fractal_schema::{notice, notice_receipt::{self, Outcome, Receipt, Surface}};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ReceiptUnknownReason {
    MissingEvidence,
    AuthorityUnknown(String),
    RegistrationUnavailable,
    SourceUnavailable,
    MalformedEvidence,
    LineageMismatch,
    ConflictingEvidence,
    QuarantinedEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DeliveryEvidence {
    Unknown { reason: ReceiptUnknownReason },
    Delivered { receipt_claim: String, completed_at: String, evidence: String },
    Failed { receipt_claim: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SurfaceDeliveryEvidence {
    pub surface: Surface,
    pub evidence: DeliveryEvidence,
}

/// Approval assesses authenticated receipt position against the historical owner chain,
/// never an actor assertion, payload timestamp, or current runtime designation.
pub(super) trait ReceiptAuthorizer {
    fn assess(&self, claim: &ClaimRecord, receipt: &Receipt) -> ReceiptAuthorization;
}

pub(super) enum ReceiptAuthorization {
    Authorized,
    Unknown { reason: String },
}

struct ChainAuthorizer<'a> {
    connection: &'a Connection,
    chain: Result<super::notice_acknowledgements::Chain, FractalError>,
}

impl ReceiptAuthorizer for ChainAuthorizer<'_> {
    fn assess(&self, claim: &ClaimRecord, receipt: &Receipt) -> ReceiptAuthorization {
        let authorization = match &self.chain {
            Ok(chain) => chain.authorize(self.connection, claim, receipt, None),
            Err(error) => return ReceiptAuthorization::Unknown {
                reason: format!("{}: {}", error.code, error.message),
            },
        };
        match authorization {
            Ok(()) => ReceiptAuthorization::Authorized,
            Err(error) => ReceiptAuthorization::Unknown {
                reason: format!("{}: {}", error.code, error.message),
            },
        }
    }
}

impl Store {
    pub fn notice_receipts(&self, source_id: &str) -> Result<Vec<SurfaceDeliveryEvidence>, FractalError> {
        let connection = self.readers.get();
        let tx = connection.unchecked_transaction().map_err(internal)?;
        let authorizer = ChainAuthorizer {
            connection: &tx,
            chain: super::notice_acknowledgements::Chain::load(&tx, None),
        };
        project(&tx, source_id, &authorizer).map_err(internal)
    }
}

fn unknown(reason: ReceiptUnknownReason) -> DeliveryEvidence {
    DeliveryEvidence::Unknown { reason }
}

fn all_unknown(reason: ReceiptUnknownReason) -> Vec<SurfaceDeliveryEvidence> {
    [Surface::Tray, Surface::Speech, Surface::Push, Surface::Log].into_iter()
        .map(|surface| SurfaceDeliveryEvidence { surface, evidence: unknown(reason.clone()) }).collect()
}

fn registration_ready(connection: &Connection, index: usize) -> Result<bool> {
    let expected = &notice::manifests()[index];
    Ok(custom::registration(connection, &expected.subject_prefix)?.is_some_and(|(hash, manifest, state)| {
        state == "ready" && hash == notice::HASHES[index]
            && manifest.subject_prefix == expected.subject_prefix
            && notice::validate_registration(&manifest, &hash).is_ok()
    }))
}

fn project(connection: &Connection, source_id: &str, authorizer: &dyn ReceiptAuthorizer) -> Result<Vec<SurfaceDeliveryEvidence>> {
    use ReceiptUnknownReason as Why;
    if notice::id_for_source(source_id).is_err() {
        return Ok(all_unknown(Why::SourceUnavailable));
    }
    if !registration_ready(connection, 0)? || !registration_ready(connection, 3)? {
        return Ok(all_unknown(Why::RegistrationUnavailable));
    }
    // Opaque quarantine headers fence evidence; never decode unadmitted replica bodies.
    let quarantined: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM replica_records WHERE state IN ('unknown','invalid') AND (
         subject_hint=?1 OR subject_hint GLOB 'custom/attention/receipt/*'
         OR subject_hint=?2 OR subject_hint=?3 OR kind_hint=?4))",
        params![source_id, notice::manifests()[0].subject(), notice::manifests()[3].subject(), notice_receipt::KIND],
        |row| row.get(0),
    )?;
    if quarantined { return Ok(all_unknown(Why::QuarantinedEvidence)); }
    let history = custom::claims(connection, source_id)?;
    let [source] = history.as_slice() else {
        return Ok(all_unknown(if history.is_empty() { Why::SourceUnavailable } else { Why::ConflictingEvidence }));
    };
    let fields = custom::fields(source);
    let manifest = &notice::manifests()[0];
    let recipient = fields.get("recipient").and_then(Value::as_str);
    let producer = source.actor.as_deref();
    if source.kind != manifest.creation_kind
        || fields.get("_registration").and_then(Value::as_str) != Some(notice::HASHES[0])
        || fractal_schema::custom::validate_fields(&manifest.claims[&manifest.creation_kind], &fields).is_err()
        || !recipient.is_some_and(|s| s.starts_with("person/") && fractal_schema::registry().validate_subject(s).is_ok())
        || !producer.is_some_and(|s| (s.starts_with("agent/") || s.starts_with("person/")) && fractal_schema::registry().validate_subject(s).is_ok())
        || fields.get("producer").is_some_and(|value| value.as_str() != producer)
    { return Ok(all_unknown(Why::SourceUnavailable)); }
    // Discover linked identities, then read each complete canonical history: never select a
    // receive-order winner or hide an update/competing creation behind a valid creation.
    let subjects = connection.prepare(
        "SELECT DISTINCT subject FROM claims WHERE subject>=?1 AND subject<?2
         AND (json_extract(body,'$.fields.source_id')=?3 OR json_extract(body,'$.fields.creation_claim')=?4)
         AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired') ORDER BY subject",
    )?.query_map(params![notice::RECEIPT_PREFIX, "custom/attention/receipt0", source_id, source.id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut receipts = BTreeMap::new();
    for subject in subjects {
        let mut history = custom::claims(connection, &subject)?.into_iter();
        let Some(claim) = history.next() else { return Ok(all_unknown(Why::MalformedEvidence)); };
        if history.next().is_some() { return Ok(all_unknown(Why::ConflictingEvidence)); }
        if claim.kind != notice_receipt::KIND { return Ok(all_unknown(Why::MalformedEvidence)); }
        let mut fields = custom::fields(&claim);
        if fields.remove("_registration").as_ref().and_then(Value::as_str) != Some(notice::HASHES[3]) {
            return Ok(all_unknown(Why::RegistrationUnavailable));
        }
        fields.remove("_basis");
        let Ok(receipt) = serde_json::from_value::<Receipt>(serde_json::to_value(fields)?) else {
            return Ok(all_unknown(Why::MalformedEvidence));
        };
        if receipt.validate().is_err() { return Ok(all_unknown(Why::MalformedEvidence)); }
        if receipt.source_id != source_id || receipt.creation_claim != source.id
            || Some(receipt.producer.as_str()) != producer || Some(receipt.recipient.as_str()) != recipient
        { return Ok(all_unknown(Why::LineageMismatch)); }
        if receipts.insert(receipt.surface, (claim, receipt)).is_some() {
            return Ok(all_unknown(Why::ConflictingEvidence));
        }
    }
    Ok([Surface::Tray, Surface::Speech, Surface::Push, Surface::Log].into_iter().map(|surface| {
        let evidence = match receipts.remove(&surface) {
            None => unknown(Why::MissingEvidence),
            Some((claim, receipt)) => match authorizer.assess(&claim, &receipt) {
                ReceiptAuthorization::Unknown { reason } => unknown(Why::AuthorityUnknown(reason)),
                ReceiptAuthorization::Authorized => match receipt.outcome {
                    Outcome::Delivered { completed_at, evidence } => DeliveryEvidence::Delivered { receipt_claim: claim.id, completed_at, evidence },
                    Outcome::Failed { reason } => DeliveryEvidence::Failed { receipt_claim: claim.id, reason },
                },
            },
        };
        SurfaceDeliveryEvidence { surface, evidence }
    }).collect())
}

#[cfg(test)]
#[path = "notice_receipts_tests.rs"]
mod tests;
