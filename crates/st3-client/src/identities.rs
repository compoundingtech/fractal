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

impl Client {
    pub async fn agent_rename_id(&self, request: &RenameRequest) -> Result<RenameReceipt, ClientError> {
        self.post("/v1/agents/rename-id", request).await
    }

    pub async fn mission_rename_id(&self, request: &RenameRequest) -> Result<RenameReceipt, ClientError> {
        self.post("/v1/missions/rename-id", request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(serde_json::to_value(receipt).unwrap(), value);
    }
}
