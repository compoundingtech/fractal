//! Terminal shell membership observations in the durable claims log.
use super::*;
use super::resource_membership::{Coverage, RequestClass};
use super::shell_resources::Execution;

pub const KIND: &str = "resource.exec-receipt";

/// Head-first subjects support the existing indexed claims prefix query without
/// creating a second receipt authority. Components are exact, slash-safe identities.
pub fn head_prefix(head: &str) -> String {
    format!("resource/exec/{}/", hex::encode(head))
}

pub(super) fn append_tx(transaction: &Transaction<'_>, origin: &str, execution: &Execution) -> Result<()> {
    let request = &execution.command.lane;
    anyhow::ensure!(request.host == origin, "only the executing host may record its receipt");
    let result = execution.result.as_ref();
    anyhow::ensure!(execution.started_at.is_none() || result.is_some(),
        "launched shell execution must persist its outcome before terminal retirement");
    let head_start = result.and_then(|result| result.head_start.as_deref()).or(execution.head_start.as_deref());
    let class = match request.class {
        RequestClass::Landing => "landing",
        RequestClass::Check => "check",
        RequestClass::Agent => "agent",
    };
    let subject = format!("{}{}/{}/{}", head_prefix(head_start.unwrap_or_default()), hex::encode(origin), class, hex::encode(&execution.command.id));
    let mut fields = serde_json::Map::new();
    for (name, value) in [
        ("exec_id", json!(execution.command.id)),
        ("host", json!(request.host)),
        ("class", json!(request.class)),
        ("owner", json!(request.owner)),
        ("coverage", json!(request.coverage)),
        ("state", json!(execution.state)),
        ("disk_reserved_bytes", json!(request.disk.as_ref().map_or(0, |disk| disk.bytes))),
        ("queue_wait_ms", json!(execution.queue_wait_ms)),
        ("wall_duration_ms", json!(result.map_or(0, |result| result.wall_duration_ms))),
    ] { fields.insert(name.into(), value); }
    // Absent observations stay absent. In particular a logical execution never
    // turns an unavailable cgroup peak into a measured zero.
    for (name, value) in [
        ("memory_bytes", request.capacity.as_ref().map(|capacity| json!(capacity.memory))),
        ("io", request.io.as_ref().map(|io| json!(io))),
        ("head_start", head_start.map(|head| json!(head))),
        ("head_end", result.and_then(|result| result.head_end.as_ref()).map(|head| json!(head))),
        ("clean_start", result.and_then(|result| result.clean_start).or(execution.clean_start).map(|clean| json!(clean))),
        ("clean_end", result.and_then(|result| result.clean_end).map(|clean| json!(clean))),
        ("exit_code", execution.exit_code.map(|code| json!(code))),
        ("exit_signal", execution.exit_signal.map(|signal| json!(signal))),
        ("memory_peak_bytes", result.and_then(|result| result.memory_peak_bytes)
            .filter(|_| request.coverage == Coverage::Enforced).map(|peak| json!(peak))),
    ] {
        if let Some(value) = value { fields.insert(name.into(), value); }
    }
    append_claim_tx(transaction, origin, &subject, KIND, None, &json!({"fields": fields}), &[], None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::resource_membership::{LaneRequest, Owner};
    use super::super::shell_resources::{Command, PayloadResult};

    fn command(id: &str) -> Command {
        Command { id: id.into(), lane: LaneRequest {
            id: format!("shell:{id}"), name: "checks".into(), host: "host-a".into(), max_members: 1,
            capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1024 }),
            class: RequestClass::Check, disk: None, io: None, coverage: Coverage::Logical,
            admission_timeout_ms: None,
            owner: Owner { run: "person/operator".into(), step: id.into(), attempt: 1, invocation: None },
        }, argv: vec!["true".into()], cwd: "/workspace".into(), environment: BTreeMap::new(), timeout_ms: None, parent: None }
    }

    fn terminal(store: &Store, id: &str, state: &str) -> Execution {
        let mut execution = store.submit_shell_resource(command(id), 10).unwrap();
        execution.state = state.into();
        execution.exit_code = Some(37);
        execution.result = Some(PayloadResult {
            head_start: Some("abc123".into()), head_end: Some("abc123".into()),
            clean_start: Some(true), clean_end: Some(false), exit_code: Some(37), exit_signal: None,
            wall_duration_ms: 1234, memory_peak_bytes: None,
        });
        execution
    }

    fn claims(store: &Store, prefix: Option<&str>) -> Vec<ClaimRecord> {
        store.claims_page_matching(ClaimsFilter { kind: Some(KIND), subject_prefix: prefix,
            ..ClaimsFilter::default() }, None, 0, None, false, 100).unwrap().claims
    }

    #[test]
    fn terminal_claim_is_atomic_once_including_cancel_and_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.db");
        {
            let store = Store::open(&path, "host-a").unwrap();
            for state in ["completed", "cancelled", "refused"] {
                let execution = terminal(&store, state, state);
                if state == "cancelled" { store.cancel_shell_resource(state).unwrap(); }
                assert!(store.update_shell_resource(&execution).unwrap());
                assert!(!store.update_shell_resource(&execution).unwrap());
                let mut changed = execution.clone(); changed.exit_code = Some(99);
                assert!(!store.update_shell_resource(&changed).unwrap());
            }
            assert_eq!(claims(&store, None).len(), 3);
        }
        let store = Store::open(&path, "host-a").unwrap();
        assert_eq!(claims(&store, None).len(), 3);
        let execution = store.shell_resource("cancelled").unwrap();
        assert!(!store.update_shell_resource(&execution).unwrap());
        assert_eq!(claims(&store, None).len(), 3);
    }

    #[test]
    fn head_query_returns_declared_profile_and_logical_peak_stays_absent() {
        let store = Store::open_memory("host-a").unwrap();
        let execution = terminal(&store, "one", "completed");
        store.update_shell_resource(&execution).unwrap();
        let page = claims(&store, Some(&head_prefix("abc123")));
        assert_eq!(page.len(), 1);
        let fields = &page[0].body["fields"];
        assert_eq!(fields["host"], "host-a"); assert_eq!(fields["class"], "check");
        assert_eq!(fields["memory_bytes"], 1024); assert_eq!(fields["disk_reserved_bytes"], 0);
        assert!(fields.get("memory_peak_bytes").is_none());
        assert!(claims(&store, Some(&head_prefix("other"))).is_empty());
        assert_eq!(fractal_schema::retention::policy().kind(KIND).class, fractal_schema::retention::Class::Durable);
    }

    #[test]
    fn failed_claim_append_rolls_back_terminal_transition() {
        let store = Store::open_memory("host-a").unwrap();
        let mut execution = terminal(&store, "one", "completed");
        execution.command.lane.host = "host-b".into();
        assert!(store.update_shell_resource(&execution).is_err());
        assert_eq!(store.shell_resource("one").unwrap().state, "waiting");
        assert!(claims(&store, None).is_empty());
    }

    #[test]
    fn launched_execution_cannot_commit_an_observationless_terminal_claim() {
        let store = Store::open_memory("host-a").unwrap();
        let mut execution = terminal(&store, "one", "cancelled");
        execution.started_at = Some(20);
        execution.result = None;
        assert!(store.update_shell_resource(&execution).is_err());
        assert_eq!(store.shell_resource("one").unwrap().state, "waiting");
        assert!(claims(&store, None).is_empty());
    }

    #[test]
    fn nested_execution_claims_only_the_charged_membership() {
        let store = Store::open_memory("host-a").unwrap();
        let parent = terminal(&store, "parent", "completed");
        let mut child_command = command("child");
        child_command.parent = Some(super::super::resource_membership::Grant {
            membership_id: "parent".into(), authority_epoch: "epoch".into(),
            generation: "generation".into(), coverage: Coverage::Logical,
        });
        let mut child = store.submit_shell_resource(child_command, 10).unwrap();
        child.state = "completed".into();
        store.update_shell_resource(&child).unwrap();
        store.update_shell_resource(&parent).unwrap();
        let page = claims(&store, None);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].body["fields"]["exec_id"], "parent");
    }
}
