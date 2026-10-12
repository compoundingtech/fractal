//! Publish launch facts and only the lease held by this execution.
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, ensure};
use crate::model::StepRunView;
use super::generated::*;
pub use super::Runtime;

#[derive(Clone, Debug)]
pub struct Facts {
    pub agent: String,
    pub principal: String,
    pub incarnation: String,
    pub harness_tool: String,
    pub harness_version: Option<String>,
    pub model_route: Option<String>,
    pub model_effort: Option<String>,
    pub session: Option<String>,
    pub host: Option<String>,
    pub worktree: Option<String>,
    pub mission_name: Option<String>,
}

/// Try to serialize lifecycle read/modify/write transactions without waiting.
///
/// # Errors
/// Returns filesystem or advisory-lock failures, including contention.
pub fn lock(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false)
        .read(true).write(true).mode(0o600)
        .open(path.with_extension("lock"))?;
    lock.try_lock()?;
    Ok(lock)
}

fn now_ms() -> Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
}

/// Build a runtime document from observed launch facts, without inventing a model.
/// Unrepresentable model facts are diagnosed and omitted.
///
/// # Errors
/// Rejects non-model facts that the authoritative schema cannot represent and invalid lease references.
pub fn build(facts: &Facts, lease: Option<&StepRunView>) -> Result<Runtime> {
    let mut runtime = Runtime {
        r#type: "agent-trace-runtime/v1".to_owned().try_into()?,
        who: AgentTraceRuntimeV1Who {
            agent: facts.agent.clone().try_into()?,
            principal: facts.principal.clone().try_into()?,
            actor: None,
            persona: None,
            supervisor: None,
        },
        why: None,
        how: AgentTraceRuntimeV1How {
            harness: AgentTraceRuntimeV1HowHarness {
                tool: facts.harness_tool.clone().try_into()?,
                version: facts.harness_version.clone().map(TryInto::try_into).transpose()?,
            },
            model: None,
            tooling_profile: None,
            runtime_profile: None,
            skills_manifest: None,
            invocation_id: None,
            issuance: None,
        },
        r#where: AgentTraceRuntimeV1Where {
            session: facts.session.clone().map(TryInto::try_into).transpose()?,
            host: facts.host.clone().map(TryInto::try_into).transpose()?,
            worktree: facts.worktree.clone().map(TryInto::try_into).transpose()?,
        },
    };
    if let Some(route) = &facts.model_route {
        update_model(&mut runtime, route, facts.model_effort.as_deref());
    }
    update_lease(&mut runtime, &facts.agent, &facts.incarnation, lease, facts.mission_name.as_deref(), now_ms()?)?;
    Ok(runtime)
}

/// Lease references use the actual mission run and step generation, not claim IDs.
/// A completed, expired, foreign, or previous-incarnation claim is idle context.
///
/// # Errors
/// Rejects a foreign trace or malformed references on a held lease.
pub fn update_lease(
    runtime: &mut Runtime,
    actor: &str,
    incarnation: &str,
    lease: Option<&StepRunView>,
    mission_name: Option<&str>,
    now_ms: u128,
) -> Result<()> {
    ensure!(runtime.who.agent.as_str() == actor, "trace belongs to another agent");
    let lease = lease.filter(|step| {
        matches!(step.status.as_str(), "claimed" | "working" | "verifying" | "blocked")
            && step.claimant.as_deref() == Some(actor)
            && step.claim_incarnation.as_deref() == Some(incarnation)
            && step.claim_expires_at_unix_ms.is_some_and(|expires| expires > now_ms)
    });
    let Some(step) = lease else {
        runtime.why = None;
        runtime.how.issuance = None;
        return Ok(());
    };
    let mission_run = &step.run;
    let step_run = step.generation.strip_prefix("run-generation/").context("lease has no step generation reference")?;
    let references = (|| -> Result<_> {
        Ok((
            AgentTraceRuntimeV1WhyMissionRun::try_from(mission_run.to_owned())?,
            AgentTraceRuntimeV1WhyStepRun::try_from(step_run.to_owned())?,
            AgentTraceRuntimeV1HowIssuanceVariant0StepRun::try_from(step.subject.clone())?,
        ))
    })();
    let (mission_run, step_run, issued_step_run) = match references {
        Ok(references) => references,
        Err(error) => {
            tracing::warn!(
                event = "agent_trace_lease_unrepresentable",
                subject = %step.subject, run = %step.run, generation = %step.generation,
                %error, "runtime authority cannot represent lease references"
            );
            runtime.why = None;
            runtime.how.issuance = None;
            return Ok(());
        }
    };
    let mission = AgentTraceRuntimeV1WhyMission {
        name: mission_name.context("claimed lease requires its mission name")?.to_owned().try_into()?,
        run: Some(mission_run),
    };
    let why_step = AgentTraceRuntimeV1WhyStep {
        name: step.step.clone().try_into()?,
        run: Some(step_run),
    };
    let issuance = AgentTraceRuntimeV1HowIssuance::Variant0(AgentTraceRuntimeV1HowIssuanceVariant0 {
        r#_tag: "st-issued".to_owned().try_into()?,
        step_run: issued_step_run,
    });
    runtime.why = Some(AgentTraceRuntimeV1Why { mission: Some(mission), step: Some(why_step) });
    runtime.how.issuance = Some(issuance);
    Ok(())
}

