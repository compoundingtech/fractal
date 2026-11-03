//! Launch-time evidence, independent of publication and restart policy.
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::model::DesiredSubject;
use crate::store::Store;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Contract {
    pub version: u8,
    pub digest: String,
    pub prompt: String,
    pub binary: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Drift {
    pub prompt: bool,
    pub binary: bool,
}

impl Contract {
    /// Unknown versions cannot establish either convergence or drift.
    pub fn drift(&self, running: &Self) -> Option<Drift> {
        (self.version == 1 && running.version == 1).then(|| Drift {
            prompt: self.prompt != running.prompt,
            binary: self.binary != running.binary,
        })
    }
}


/// Resolve prompt bytes with the same planner that materializes the declaration.
/// Only digests leave this boundary; prompts and tool configuration can contain secrets.
pub fn capture(
    store: &Store,
    desired: &DesiredSubject,
    binaries: &BTreeMap<String, String>,
) -> Result<Contract> {
    let member = desired.member.as_ref().context("launch contract needs a member")?;
    let workspace = Path::new(&member.workspace);
    let mut files = BTreeMap::new();
    if let Some(render) = desired.desired["children"].as_array()
        .and_then(|children| children.iter().find(|child| crate::render::name(child) == Some("render")))
    {
        let (writes, _) = crate::render::prepare_render(store, render, workspace)?;
        for write in writes.into_iter().filter(|write| !write.append_lines) {
            let path = write.destination.strip_prefix(workspace).unwrap_or(&write.destination);
            files.insert(path.to_string_lossy().into_owned(),
                (hex::encode(Sha256::digest(&write.bytes)), write.mode));
        }
    }
    let prompt = smallclaims::hash::canonical_hash(&(
        "launch-prompt-v1", &member.driver, crate::model::authored_launch(&member.launch), files,
    ))?;
    let binary = smallclaims::hash::canonical_hash(&("launch-binary-v1", binaries))?;
    let digest = smallclaims::hash::canonical_hash(&("launch-contract-v1", &prompt, &binary))?;
    Ok(Contract { version: 1, digest, prompt, binary })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn declaration(workspace: &Path) -> DesiredSubject {
        crate::graph::parse_test_intent(&format!(
            "version 2\nagent \"worker\" {{ workspace {:?}; harness \"omp\" {{ model \"model-a\"; args \"--tools\" \"read,write\"; }} }}",
            workspace.to_str().unwrap()), "node").unwrap().subjects.into_values().next().unwrap()
    }

    fn render(desired: &mut DesiredSubject, source: &Path, destination: &str) {
        desired.desired["children"].as_array_mut().unwrap().push(json!({
            "name":"render", "arguments":[], "properties":{}, "children":[{
                "name":"copy", "arguments":[source.to_str().unwrap(), destination],
                "properties":{}, "children":[]
            }]
        }));
    }

    #[test]
    fn every_declared_input_changes_only_its_component() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("node").unwrap();
        let desired = declaration(root.path());
        let binaries = BTreeMap::from([
            ("launcher".into(), "binary-a".into()), ("provider".into(), "provider-a".into()),
        ]);
        let old = capture(&store, &desired, &binaries).unwrap();
        assert_eq!(old, capture(&store, &desired, &binaries).unwrap());
        for (from, to) in [("model-a", "model-b"), ("read,write", "read")] {
            let mut changed = desired.clone();
            let crate::model::LaunchSpec::Argv(argv) = &mut changed.member.as_mut().unwrap().launch else { panic!() };
            *argv.iter_mut().find(|arg| *arg == from).unwrap() = to.into();
            let new = capture(&store, &changed, &binaries).unwrap();
            assert_ne!(new.digest, old.digest);
            assert_eq!(new.drift(&old), Some(Drift { prompt: true, binary: false }));
        }
        let mut changed = desired.clone();
        changed.member.as_mut().unwrap().driver = Some("pi".into());
        assert_ne!(old.digest, capture(&store, &changed, &binaries).unwrap().digest);
        for role in ["launcher", "provider"] {
            let mut changed = binaries.clone();
            changed.insert(role.into(), "binary-b".into());
            let new = capture(&store, &desired, &changed).unwrap();
            assert_ne!(new.digest, old.digest);
            assert_eq!(new.drift(&old), Some(Drift { prompt: false, binary: true }));
        }
    }

    #[test]
    fn persona_context_and_tool_file_bytes_are_not_reconstructed_from_paths() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("node").unwrap();
        for destination in ["PERSONA.md", "AGENTS.md", "tools.json"] {
            let source = root.path().join("source");
            std::fs::write(&source, "before").unwrap();
            let mut desired = declaration(root.path());
            render(&mut desired, &source, destination);
            let old = capture(&store, &desired, &BTreeMap::new()).unwrap();
            std::fs::write(&source, "after").unwrap();
            let new = capture(&store, &desired, &BTreeMap::new()).unwrap();
            assert_ne!(new.digest, old.digest);
            assert_eq!(new.drift(&old), Some(Drift { prompt: true, binary: false }));
            assert_eq!(new.drift(&new), Some(Drift { prompt: false, binary: false }));
        }
    }

    #[test]
    fn executable_content_changes_without_a_path_change() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("provider");
        std::fs::write(&executable, "before").unwrap();
        let old = fractal_drivers::harness_admission::executable_identity(&executable).unwrap();
        std::fs::write(&executable, "after").unwrap();
        assert_ne!(old, fractal_drivers::harness_admission::executable_identity(&executable).unwrap());
    }
}
