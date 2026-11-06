//! Host-local launch identity; publication never changes incarnation evidence.
use std::{collections::BTreeMap, path::Path, sync::OnceLock};

use anyhow::{Context as _, Result};
use crate::launch_contract::Contract;
use crate::model::{DesiredSubject, LaunchSpec, MemberSpec};
use crate::store::Store;

type Prepare = Box<dyn Fn(&DesiredSubject) -> Result<MemberSpec> + Send + Sync>;
#[derive(Default)]
struct Preparation(OnceLock<Prepare>);
pub(crate) fn install(store: &Store, prepare: Prepare) {
    let _ = store.extension::<Preparation>().0.set(prepare);
}

pub fn declared(store: &Store, desired: &DesiredSubject) -> Result<Contract> {
    let member = prepared(store, desired)?;
    capture(store, desired, &member, false)
}

/// The declared contract if every executable it names is already measured, or `None` while one
/// is measured in the background. A read never streams an executable's bytes (#5209).
pub fn declared_ready(store: &Store, desired: &DesiredSubject) -> Result<Option<Contract>> {
    declared_ready_inputs(store, desired, &|_| Ok(()))
}

pub(crate) fn declared_ready_inputs(store: &Store, desired: &DesiredSubject, input: &dyn Fn(&Path) -> Result<()>) -> Result<Option<Contract>> {
    let member = prepared(store, desired)?;
    let ready = Measures {
        environment: &|declared, executable| crate::reconcile::member_environment_ready(declared, executable, None),
        identity: &|path, resolve, input| fractal_drivers::harness_admission::executable_identity_ready_observed(path, resolve, input),
    };
    capture_with(store, desired, &member, false, &ready, input)
}

fn prepared(store: &Store, desired: &DesiredSubject) -> Result<MemberSpec> {
    match store.extension::<Preparation>().0.get() {
        Some(prepare) => prepare(desired),
        None => desired.member.clone().context("launch contract needs a member"),
    }
}

/// How a capture reads the member's environment and executable identities: waiting for them, or
/// `None` while one is still measured off the caller.
struct Measures<'a> {
    environment: &'a EnvironmentMeasure,
    identity: &'a IdentityMeasure,
}

type EnvironmentMeasure = dyn Fn(&BTreeMap<String, String>, &Path) -> Result<Option<BTreeMap<String, String>>>;
type IdentityMeasure = dyn Fn(&Path, &dyn Fn(&str) -> Result<std::path::PathBuf>, &dyn Fn(&Path) -> Result<()>) -> Result<Option<String>>;

pub(crate) fn capture(store: &Store, desired: &DesiredSubject, member: &MemberSpec, running: bool) -> Result<Contract> {
    let blocking = Measures {
        environment: &|declared, executable| crate::reconcile::member_environment(declared, executable, None).map(Some),
        identity: &|path, resolve, _| fractal_drivers::harness_admission::executable_identity_with(path, resolve).map(Some),
    };
    capture_with(store, desired, member, running, &blocking, &|_| Ok(()))?
        .context("a blocking measurement always answers")
}

fn capture_with(store: &Store, desired: &DesiredSubject, member: &MemberSpec, running: bool, measures: &Measures<'_>, input: &dyn Fn(&Path) -> Result<()>) -> Result<Option<Contract>> {
    let executable = crate::reconcile::launch_executable()?;
    let Some(mut environment) = (measures.environment)(&member.environment, &executable)? else {
        return Ok(None);
    };
    if let Some(path) = environment.get_mut("PATH") {
        *path = std::env::join_paths(std::env::split_paths(path).map(|entry|
            if entry.is_absolute() { entry } else { Path::new(&member.cwd).join(entry) }))?.to_string_lossy().into_owned();
    }
    let resolve = |program: &str| {
        let mut program = program.to_owned();
        fractal_runtime::expand_path_placeholder(&mut program, &environment);
        if Path::new(&program).components().count() == 1 {
            for directory in std::env::split_paths(environment.get("PATH").map(String::as_str).unwrap_or("")) {
                input(&directory.join(&program))?;
            }
        } else {
            input(&Path::new(&member.cwd).join(&program))?;
        }
        let path = fractal_runtime::resolve_executable(&program, &environment)?;
        Ok(if path.is_absolute() { path } else { Path::new(&member.cwd).join(path) })
    };
    let authored = desired.member.as_ref().context("launch contract needs a member")?;
    let provider = match &authored.launch {
        LaunchSpec::Shell(_) => "sh",
        LaunchSpec::Argv(argv) => {
            let position = if authored.driver.is_some() {
                argv.iter().position(|arg| arg == "--").context("harness launch has no provider")? + 1
            } else { 0 };
            argv.get(position).context("launch has no program")?
        }
    };
    let mut binaries = BTreeMap::new();
    for (role, path) in [("launcher", executable), ("provider", resolve(provider)?)] {
        // Start every pending measurement before answering, so one later read finds them all.
        binaries.insert(role.to_owned(), (measures.identity)(&path, &resolve, input)?);
    }
    let Some(binaries) = binaries.into_iter().map(|(role, identity)| identity.map(|identity| (role, identity)))
        .collect::<Option<BTreeMap<_, _>>>()
    else {
        return Ok(None);
    };
    crate::launch_contract::capture_effective_observed(store, desired, &binaries, &environment, running, input)
        .map(|(contract, _)| Some(contract))
}

