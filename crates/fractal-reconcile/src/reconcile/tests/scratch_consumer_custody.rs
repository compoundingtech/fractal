//! Public producer/release regressions for durable native and shell directory custody.
use super::*;
use crate::store::resource_membership::{Admission, Coverage, Fit, LaneRequest, Owner, RequestClass};
use crate::store::scratch_resources::{OwnerObservation, ScratchHandle, ScratchReleaseProof, ScratchRequest};
use crate::store::shell_resources::Command;

struct Fixture {
    root: tempfile::TempDir,
    store: Store,
    handle: ScratchHandle,
    runtime: NativeRuntime,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("node").unwrap();
        apply_source(&store, r#"version 2
mission "scratch-custody" state="ready" {
 goal "Retain scratch while consumers hold it."
 step "work" { agentless }
}"#, "scratch-custody-definition");
        let run = store.create_mission_run(&MissionRunRequest {
            mission: "scratch-custody".into(), revision: None,
            workspace: root.path().display().to_string(), requester: Some("person/operator".into()),
            mode: None, inputs: BTreeMap::new(), idempotency_key: "scratch-custody".into(),
        }).unwrap();
        store.start_resource_authority("node", "epoch").unwrap();
        let handle = store.acquire_scratch(&ScratchRequest {
            id: "scratch/custody".into(), host: "node".into(), owner: run.subject,
            owner_generation: run.generation,
            root: root.path().canonicalize().unwrap().join("dedicated").display().to_string(),
        }).unwrap();
        let runtime = NativeRuntime::new(root.path(), None, Path::new("unused-pty"));
        Self { root, store, handle, runtime }
    }

    fn request(&self, disk_path: Option<String>) -> LaneRequest {
        LaneRequest {
            id: "native-consumer".into(), name: "checks".into(), host: "node".into(), max_members: 1,
            capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 256 << 20 }),
            class: RequestClass::Check, disk: None, io: None, coverage: Coverage::Logical,
            admission_timeout_ms: None, profile: None, profile_estimate: None, disk_path,
            owner: Owner { run: "person/operator".into(), step: "native-consumer".into(), attempt: 1, invocation: None },
        }
    }

    fn admit(&self, request: &LaneRequest) -> crate::store::resource_membership::Grant {
        let now = u64::try_from(now_ms()).unwrap();
        let Admission::Admitted(grant) = self.store.request_resource_membership(request,
            Fit { room: Some(fractal_runtime::Capacity { cores: 32, memory: 128 << 30 }), ..Fit::default() }, now).unwrap()
            else { panic!("expected logical admission"); };
        grant
    }

    fn member(&self, request: &LaneRequest, cwd: &str) -> MemberSpec {
        let intent = crate::graph::parse_test_intent(&format!(r#"version 2
exec "consumer" {{ command "/bin/sleep 60"; workspace "{}"; }}"#, self.root.path().display()), "consumer").unwrap();
        let mut member = intent.subjects.values().find_map(|subject| subject.member.clone()).unwrap();
        member.runtime_id = "scratch-consumer".into();
        member.cwd = cwd.into();
        member.launch = LaunchSpec::Argv(vec!["/bin/sleep".into(), "60".into()]);
        super::super::resource_lanes::set_request(&mut member, request).unwrap();
        member
    }

    fn release(&self) -> bool {
        self.store.release_scratch(&self.handle, &ScratchReleaseProof::after_quiescence(
            &self.handle.owner, &self.handle.owner_generation)).unwrap()
    }

    fn stop_processes(&self) -> Result<()> {
        for member in self.store.resource_memberships_for_host("node")? {
            for root in member.roots {
                self.runtime.kill(&root.runtime_id, false,
                    root.execution.as_ref().map(|execution| execution.generation_id.as_str()))?;
            }
        }
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.stop_processes();
    }
}

fn native_cwd(different_disk_path: bool) {
    // A positively observed unrelated cwd must not block the allocation forever.
    let outside = Fixture::new();
    let outside_request = outside.request(None);
    outside.admit(&outside_request);
    let outside_member = outside.member(&outside_request, outside.root.path().to_str().unwrap());
    outside.runtime.start_resource_guarded(&outside.store, &outside_member, None, &|| Ok(())).unwrap();
    let outside_released = outside.release();
    outside.stop_processes().unwrap();
    assert!(outside_released, "known outside native cwd is not a scratch consumer");
    let fixture = Fixture::new();
    let request = fixture.request(different_disk_path.then(|| fixture.root.path().display().to_string()));
    let grant = fixture.admit(&request);
    let member = fixture.member(&request, &fixture.handle.path);
    fixture.runtime.start_resource_guarded(&fixture.store, &member, None, &|| Ok(())).unwrap();
    assert!(!fixture.release(), "native cwd must retain scratch independently of lane diskPath");
    assert!(Path::new(&fixture.handle.path).is_dir());
    fixture.store.end_resource_execution(&grant, &member.runtime_id, u64::try_from(now_ms()).unwrap()).unwrap();
    assert!(!fixture.release(), "payload exit alone is not whole-tree retirement");
    fixture.stop_processes().unwrap();
    assert!(!fixture.release(), "logical-only process cleanup is not whole-tree retirement");
}

#[test]
fn live_native_cwd_without_disk_path_blocks_scratch_release() { native_cwd(false); }

#[test]
fn live_native_cwd_with_unrelated_disk_path_blocks_scratch_release() { native_cwd(true); }

#[test]
fn uncertain_native_preparation_without_disk_path_retains_scratch() {
    let fixture = Fixture::new();
    let request = fixture.request(None);
    let grant = fixture.admit(&request);
    fixture.store.prepare_resource_membership(&grant, "unobserved-preparation").unwrap();
    assert!(!fixture.release());
    assert_eq!(fixture.store.scratch_inventory("node", None, 10).unwrap()[0].consumer_observation,
        OwnerObservation::Unknown);
}

fn shell_alias(retarget: bool) {
    let fixture = Fixture::new();
    let alias = fixture.root.path().join("cwd-alias");
    std::os::unix::fs::symlink(&fixture.handle.path, &alias).unwrap();
    let request = fixture.request(None);
    let command = Command { id: "shell-consumer".into(), request_id: None, lane: request,
        argv: vec!["/bin/sleep".into(), "60".into()], cwd: alias.display().to_string(),
        environment: BTreeMap::new(), timeout_ms: None, parent: None };
    let mut execution = fixture.store.submit_shell_resource(command, 1).unwrap();
    // The real process keeps its kernel cwd after the pathname disappears. No native
    // membership is registered here: shell acceptance itself must retain its custody.
    let process = std::process::Command::new("/bin/sleep").arg("60").current_dir(&alias).spawn().unwrap();
    struct Child(std::process::Child);
    impl Drop for Child { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
    // Keep cleanup active even if the release assertion fails.
    let child = Child(process);
    execution.state = "running".into();
    fixture.store.update_shell_resource(&execution).unwrap();
    std::fs::remove_file(&alias).unwrap();
    if retarget { std::os::unix::fs::symlink(fixture.root.path(), &alias).unwrap(); }
    assert!(!fixture.release(), "a removed or retargeted shell cwd alias is not outside evidence");
    assert!(Path::new(&fixture.handle.path).is_dir());
    drop(child);
    execution.state = "completed".into();
    fixture.store.update_shell_resource(&execution).unwrap();
    assert!(fixture.release(), "terminal shell consumers no longer hold scratch");
}

#[test]
fn running_shell_removed_cwd_alias_retains_scratch() { shell_alias(false); }

#[test]
fn running_shell_retargeted_cwd_alias_retains_scratch() { shell_alias(true); }
