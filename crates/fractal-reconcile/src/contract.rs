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
    let preparation = store.extension::<Preparation>();
    let member = match preparation.0.get() {
        Some(prepare) => prepare(desired)?,
        None => desired.member.clone().context("launch contract needs a member")?,
    };
    capture(store, desired, &member, false)
}

pub(crate) fn capture(store: &Store, desired: &DesiredSubject, member: &MemberSpec, running: bool) -> Result<Contract> {
    let executable = crate::reconcile::launch_executable()?;
    let mut environment = crate::reconcile::member_environment(&member.environment, &executable, None)?;
    if let Some(path) = environment.get_mut("PATH") {
        *path = std::env::join_paths(std::env::split_paths(path).map(|entry|
            if entry.is_absolute() { entry } else { Path::new(&member.cwd).join(entry) }))?.to_string_lossy().into_owned();
    }
    let resolve = |program: &str| {
        let mut program = program.to_owned();
        fractal_runtime::expand_path_placeholder(&mut program, &environment);
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
    let binaries = BTreeMap::from([("launcher", executable), ("provider", resolve(provider)?)])
        .into_iter().map(|(role, path)| Ok((role.to_owned(),
            fractal_drivers::harness_admission::executable_identity_with(&path, &resolve)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    crate::launch_contract::capture_effective(store, desired, &binaries, &environment, running)
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
    match (declared(store, desired), running) {
        (Ok(declared), Ok(running)) => {
            let running = running.flatten();
            let drift = running.as_ref().and_then(|running| declared.drift(running));
            serde_json::json!({"declared":declared,"running":running,"drift":drift})
        }
        (declared, running) => serde_json::json!({
            "declared":declared.as_ref().ok(), "running":running.as_ref().ok().and_then(Option::as_ref).and_then(Option::as_ref),
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
        prepared.environment.insert("PATH".into(), root.path().join("overlay").display().to_string());
        prepared.environment.insert("CLAUDE_CONFIG_DIR".into(), root.path().join("overlay").display().to_string());
        let expected = capture(&store, &desired, &prepared, true).unwrap();
        install(&store, Box::new(move |_| Ok(prepared.clone())));
        assert_ne!(original.binary, expected.binary);
        assert_eq!(declared(&store, &desired).unwrap(), expected);
        std::fs::write(root.path().join("overlay/CLAUDE.md"), "effective account context").unwrap();
        assert_ne!(declared(&store, &desired).unwrap().prompt, expected.prompt);
    }
}
