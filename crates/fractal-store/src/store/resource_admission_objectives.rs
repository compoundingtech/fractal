//! Admission evidence commits atomically with host custody, never from an SLO read.
use super::*;
use super::resource_membership::Membership;

pub const KIND: &str = "resource.admission.observed";
pub const COVERAGE_KIND: &str = "resource.admission.coverage";

const RECOVERY_FEATURE: super::declared_admission::FleetFeature = super::declared_admission::FleetFeature {
    keys: &["resource_recovery_observations"],
    code: "resource-recovery-feature-barrier",
    carries: "recovery-class admission observations",
};

pub(super) fn validate_publication(connection: &Connection, host: &str, class: &fractal_runtime::RequestClass) -> Result<(), FractalError> {
    if *class == fractal_runtime::RequestClass::Recovery {
        super::declared_admission::validate_fleet(connection, host, &RECOVERY_FEATURE)?;
    }
    Ok(())
}

/// Process-local authority acquisition proof; a persisted cut alone cannot prove a live producer.
#[derive(Default)]
pub struct Coverage(pub std::sync::atomic::AtomicBool);

/// Withheld live evidence revokes durable completeness in the same custody transaction.
/// Only a full replay may restore it; the process-local bit proves authority acquisition.
pub(super) fn observe(tx: &Transaction<'_>, member: &Membership) -> Result<bool> {
    let complete = publish(tx, member)?;
    let subject = format!("resource-admission/{}", member.request.host);
    if let Some(coverage) = latest_claim_of_kind_tx(tx, &subject, COVERAGE_KIND)? {
        let fields = &coverage.body["fields"];
        if !complete && fields["complete"] != false {
            let mut fields = fields.clone();
            fields["complete"] = json!(false);
            append_claim_tx(tx, &member.request.host, &subject, COVERAGE_KIND, None,
                &json!({"fields":fields,"evidence":[]}), &[], None)?;
        } else if complete && fields["complete"] == false {
            // Capability restoration alone is not evidence restoration. Replay even
            // retired rows before certifying the current authority's complete cut.
            return recover(tx, &member.request.host, fields["epoch"].as_str()
                .ok_or_else(|| anyhow::anyhow!("admission coverage epoch missing"))?);
        }
    }
    Ok(complete)
}

fn publish(tx: &Transaction<'_>, member: &Membership) -> Result<bool> {
    if let Err(error) = member.request.class.ensure_supported() {
        tracing::warn!(membership = %member.request.id, %error, "skipping unsupported-class admission observation; authoritative custody retained");
        return Ok(false);
    }
    if let Err(error) = validate_publication(tx, &member.request.host, &member.request.class) {
        tracing::warn!(membership = %member.request.id, %error, "deferring recovery observation until every active fleet reader supports it");
        return Ok(false);
    }
    // Child/inherited contexts never create memberships and therefore never add samples.
    // Preserve the accepted request epoch across authority restarts.
    let subject = format!("resource-admission/{}/{}", member.request.host, member.request.id);
    // Never replicate replayable grants, runtime roots, or child execution custody.
    let fields = json!({
        "id": member.request.id, "host": member.request.host,
        "class": member.request.class, "coverage": member.request.coverage,
        "epoch": member.clock_epoch, "owner": member.request.owner,
        "queued_at": member.queued_at, "admitted_at": member.admitted_at,
        "retired_at": member.retired_at, "state": member.state,
        "refusal": member.refusal_code, "finite": member.request.capacity.is_some(),
        "admission_timeout_ms": member.request.admission_timeout_ms,
        "clock_uncertain": member.clock_uncertain,
    });
    if let Some(previous) = latest_claim_of_kind_tx(tx, &subject, KIND)?
        && previous.body.get("fields") == Some(&fields) { return Ok(true); }
    append_claim_tx(tx, &member.request.host, &subject, KIND, None,
        &json!({"fields": fields, "evidence": []}), &[], None)?;
    Ok(true)
}

/// Replay understood membership observations at authority acquisition. Unsupported classes
/// retain authoritative custody and mark observation coverage incomplete instead of aborting.
pub(super) fn recover(tx: &Transaction<'_>, host: &str, epoch: &str) -> Result<bool> {
    let mut statement = tx.prepare("SELECT body FROM resource_memberships_v1 WHERE host=?1 ORDER BY id")?;
    let members = statement.query_map([host], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    let mut complete = true;
    for body in members { complete &= publish(tx, &serde_json::from_str(&body)?)?; }
    append_claim_tx(tx, host, &format!("resource-admission/{host}"), COVERAGE_KIND, None,
        &json!({"fields": {"host": host, "epoch": epoch, "complete": complete}, "evidence": []}), &[], None)?;
    Ok(complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_cut_and_membership_evidence_survive_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.db");
        {
            let store = Store::open(&path, "example-host").unwrap();
            store.start_resource_authority("example-host", "epoch-one").unwrap();
            assert_eq!(store.claims_by_kinds_after(&[COVERAGE_KIND], None, 0, 10).unwrap().claims.len(), 1);
        }
        let store = Store::open(&path, "example-host").unwrap();
        store.start_resource_authority("example-host", "epoch-two").unwrap();
        assert_eq!(store.claims_by_kinds_after(&[COVERAGE_KIND], None, 0, 10).unwrap().claims.len(), 2);
    }
}