/// Decode with the authoritative schema, preserving every launch and model fact.
///
/// # Errors
/// Propagates locking, reading, authoritative decoding, lease validation, and publication failures.
pub fn refresh(
    path: &Path,
    actor: &str,
    incarnation: &str,
    lease: Option<&StepRunView>,
    mission_name: Option<&str>,
) -> Result<Runtime> {
    let _lock = lock(path)?;
    let mut runtime: Runtime = serde_json::from_slice(&std::fs::read(path)?)?;
    update_lease(&mut runtime, actor, incarnation, lease, mission_name, now_ms()?)?;
    write_atomic(path, &runtime)?;
    Ok(runtime)
}

/// Keep reduced seat ownership authoritative; use only an explicitly declared
/// host trace principal when that reduction yields no human.
pub fn resolve_principal(reduced: Option<String>, declared: Option<&str>) -> Option<String> {
    if reduced.is_some() {
        return reduced;
    }
    let declared = declared?;
    if !declared.strip_prefix("person/").is_some_and(|name| !name.is_empty()) {
        tracing::warn!(
            event = "agent_trace_fallback_principal_invalid",
            "ignoring declared trace principal that is not person/<name>"
        );
        return None;
    }
    match AgentTraceRuntimeV1WhoPrincipal::validate(declared) {
        Ok(()) => Some(declared.to_owned()),
        Err(error) => {
            tracing::warn!(
                event = "agent_trace_fallback_principal_invalid", %error,
                "ignoring declared trace principal outside the runtime authority"
            );
            None
        }
    }
}

/// Replace the human principal from reduced ownership or reviewed host declaration.
///
/// # Errors
/// Rejects a principal that the authoritative schema cannot represent.
pub fn update_principal(runtime: &mut Runtime, principal: String) -> Result<()> {
    runtime.who.principal = principal.try_into()?;
    Ok(())
}

/// Replace model facts only when an actual selected route is known.
///
/// A route or effort outside the authoritative schema clears the previous model
/// and emits a structured diagnostic; trace metadata must not block execution.
pub fn update_model(runtime: &mut Runtime, route: &str, effort: Option<&str>) {
    let model = (|| -> Result<_> {
        Ok(AgentTraceRuntimeV1HowModel {
            route: route.to_owned().try_into()?,
            effort: effort.map(str::to_owned).map(TryInto::try_into).transpose()?,
        })
    })();
    runtime.how.model = match model {
        Ok(model) => Some(model),
        Err(error) => {
            tracing::warn!(
                event = "agent_trace_model_unrepresentable",
                agent = %runtime.who.agent.as_str(), route, effort,
                %error, "runtime authority cannot represent model facts"
            );
            None
        }
    };
}

