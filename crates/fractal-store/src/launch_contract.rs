//! Launch-time evidence, independent of publication and restart policy.
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Component, Path, PathBuf};
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

/// Capture the next declaration, using its member environment.
pub fn capture(store: &Store, desired: &DesiredSubject, binaries: &BTreeMap<String, String>) -> Result<Contract> {
    let member = desired.member.as_ref().context("launch contract needs a member")?;
    capture_effective(store, desired, binaries, &member.environment, false)
}

/// Only digests leave this boundary. Running evidence reads installed destinations, never sources.
pub fn capture_effective(
    store: &Store,
    desired: &DesiredSubject,
    binaries: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
    running: bool,
) -> Result<Contract> {
    let member = desired.member.as_ref().context("launch contract needs a member")?;
    let workspace = Path::new(&member.workspace);
    let launch = match &member.launch {
        crate::model::LaunchSpec::Shell(source) => vec![source.as_str()],
        crate::model::LaunchSpec::Argv(argv) => argv.iter().enumerate().map(|(i, arg)| {
            // The wrapper executable belongs to binary identity, not prompt identity.
            if i == 0 && argv.get(1).is_some_and(|arg| arg == "driver") { "driver-wrapper" } else { arg }
        }).collect(),
    };
    let normalize = |path: PathBuf| path.components().filter(|c| *c != Component::CurDir).collect::<PathBuf>();
    let mut files = BTreeMap::new();
    let mut paths = BTreeSet::new();
    if let Some(render) = crate::render::children(&desired.desired).iter()
        .find(|child| crate::render::name(child) == Some("render"))
    {
        if running {
            for operation in crate::render::children(render) {
                let index = match crate::render::name(operation) {
                    Some("copy") => 1,
                    Some("file" | "json-upsert" | "ensure-line") => 0,
                    Some("git-exclude") => continue,
                    _ => anyhow::bail!("unknown render operation"),
                };
                let destination = operation["arguments"][index].as_str().context("render destination is not text")?;
                paths.insert(normalize(workspace.join(destination)));
            }
        } else {
            let (writes, _) = crate::render::prepare_render(store, render, workspace)?;
            for write in writes.into_iter().filter(|write| !write.append_lines) {
                context_imports(&write.destination, &write.bytes, &mut paths)?;
                files.insert(normalize(write.destination), (hex::encode(Sha256::digest(&write.bytes)), write.mode));
            }
        }
    }
    // Native instruction conventions: https://github.com/can1357/oh-my-pi/blob/main/docs/context-files.md
    // Include workspace and ancestor instructions even when no render operation owns them.
    for ancestor in workspace.ancestors().chain(Path::new(&member.cwd).ancestors()) {
        for name in ["AGENTS.md", "CLAUDE.md", "PERSONA.md", ".omp/AGENTS.md", ".omp/RULES.md",
            ".claude/CLAUDE.md", ".omp/config.yml", ".omp/config.yaml", ".omp/mcp.json",
            ".mcp.json", ".claude/settings.json", ".claude/settings.local.json", ".codex/config.toml"] {
            let path = normalize(ancestor.join(name));
            if path.is_file() { paths.insert(path); }
        }
    }
    for (variable, names) in [("HOME", &[".claude/CLAUDE.md", ".omp/AGENTS.md", ".omp/config.yml", ".codex/config.toml"][..]),
        ("CLAUDE_CONFIG_DIR", &["CLAUDE.md", "settings.json"]), ("CODEX_HOME", &["AGENTS.md", "config.toml"]),
        ("PI_CODING_AGENT_DIR", &["AGENTS.md", "settings.json", "mcp.json"])] {
        if let Some(base) = environment.get(variable) {
            for name in names {
                let path = normalize(Path::new(base).join(name));
                if path.is_file() { paths.insert(path); }
            }
        }
    }
    let resolve = |value: &str| {
        if let Some(relative) = value.strip_prefix("~/") {
            environment.get("HOME").map(|home| Path::new(home).join(relative))
                .context("HOME is required for a launch input path")
        } else { Ok(workspace.join(value)) }
    };
    // These native options accept files (Claude also accepts inline JSON).
    for (i, argument) in launch.iter().enumerate() {
        let (flag, value) = argument.split_once('=').unwrap_or((argument, launch.get(i + 1).copied().unwrap_or("")));
        if matches!(flag, "--settings" | "--mcp-config" | "--config" | "--system-prompt-file" | "--append-system-prompt-file")
            && !value.is_empty() && !value.starts_with('{')
        {
            paths.insert(normalize(resolve(value)?));
        }
    }
    if let Some(overlays) = environment.get("PI_CONFIG_FILES") {
        for path in std::env::split_paths(overlays) { paths.insert(normalize(workspace.join(path))); }
    }
    while let Some(path) = paths.pop_first() {
        if let std::collections::btree_map::Entry::Vacant(entry) = files.entry(path) {
            let path = entry.key();
            let bytes = std::fs::read(path).with_context(|| format!("read launch input {}", path.display()))?;
            context_imports(path, &bytes, &mut paths)?;
            let mode = std::fs::metadata(path)?.permissions().mode() & 0o7777;
            entry.insert((hex::encode(Sha256::digest(bytes)), mode));
        }
    }
    let files: BTreeMap<_, _> = files.into_iter().map(|(path, value)| {
        (path.strip_prefix(workspace).unwrap_or(&path).to_string_lossy().into_owned(), value)
    }).collect();
    let prompt = smallclaims::hash::canonical_hash(&("launch-prompt-v1", &member.driver, launch, files))?;
    let binary = smallclaims::hash::canonical_hash(&("launch-binary-v1", binaries))?;
    let digest = smallclaims::hash::canonical_hash(&("launch-contract-v1", &prompt, &binary))?;
    Ok(Contract { version: 1, digest, prompt, binary })
}

