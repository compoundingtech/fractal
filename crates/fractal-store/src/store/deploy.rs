//! Offline deployment authorization against one pinned local claim/signature/receipt snapshot.
use super::*;
use fractal_schema::deploy_policy::Policy;
use smallclaims::principal::Verdict;

use fractal_schema::deploy::{VerifyGoRequest, VerifyGoReceipt};

type Checked<T> = std::result::Result<T, St3Error>;
const UNREPAIRED_REPLICA: &str = "NOT EXISTS (SELECT 1 FROM replica_records WHERE replica_records.claim_id=claims.id AND replica_records.state='repaired')";
fn refuse(code: &str, message: impl Into<String>) -> St3Error { St3Error::new(code, message) }
fn fields(claim: &ClaimRecord) -> Checked<BTreeMap<String, Value>> {
    let fields = serde_json::from_value(claim.body.get("fields").cloned().unwrap_or(Value::Null))
        .map_err(|e| refuse("deploy-invalid-record", format!("{}: {e}", claim.id)))?;
    fractal_schema::registry().validate_claim(&claim.subject, &claim.kind, &fields)
        .map_err(|e| refuse(e.code, e.message))?;
    Ok(fields)
}
fn text<'a>(fields: &'a BTreeMap<String, Value>, name: &str) -> Checked<&'a str> {
    fields.get(name).and_then(Value::as_str)
        .ok_or_else(|| refuse("deploy-invalid-record", format!("missing string {name}")))
}
fn timestamp(fields: &BTreeMap<String, Value>, name: &str) -> Checked<u128> {
    let time = chrono::DateTime::parse_from_rfc3339(text(fields, name)?)
        .map_err(|e| refuse("deploy-invalid-time", e.to_string()))?;
    u128::try_from(time.timestamp_millis()).map_err(|_| refuse("deploy-invalid-time", "negative timestamp"))
}
fn covers(stop: &BTreeMap<String, Value>, go: &BTreeMap<String, Value>) -> Checked<bool> {
    let host = text(go, "host")?;
    let scope = text(stop, "scope")?;
    // The v1 scope is either the installer entry or an exact train identifier.
    Ok(stop.get("hosts").and_then(Value::as_array).is_some_and(|hosts| hosts.iter().any(|h| h.as_str() == Some(host)))
        && (scope == "installer-entry" || scope == text(go, "trainId")?))
}

impl Store {
    /// Does not sync, judge, repair projections, consume a GO, or write any state.
    pub fn verify_deploy_go(&self, policy: &Policy, request: &VerifyGoRequest, now_unix_ms: u128) -> Checked<VerifyGoReceipt> {
        self.read_snapshot(|_| Ok(self.verify_deploy_go_snapshot(policy, request, now_unix_ms)))
            .map_err(internal)?
    }

