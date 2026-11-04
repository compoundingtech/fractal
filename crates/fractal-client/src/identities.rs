//! Native identity naming uses stable storage identities, not replacement agents or runs.
use crate::{Client, ClientError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IdentityKind {
    Agent,
    Mission,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RenameRequest {
    pub from: String,
    pub to: String,
    pub actor: String,
    pub reason: String,
    #[serde(default)]
    pub expected_naming_revision: Option<String>,
    #[serde(default)]
    pub expected_declaration: Option<String>,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RenameReceipt {
    pub operation_id: String,
    pub idempotency_key: String,
    pub applied: String,
    pub kind: IdentityKind,
    pub identity: String,
    pub from: String,
    pub to: String,
    pub canonical_id: String,
    pub naming_revision: String,
    pub claim_id: String,
    pub preserved_incarnation: Option<String>,
    pub replication: RenameReplication,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RenameReplication {
    /// `local-durable` does not assert fleet convergence.
    pub state: String,
    pub frontier: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RenameProofExportRequest {
    pub name: String,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub include_co_batched_claims: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FleetAnchor {
    pub fleet_anchor: String,
}

#[derive(Deserialize)]
struct NativeValue<T> {
    value: T,
}

// Native routes use the st3.v1 value envelope, not the client-v0 snapshot envelope.
#[derive(Deserialize)]
struct RenameResponse {
    value: RenameReceipt,
}

impl Client {
    /// Export the exact signed evidence, preserving the versioned bundle's wire shape.
    pub async fn agent_rename_proof_export(
        &self,
        request: &RenameProofExportRequest,
    ) -> Result<serde_json::Value, ClientError> {
        Ok(self.post::<NativeValue<serde_json::Value>>(
            "/v1/agents/rename-id/proof/export", request,
        ).await?.value)
    }

    pub async fn mission_rename_proof_export(
        &self,
        request: &RenameProofExportRequest,
    ) -> Result<serde_json::Value, ClientError> {
        Ok(self.post::<NativeValue<serde_json::Value>>(
            "/v1/missions/rename-id/proof/export", request,
        ).await?.value)
    }

    pub async fn fleet_anchor(&self) -> Result<FleetAnchor, ClientError> {
        Ok(self.get::<NativeValue<FleetAnchor>>("/v1/fleet/anchor").await?.value)
    }

    pub async fn agent_rename_id(&self, request: &RenameRequest) -> Result<RenameReceipt, ClientError> {
        Ok(self.post::<RenameResponse>("/v1/agents/rename-id", request).await?.value)
    }

    pub async fn mission_rename_id(&self, request: &RenameRequest) -> Result<RenameReceipt, ClientError> {
        Ok(self.post::<RenameResponse>("/v1/missions/rename-id", request).await?.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_export_preserves_revision_tokens_and_native_bundle_envelope() {
        let request: RenameProofExportRequest = serde_json::from_value(serde_json::json!({
            "name": "agent/example-storage",
            "revision": "claim/exact-revision",
        })).unwrap();
        assert_eq!(request.revision.as_deref(), Some("claim/exact-revision"));
        assert!(!request.include_co_batched_claims);
        assert_eq!(serde_json::to_value(&request).unwrap()["name"], "agent/example-storage");
        let response: NativeValue<serde_json::Value> = serde_json::from_value(serde_json::json!({
            "api_version": "st3.v1",
            "value": { "schema": "fractal.rename-proof.v1", "envelopes": [] },
        })).unwrap();
        assert_eq!(response.value["schema"], "fractal.rename-proof.v1");
        let anchor: NativeValue<FleetAnchor> = serde_json::from_value(serde_json::json!({
            "api_version": "st3.v1",
            "value": { "fleet_anchor": "caller-distributed-anchor" },
        })).unwrap();
        assert_eq!(anchor.value.fleet_anchor, "caller-distributed-anchor");
    }

    #[test]
    fn request_optional_fences_and_full_ids_round_trip() {
        let value = serde_json::json!({
            "from": "agent/example-old", "to": "agent/example-new",
            "actor": "person/robin", "reason": "Clarify responsibility",
            "idempotency_key": "example-rename"
        });
        let request: RenameRequest = serde_json::from_value(value).unwrap();
        assert_eq!(request.expected_naming_revision, None);
        assert_eq!(request.expected_declaration, None);
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["from"], "agent/example-old");
        assert_eq!(wire["to"], "agent/example-new");
        assert!(wire.get("manifest").is_none());
    }

    #[test]
    fn receipt_preserves_identity_and_local_frontier() {
        let value = serde_json::json!({
            "operation_id": "claim/example", "idempotency_key": "example-rename",
            "applied": "already-applied",
            "kind": "agent", "identity": "agent/example-old",
            "from": "agent/example-old", "to": "agent/example-new",
            "canonical_id": "agent/example-new", "naming_revision": "claim/example",
            "claim_id": "claim/example", "preserved_incarnation": "incarnation/example",
            "replication": {"state": "local-durable", "frontier": 7}
        });
        let receipt: RenameReceipt = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(receipt.identity, "agent/example-old");
        let response: RenameResponse = serde_json::from_value(serde_json::json!({
            "api_version": "st3.v1",
            "request_id": "request/example-rename",
            "snapshot_host": "example",
            "store_index": 7,
            "value": value,
        })).unwrap();
        assert_eq!(response.value, receipt);
        assert_eq!(serde_json::to_value(receipt).unwrap(), value);
    }
}
