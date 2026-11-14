#![allow(unused_imports)]
use crate::store::*;
use smallclaims::store::checkpoint_agreement::*;

pub const FLEET: &str = "fleet/test";

pub fn sync(nodes: &[&Store]) {
    for node in nodes {
        node.bind_fleet(FLEET).unwrap();
    }
    for _ in 0..3 {
        for source in nodes {
            for target in nodes {
                if source.origin == target.origin {
                    continue;
                }
                let exchange = source
                    .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
                    .unwrap();
                target
                    .receive_replication_exchange(&source.origin, FLEET, &exchange)
                    .unwrap();
                target.validate_replication_backlog().unwrap();
                target.project_replication_backlog().unwrap();
                target.apply_replication_repairs().unwrap();
            }
        }
    }
}

pub fn observe(store: &Store, n: usize) {
    for index in 0..n {
        store
            .append_as_older_build_for_test(&ClaimInput {
                subject: format!("daemon/{}", store.origin),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("slow-request")),
                    ("reason".into(), json!(format!("slow {index}"))),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!(
                    "{}-slow-{index}-{}",
                    store.origin,
                    Uuid::now_v7().simple()
                )),
            })
            .unwrap();
    }
}

/// A context two days after tomorrow's cut, so everything written now is before the cut.
pub fn context(scratch: &Path, days_later: u128) -> CheckpointContext {
    CheckpointContext {
        now_unix_ms: now_ms() + (3 + days_later) * DAY_MS,
        configured_peers: Vec::new(),
        scratch: scratch.to_path_buf(),
        reviewer: "person/operator".into(),
    }
}

pub fn step(store: &Store, context: &CheckpointContext) -> Vec<CheckpointAction> {
    store.checkpoint_step(context).unwrap()
}

pub fn kinds(actions: &[CheckpointAction]) -> Vec<&'static str> {
    actions
        .iter()
        .map(|action| match action {
            CheckpointAction::Sealed { .. } => "sealed",
            CheckpointAction::Verified { .. } => "verified",
            CheckpointAction::ProofFailed { .. } => "proof-failed",
            CheckpointAction::AttentionRequested { .. } => "attention",
            CheckpointAction::AttentionWithdrawn { .. } => "withdrawn",
            CheckpointAction::Trimmed { .. } => "trimmed",
            CheckpointAction::ManifestNeeded { .. } => "manifest-needed",
            CheckpointAction::TrimGraphChanged { .. } => "graph-changed",
            CheckpointAction::TrimDeferred { .. } => "deferred",
            CheckpointAction::SweptTrimmedClaimRows { .. } => "swept",
            CheckpointAction::HeldBack { .. } => "held-back",
        })
        .collect()
}

pub fn stable_cuts(store: &Store) -> Vec<u128> {
    stable_checkpoints(&store.checkpoint_claims().unwrap())
        .into_keys()
        .collect()
}

pub fn claim_ids(store: &Store) -> BTreeSet<String> {
    store
        .claims_page(None, None, 0, None, false, 100_000)
        .unwrap()
        .claims
        .into_iter()
        .map(|claim| claim.id)
        .collect()
}