    fn deploy_record(&self, id: &str, kind: &str, signers: &[String]) -> Checked<(ClaimRecord, BTreeMap<String, Value>)> {
        let claim = self.readers.get().query_row(&format!(
            "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims
             WHERE id=?1 AND {UNREPAIRED_CLAIM} AND {UNREPAIRED_REPLICA}"), [id], claim_from_row)
            .optional().map_err(internal)?
            .ok_or_else(|| refuse("deploy-dangling-reference", format!("missing active {kind} {id}")))?;
        if claim.kind != kind { return Err(refuse("deploy-reference-kind", format!("{id} is not {kind}"))); }
        let fields = fields(&claim)?;
        let auth = self.claim_authorization(id).map_err(internal)?;
        if auth.verdict != Verdict::Verified {
            return Err(refuse("deploy-unverified", format!("{id}: {}", auth.verdict.name())));
        }
        let signature = auth.signature.ok_or_else(|| refuse("deploy-unverified", format!("{id}: missing proof")))?;
        // A partial field signature or on-behalf attribution is not whole-record authority.
        if signature.on_behalf.is_some() || signature.format.is_some() || !signature.signed_fields.is_empty()
            || !signers.contains(&signature.signer) {
            return Err(refuse("deploy-unauthorized-signer", format!("{id}: unauthorized or ambiguous signer")));
        }
        Ok((claim, fields))
    }

    fn deploy_go_record(&self, policy: &Policy, id: &str, now: Option<u128>) -> Checked<(ClaimRecord, BTreeMap<String, Value>)> {
        let (claim, go) = self.deploy_record(id, "deploy.go", &policy.go_signers)?;
        if policy.refused_verifier_hosts.iter().any(|h| Some(h.as_str()) == go.get("host").and_then(Value::as_str))
            || !policy.consumption_signers.iter().any(|a| Some(a.as_str()) == go.get("actor").and_then(Value::as_str)) {
            return Err(refuse("deploy-policy-refused", "GO host or executor is refused by packaged policy"));
        }
        let issued = timestamp(&go, "issuedAt")?;
        let expires = timestamp(&go, "expiresAt")?;
        if now.is_some_and(|now| now < issued || now >= expires) || expires <= issued || expires - issued > 4 * 60 * 60 * 1000 {
            return Err(refuse("deploy-invalid-time", "GO is not within its bounded issue/expiry interval"));
        }
        let (_, compat) = self.deploy_record(text(&go, "compatSignoffId")?, "deploy.compat-signoff", &policy.compat_signoff_signers)?;
        if compat.get("trainId") != go.get("trainId") || compat.get("oldCommit") != go.get("old").and_then(|v| v.get("commit"))
            || compat.get("newCommit") != go.get("new").and_then(|v| v.get("commit")) {
            return Err(refuse("deploy-compat-mismatch", "compatibility signoff does not bind GO train and commits"));
        }
        Ok((claim, go))
    }

    fn verify_deploy_go_snapshot(&self, policy: &Policy, request: &VerifyGoRequest, now: u128) -> Checked<VerifyGoReceipt> {
        if policy.version != 1 { return Err(refuse("deploy-policy-refused", "unsupported policy version")); }
        if request.host != self.origin || policy.refused_verifier_hosts.contains(&self.origin) {
            return Err(refuse("deploy-policy-refused", "verification is authorized only on the allowed executing host"));
        }
        let (claim, go) = self.deploy_go_record(policy, &request.go_id, Some(now))?;
        for (name, expected) in [
            ("host", json!(request.host)), ("actor", json!(request.actor)),
            ("operation", json!(request.operation)), ("trainId", json!(request.train_id)),
            ("old", json!(request.old)), ("new", json!(request.new)),
            ("previewDigest", json!(request.preview_digest)), ("failureRestore", json!(request.failure_restore)),
        ] {
            if go.get(name) != Some(&expected) { return Err(refuse("deploy-request-mismatch", format!("GO does not match {name}"))); }
        }
        let connection = self.readers.get();
        let unreadable: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records WHERE state IN ('pending','unknown','invalid') AND kind_hint LIKE 'deploy.%')
             OR EXISTS(SELECT 1 FROM claims JOIN projection_health ON projection_health.aggregate LIKE '%:' || claims.id
                       WHERE claims.kind LIKE 'deploy.%' AND projection_health.status!='healthy')",
            [], |row| row.get(0)).map_err(internal)?;
        if unreadable {
            return Err(refuse("deploy-incomplete-history", "deployment history contains unreadable or quarantined records"));
        }
        let last: Option<String> = connection.query_row(
            "SELECT last_success_at_unix_ms FROM replication_peers WHERE peer=?1", [&claim.origin], |r| r.get(0))
            .optional().map_err(internal)?.flatten();
        let last = last.and_then(|s| s.parse::<u128>().ok())
            .ok_or_else(|| refuse("deploy-stale-replica", "no successful sync receipt for GO origin"))?;
        if last > now || now - last > 10 * 60 * 1000 {
            return Err(refuse("deploy-stale-replica", "GO origin sync receipt is stale or in the future"));
        }
        let mut query = connection.prepare_cached(&format!(
            "SELECT id,kind FROM claims WHERE kind IN ('deploy.go-revoked','deploy.go-consumed','deploy.stop','deploy.stop-lifted')
             AND {UNREPAIRED_CLAIM} AND {UNREPAIRED_REPLICA} ORDER BY store_index")).map_err(internal)?;
        let records = query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
        let mut stops = BTreeMap::new();
        let mut lifts = Vec::new();
        for (id, kind) in records {
            let signers = match kind.as_str() {
                "deploy.go-consumed" => &policy.consumption_signers,
                "deploy.go-revoked" => &policy.go_signers,
                _ => &policy.stop_signers,
            };
            let (record, data) = self.deploy_record(&id, &kind, signers)?;
            match kind.as_str() {
                "deploy.go-revoked" | "deploy.go-consumed" => {
                    let target_id = text(&data, "goId")?;
                    let (_, target) = self.deploy_record(target_id, "deploy.go", &policy.go_signers)?;
                    if kind == "deploy.go-consumed" {
                        let actor = text(&target, "actor")?;
                        self.deploy_record(&id, &kind, &[actor.to_owned()])?;
                        if record.origin != text(&target, "host")? {
                            return Err(refuse("deploy-consumption-origin", "consumption must originate on GO host"));
                        }
                        if data.get("operation") != target.get("operation") {
                            return Err(refuse("deploy-consumption-operation", "consumption operation must match its GO"));
                        }
                    }
                    if target_id == request.go_id {
                        return Err(refuse(if kind == "deploy.go-revoked" { "deploy-revoked" } else { "deploy-consumed" }, "GO is no longer unused authority"));
                    }
                }
                "deploy.stop" => {
                    let stop_id = text(&data, "stopId")?.to_owned();
                    if stops.insert(stop_id, data).is_some() { return Err(refuse("deploy-ambiguous-stop", "duplicate STOP identifier")); }
                }
                "deploy.stop-lifted" => lifts.push(data),
                _ => unreachable!(),
            }
        }
        let mut lifted = BTreeSet::new();
        for lift in lifts {
            let stop_id = text(&lift, "stopId")?;
            let stop = stops.get(stop_id).ok_or_else(|| refuse("deploy-dangling-reference", "lift references missing STOP"))?;
            // Explicit referential validation, not recursive verification of release GO state.
            let (_, release) = self.deploy_go_record(policy, text(&lift, "releaseGoId")?, None)?;
            if !covers(stop, &release)? {
                return Err(refuse("deploy-lift-mismatch", "release GO does not bind the referenced STOP scope and host"));
            }
            if release.get("host") == go.get("host") && release.get("trainId") == go.get("trainId") {
                lifted.insert(stop_id.to_owned());
            }
        }
        let combined = go.get("combined");
        let bound_stop = combined.and_then(|v| v.get("stopId")).and_then(Value::as_str);
        if let Some(id) = bound_stop {
            let stop = stops.get(id).ok_or_else(|| refuse("deploy-dangling-reference", "combined GO references missing STOP"))?;
            if lifted.contains(id) || !covers(stop, &go)? {
                return Err(refuse("deploy-combined-mismatch", "combined GO requires an active covering STOP"));
            }
        }
        for (id, stop) in &stops {
            if covers(stop, &go)? && !lifted.contains(id) && bound_stop != Some(id.as_str()) {
                return Err(refuse("deploy-stopped", format!("active STOP {id}")));
            }
        }
        Ok(VerifyGoReceipt {
            go_id: claim.id, origin: claim.origin, actor: request.actor.clone(), host: request.host.clone(),
            operation: request.operation.clone(), train_id: request.train_id.clone(), expires_at: text(&go, "expiresAt")?.to_owned(),
            policy_path: Policy::packaged_path().map(str::to_owned), policy_hash: policy.content_hash().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests;