/// Running identity is exclusively the receipt for the observed incarnation, never its token's
/// current files. Old receipts without a contract remain unknown until an actual replacement.
pub fn running(store: &Store, subject: &str, incarnation: &str) -> Result<Option<Contract>> {
    Ok(store.observations_for(subject, "runtime.action.succeeded")?
        .into_iter().rev().find_map(|claim| {
            let fields = &claim.body["fields"];
            (fields["action"] == "start" && fields["incarnation_id"].as_str() == Some(incarnation))
                .then(|| fields.get("launch_contract").cloned())
                .flatten()
        }).map(serde_json::from_value).transpose()?)
}

/// A local diagnostic, not a snapshot assertion about files on another host.
pub fn status(store: &Store, desired: &DesiredSubject, incarnation: Option<&str>) -> serde_json::Value {
    let running = incarnation.map(|id| running(store, &desired.subject, id)).transpose();
    match (declared_ready(store, desired), running) {
        (Ok(Some(declared)), Ok(running)) => {
            let running = running.flatten();
            let drift = running.as_ref().and_then(|running| declared.drift(running));
            serde_json::json!({"declared":declared,"running":running,"drift":drift})
        }
        // An executable is still being measured; a later read reports the declared contract.
        (Ok(None), Ok(running)) => serde_json::json!({"declared":null,"running":running.flatten(),"drift":null,"pending":true}),
        (declared, running) => serde_json::json!({
            "declared":declared.as_ref().ok().and_then(Option::as_ref), "running":running.as_ref().ok().and_then(Option::as_ref).and_then(Option::as_ref),
            "drift":null, "error":declared.err().or_else(|| running.err()).map(|error| error.to_string())
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;
    #[test]
    fn relative_provider_and_runtime_path_use_the_prepared_member() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("node").unwrap();
        let mut desired = crate::graph::parse_test_intent(&format!("version 2\nagent \"worker\" {{ workspace {:?}; harness \"omp\" {{ model \"model-a\"; }} }}", root.path().to_str().unwrap()), "node").unwrap().subjects.into_values().next().unwrap();
        for directory in ["bin", "overlay"] {
            std::fs::create_dir(root.path().join(directory)).unwrap();
            std::fs::write(root.path().join(directory).join("node"), directory).unwrap();
        }
        std::fs::write(root.path().join("bin/provider"), "#!/usr/bin/env node\n").unwrap();
        for path in ["bin/node", "overlay/node", "bin/provider"] {
            std::fs::set_permissions(root.path().join(path), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let member = desired.member.as_mut().unwrap();
        member.cwd = root.path().display().to_string();
        member.driver = None;
        member.launch = LaunchSpec::Argv(vec!["./bin/provider".into()]);
        member.environment.insert("PATH".into(), root.path().join("bin").display().to_string());
        let mut prepared = member.clone();
        let original = declared(&store, &desired).unwrap();
        prepared.environment.insert("PATH".into(), std::env::join_paths([root.path().join("missing"), root.path().join("overlay")]).unwrap().to_string_lossy().into_owned());
        prepared.environment.insert("CLAUDE_CONFIG_DIR".into(), root.path().join("overlay").display().to_string());
        let expected = capture(&store, &desired, &prepared, true).unwrap();
        install(&store, Box::new(move |_| Ok(prepared.clone())));
        assert_ne!(original.binary, expected.binary);
        assert_eq!(declared(&store, &desired).unwrap(), expected);
        let seen = std::cell::RefCell::new(Vec::new());
        declared_ready_inputs(&store, &desired, &|path| {
            seen.borrow_mut().push(path.to_owned()); Ok(())
        }).unwrap();
        assert!(seen.borrow().contains(&root.path().join("missing/node")), "shadowing executable creation must invalidate");
        assert!(seen.borrow().contains(&root.path().join("overlay/node")));
        std::fs::write(root.path().join("overlay/CLAUDE.md"), "effective account context").unwrap();
        assert_ne!(declared(&store, &desired).unwrap().prompt, expected.prompt);
    }
}
