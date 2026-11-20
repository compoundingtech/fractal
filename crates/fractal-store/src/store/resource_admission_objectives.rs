//! Admission evidence commits atomically with host custody, never from an SLO read.
use super::*;
use super::resource_membership::Membership;

pub const KIND: &str = "resource.admission.observed";
pub const COVERAGE_KIND: &str = "resource.admission.coverage";

/// Process-local authority acquisition proof; a persisted cut alone cannot prove a live producer.
#[derive(Default)]
pub struct Coverage(pub std::sync::atomic::AtomicBool);

pub(super) fn observe(tx: &Transaction<'_>, member: &Membership) -> Result<bool> {
    if let Err(error) = member.request.class.ensure_supported() {
        tracing::warn!(membership = %member.request.id, %error,
            "withholding unsupported-class admission observation; authoritative custody retained");
        return Ok(false);
    }
    if member.refusal_code == Some(super::resource_membership::RefusalCode::RecoveryNotAllowed) {
        let feature = super::declared_admission::FleetFeature {
            keys: &["resource_recovery_observations"],
            code: "resource-recovery-feature-barrier",
            carries: "recovery admission observations",
        };
        if let Err(error) = super::declared_admission::validate_fleet(tx, &member.request.host, &feature) {
            tracing::warn!(membership = %member.request.id, %error,
                "withholding recovery refusal observation until every active reader supports it");
            return Ok(false);
        }
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

/// Replay only understood, fleet-compatible evidence without granting newer classes authority.
/// Withheld observations retain custody and leave the local source cut uncertified.
pub(super) fn recover(tx: &Transaction<'_>, host: &str, epoch: &str) -> Result<bool> {
    let mut statement = tx.prepare("SELECT body FROM resource_memberships_v1 WHERE host=?1 ORDER BY id")?;
    let members = statement.query_map([host], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    let mut complete = true;
    for body in members {
        let member: Membership = serde_json::from_str(&body)?;
        complete &= observe(tx, &member)?;
    }
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
