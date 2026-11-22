//! Reader-first activity contract; no route, runtime query or producer is installed.
//! Phase 2 must serve this from a maintained incarnation/workspace projection.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActivityStatus {
    Desired,
    Starting,
    Running,
    Waiting,
    Down,
    Stopped,
    Failed,
    Suspended,
    Retired,
    Unknown,
}

impl ActivityStatus {
    /// Preserve unfamiliar observed values without extending the closed status set.
    pub fn from_observed(raw: Option<&str>) -> (Self, Option<&str>) {
        let status = match raw {
            Some("desired") => Self::Desired,
            Some("starting") => Self::Starting,
            Some("running") => Self::Running,
            Some("waiting") => Self::Waiting,
            Some("down") => Self::Down,
            Some("stopped") => Self::Stopped,
            Some("failed") => Self::Failed,
            Some("suspended") => Self::Suspended,
            Some("retired") => Self::Retired,
            _ => return (Self::Unknown, raw),
        };
        (status, None)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActivityLayer {
    Current,
    Historical,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IncompleteReason {
    NoActual,
    IndeterminateAuthority,
    MissingWorkspace,
    MissingHost,
    MissingIncarnation,
    UnresolvedLiveness,
}

/// Identity is (id, incarnation_id, host, workspace), not the agent ID alone.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActivityAgent {
    pub id: String,
    pub layer: ActivityLayer,
    pub incarnation_id: String,
    pub host: String,
    /// An authoritative absolute physical path on host, never a URI or declaration fallback.
    pub workspace: String,
    pub status: ActivityStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_raw: Option<String>,
    pub actual_claim: String,
    pub actual_origin: String,
    /// Advisory only; a newer start is not proof this incarnation ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActivityIncomplete {
    pub id: String,
    pub layer: ActivityLayer,
    pub incarnation_id: Option<String>,
    pub host: Option<String>,
    pub reason: IncompleteReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActivitySnapshot {
    #[serde(flatten)]
    pub snapshot: crate::api::ClientSnapshot,
    pub local_host_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActivityEnvelope {
    pub snapshot: ActivitySnapshot,
    pub agents: Vec<ActivityAgent>,
    pub incomplete: Vec<ActivityIncomplete>,
    pub next_cursor: Option<String>,
    /// True only on the final page when no page at this cut had incomplete evidence.
    /// Consumers must accumulate the whole chain; this is not proof of filesystem absence.
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn known_statuses_are_closed_and_unknown_values_preserve_raw_text() {
        for raw in ["desired", "starting", "running", "waiting", "down", "stopped",
            "failed", "suspended", "retired"] {
            let (status, extra) = ActivityStatus::from_observed(Some(raw));
            assert_eq!(serde_json::to_value(status).unwrap(), raw);
            assert_eq!(extra, None);
        }
        assert_eq!(ActivityStatus::from_observed(Some("future-status")),
            (ActivityStatus::Unknown, Some("future-status")));
        assert_eq!(ActivityStatus::from_observed(None), (ActivityStatus::Unknown, None));
        assert!(serde_json::from_value::<ActivityStatus>(json!("future-status")).is_err());
    }

    #[test]
    fn incarnation_rows_and_snapshot_round_trip_without_changing_the_envelope_shape() {
        let row = json!({
            "id":"agent/example/worker", "layer":"historical", "incarnation_id":"one",
            "host":"host/example", "workspace":"/worktrees/physical", "status":"unknown",
            "status_raw":"future-status", "actual_claim":"claim/example",
            "actual_origin":"example", "superseded_by":"two"
        });
        let mut second_path = row.clone();
        second_path["workspace"] = json!("/worktrees/other");
        let envelope = json!({
            "snapshot": {
                "id":"snapshot/example", "host_id":"host/example", "local_host_id":"host/example",
                "store_index":7, "projection_version":"example", "created_at":"2026-01-01T00:00:00Z"
            },
            "agents":[row, second_path],
            "incomplete":[{
                "id":"agent/example/missing", "layer":"current", "incarnation_id":null,
                "host":null, "reason":"missing-incarnation"
            }],
            "next_cursor":null, "complete":false
        });
        let parsed: ActivityEnvelope = serde_json::from_value(envelope.clone()).unwrap();
        assert_eq!(parsed.agents.len(), 2);
        assert_eq!(parsed.agents[0].id, parsed.agents[1].id);
        assert_eq!(parsed.agents[0].incarnation_id, parsed.agents[1].incarnation_id);
        assert_ne!(parsed.agents[0].workspace, parsed.agents[1].workspace);
        assert_eq!(serde_json::to_value(parsed).unwrap(), envelope);
    }

    #[test]
    fn incomplete_reasons_are_closed_and_optional_row_fields_stay_optional() {
        for raw in ["no-actual", "indeterminate-authority", "missing-workspace",
            "missing-host", "missing-incarnation", "unresolved-liveness"] {
            let reason: IncompleteReason = serde_json::from_value(json!(raw)).unwrap();
            assert_eq!(serde_json::to_value(reason).unwrap(), raw);
        }
        assert!(serde_json::from_value::<IncompleteReason>(json!("future-reason")).is_err());
        let row = json!({
            "id":"agent/example/worker", "layer":"current", "incarnation_id":"one",
            "host":"host/example", "workspace":"/worktrees/physical", "status":"running",
            "actual_claim":"claim/example", "actual_origin":"example"
        });
        let parsed: ActivityAgent = serde_json::from_value(row.clone()).unwrap();
        assert_eq!(parsed.status_raw, None);
        assert_eq!(parsed.superseded_by, None);
        assert_eq!(serde_json::to_value(parsed).unwrap(), row);
    }
}
