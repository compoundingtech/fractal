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

pub const REPLAY_PAGE_SIZE: usize = 128;

#[derive(Default)]
struct Maintenance(std::sync::atomic::AtomicBool);

pub(super) fn activate(store: &Store) {
    store.fractal.extensions.get::<Maintenance>().0.store(true, std::sync::atomic::Ordering::Release);
}

impl Store {
    /// One bounded maintenance page, including retired rows. Returns whether another
    /// tick is required. No membership traffic or request-time SLO read drives repair.
    pub fn replay_resource_admission_observations(&self) -> Result<bool> {
        if !self.fractal.extensions.get::<Maintenance>().0.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(false);
        }
        let mut connection = self.connection.write();
        let tx = connection.transaction()?;
        let subject = format!("resource-admission/{}", self.origin);
        let Some(coverage) = latest_claim_of_kind_tx(&tx, &subject, COVERAGE_KIND)? else {
            return Ok(false);
        };
        if coverage.body["fields"]["complete"] == true { return Ok(false); }
        let mut statement = tx.prepare(
            "SELECT pending.membership_id,membership.body FROM resource_admission_pending_v1 AS pending
             JOIN resource_memberships_v1 AS membership ON membership.id=pending.membership_id
             WHERE pending.host=?1 ORDER BY pending.rowid LIMIT ?2")?;
        let members = statement.query_map(rusqlite::params![self.origin, REPLAY_PAGE_SIZE],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for (id, body) in &members {
            let member: Membership = serde_json::from_str(body)?;
            let published = publish(&tx, &member)?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-hbe53b1627d37 owner=fractal_store::store::resource_admission_objectives TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM resource_admission_pending_v1 WHERE membership_id=?1", [id])?;
            if !published {
                // Rotate blocked IDs behind already-pending work. New traffic cannot
                // reset a traversal or starve existing IDs, and each tick is bounded.
                enqueue(&tx, &member)?;
            }
        }
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM resource_admission_pending_v1 WHERE host=?1)",
            [&self.origin], |row| row.get(0))?;
        if !pending {
            let mut fields = coverage.body["fields"].clone();
            fields["complete"] = json!(true);
            append_claim_tx(&tx, &self.origin, &subject, COVERAGE_KIND, None,
                &json!({"fields":fields,"evidence":[]}), &[], None)?;
        }
        tx.commit()?;
        if !pending {
            self.fractal.extensions.get::<Coverage>().0.store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(pending)
    }
}

fn enqueue(tx: &Transaction<'_>, member: &Membership) -> Result<()> {
    tx.execute(/* sqlite-large-value: allow-for-now lv-h7ae18f03ddc9 owner=fractal_store::store::resource_admission_objectives TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO resource_admission_pending_v1(host,membership_id) VALUES(?1,?2)",
        rusqlite::params![member.request.host,member.request.id])?;
    Ok(())
}

/// Withheld live evidence revokes durable completeness in the same custody transaction.
/// Only draining the pending queue may restore it; the local bit proves authority acquisition.
pub(super) fn observe(tx: &Transaction<'_>, member: &Membership) -> Result<bool> {
    let complete = publish(tx, member)?;
    if complete {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h35fccbf73a79 owner=fractal_store::store::resource_admission_objectives TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM resource_admission_pending_v1 WHERE membership_id=?1", [&member.request.id])?;
    } else {
        enqueue(tx, member)?;
    }
    let subject = format!("resource-admission/{}", member.request.host);
    if let Some(coverage) = latest_claim_of_kind_tx(tx, &subject, COVERAGE_KIND)? {
        let fields = &coverage.body["fields"];
        if !complete && fields["complete"] != false {
            let mut fields = fields.clone();
            fields["complete"] = json!(false);
            append_claim_tx(tx, &member.request.host, &subject, COVERAGE_KIND, None,
                &json!({"fields":fields,"evidence":[]}), &[], None)?;
        }
    }
    Ok(complete)
}

fn publish(tx: &Transaction<'_>, member: &Membership) -> Result<bool> {
    if let Err(error) = member.request.class.ensure_supported() {
        tracing::warn!(membership = %member.request.id, %error,
            "withholding unsupported-class admission observation; authoritative custody retained");
        return Ok(false);
    }
    if (member.request.class == fractal_runtime::RequestClass::Recovery
        || member.refusal_code == Some(super::resource_membership::RefusalCode::RecoveryNotAllowed))
        && let Err(error) = super::declared_admission::validate_fleet(tx, &member.request.host, &RECOVERY_FEATURE) {
        tracing::warn!(membership = %member.request.id, %error,
            "deferring recovery observation until every active fleet reader supports it");
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
