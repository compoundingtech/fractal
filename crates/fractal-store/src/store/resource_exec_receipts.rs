//! Terminal execution observations in the existing durable claims log.
use super::*;
use super::resource_membership::{Coverage, RequestClass};
use super::shell_resources::Execution;

pub const KIND: &str = "resource.exec-receipt";

/// Head-first subjects support the existing indexed claims prefix query without
/// creating a second receipt authority. Components are exact, slash-safe identities.
pub fn head_prefix(head: &str) -> String {
    let component = if head.is_empty() { "_".into() } else { hex::encode(head) };
    format!("resource/exec/{component}/")
}

use fractal_runtime::argv_sha256;

pub fn subject(origin: &str, execution: &Execution) -> String {
    let head = execution.result.as_ref().and_then(|result| result.head_start.as_deref())
        .or(execution.head_start.as_deref()).unwrap_or_default();
    let class = match execution.command.lane.class {
        RequestClass::Landing => "landing", RequestClass::Check => "check", RequestClass::Agent => "agent",
    };
    format!("{}{}/{}/{}", head_prefix(head), hex::encode(origin), class, hex::encode(&execution.command.id))
}

/// The same immutable engine observations feed the durable claim and terminal receipt file.
pub fn receipt_fields(execution: &Execution) -> serde_json::Map<String, Value> {
    let request = &execution.command.lane;
    let result = execution.result.as_ref();
    let head_start = result.and_then(|result| result.head_start.as_deref()).or(execution.head_start.as_deref());
    let mut fields = serde_json::Map::new();
    for (name, value) in [
        ("exec_id", json!(execution.command.id)),
        ("argv", json!(execution.command.argv)),
        ("argv_sha256", json!(argv_sha256(&execution.command.argv))),
        ("launched", json!(execution.started_at.is_some())),
        ("host", json!(request.host)),
        ("class", json!(request.class)),
        ("owner", json!(request.owner)),
        ("coverage", json!(request.coverage)),
        ("state", json!(execution.state)),
        ("timed_out", json!(execution.timed_out)),
        ("refusal_code", json!(execution.refusal_code)),
        ("disk_reserved_bytes", json!(request.disk.as_ref().map_or(0, |disk| disk.bytes))),
        ("queue_wait_ms", json!(execution.queue_wait_ms)),
        ("wall_duration_ms", json!(result.map_or(0, |result| result.wall_duration_ms))),
    ] { fields.insert(name.into(), value); }
    // Absent observations stay absent. In particular a logical execution never
    // turns an unavailable cgroup peak into a measured zero.
    for (name, value) in [
        ("memory_bytes", request.capacity.as_ref().map(|capacity| json!(capacity.memory))),
        ("request_id", execution.command.request_id.as_ref().map(|request_id| json!(request_id))),
        ("profile_key", request.profile.as_ref().map(|key| json!(key))),
        ("profile_estimate", request.profile_estimate.as_ref().map(|estimate| json!(estimate))),
        ("disk_path", request.disk.as_ref().map(|disk| json!(disk.path))),
        ("refusal_reason", execution.refusal_code.map(|_| json!(execution.reason))),
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
    fields
}

pub(super) fn append_tx(transaction: &Transaction<'_>, origin: &str, execution: &Execution) -> Result<()> {
    let request = &execution.command.lane;
    anyhow::ensure!(request.host == origin, "only the executing host may record its receipt");
    let result = execution.result.as_ref();
    anyhow::ensure!(execution.started_at.is_none() || result.is_some(),
        "launched shell execution must persist its outcome before terminal retirement");
    let subject = subject(origin, execution);
    let fields = receipt_fields(execution);
    append_claim_tx(transaction, origin, &subject, KIND, None, &json!({"fields": fields}), &[], None)?;
    Ok(())
}

/// An exact read selector, not an admission request or a request for the latest success.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptQuery {
    pub step_run: String,
    pub attempt: u32,
    pub generation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReceiptFields {
    pub exec_id: String,
    pub runtime_generation: String,
    pub argv: Vec<String>,
    pub argv_sha256: String,
    pub owner: super::resource_membership::Owner,
    pub host: String,
    pub class: RequestClass,
    pub coverage: Coverage,
    pub state: String,
    pub launched: bool,
    pub retired_at: u64,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub profile_key: Option<String>,
    pub head_start: Option<String>,
    pub head_end: Option<String>,
    pub clean_start: Option<bool>,
    pub clean_end: Option<bool>,
    #[serde(flatten)]
    pub observations: serde_json::Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptReference {
    pub claim_id: String,
    pub subject: String,
    pub origin: String,
    pub fields: ReceiptFields,
}

impl Store {
    /// Read authoritative claims through the exact owner/attempt index, never desired state.
    pub fn resource_exec_receipt(&self, query: &ReceiptQuery) -> Result<ReceiptReference, FractalError> {
        if query.step_run.is_empty() || query.attempt == 0 {
            return Err(FractalError::new("resource-receipt-invalid-selector", "stepRun and a positive attempt are required"));
        }
        let connection = self.readers.get();
        let sql = if query.generation.is_some() {
            "SELECT id, store_index, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms
             FROM claims INDEXED BY claims_resource_exec_owner_attempt_index
             WHERE kind='resource.exec-receipt' AND json_extract(body,'$.fields.owner.step')=?1
               AND json_extract(body,'$.fields.owner.attempt')=?2
               AND json_extract(body,'$.fields.runtime_generation')=?3 LIMIT 2"
        } else {
            "SELECT id, store_index, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms
             FROM claims INDEXED BY claims_resource_exec_owner_attempt_index
             WHERE kind='resource.exec-receipt' AND json_extract(body,'$.fields.owner.step')=?1
               AND json_extract(body,'$.fields.owner.attempt')=?2 AND ?3 IS NULL LIMIT 2"
        };
        let mut statement = connection.prepare_cached(sql).map_err(internal)?;
        let mut rows = statement.query_map(rusqlite::params![query.step_run, query.attempt, query.generation], claim_from_row)
            .map_err(internal)?;
        let Some(mut claim) = rows.next().transpose().map_err(internal)? else {
            if query.generation.is_some() {
                return Err(FractalError::new("resource-receipt-stale-generation", "no receipt binds the requested runtime generation"));
            }
            return Err(FractalError::new("resource-receipt-missing",
                "no eligible terminal receipt for this exact step-run and attempt; historical launches without a snapshot, nonlaunches, and unconfirmed retirement are not proof"));
        };
        if rows.next().transpose().map_err(internal)?.is_some() {
            return Err(FractalError::new("resource-receipt-ambiguous", "multiple receipts bind this selector; select the exact runtime generation"));
        }
        let fields: ReceiptFields = serde_json::from_value(claim.body.get_mut("fields").map(Value::take).unwrap_or(Value::Null))
            .map_err(|_| FractalError::new("resource-receipt-ineligible", "receipt lacks a complete immutable launch, outcome, or retirement binding"))?;
        if claim.actor.is_some() || claim.origin != fields.host || !fields.launched || fields.state != "retired"
            || (fields.exit_code.is_none() && fields.exit_signal.is_none())
            || fields.argv_sha256 != argv_sha256(&fields.argv)
        {
            return Err(FractalError::new("resource-receipt-ineligible", "receipt is not a host-owned retired payload outcome"));
        }
        Ok(ReceiptReference { claim_id: claim.id, subject: claim.subject, origin: claim.origin, fields })
    }
}

/// Only the transaction confirming the complete owning tree retired calls this. Missing
/// historical facts permit capacity release but never fabricate an eligible receipt.
pub(super) fn append_root_tx(transaction: &Transaction<'_>, origin: &str,
    member: &super::resource_membership::Membership, root: &super::resource_membership::ExecutionRoot,
    generation: &fractal_runtime::ExecGeneration) -> Result<()> {
    let request = &member.request;
    anyhow::ensure!(request.host == origin && member.state == "retired" && member.retired_at.is_some(),
        "only the executing host may record a retired resource root");
    let Some(snapshot) = &generation.payload_launch else { return Ok(()); };
    if generation.payload_released_at_unix_ms.is_none()
        || (generation.exit_code.is_none() && generation.exit_signal.is_none()) { return Ok(()); }
    let Some(grant) = &member.grant else { anyhow::bail!("retired resource root has no membership identity"); };
    anyhow::ensure!(root.execution.as_ref().is_some_and(|identity| identity.generation_id == generation.generation_id)
        && generation.group_exited && generation.resource_membership.as_ref().is_some_and(|binding|
            binding.membership_id == grant.membership_id && binding.generation == grant.generation
                && binding.authority_epoch == grant.authority_epoch)
        && snapshot.argv_sha256 == argv_sha256(&snapshot.argv), "resource receipt generation binding changed");
    let end = generation.retirement_repository.as_ref();
    let mut fields = serde_json::Map::new();
    for (key, value) in [
        ("exec_id", json!(root.runtime_id)), ("runtime_generation", json!(generation.generation_id)),
        ("argv", json!(snapshot.argv)), ("argv_sha256", json!(snapshot.argv_sha256)),
        ("owner", json!(request.owner)), ("host", json!(request.host)), ("class", json!(request.class)),
        ("coverage", json!(request.coverage)), ("state", json!("retired")), ("launched", json!(true)),
        ("retired_at", json!(member.retired_at)), ("exit_code", json!(generation.exit_code)),
        ("exit_signal", json!(generation.exit_signal)), ("profile_key", json!(request.profile)),
        ("profile_estimate", json!(request.profile_estimate)), ("memory_bytes", json!(request.capacity.map(|capacity| capacity.memory))),
        ("disk_reserved_bytes", json!(request.disk.as_ref().map_or(0, |disk| disk.bytes))),
        ("io", json!(request.io)), ("head_start", json!(snapshot.repository.head)),
        ("clean_start", json!(snapshot.repository.clean)),
        ("head_end", json!(end.and_then(|observation| observation.head.as_ref()))),
        ("clean_end", json!(end.and_then(|observation| observation.clean))),
    ] {
        if !value.is_null() { fields.insert(key.into(), value); }
    }
    let subject = format!("{}root/{}/{}/{}", head_prefix(snapshot.repository.head.as_deref().unwrap_or_default()),
        hex::encode(origin), hex::encode(&root.runtime_id), hex::encode(&generation.generation_id));
    append_claim_tx(transaction, origin, &subject, KIND, None, &json!({"fields": fields}), &[], None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::resource_membership::{LaneRequest, Owner};
    use super::super::shell_resources::{Command, PayloadResult};

    fn direct(kind: &str, attempt: u32, generation: &str) -> (super::super::resource_membership::Membership, fractal_runtime::ExecGeneration) {
        let mut request = command("proof").lane;
        request.id = format!("{kind}:proof:{attempt}");
        request.owner.attempt = attempt;
        request.profile = Some("proof-profile".into());
        let member = serde_json::from_value(json!({
            "request": request, "state": "retired", "reason": "tree empty", "queuedAt": 1,
            "retiredAt": 30, "grant": {"membershipId":"member", "generation":"grant",
                "authorityEpoch":"epoch", "coverage":"logical"},
            "roots": [{"runtimeId":"proof-root", "execution":{"generationId":generation}}]
        })).unwrap();
        let argv = vec!["sh".to_owned(), "-c".to_owned(), "exit 7".to_owned()];
        let execution = serde_json::from_value(json!({
            "schema":"st.runtime.exec-generation.v1", "pid":123, "createdAtUnixMs":1,
            "startToken":1, "generationId":generation, "groupExited":true, "exitCode":7,
            "payloadReleasedAtUnixMs":10, "resourceMembership":{"membershipId":"member",
                "generation":"grant", "authorityEpoch":"epoch"},
            "payloadLaunch":{"argv":argv, "argvSha256":argv_sha256(&argv), "cwd":"/workspace",
                "repository":{"head":"head-one", "clean":true}},
            "retirementRepository":{"head":"head-one", "clean":false}
        })).unwrap();
        (member, execution)
    }

    fn append_direct(store: &Store, member: &super::super::resource_membership::Membership,
        execution: &fractal_runtime::ExecGeneration) -> Result<()> {
        let mut connection = store.connection.write();
        let tx = connection.transaction()?;
        append_root_tx(&tx, "host-a", member, &member.roots[0], execution)?;
        tx.commit()?;
        Ok(())
    }

    fn selector(attempt: u32, generation: Option<&str>) -> ReceiptQuery {
        ReceiptQuery { step_run: "proof".into(), attempt, generation: generation.map(str::to_owned) }
    }

    #[test]
    fn direct_dsl_and_program_receipts_require_retirement_and_preserve_outcomes() {
        for kind in ["dsl", "program"] {
            let store = Store::open_memory("host-a").unwrap();
            let (mut member, mut execution) = direct(kind, 1, "runtime-one");
            member.state = "retiring".into();
            assert!(append_direct(&store, &member, &execution).is_err());
            assert_eq!(store.resource_exec_receipt(&selector(1, None)).unwrap_err().code, "resource-receipt-missing");
            member.state = "retired".into();
            append_direct(&store, &member, &execution).unwrap();
            let receipt = store.resource_exec_receipt(&selector(1, None)).unwrap();
            assert_eq!(receipt.fields.exit_code, Some(7));
            assert_eq!(receipt.fields.exit_signal, None);
            assert_eq!(receipt.fields.argv, execution.payload_launch.as_ref().unwrap().argv);
            assert_eq!(receipt.fields.profile_key.as_deref(), Some("proof-profile"));
            assert_eq!(receipt.fields.head_start.as_deref(), Some("head-one"));
            assert_eq!(receipt.fields.clean_end, Some(false));
            assert!(!receipt.fields.observations.contains_key("memory_peak_bytes"));
            let response = serde_json::to_value(&receipt).unwrap();
            assert!(response.get("grant").is_none());
            assert!(response["fields"].get("resource_membership").is_none());
            assert!(!receipt.claim_id.is_empty());
            assert_eq!(claims(&store, None)[0].kind, KIND);

            member.roots[0].runtime_id = "signal-root".into();
            member.roots[0].execution.as_mut().unwrap().generation_id = "runtime-signal".into();
            execution.generation_id = "runtime-signal".into();
            execution.exit_code = None;
            execution.exit_signal = Some(15);
            append_direct(&store, &member, &execution).unwrap();
            let signal = store.resource_exec_receipt(&selector(1, Some("runtime-signal"))).unwrap();
            assert_eq!(signal.fields.exit_signal, Some(15));
            assert_eq!(signal.fields.exit_code, None);
            assert_eq!(store.resource_exec_receipt(&selector(1, None)).unwrap_err().code, "resource-receipt-ambiguous");
        }
    }

    #[test]
    fn historical_missing_outcome_and_nonlaunches_never_create_proof() {
        for missing in ["snapshot", "outcome", "release"] {
            let store = Store::open_memory("host-a").unwrap();
            let (member, mut execution) = direct("dsl", 1, "runtime-one");
            match missing {
                "snapshot" => execution.payload_launch = None,
                "outcome" => { execution.exit_code = None; execution.exit_signal = None; }
                "release" => execution.payload_released_at_unix_ms = None,
                _ => unreachable!(),
            }
            append_direct(&store, &member, &execution).unwrap();
            assert!(claims(&store, None).is_empty());
            assert_eq!(store.resource_exec_receipt(&selector(1, None)).unwrap_err().code, "resource-receipt-missing");
        }
    }

    #[test]
    fn exact_receipt_lookup_fences_attempt_generation_command_and_finalizer_identity() {
        let store = Store::open_memory("host-a").unwrap();
        let (member, execution) = direct("program", 1, "runtime-one");
        append_direct(&store, &member, &execution).unwrap();
        let first = store.resource_exec_receipt(&selector(1, None)).unwrap();
        assert_eq!(store.resource_exec_receipt(&selector(2, None)).unwrap_err().code, "resource-receipt-missing");
        assert_eq!(store.resource_exec_receipt(&selector(1, Some("stale"))).unwrap_err().code, "resource-receipt-stale-generation");
        let own_step = ReceiptQuery { step_run: "finalizer".into(), ..selector(1, None) };
        assert_eq!(store.resource_exec_receipt(&own_step).unwrap_err().code, "resource-receipt-missing");
        let (member, mut changed) = direct("program", 2, "runtime-two");
        let snapshot = changed.payload_launch.as_mut().unwrap();
        snapshot.argv[2] = "exit 0".into();
        snapshot.argv_sha256 = argv_sha256(&snapshot.argv);
        append_direct(&store, &member, &changed).unwrap();
        let second = store.resource_exec_receipt(&selector(2, None)).unwrap();
        assert_ne!(first.fields.argv_sha256, second.fields.argv_sha256);
        assert_ne!(first.claim_id, second.claim_id);
        assert_eq!(store.resource_exec_receipt(&selector(1, None)).unwrap().claim_id, first.claim_id);
        changed.generation_id = "replacement".into();
        assert!(append_direct(&store, &member, &changed).is_err());
        let mut desired = member.clone();
        desired.request.host = "another-host".into();
        assert!(append_direct(&store, &desired, &execution).is_err());
        let original = store.resource_exec_receipt(&selector(1, None)).unwrap();
        assert_eq!(original.fields.profile_key.as_deref(), Some("proof-profile"));
        assert_eq!(original.fields.head_start.as_deref(), Some("head-one"));
    }

    #[test]
    fn receipt_restart_reads_original_claim_through_exact_owner_index() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("receipts.db");
        let original;
        {
            let store = Store::open(&path, "host-a").unwrap();
            let (member, execution) = direct("dsl", 1, "runtime-one");
            append_direct(&store, &member, &execution).unwrap();
            original = store.resource_exec_receipt(&selector(1, None)).unwrap().claim_id;
        }
        let store = Store::open(&path, "host-a").unwrap();
        let receipt = store.resource_exec_receipt(&selector(1, Some("runtime-one"))).unwrap();
        assert_eq!(receipt.claim_id, original);
        assert_eq!(receipt.fields.argv[2], "exit 7");
        let connection = store.readers.get();
        let plan: String = connection.query_row("EXPLAIN QUERY PLAN SELECT id FROM claims
            INDEXED BY claims_resource_exec_owner_attempt_index WHERE kind='resource.exec-receipt'
            AND json_extract(body,'$.fields.owner.step')='proof'
            AND json_extract(body,'$.fields.owner.attempt')=1", [], |row| row.get(3)).unwrap();
        assert!(plan.contains("SEARCH") && plan.contains("claims_resource_exec_owner_attempt_index"), "{plan}");
    }

    fn command(id: &str) -> Command {
        Command { id: id.into(), request_id: None, lane: LaneRequest {
            id: format!("shell:{id}"), name: "checks".into(), host: "host-a".into(), max_members: 1,
            capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1024 }),
            class: RequestClass::Check, disk: None, io: None, coverage: Coverage::Logical,
            admission_timeout_ms: None,
            profile: None, profile_estimate: None, disk_path: None,
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
    fn landing_refusal_remains_typed_in_the_durable_terminal_receipt() {
        use super::super::resource_membership::{Admission, Fit, RefusalCode};
        let store = Store::open_memory("host-a").unwrap();
        store.start_resource_authority("host-a", "epoch").unwrap();
        let mut command = command("denied-landing");
        command.lane.class = RequestClass::Landing;
        let mut execution = store.submit_shell_resource(command, 10).unwrap();
        let admission = store.request_resource_membership(&execution.command.lane,
            Fit { room: Some(fractal_runtime::Capacity { cores: 2, memory: 4096 }), ..Fit::default() }, 10).unwrap();
        let Admission::Refused { code, reason } = admission else { panic!("landing must be refused") };
        assert_eq!(code, RefusalCode::UnauthorizedLanding);
        execution.state = "refused".into();
        execution.refusal_code = Some(code);
        execution.reason = reason;
        execution.ended_at = Some(11);
        store.update_shell_resource(&execution).unwrap();
        let receipt = claims(&store, None);
        assert_eq!(receipt.len(), 1);
        assert_eq!(receipt[0].body["fields"]["class"], "landing");
        assert_eq!(receipt[0].body["fields"]["refusal_code"], "unauthorized-landing");
        assert!(receipt[0].body["fields"]["refusal_reason"].as_str().unwrap().contains("allowedActors"));
        assert_eq!(store.shell_resource("denied-landing").unwrap().refusal_code, Some(code));
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
        assert_eq!(fields["argv"], json!(["true"]));
        assert_eq!(fields["argv_sha256"], argv_sha256(&execution.command.argv));
        let stored = store.shell_resource("one").unwrap();
        assert_eq!(stored.root_exec_id, "one");
        assert_eq!(stored.receipt_subject.as_deref(), Some(page[0].subject.as_str()));
        assert!(fields.get("memory_peak_bytes").is_none());
        assert!(claims(&store, Some(&head_prefix("other"))).is_empty());
        assert_eq!(fractal_schema::retention::policy().kind(KIND).class, fractal_schema::retention::Class::Durable);
    }

    #[test]
    fn unobserved_head_uses_a_nonempty_reserved_subject_component() {
        let store = Store::open_memory("host-a").unwrap();
        let mut execution = terminal(&store, "one", "cancelled");
        execution.result = None;
        store.update_shell_resource(&execution).unwrap();
        let page = claims(&store, Some("resource/exec/_/"));
        assert_eq!(page.len(), 1);
        assert!(page[0].body["fields"].get("head_start").is_none());
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
        store.start_resource_authority("host-a", "epoch").unwrap();
        let grant = match store.request_resource_membership(&parent.command.lane,
            super::super::resource_membership::Fit {
                room: parent.command.lane.capacity, ..Default::default()
            }, 10).unwrap() {
            super::super::resource_membership::Admission::Admitted(grant) => grant,
            other => panic!("expected admitted parent: {other:?}"),
        };
        let mut child_command = command("child");
        child_command.lane = parent.command.lane.clone();
        child_command.parent = Some(grant);
        let mut child = store.submit_shell_resource(child_command, 10).unwrap();
        child.state = "completed".into();
        store.update_shell_resource(&child).unwrap();
        store.update_shell_resource(&parent).unwrap();
        let page = claims(&store, None);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].body["fields"]["exec_id"], "parent");
    }
}
