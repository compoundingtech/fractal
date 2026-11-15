//! Resolve managed harnesses before admission, resume helpers, or provider startup.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

// Runtime-profile contract owned by the wrapper publisher.
const WRAPPER_DIRECTORY_KEY: &str = "harnessBin";

// A driver owns one seat. Keep launch-only environment separate from the driver's environment.
struct ProviderEnvironment {
    path: std::ffi::OsString,
    subject: String,
}
static PROVIDER_ENVIRONMENT: std::sync::OnceLock<ProviderEnvironment> = std::sync::OnceLock::new();

/// Select the managed executable, including when argv names a raw provider by absolute path.
/// Other harnesses retain their own launch contracts.
pub fn resolve(harness: &str, argv: &mut [String], spec: &serde_json::Value, subject: &str) -> Result<()> {
    if is_person_bound(spec)? {
        return Ok(());
    }
    ensure!(subject.starts_with("agent/"), "managed harness launch requires a fenced agent subject");
    let path = resolve_from_source(harness, argv, spec, &|name| std::env::var_os(name), &|_| Ok(()))?;
    if let Some(existing) = PROVIDER_ENVIRONMENT.get() {
        ensure!(existing.path == path && existing.subject == subject,
            "driver already selected a different managed provider environment");
    } else {
        PROVIDER_ENVIRONMENT.set(ProviderEnvironment { path, subject: subject.into() })
            .map_err(|_| anyhow::anyhow!("managed provider environment already selected"))?;
    }
    Ok(())
}

/// Resolve a daemon seat using the effective captured launch environment, without global state.
/// Register profile and wrapper inputs before reading or validating them, including on failure.
pub fn resolve_with_environment(
    harness: &str,
    argv: &mut [String],
    spec: &serde_json::Value,
    environment: &std::collections::BTreeMap<String, String>,
    input: &dyn Fn(&Path) -> Result<()>,
) -> Result<std::ffi::OsString> {
    resolve_from_source(harness, argv, spec, &|name| environment.get(name).map(std::ffi::OsString::from), input)
}

/// Person-owned terminal bindings retain their native harness launch contract.
/// Callers supply the declaration read through the driver's authenticated client.
pub fn is_person_bound(spec: &serde_json::Value) -> Result<bool> {
    let Some(binding) = spec.get("children").and_then(serde_json::Value::as_array)
        .and_then(|nodes| nodes.iter().find(|node| node["name"] == "bind-terminal")) else {
        return Ok(false);
    };
    let subject = binding.get("arguments").and_then(serde_json::Value::as_array)
        .and_then(|arguments| arguments.first()).and_then(serde_json::Value::as_str)
        .context("terminal binding is missing its subject")?;
    let owner = fractal_schema::owned_terminals::owner(subject)
        .map_err(|error| anyhow::anyhow!("{}", error.message))?;
    Ok(owner.is_some_and(|owner| owner.starts_with("person/")))
}

fn resolve_from_source(
    harness: &str,
    argv: &mut [String],
    spec: &serde_json::Value,
    env: &impl Fn(&str) -> Option<std::ffi::OsString>,
    input: &dyn Fn(&Path) -> Result<()>,
) -> Result<std::ffi::OsString> {
    ensure!(matches!(harness, "omp" | "claude" | "codex"), "unsupported managed harness {harness}");
    if is_person_bound(spec)? {
        return Ok(env("PATH").unwrap_or_default());
    }
    let profile = profile_path_from(env)?;
    let environment_persona = env("AGENT_PERSONA")
        .map(|value| value.into_string().map_err(|_| anyhow::anyhow!("AGENT_PERSONA is not UTF-8")))
        .transpose()?;
    let persona = environment_persona.as_deref().unwrap_or("generalist");
    resolve_for_persona(harness, argv, &profile, persona, input)?;
    let bin = Path::new(&argv[0]).parent().context("managed wrapper has no parent")?;
    prefixed_path(bin, env("PATH").as_deref())
}