fn context_imports(path: &Path, bytes: &[u8], paths: &mut BTreeSet<PathBuf>) -> Result<()> {
    if path.extension().is_some_and(|extension| extension == "md")
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        for import in text.split_whitespace().filter_map(|word| word.strip_prefix('@')) {
            let imported = path.parent().unwrap_or(Path::new(".")).join(import);
            if imported.is_file() { paths.insert(std::fs::canonicalize(imported)?); }
        }
    }
    Ok(())
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
    fn authored_settings_and_channels_without_model_are_retained() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("node").unwrap();
        let mut desired = declaration(root.path());
        desired.member.as_mut().unwrap().launch = crate::model::LaunchSpec::Argv(vec![
            "/launch/one".into(), "driver".into(), "claude".into(), "--".into(),
            "claude".into(), "--settings".into(), "{}".into(), "--channels".into(), "first".into(),
        ]);
        let old = capture(&store, &desired, &BTreeMap::new()).unwrap();
        for (index, value) in [(6, "{\"tools\":[]}"), (8, "second")] {
            let mut changed = desired.clone();
            let crate::model::LaunchSpec::Argv(argv) = &mut changed.member.as_mut().unwrap().launch else { panic!() };
            argv[index] = value.into();
            assert_ne!(old.prompt, capture(&store, &changed, &BTreeMap::new()).unwrap().prompt);
        }
        let crate::model::LaunchSpec::Argv(argv) = &mut desired.member.as_mut().unwrap().launch else { panic!() };
        argv[0] = "/launch/two".into();
        assert_eq!(old, capture(&store, &desired, &BTreeMap::new()).unwrap());
    }

    #[test]
    fn unrendered_ancestor_context_and_declared_tools_change_prompt() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let store = Store::open_memory("node").unwrap();
        let mut desired = declaration(&workspace);
        let crate::model::LaunchSpec::Argv(argv) = &mut desired.member.as_mut().unwrap().launch else { panic!() };
        argv.extend(["--mcp-config".into(), "tools.json".into()]);
        std::fs::write(workspace.join("tools.json"), "{}").unwrap();
        for path in [root.path().join("AGENTS.md"), workspace.join("CLAUDE.md"), workspace.join("tools.json")] {
            std::fs::write(&path, "before").unwrap();
            let old = capture(&store, &desired, &BTreeMap::new()).unwrap();
            std::fs::write(&path, "after").unwrap();
            assert_ne!(old.prompt, capture(&store, &desired, &BTreeMap::new()).unwrap().prompt);
        }
    }

    #[test]
    fn queued_source_changes_do_not_replace_installed_launch_evidence() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_memory("node").unwrap();
        let source = root.path().join("source");
        std::fs::write(&source, "A").unwrap();
        let mut desired = declaration(root.path());
        render(&mut desired, &source, "./AGENTS.md");
        crate::render::atomic_write_mode(&root.path().join("AGENTS.md"), b"A", 0o644).unwrap();
        let old = capture(&store, &desired, &BTreeMap::new()).unwrap();
        std::fs::write(&source, "B").unwrap();
        let next = capture(&store, &desired, &BTreeMap::new()).unwrap();
        assert_ne!(old.prompt, next.prompt);
        std::fs::remove_file(&source).unwrap();
        assert_eq!(old, capture_effective(&store, &desired, &BTreeMap::new(), &BTreeMap::new(), true).unwrap());
        crate::render::atomic_write_mode(&root.path().join("AGENTS.md"), b"B", 0o644).unwrap();
        assert_eq!(next, capture_effective(&store, &desired, &BTreeMap::new(), &BTreeMap::new(), true).unwrap());
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