/// Same-directory exclusive temporary creation and rename keep readers atomic.
/// The file is private even when replacing a predecessor with broader permissions.
///
/// # Errors
/// Propagates serialization, temporary-file, permission, sync, and rename failures.
pub fn write_atomic(path: &Path, runtime: &Runtime) -> Result<()> {
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new().prefix(".agent-trace-").tempfile_in(parent)?;
    temporary.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut temporary, runtime)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declared_principal_is_only_a_validated_fallback_for_missing_reduction() {
        for declared in [None, Some("person/fallback"), Some("agent/not-a-human")] {
            assert_eq!(resolve_principal(Some("person/reduced".into()), declared),
                Some("person/reduced".into()));
        }
        assert_eq!(resolve_principal(None, Some("person/fallback")), Some("person/fallback".into()));
        assert_eq!(resolve_principal(None, None), None);
        for invalid in ["agent/not-a-human", "person/",
            " person/fallback", "person/fallback\n", "person/fallback "] {
            assert_eq!(resolve_principal(None, Some(invalid)), None, "{invalid:?}");
        }
    }

    const MISSION: &str = "0123456789abcdef0123456789abcdef";
    const GENERATION: &str = "abcdef0123456789abcdef0123456789";

    fn facts() -> Facts {
        Facts {
            agent: "agent/example".into(), incarnation: "execution-one".into(),
            principal: "person/example".into(),
            harness_tool: "omp".into(), harness_version: Some("1.0.0".into()),
            model_route: Some("openai/gpt-5".into()), model_effort: Some("high".into()),
            session: Some("demo.one".into()), host: Some("test-host".into()),
            worktree: Some("/workspace/example".into()), mission_name: Some("example-mission".into()),
        }
    }

    fn lease() -> StepRunView {
        serde_json::from_value(json!({
            "subject":format!("step-run/{GENERATION}/work"), "run":format!("mission-run/example/repo/example-mission/{MISSION}"),
            "generation":format!("run-generation/{GENERATION}"), "step":"work", "definition_hash":"definition",
            "status":"claimed", "attempt":1, "assigned_to":"agent/example", "agentless":false,
            "title":null, "worker_reported":false, "claimant":"agent/example", "claim_incarnation":"execution-one",
            "claim_expires_at_unix_ms":u64::MAX, "readiness_epoch":1, "blocked_reason":null,
            "not_before_unix_ms":null, "created_at_unix_ms":1, "updated_at_unix_ms":2
        })).unwrap()
    }

    #[test]
    fn idle_omits_why_issuance_and_unknown_model() {
        let mut facts = facts();
        facts.model_route = None;
        let value = serde_json::to_value(build(&facts, None).unwrap()).unwrap();
        assert!(value.get("why").is_none());
        assert!(value["how"].get("issuance").is_none());
        assert!(value["how"].get("model").is_none());
    }

    #[test]
    fn claimed_lease_maps_actual_references() {
        let value = serde_json::to_value(build(&facts(), Some(&lease())).unwrap()).unwrap();
        assert_eq!(value["why"]["mission"], json!({"name":"example-mission", "run":format!("mission-run/example/repo/example-mission/{MISSION}")}));
        assert_eq!(value["why"]["step"], json!({"name":"work", "run":GENERATION}));
        assert_eq!(value["how"]["issuance"], json!({"_tag":"st-issued", "step_run":format!("step-run/{GENERATION}/work")}));
    }

    #[test]
    fn expired_foreign_previous_and_completed_claims_are_idle() {
        let facts = facts();
        for case in 0..4 {
            let mut step = lease();
            match case {
                0 => step.claim_expires_at_unix_ms = Some(10),
                1 => step.claimant = Some("agent/other".into()),
                2 => step.claim_incarnation = Some("execution-previous".into()),
                _ => step.status = "completed".into(),
            }
            let mut runtime = build(&facts, None).unwrap();
            update_lease(&mut runtime, &facts.agent, &facts.incarnation, Some(&step), Some("example-mission"), 10).unwrap();
            assert!(runtime.why.is_none());
            assert!(runtime.how.issuance.is_none());
        }
    }

    #[test]
    fn atomic_private_roundtrip_and_claim_complete_refresh() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.json");
        let facts = facts();
        let initial = build(&facts, None).unwrap();
        write_atomic(&path, &initial).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let decoded: Runtime = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded, initial);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let claimed = refresh(&path, &facts.agent, &facts.incarnation, Some(&lease()), Some("example-mission")).unwrap();
        assert!(claimed.why.is_some());
        assert_eq!(claimed.how.model, initial.how.model);
        assert_eq!(claimed.how.harness, initial.how.harness);
        assert_eq!(claimed.r#where, initial.r#where);
        let completed = refresh(&path, &facts.agent, &facts.incarnation, None, None).unwrap();
        assert_eq!(completed, initial);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let files: std::collections::BTreeSet<_> = std::fs::read_dir(root.path())
            .unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(files, ["runtime.json", "runtime.lock"].map(std::ffi::OsString::from).into());
    }

    #[test]
    fn ad_hoc_lease_preserves_actual_run_generation_and_subject() {
        let facts = facts();
        let mut step = lease();
        step.run = format!("mission-run/work/{MISSION}");
        step.generation = format!("run-generation/work-{GENERATION}");
        step.subject = format!("step-run/work-{GENERATION}/work");
        let runtime = build(&facts, Some(&step)).unwrap();
        let value = serde_json::to_value(&runtime).unwrap();
        assert_eq!(value["why"]["mission"]["run"], step.run);
        assert_eq!(value["why"]["step"]["run"], format!("work-{GENERATION}"));
        assert_eq!(value["how"]["issuance"]["step_run"], step.subject);
        let decoded: Runtime = serde_json::from_slice(&serde_json::to_vec(&runtime).unwrap()).unwrap();
        assert_eq!(decoded, runtime);
    }

    #[test]
    fn unrepresentable_lease_omits_purpose_instead_of_fabricating_references() {
        let mut step = lease();
        step.generation = "run-generation/not-a-generation".into();
        let runtime = build(&facts(), Some(&step)).unwrap();
        assert!(runtime.why.is_none());
        assert!(runtime.how.issuance.is_none());
    }

    #[test]
    fn refresh_rejects_foreign_agent_without_replacing_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.json");
        let initial = build(&facts(), None).unwrap();
        write_atomic(&path, &initial).unwrap();
        assert!(refresh(&path, "agent/other", "execution-one", Some(&lease()), Some("example-mission")).is_err());
        let decoded: Runtime = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(decoded, initial);
    }

    #[test]
    fn logical_and_dotted_identities_keep_human_principal() {
        for (subject, expected) in [
            ("agent/example/worker", "agent/example/worker"),
            ("agent/demo.worker", "agent/demo.worker"),
            ("agent/host-a.ci-watcher", "agent/host-a.ci-watcher"),
        ] {
            let mut facts = facts();
            facts.agent = subject.into();
            let mut runtime = build(&facts, None).unwrap();
            assert_eq!(runtime.who.agent.as_str(), expected);
            assert_eq!(runtime.who.principal.as_str(), "person/example");
            assert!(runtime.who.actor.is_none());
            let mut step = lease();
            step.claimant = Some(subject.into());
            update_lease(&mut runtime, subject, &facts.incarnation, Some(&step), Some("example-mission"), 10).unwrap();
            assert!(runtime.how.issuance.is_some());
        }
    }

    #[test]
    fn model_update_preserves_lease_and_launch_facts() {
        let mut runtime = build(&facts(), Some(&lease())).unwrap();
        let initial = runtime.clone();
        update_model(&mut runtime, "anthropic/claude-sonnet-4", None);
        assert_eq!(runtime.why, initial.why);
        assert_eq!(runtime.how.issuance, initial.how.issuance);
        assert_eq!(runtime.how.harness, initial.how.harness);
        assert_eq!(runtime.who, initial.who);
        assert_eq!(runtime.r#where, initial.r#where);
        let model = runtime.how.model.as_ref().unwrap();
        assert_eq!(model.route.as_str(), "anthropic/claude-sonnet-4");
        assert!(model.effort.is_none());
    }

    #[test]
    fn unrepresentable_model_build_omits_model_and_preserves_launch_and_lease() {
        let initial = build(&facts(), Some(&lease())).unwrap();
        for (route, effort) in [
            ("provider/abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz0123456789", None),
            ("invalid provider/model", None),
            ("", None),
            ("anthropic/claude-sonnet-4", Some("invalid-effort")),
        ] {
            let mut facts = facts();
            facts.model_route = Some(route.into());
            facts.model_effort = effort.map(str::to_owned);
            let runtime = build(&facts, Some(&lease())).unwrap();
            let mut expected = initial.clone();
            expected.how.model = None;
            assert_eq!(runtime, expected, "{route} {effort:?}");
            let value = serde_json::to_value(&runtime).unwrap();
            assert!(value["how"].get("model").is_none());
            assert_eq!(serde_json::from_value::<Runtime>(value).unwrap(), runtime);
        }
    }

    #[test]
    fn unrepresentable_model_update_clears_stale_model_and_preserves_other_facts() {
        let initial = build(&facts(), Some(&lease())).unwrap();
        for (route, effort) in [
            ("provider/abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz0123456789", None),
            ("invalid provider/model", None),
            ("", None),
            ("anthropic/claude-sonnet-4", Some("invalid-effort")),
        ] {
            let mut runtime = initial.clone();
            update_model(&mut runtime, route, effort);
            let mut expected = initial.clone();
            expected.how.model = None;
            assert_eq!(runtime, expected, "{route} {effort:?}");
            update_model(&mut runtime, "anthropic/claude-sonnet-4", Some("high"));
            let model = runtime.how.model.as_ref().unwrap();
            assert_eq!(model.route.as_str(), "anthropic/claude-sonnet-4");
            assert_eq!(model.effort.as_ref().unwrap().as_str(), "high");
        }
    }

    #[test]
    fn namespaced_model_route_preserves_selected_route() {
        for route in [
            "openrouter/anthropic/claude-sonnet-4",
            "openrouter/Anthropic/Claude-Sonnet:latest",
        ] {
            let mut facts = facts();
            facts.model_route = Some(route.into());
            let mut runtime = build(&facts, Some(&lease())).unwrap();
            assert_eq!(runtime.how.model.as_ref().unwrap().route.as_str(), route);
            update_model(&mut runtime, route, None);
            let model = runtime.how.model.as_ref().unwrap();
            assert_eq!(model.route.as_str(), route);
            assert!(model.effort.is_none());
            let value = serde_json::to_value(&runtime).unwrap();
            assert_eq!(value["how"]["model"]["route"], route);
            assert_eq!(serde_json::from_value::<Runtime>(value).unwrap(), runtime);
        }
    }

    #[test]
    fn unrepresentable_seat_identity_rejects_build() {
        let mut facts = facts();
        facts.agent = "agent/demo.invalid seat".into();
        let error = build(&facts, None).unwrap_err();
        assert!(error.to_string().contains("AgentTraceRuntimeV1WhoAgent"));
    }

    #[test]
    fn contended_sidecar_lock_returns_without_waiting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.json");
        let guard = lock(&path).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let contender = std::thread::spawn(move || {
            sender.send(lock(&path).is_err()).unwrap();
        });
        assert!(receiver.recv_timeout(std::time::Duration::from_secs(1)).unwrap());
        contender.join().unwrap();
        drop(guard);
    }

    #[test]
    fn invalid_typed_refresh_keeps_predecessor_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.json");
        let mut value = serde_json::to_value(build(&facts(), None).unwrap()).unwrap();
        value["how"]["model"]["route"] = json!("unknown");
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(refresh(&path, "agent/example", "execution-one", Some(&lease()), Some("example-mission")).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn all_held_work_statuses_keep_issuance() {
        for status in ["claimed", "working", "verifying", "blocked"] {
            let mut step = lease();
            step.status = status.into();
            assert!(build(&facts(), Some(&step)).unwrap().how.issuance.is_some());
        }
    }
}