fn profile_path_from(env: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<PathBuf> {
    if let Some(profile) = env("AGENT_RUNTIME_PROFILE") {
        ensure!(!profile.is_empty(), "AGENT_RUNTIME_PROFILE is empty; refusing raw provider");
        let profile = PathBuf::from(profile);
        ensure!(profile.is_absolute(), "AGENT_RUNTIME_PROFILE must be absolute: {}", profile.display());
        return Ok(profile);
    }
    let home = env("HOME").context("HOME is required to discover the managed runtime profile")?;
    let home = PathBuf::from(home);
    ensure!(home.is_absolute(), "HOME must be absolute to discover the managed runtime profile");
    Ok(home.join(".local/state/nix/profiles/coding-agent-runtime/share/coding-agents/profile.json"))
}

fn resolve_for_persona(
    harness: &str,
    argv: &mut [String],
    profile: &Path,
    persona: &str,
    input: &dyn Fn(&Path) -> Result<()>,
) -> Result<()> {
    let program = argv.first_mut().context("managed provider argv is empty")?;
    input(profile)?;
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(profile)
            .with_context(|| format!("read managed runtime profile {}", profile.display()))?,
    ).with_context(|| format!("parse managed runtime profile {}", profile.display()))?;
    let bin = manifest.get("personas")
        .and_then(|personas| personas.get("launch"))
        .and_then(|launch| launch.get(persona))
        .and_then(|launch| launch.get(WRAPPER_DIRECTORY_KEY))
        .and_then(serde_json::Value::as_str)
        .filter(|bin| !bin.is_empty())
        .map(Path::new)
        .with_context(|| format!(
            "managed runtime profile {} is missing personas.launch.{persona}.{WRAPPER_DIRECTORY_KEY}; refusing raw {harness}",
            profile.display(),
        ))?;
    ensure!(bin.is_absolute(), "managed wrapper directory must be absolute: {}", bin.display());
    let wrapper = bin.join(harness);
    input(&wrapper)?;
    let metadata = std::fs::metadata(&wrapper)
        .with_context(|| format!("managed {harness} wrapper is missing: {}; refusing raw provider", wrapper.display()))?;
    ensure!(metadata.is_file(), "managed {harness} wrapper is not a file: {}", wrapper.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        ensure!(metadata.permissions().mode() & 0o111 != 0,
            "managed {harness} wrapper is not executable: {}", wrapper.display());
    }
    // An absolute executable gives the same wrapper precedence as the kit's PATH without
    // changing the driver's environment or letting an explicit raw argv bypass the wrapper.
    *program = wrapper.to_str().context("managed wrapper path is not UTF-8")?.to_owned();
    Ok(())
}

/// Apply the managed launch environment only to a provider child.
pub fn apply_provider_environment(command: &mut std::process::Command) -> std::io::Result<()> {
    if let Some(environment) = PROVIDER_ENVIRONMENT.get() {
        apply_agent_identity(command, &environment.subject, &|name| std::env::var_os(name))?;
        apply_path(command, Some(&environment.path));
    }
    Ok(())
}

fn apply_agent_identity(
    command: &mut std::process::Command,
    subject: &str,
    inherited: &impl Fn(&str) -> Option<std::ffi::OsString>,
) -> std::io::Result<()> {
    for name in ["ST_AGENT", "FRACTAL_AGENT"] {
        let value = command.get_envs().find(|(key, _)| *key == std::ffi::OsStr::new(name))
            .map(|(_, value)| value.map(std::ffi::OsStr::to_owned))
            .unwrap_or_else(|| inherited(name));
        if let Some(value) = value
            && value != std::ffi::OsStr::new(subject) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
                format!("managed provider {name}={value:?} does not match fenced subject {subject}; refusing launch")));
        }
    }
    command.env("ST_AGENT", subject).env("FRACTAL_AGENT", subject);
    Ok(())
}

