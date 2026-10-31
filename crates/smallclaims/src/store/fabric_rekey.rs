//! Signed, non-routing rekey metadata shares the member endpoint announcement.
//! The publication mutex prevents ordinary worker refreshes from erasing a transition.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FabricRekeyRequest {
    pub action: FabricRekeyAction,
    pub old_node: String,
    pub next_node: String,
    pub deadline: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<FabricRekeyState>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FabricRekeyAction { Prepare, Commit, Abort, Receipt }

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FabricRekeyState { Prepared, Committed }

fn metadata(endpoint: &Value) -> bool {
    matches!(endpoint["transport"].as_str(), Some("fabric-rekey" | "fabric-rekey-receipt"))
}

fn selectors(endpoint: &Value, request: &FabricRekeyRequest) -> bool {
    endpoint["old_node"] == request.old_node
        && endpoint["next_node"] == request.next_node
        && endpoint["deadline"].as_u64() == Some(request.deadline)
}

fn node_id(node: &str) -> bool {
    node.len() == 64 && node.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

impl Store {
    fn own_endpoint_fields(&self) -> Result<Value> {
        let key = self.member_public_key().context("this node has no fleet member key")?;
        Ok(self.latest_claim(&format!("host/{}", self.origin), Some("fleet.member-endpoints"))?
            .map(|claim| claim.body["fields"].clone())
            .filter(|fields| fields["member_key"] == key)
            .unwrap_or_else(|| json!({"endpoints":[]})))
    }

    pub(super) fn preserve_fabric_rekey(&self, mode: &str, endpoints: &[Value], clear: bool) -> Result<Vec<Value>> {
        // Metadata is writable only through the validated transition operation, not the
        // worker's generic endpoint publication endpoint.
        let mut result: Vec<_> = endpoints.iter()
            .filter(|entry| !metadata(entry) && (mode == "listening" || entry["transport"] != "fabric"))
            .cloned().collect();
        if self.member_public_key().is_none() { return Ok(result); }
        let fields = self.own_endpoint_fields()?;
        if let Some(previous) = fields["endpoints"].as_array() {
            if clear {
                // An explicit transport withdrawal wins over rotation metadata.
                // The settings CLI serializes it with rekey using its fleet lock.
                result.extend(previous.iter().filter(|entry| entry["transport"] == "fabric-rekey-receipt").cloned());
                return Ok(result);
            }
            result.extend(previous.iter().filter(|entry| metadata(entry)).cloned());
            if let Some(rotation) = previous.iter().find(|entry| entry["transport"] == "fabric-rekey") {
                let node = if rotation["state"] == "committed" { &rotation["next_node"] } else { &rotation["old_node"] };
                for endpoint in &mut result {
                    if endpoint["transport"] == "fabric" { endpoint["node"] = node.clone(); }
                }
                // A refresh before Fabric is ready must not remove the transition's route.
                if !result.iter().any(|entry| entry["transport"] == "fabric") {
                    result.extend(previous.iter().filter(|entry| entry["transport"] == "fabric").cloned());
                }
            }
        }
        Ok(result)
    }

    /// Publish and seal before replying. `receipt` is called by the local adapter only
    /// after its verified runtime snapshot proves grants for the selected state installed.
    pub fn fabric_rekey(&self, request: &FabricRekeyRequest, build: &str) -> Result<bool> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        self.fabric_rekey_at(request, build, now)
    }

    fn fabric_rekey_at(&self, request: &FabricRekeyRequest, build: &str, now: u64) -> Result<bool> {
        anyhow::ensure!(node_id(&request.old_node) && node_id(&request.next_node)
            && request.old_node != request.next_node, "rekey requires distinct canonical Fabric node ids");
        let _publication = self.fleet_endpoints_publication.lock().unwrap_or_else(PoisonError::into_inner);
        let membership = self.fleet_membership()?;
        let crate::fleet::MemberState::Current(own) = membership.state(&self.origin) else {
            anyhow::bail!("rekey requires a current local fleet member");
        };
        anyhow::ensure!(Some(own.member_key.as_str()) == self.member_public_key().as_deref(), "local member key mismatch");
        let fields = self.own_endpoint_fields()?;
        let mut endpoints = fields["endpoints"].as_array().cloned().unwrap_or_default();
        if request.action == FabricRekeyAction::Receipt {
            let target_key = request.member_key.as_deref().context("receipt requires member_key")?;
            anyhow::ensure!(target_key != own.member_key, "own receipts do not acknowledge a rekey");
            let state = request.state.context("receipt requires state")?;
            let expected_state = serde_json::to_value(state)?;
            let target = membership.incarnations().find(|member| member.member_key == target_key
                && matches!(membership.state(&member.name), crate::fleet::MemberState::Current(current) if current.member_key == target_key))
                .context("receipt target is not a current member")?;
            anyhow::ensure!(request.deadline > now && target.endpoints.iter().any(|entry|
                entry["transport"] == "fabric-rekey" && selectors(entry, request)
                    && entry["state"] == expected_state), "receipt must match an exact live target transition");
            let receipt = json!({"transport":"fabric-rekey-receipt", "member_key":target_key,
                "old_node":request.old_node,"next_node":request.next_node,
                "deadline":request.deadline,"state":state});
            endpoints.retain(|entry| !(entry["transport"] == "fabric-rekey-receipt" && entry["member_key"] == target_key));
            endpoints.push(receipt);
        } else {
            anyhow::ensure!(request.member_key.is_none() && request.state.is_none(), "member_key and state are receipt-only selectors");
            let previous = endpoints.iter().find(|entry| entry["transport"] == "fabric-rekey");
            let next_state = match request.action {
                FabricRekeyAction::Prepare => {
                    anyhow::ensure!(request.deadline > now && request.deadline <= now.saturating_add(600), "prepare deadline must be within 600 seconds");
                    if let Some(previous) = previous {
                        if selectors(previous, request) {
                            anyhow::ensure!(previous["state"] == "prepared", "transition already ended preparation");
                        } else {
                            anyhow::ensure!(previous["state"] == "aborted" || previous["deadline"].as_u64().is_some_and(|deadline| deadline <= now), "conflicting active rekey");
                            // A pending Fabric key may be reused by a new operation only
                            // after an aborted operation's original window has elapsed.
                            let ended = previous["state"] == "aborted"
                                && previous["deadline"].as_u64().is_some_and(|deadline| deadline <= now);
                            anyhow::ensure!(previous["old_node"] != request.old_node
                                || previous["next_node"] != request.next_node || ended,
                                "retries cannot extend a rekey deadline");
                        }
                    }
                    anyhow::ensure!(own.endpoints.iter().any(|entry| entry["transport"] == "fabric" && entry["node"] == request.old_node), "prepare old_node is not the current Fabric member endpoint");
                    "prepared"
                }
                FabricRekeyAction::Commit | FabricRekeyAction::Abort => {
                    if request.action == FabricRekeyAction::Abort && previous.is_none() {
                        anyhow::ensure!(own.endpoints.iter().any(|entry| entry["transport"] == "fabric"
                            && entry["node"] == request.old_node), "abort without preparation requires the current old endpoint");
                        return Ok(false);
                    }
                    let previous = previous.context("no prepared rekey")?;
                    anyhow::ensure!(selectors(previous, request), "transition selectors do not match");
                    if request.action == FabricRekeyAction::Commit {
                        anyhow::ensure!(request.deadline > now && matches!(previous["state"].as_str(), Some("prepared" | "committed")), "commit requires a live prepared transition");
                        "committed"
                    } else {
                        // Roll back even after a committed acknowledgement timeout: the CLI
                        // has not activated the next local identity yet.
                        anyhow::ensure!(matches!(previous["state"].as_str(), Some("prepared" | "committed" | "aborted")), "invalid abort transition");
                        "aborted"
                    }
                }
                FabricRekeyAction::Receipt => unreachable!(),
            };
            endpoints.retain(|entry| entry["transport"] != "fabric-rekey");
            endpoints.push(json!({"transport":"fabric-rekey","old_node":request.old_node,
                "next_node":request.next_node,"deadline":request.deadline,"state":next_state}));
            for endpoint in &mut endpoints {
                if endpoint["transport"] == "fabric" {
                    endpoint["node"] = json!(if next_state == "committed" { &request.next_node } else { &request.old_node });
                }
            }
        }
        self.publish_fleet_endpoints_locked(fields["mode"].as_str().unwrap_or(&own.mode), &endpoints, build)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::MemberKey;
    use crate::store::runtime::Plain;

    fn configure(store: &Store) -> Arc<MemberKey> {
        store.bind_fleet("rekey-test").unwrap();
        let (key, _) = MemberKey::generate().unwrap();
        let key = Arc::new(key);
        store.pin_fleet_anchor(key.public()).unwrap();
        store.set_member_key(Some(key.clone())).unwrap();
        store.admit_fleet_anchor("rekey-test", key.public(), "listening").unwrap();
        store.publish_fleet_endpoints("listening", &[json!({
            "transport":"fabric","node":"a".repeat(64),"protocol":"st3/fleet/rekey-test"
        })], "test").unwrap();
        key
    }

    fn request(action: FabricRekeyAction) -> FabricRekeyRequest {
        FabricRekeyRequest {
            action, old_node: "a".repeat(64), next_node: "b".repeat(64),
            deadline: 1600, member_key: None, state: None,
        }
    }

    fn current_endpoints(store: &Store) -> Vec<Value> {
        let view = store.fleet_view_sealed().unwrap();
        view.members.iter().find(|member| member.name == "anchor").unwrap().endpoints.clone()
    }

    #[test]
    fn signed_transition_survives_worker_refresh_and_abort_from_committed() {
        let store = Store::open_memory("anchor", Arc::new(Plain)).unwrap();
        configure(&store);
        let mut request = request(FabricRekeyAction::Prepare);
        assert!(store.fabric_rekey_at(&request, "test", 1000).unwrap());
        assert!(!store.fabric_rekey_at(&request, "test", 1001).unwrap());
        let prepared = current_endpoints(&store);
        assert!(prepared.iter().any(|entry| entry["state"] == "prepared"));
        let routes = crate::fleet::transport::routes_from_endpoints(&prepared,
            crate::fleet::transport::LocalTransports { fabric: true, ..Default::default() });
        assert_eq!(routes.len(), 1, "signed rekey metadata must never become a route");
        store.publish_fleet_endpoints("listening", &[json!({
            "transport":"fabric","node":"c".repeat(64),"protocol":"st3/fleet/rekey-test"
        })], "test").unwrap();
        assert_eq!(current_endpoints(&store), prepared);
        request.action = FabricRekeyAction::Commit;
        assert!(store.fabric_rekey_at(&request, "test", 1002).unwrap());
        assert!(!store.fabric_rekey_at(&request, "test", 1003).unwrap());
        store.publish_fleet_endpoints("listening", &[], "test").unwrap();
        let committed = current_endpoints(&store);
        assert!(committed.iter().any(|entry| entry["transport"] == "fabric" && entry["node"] == request.next_node));
        request.action = FabricRekeyAction::Abort;
        assert!(store.fabric_rekey_at(&request, "test", 1601).unwrap());
        assert!(!store.fabric_rekey_at(&request, "test", 1602).unwrap());
        let aborted = current_endpoints(&store);
        assert!(aborted.iter().any(|entry| entry["transport"] == "fabric" && entry["node"] == request.old_node));
        assert!(aborted.iter().any(|entry| entry["state"] == "aborted"));
    }

    #[test]
    fn prepare_refuses_wrong_identity_conflicts_extended_retries_and_expired_commit() {
        let store = Store::open_memory("anchor", Arc::new(Plain)).unwrap();
        configure(&store);
        let mut invalid = request(FabricRekeyAction::Prepare);
        invalid.old_node = "c".repeat(64);
        assert!(store.fabric_rekey_at(&invalid, "test", 1000).is_err());
        invalid = request(FabricRekeyAction::Prepare);
        invalid.deadline = 1601;
        assert!(store.fabric_rekey_at(&invalid, "test", 1000).is_err());
        let prepared = request(FabricRekeyAction::Prepare);
        store.fabric_rekey_at(&prepared, "test", 1000).unwrap();
        invalid.deadline = 1600;
        invalid.next_node = "c".repeat(64);
        assert!(store.fabric_rekey_at(&invalid, "test", 1000).is_err());
        invalid = prepared.clone();
        invalid.deadline = 1601;
        assert!(store.fabric_rekey_at(&invalid, "test", 1001).is_err());
        assert!(store.fabric_rekey_at(&invalid, "test", 1600).is_err());
        invalid = prepared;
        invalid.action = FabricRekeyAction::Commit;
        assert!(store.fabric_rekey_at(&invalid, "test", 1600).is_err());
    }

    #[test]
    fn crashed_prepare_resumes_original_signed_deadline() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("claims.db");
        let store = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
        let key = configure(&store);
        let prepared = request(FabricRekeyAction::Prepare);
        store.fabric_rekey_at(&prepared, "test", 1000).unwrap();
        drop(store);
        let store = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
        store.set_member_key(Some(key)).unwrap();
        assert!(!store.fabric_rekey_at(&prepared, "test", 1100).unwrap());
        let mut extended = prepared.clone();
        extended.deadline += 1;
        assert!(store.fabric_rekey_at(&extended, "test", 1100).is_err());
        assert!(current_endpoints(&store).iter().any(|entry| entry["deadline"] == 1600));
    }

    #[test]
    fn abort_before_first_prepare_is_safe_and_receipts_refuse_own_or_missing_target() {
        let store = Store::open_memory("anchor", Arc::new(Plain)).unwrap();
        let key = configure(&store);
        assert!(!store.fabric_rekey_at(&request(FabricRekeyAction::Abort), "test", 1601).unwrap());
        let mut receipt = request(FabricRekeyAction::Receipt);
        receipt.member_key = Some(key.public().to_owned());
        receipt.state = Some(FabricRekeyState::Prepared);
        assert!(store.fabric_rekey_at(&receipt, "test", 1000).is_err());
        receipt.member_key = Some("missing".into());
        assert!(store.fabric_rekey_at(&receipt, "test", 1000).is_err());
    }

    #[test]
    fn receipt_is_signed_and_bound_to_remote_live_state_and_all_selectors() {
        let anchor = Store::open_memory("anchor", Arc::new(Plain)).unwrap();
        let anchor_key = configure(&anchor);
        let peer = Store::open_memory("peer", Arc::new(Plain)).unwrap();
        let (peer_key, _) = MemberKey::generate().unwrap();
        let peer_key = Arc::new(peer_key);
        peer.bind_fleet("rekey-test").unwrap();
        peer.pin_fleet_anchor(anchor_key.public()).unwrap();
        peer.set_member_key(Some(peer_key.clone())).unwrap();
        anchor.append_claim(&ClaimInput {
            subject: "host/peer".into(), kind: "fleet.member-admitted".into(), actor: None,
            fields: BTreeMap::from([
                ("fleet_id".into(), json!("rekey-test")),
                ("member_key".into(), json!(peer_key.public())),
                ("via".into(), json!("invite")), ("mode".into(), json!("listening")),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let mut rotation = request(FabricRekeyAction::Prepare);
        anchor.fabric_rekey_at(&rotation, "test", 1000).unwrap();
        let exchange = anchor.export_replication_exchange("rekey-test", &ReplicationInventory::default()).unwrap();
        peer.receive_replication_exchange("anchor", "rekey-test", &exchange).unwrap();
        peer.validate_replication_backlog().unwrap();
        peer.apply_replication_repairs().unwrap();
        peer.project_replication_backlog().unwrap();
        let mut receipt = request(FabricRekeyAction::Receipt);
        receipt.member_key = Some(anchor_key.public().to_owned());
        receipt.state = Some(FabricRekeyState::Committed);
        assert!(peer.fabric_rekey_at(&receipt, "test", 1001).is_err());
        receipt.state = Some(FabricRekeyState::Prepared);
        receipt.deadline -= 1;
        assert!(peer.fabric_rekey_at(&receipt, "test", 1001).is_err());
        receipt.deadline += 1;
        assert!(peer.fabric_rekey_at(&receipt, "test", 1001).unwrap());
        assert!(!peer.fabric_rekey_at(&receipt, "test", 1002).unwrap());
        peer.publish_fleet_endpoints("listening", &[], "test").unwrap();
        let view = peer.fleet_view_sealed().unwrap();
        let own = view.members.iter().find(|member| member.name == "peer").unwrap();
        assert!(own.endpoints.iter().any(|entry| entry["transport"] == "fabric-rekey-receipt"
            && entry["state"] == "prepared" && entry["member_key"] == anchor_key.public()));
        rotation.action = FabricRekeyAction::Commit;
        anchor.fabric_rekey_at(&rotation, "test", 1003).unwrap();
        let exchange = anchor.export_replication_exchange("rekey-test", &ReplicationInventory::default()).unwrap();
        peer.receive_replication_exchange("anchor", "rekey-test", &exchange).unwrap();
        peer.validate_replication_backlog().unwrap();
        peer.apply_replication_repairs().unwrap();
        peer.project_replication_backlog().unwrap();
        assert!(peer.fabric_rekey_at(&receipt, "test", 1004).is_err());
        receipt.state = Some(FabricRekeyState::Committed);
        assert!(peer.fabric_rekey_at(&receipt, "test", 1004).unwrap());
        assert!(peer.fabric_rekey_at(&receipt, "test", 1600).is_err());
        peer.publish_fleet_endpoints_clearing_fabric_rekey("listening", &[], "test").unwrap();
        let view = peer.fleet_view_sealed().unwrap();
        let own = view.members.iter().find(|member| member.name == "peer").unwrap();
        assert!(own.endpoints.iter().any(|entry| entry["transport"] == "fabric-rekey-receipt"
            && entry["state"] == "committed"), "settings clear must preserve remote receipts");
    }

    #[test]
    fn explicit_clear_withdraws_committed_route_without_resurrection() {
        let store = Store::open_memory("anchor", Arc::new(Plain)).unwrap();
        configure(&store);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut rotation = request(FabricRekeyAction::Prepare);
        rotation.deadline = now + 500;
        store.fabric_rekey_at(&rotation, "test", now).unwrap();
        rotation.action = FabricRekeyAction::Commit;
        store.fabric_rekey_at(&rotation, "test", now).unwrap();
        assert!(store.publish_fleet_endpoints_clearing_fabric_rekey("listening", &[], "test").unwrap());
        assert!(current_endpoints(&store).is_empty());
        assert!(!store.publish_fleet_endpoints("listening", &[], "test").unwrap());
        assert!(current_endpoints(&store).is_empty());
    }

    #[test]
    fn dial_out_publication_clears_completed_rotation_and_never_advertises_fabric() {
        let store = Store::open_memory("anchor", Arc::new(Plain)).unwrap();
        configure(&store);
        let rotation = request(FabricRekeyAction::Prepare);
        store.fabric_rekey_at(&rotation, "test", 1000).unwrap();
        // Its historical deadline is expired: changing mode can withdraw the route.
        store.publish_fleet_endpoints("dial-out", &[json!({
            "transport":"fabric","node":rotation.old_node,"protocol":"st3/fleet/rekey-test"
        })], "test").unwrap();
        assert!(current_endpoints(&store).is_empty());
        assert_eq!(store.fleet_view_sealed().unwrap().members[0].mode, "dial-out");
    }

    #[test]
    fn expired_aborted_operation_can_start_again_with_same_pending_key() {
        let store = Store::open_memory("anchor", Arc::new(Plain)).unwrap();
        configure(&store);
        let mut rotation = request(FabricRekeyAction::Prepare);
        store.fabric_rekey_at(&rotation, "test", 1000).unwrap();
        rotation.action = FabricRekeyAction::Abort;
        store.fabric_rekey_at(&rotation, "test", 1001).unwrap();
        rotation.action = FabricRekeyAction::Prepare;
        rotation.deadline = 1700;
        assert!(store.fabric_rekey_at(&rotation, "test", 1100).is_err());
        assert!(store.fabric_rekey_at(&rotation, "test", 1600).unwrap());
        assert!(current_endpoints(&store).iter().any(|entry|
            entry["transport"] == "fabric-rekey" && entry["state"] == "prepared"
                && entry["deadline"] == 1700));
    }
}
