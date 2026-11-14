//! External assistants have their own versioned claims, never a delegated person chain.
use super::*;
pub use super::external_grants::{
    BEGUN, GRANTED, MCP_PRINCIPAL, POLICY, PRINCIPAL, REVOKED, ROUTES, SCOPES, TRANSITIONS,
    Profile, append, audit, authenticate, authorize_route, effective, fields, latest,
    owning_person, principal, projection, session, subject, trusted_issuer,
};
pub mod authorization;
pub mod reads;
pub mod receipts;
pub mod native;

pub use authorization::{ReadReceipt, finish as finish_read, pin as pin_read_trust, read as read_projection};

pub fn device_resources(state: &AppState, snapshot: &ClientSnapshot) -> Result<Vec<Value>, ApiError> {
    let mut claims = Vec::new();
    for kind in [GRANTED, POLICY, REVOKED] {
        let mut before = snapshot.store_index.checked_add(1);
        loop {
            let page = state.store.claims_for_kind_at(kind, before, true, 256).map_err(ApiError::internal)?;
            claims.extend(page.claims);
            let Some(cursor) = page.next_cursor else { break };
            before = Some(cursor);
        }
    }
    claims.sort_by_key(|claim| claim.store_index);
    let mut grants = BTreeMap::<&str, (&ClaimRecord, bool)>::new();
    for claim in &claims {
        if claim.kind == REVOKED {
            if let Some((_, revoked)) = grants.get_mut(claim.subject.as_str()) { *revoked = true; }
        } else {
            let revoked = grants.get(claim.subject.as_str()).is_some_and(|(_, revoked)| *revoked);
            grants.insert(&claim.subject, (claim, revoked));
        }
    }
    grants.into_values().map(|(claim, revoked)| {
        let source = fields(claim)?;
        let expires = source["expires_at_unix_ms"].as_u64().map(u128::from).unwrap_or(0);
        let state = if revoked {"revoked"} else if expires <= client_snapshot_time(snapshot) {"expired"} else {"active"};
        Ok(json!({"id":source["device_id"],"kind":"device","revision":claim.id,
            "updated_at":client_timestamp(claim.accepted_at_unix_ms),"name":source["device_name"],
            "principal":principal(source)?,"sender_class":"external-assistant","grant_kind":"assistant",
            "grant_id":source["grant_id"],"session_actor":source["session_actor"],"state":state,
            "scopes":source["scopes"],"expires_at":client_timestamp(expires),
            "policy_revision":source["policy_revision"],"minimal_read_mode":source["minimal_read_mode"],
            "operational":{"layer":if state=="active" {"current"} else {"history"},
                "actionable":state=="active","reasons":if state=="active" {Vec::<&str>::new()} else {vec![state]}}}))
    }).collect()
}