fn apply_path(command: &mut std::process::Command, path: Option<&std::ffi::OsStr>) {
    if let Some(path) = path {
        command.env("PATH", path);
    }
}

pub fn provider_command(program: impl AsRef<std::ffi::OsStr>) -> std::io::Result<std::process::Command> {
    let mut command = std::process::Command::new(program);
    apply_provider_environment(&mut command)?;
    Ok(command)
}

fn prefixed_path(bin: &Path, inherited: Option<&std::ffi::OsStr>) -> Result<std::ffi::OsString> {
    let mut paths = vec![bin.to_path_buf()];
    if let Some(inherited) = inherited {
        paths.extend(std::env::split_paths(inherited).filter(|path| path.as_path() != bin));
    }
    std::env::join_paths(paths).context("prepend managed harness directory to provider PATH")
}


#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("managed/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let profile = root.path().join("profile.json");
        std::fs::write(&profile, serde_json::to_vec(&serde_json::json!({
            "personas": {"launch": {"generalist": {(WRAPPER_DIRECTORY_KEY): bin}}},
        })).unwrap()).unwrap();
        (root, bin, profile)
    }

    #[test]
    fn managed_wrapper_takes_precedence_over_login_path_and_raw_argv() {
        let (_root, bin, profile) = fixture();
        for harness in ["omp", "claude", "codex"] {
            let wrapper = bin.join(harness);
            std::fs::write(&wrapper, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            for program in [harness.to_owned(), format!("/raw/bin/{harness}")] {
                let mut argv = vec![program, "--model".into(), "example".into()];
                resolve_for_persona(harness, &mut argv, &profile, "generalist", &|_| Ok(())).unwrap();
                assert_eq!(Path::new(&argv[0]), wrapper);
                assert_eq!(&argv[1..], ["--model", "example"]);
            }
        }
    }

    #[test]
    fn missing_wrapper_refuses_instead_of_searching_path() {
        let (_root, bin, profile) = fixture();
        for program in ["omp", "/raw/bin/omp"] {
            let mut argv = vec![program.to_owned()];
            let error = resolve_for_persona("omp", &mut argv, &profile, "generalist", &|_| Ok(())).unwrap_err();
            assert!(format!("{error:#}").contains(bin.join("omp").to_str().unwrap()));
            assert_eq!(argv[0], program);
        }
    }

    #[test]
    fn missing_wrapper_directory_refuses() {
        let (_root, bin, profile) = fixture();
        std::fs::remove_dir(&bin).unwrap();
        let error = resolve_for_persona("omp", &mut ["omp".into()], &profile, "generalist", &|_| Ok(())).unwrap_err();
        assert!(format!("{error:#}").contains(bin.join("omp").to_str().unwrap()));
    }

    #[test]
    fn missing_profile_key_refuses_raw_provider() {
        let (_root, _bin, profile) = fixture();
        std::fs::write(&profile, "{}").unwrap();
        let error = resolve_for_persona("omp", &mut ["omp".into()], &profile, "generalist", &|_| Ok(())).unwrap_err();
        assert!(format!("{error:#}").contains(WRAPPER_DIRECTORY_KEY));
    }

    #[test]
    fn missing_persona_does_not_fall_back_to_generalist() {
        let (_root, _bin, profile) = fixture();
        let error = resolve_for_persona("omp", &mut ["omp".into()], &profile, "unknown", &|_| Ok(())).unwrap_err();
        assert!(format!("{error:#}").contains("personas.launch.unknown.harnessBin"));
    }

    #[test]
    fn profile_discovery_prefers_explicit_path_and_keeps_stable_link() {
        let explicit = profile_path_from(&|name| match name {
            "AGENT_RUNTIME_PROFILE" => Some("/custom/profile.json".into()),
            "HOME" => Some("/home/example".into()),
            _ => None,
        }).unwrap();
        assert_eq!(explicit, Path::new("/custom/profile.json"));
        let stable = profile_path_from(&|name| (name == "HOME").then(|| "/home/example".into())).unwrap();
        assert_eq!(stable, Path::new(
            "/home/example/.local/state/nix/profiles/coding-agent-runtime/share/coding-agents/profile.json",
        ));
        assert!(profile_path_from(&|_| None).is_err());
        assert!(profile_path_from(&|name| (name == "AGENT_RUNTIME_PROFILE").then(|| "".into())).is_err());
    }

    #[test]
    fn environment_steward_selects_its_wrapper_without_a_spec_persona() {
        let (_root, bin, profile) = fixture();
        std::fs::write(&profile, serde_json::to_vec(&serde_json::json!({
            "personas": {"launch": {"steward": {"harnessBin": bin}}},
        })).unwrap()).unwrap();
        let wrapper = bin.join("omp");
        std::fs::write(&wrapper, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let environment = std::collections::BTreeMap::from([
            ("AGENT_RUNTIME_PROFILE".into(), profile.display().to_string()),
            ("AGENT_PERSONA".into(), "steward".into()),
        ]);
        let mut argv = ["omp".into()];
        resolve_with_environment("omp", &mut argv, &serde_json::json!({}), &environment, &|_| Ok(())).unwrap();
        assert_eq!(Path::new(&argv[0]), wrapper);
    }

    #[test]
    fn provider_path_starts_with_managed_bin_without_changing_driver_path() {
        let before = std::env::var_os("PATH");
        let path = prefixed_path(Path::new("/managed/bin"), Some(std::ffi::OsStr::new("/native/bin:/managed/bin:/usr/bin:/managed/bin"))).unwrap();
        let mut command = std::process::Command::new("/managed/bin/omp");
        apply_path(&mut command, Some(&path));
        let provider_path = command.get_envs().find(|(name, _)| *name == std::ffi::OsStr::new("PATH"))
            .and_then(|(_, value)| value).unwrap();
        assert_eq!(std::env::split_paths(provider_path).collect::<Vec<_>>(),
            [PathBuf::from("/managed/bin"), PathBuf::from("/native/bin"), PathBuf::from("/usr/bin")]);
        assert_eq!(std::env::var_os("PATH"), before);
    }

    #[test]
    fn explicit_relative_profile_is_refused() {
        let error = profile_path_from(&|name| (name == "AGENT_RUNTIME_PROFILE")
            .then(|| "relative/profile.json".into())).unwrap_err();
        assert!(format!("{error:#}").contains("AGENT_RUNTIME_PROFILE must be absolute"));
    }

    #[test]
    fn malformed_profile_and_relative_wrapper_directory_refuse() {
        let (_root, _bin, profile) = fixture();
        std::fs::write(&profile, "{").unwrap();
        let error = resolve_for_persona("omp", &mut ["omp".into()], &profile, "generalist", &|_| Ok(())).unwrap_err();
        assert!(format!("{error:#}").contains("parse managed runtime profile"));
        std::fs::write(&profile, r#"{"personas":{"launch":{"generalist":{"harnessBin":"relative/bin"}}}}"#).unwrap();
        let error = resolve_for_persona("omp", &mut ["omp".into()], &profile, "generalist", &|_| Ok(())).unwrap_err();
        assert!(format!("{error:#}").contains("managed wrapper directory must be absolute"));
    }

    #[cfg(unix)]
    #[test]
    fn profile_link_is_read_again_for_each_launch_and_broken_link_refuses() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let (root, first_bin, first_profile) = fixture();
        let second_bin = root.path().join("second/bin");
        std::fs::create_dir_all(&second_bin).unwrap();
        for bin in [&first_bin, &second_bin] {
            let wrapper = bin.join("codex");
            std::fs::write(&wrapper, "#!/bin/sh\nprintf managed-helper\n").unwrap();
            std::fs::set_permissions(wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let second_profile = root.path().join("second-profile.json");
        std::fs::write(&second_profile, serde_json::to_vec(&serde_json::json!({
            "personas": {"launch": {"generalist": {"harnessBin": second_bin}}},
        })).unwrap()).unwrap();
        let link = root.path().join("current-profile.json");
        let environment = std::collections::BTreeMap::from([
            ("AGENT_RUNTIME_PROFILE".into(), link.display().to_string()),
            ("PATH".into(), "/missing/native/bin".into()),
        ]);
        for (profile, bin) in [(&first_profile, &first_bin), (&second_profile, &second_bin)] {
            symlink(profile, &link).unwrap();
            let mut argv = ["codex".into(), "--version".into()];
            let path = resolve_with_environment("codex", &mut argv, &serde_json::json!({}), &environment, &|_| Ok(())).unwrap();
            assert_eq!(Path::new(&argv[0]), bin.join("codex"));
            assert_eq!(std::env::split_paths(&path).next().unwrap(), *bin);
            let output = std::process::Command::new(&argv[0]).arg(&argv[1]).env("PATH", path).output().unwrap();
            assert_eq!(output.stdout, b"managed-helper");
            std::fs::remove_file(&link).unwrap();
        }
        symlink(root.path().join("missing-profile.json"), &link).unwrap();
        let error = resolve_with_environment("codex", &mut ["codex".into()], &serde_json::json!({}), &environment, &|_| Ok(())).unwrap_err();
        assert!(format!("{error:#}").contains("read managed runtime profile"));
    }

    #[test]
    fn person_bound_driver_keeps_native_argv_without_a_managed_profile() {
        let spec = serde_json::json!({"children": [{
            "name": "bind-terminal",
            "arguments": ["pty/person/avery/019a0000-0000-7000-8000-000000000001"],
        }]});
        let environment = std::collections::BTreeMap::from([("PATH".into(), "/native/bin".into())]);
        for harness in ["omp", "claude", "codex"] {
            let mut argv = [format!("/native/{harness}")];
            let path = resolve_with_environment(harness, &mut argv, &spec, &environment, &|_| Ok(())).unwrap();
            assert_eq!(path, std::ffi::OsStr::new("/native/bin"));
            assert_eq!(argv[0], format!("/native/{harness}"));
        }
        let agent_binding = serde_json::json!({"children": [{
            "name": "bind-terminal",
            "arguments": ["pty/agent/worker/019a0000-0000-7000-8000-000000000001"],
        }]});
        assert!(!is_person_bound(&agent_binding).unwrap());
        assert!(resolve_with_environment("omp", &mut ["omp".into()], &agent_binding, &environment, &|_| Ok(())).is_err());
    }

    #[test]
    fn provider_identity_refuses_mismatches_and_sets_matching_aliases() {
        let subject = "agent/worker";
        for (st, fractal) in [
            (Some("agent/wrong"), None),
            (None, Some("agent/wrong")),
            (Some("agent/wrong"), Some("agent/wrong")),
            (Some(subject), Some("agent/other")),
            (Some(""), None),
        ] {
            let mut command = std::process::Command::new("omp");
            let error = apply_agent_identity(&mut command, subject, &|name| {
                match name { "ST_AGENT" => st, "FRACTAL_AGENT" => fractal, _ => None }
                    .map(std::ffi::OsString::from)
            }).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains("does not match fenced subject"));
        }
        for inherited in [None, Some(subject)] {
            let mut command = std::process::Command::new("omp");
            apply_agent_identity(&mut command, subject, &|_| inherited.map(Into::into)).unwrap();
            let values = command.get_envs().collect::<std::collections::BTreeMap<_, _>>();
            for name in ["ST_AGENT", "FRACTAL_AGENT"] {
                assert_eq!(values[std::ffi::OsStr::new(name)], Some(std::ffi::OsStr::new(subject)));
            }
        }
        let mut overridden = std::process::Command::new("omp");
        overridden.env("ST_AGENT", "agent/wrong");
        assert!(apply_agent_identity(&mut overridden, subject, &|_| Some(subject.into())).is_err());
    }
}
