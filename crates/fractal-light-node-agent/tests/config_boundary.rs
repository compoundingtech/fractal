//! Real operator-file and executable boundaries. No native backend is launched here.

#[cfg(unix)]
mod common;
#[cfg(unix)]
mod unix {
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read as _, Write as _},
        os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _, symlink},
        path::{Path, PathBuf},
        process::{Command, Output},
    };

    use fractal_auth::{FleetAuth, MemberKey};
    use fractal_light_node_agent::config::{Config, prepare_state, private_file, private_state_file};
    use fractal_light_node_core::{
        config::{LightNodeGrant, LightNodeOperation, LightNodeScope, LightNodeSessionConfig},
        custody::{LocalSession, MemberLifecycle, SessionCustody},
    };
    use tempfile::TempDir;

    struct Fixture {
        root: TempDir,
        config: Config,
        config_path: PathBuf,
    }

    fn write_private(path: &Path, bytes: &[u8]) {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600)
            .open(path).unwrap();
        file.write_all(bytes).unwrap();
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn chmod(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn owned_session(subject: &str) -> LocalSession {
        LocalSession {
            subject: subject.into(),
            runtime_id: "explicit-native-runtime".into(),
            incarnation: "native-generation-1".into(),
            native_incarnation: "native-generation-1".into(),
            terminal: false,
            driver: None,
            lifecycle: MemberLifecycle::Service,
            custody: SessionCustody::Owned,
            start_operation: None,
            invocation: None,
        }
    }

    impl Fixture {
        fn new() -> Self {
            // Canonicalize the temporary root too: on some Unix hosts the system
            // temporary directory itself is reached through a symlink.
            let root = tempfile::tempdir().unwrap();
            let physical_root = root.path().canonicalize().unwrap();
            let (home_key, _) = MemberKey::generate().unwrap();
            let (host_key, document) = MemberKey::generate().unwrap();
            let native_state_dir = physical_root.join("native");
            let pty_root = physical_root.join("pty");
            fs::create_dir(&native_state_dir).unwrap();
            fs::create_dir(&pty_root).unwrap();
            let fleet_secret_file = physical_root.join("fleet.secret");
            let member_key_file = physical_root.join("member.key");
            let secret = vec![0x9a; 32];
            FleetAuth::from_secret_bytes("boundary-fleet", secret.clone()).unwrap();
            write_private(&fleet_secret_file, &secret);
            write_private(&member_key_file, &document);
            let config = Config {
                version: 1,
                state_dir: physical_root.join("state"),
                native_state_dir,
                pty_root,
                // Deliberately unavailable: every CLI case must finish before
                // native backend initialization or executable discovery.
                pty_binary: Some(physical_root.join("absent-pty")),
                fleet_id: "boundary-fleet".into(),
                fleet_secret_file,
                member_key_file,
                session: LightNodeSessionConfig::Agent {
                    home_endpoint: "ws://127.0.0.1:1/v1/light-node/session".into(),
                    grant: LightNodeGrant {
                        home: "boundary-home".into(),
                        home_key: home_key.public().into(),
                        host: "boundary-seat".into(),
                        host_key: host_key.public().into(),
                        epoch: 1,
                        grant: LightNodeScope {
                            subjects: vec!["agent/example/worker".into()],
                            operations: vec![LightNodeOperation::Observe, LightNodeOperation::Stop],
                        },
                    },
                },
                native_sessions: vec![owned_session("agent/example/worker")],
            };
            let config_path = physical_root.join("config.json");
            let fixture = Self { root, config, config_path };
            fixture.save();
            fixture
        }

        fn save(&self) {
            let bytes = serde_json::to_vec(&self.config).unwrap();
            if self.config_path.exists() {
                fs::write(&self.config_path, bytes).unwrap();
            } else {
                write_private(&self.config_path, &bytes);
            }
        }

        fn grant_mut(&mut self) -> &mut LightNodeGrant {
            let LightNodeSessionConfig::Agent { grant, .. } = &mut self.config.session else {
                panic!("fixture must have an agent grant");
            };
            grant
        }

        fn command(&self) -> Command {
            let mut command = Command::new(crate::common::agent_binary());
            command.env_clear().env("HOME", self.root.path()).current_dir(self.root.path());
            command
        }

        fn startup_failure(&self, description: &str) {
            self.save();
            let output = self.command().arg("--config").arg(&self.config_path).output().unwrap();
            assert_failure(&output, description);
            assert!(!self.config.socket_path().exists(), "rejected startup exposed a local API");
            assert_eq!(fs::read_dir(&self.config.pty_root).unwrap().count(), 0);
            assert_eq!(fs::read_dir(&self.config.native_state_dir).unwrap().count(), 0);
            if self.config.state_dir.exists() {
                for entry in fs::read_dir(&self.config.state_dir).unwrap() {
                    let name = entry.unwrap().file_name();
                    assert!(matches!(name.to_str(), Some("agent.lock" | "session.sqlite3" |
                        "session.sqlite3-wal" | "session.sqlite3-shm" | "session.sqlite3-journal")),
                        "failed startup created unexpected state: {name:?}");
                }
            }
        }
    }

    fn assert_failure(output: &Output, description: &str) {
        assert!(!output.status.success(), "invalid startup unexpectedly succeeded");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(description), "missing rejection {description:?}: {stderr}");
    }

    fn assert_start_unsupported(error: anyhow::Error) {
        let wire = error.downcast_ref::<fractal_wire::Error>()
            .expect("Start refusal must preserve the canonical wire error");
        assert_eq!(wire.code, "light-node-start-not-supported");
    }

    #[test]
    fn private_config_round_trips_canonical_grant_and_explicit_custody() {
        let fixture = Fixture::new();
        let config = Config::read(&fixture.config_path).unwrap();
        assert_eq!(serde_json::to_value(&config).unwrap(), serde_json::to_value(&fixture.config).unwrap());
        assert_eq!(config.socket_path(), config.state_dir.join("agent.sock"));
        assert!(!config.state_dir.exists(), "reading configuration must not initialize runtime state");
    }

    #[test]
    fn start_never_becomes_supported_through_config_or_json() {
        let mut fixture = Fixture::new();
        fixture.grant_mut().grant.operations.push(LightNodeOperation::Start);
        assert_start_unsupported(fixture.config.validate().unwrap_err());
        fixture.save();
        assert_start_unsupported(Config::read(&fixture.config_path).unwrap_err());
        fixture.startup_failure("light-node-start-not-supported");
        assert!(!fixture.config.state_dir.exists());
    }

    #[test]
    fn physical_binary_refuses_home_role() {
        let mut fixture = Fixture::new();
        let grant = fixture.grant_mut().clone();
        fixture.config.session = LightNodeSessionConfig::Home {
            listen: "127.0.0.1:0".parse().unwrap(), grants: vec![grant],
        };
        assert!(fixture.config.validate().is_err());
        fixture.save();
        assert!(Config::read(&fixture.config_path).is_err());
        fixture.startup_failure("accepts only an agent session");
        assert!(!fixture.config.state_dir.exists());
    }

    #[test]
    fn custody_requires_exact_subject_and_no_duplicate_provisioning() {
        let mut fixture = Fixture::new();
        fixture.config.native_sessions = vec![owned_session("agent/example/worker-other")];
        assert!(fixture.config.validate().is_err());
        fixture.save();
        assert!(Config::read(&fixture.config_path).is_err());
        fixture.config.native_sessions = vec![owned_session("agent/example/worker"); 2];
        assert!(fixture.config.validate().is_err());
        fixture.config.native_sessions = vec![owned_session("agent/example/worker")];
        fixture.grant_mut().grant.subjects = vec!["agent/example/*".into()];
        assert!(fixture.config.validate().is_err());
        fixture.grant_mut().grant.subjects = vec!["agent/example/worker".into(); 2];
        assert!(fixture.config.validate().is_err());
        fixture.grant_mut().grant.subjects = vec!["agent/example/worker".into()];
        fixture.config.validate().unwrap();
        assert!(fixture.config.session.validate_for("another-seat").is_err());
        fixture.config.session.validate_for("boundary-seat").unwrap();
    }

    #[test]
    fn admission_requires_distinct_well_formed_public_keys() {
        let mut fixture = Fixture::new();
        let original = fixture.grant_mut().clone();
        for malformed in ["", "not base64url!", "AQ", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="] {
            fixture.grant_mut().host_key = malformed.into();
            assert!(fixture.config.validate().is_err());
            fixture.grant_mut().host_key = original.host_key.clone();
            fixture.grant_mut().home_key = malformed.into();
            assert!(fixture.config.validate().is_err());
            fixture.grant_mut().home_key = original.home_key.clone();
        }
        fixture.grant_mut().host_key = original.home_key;
        assert!(fixture.config.validate().is_err());
    }

    #[test]
    fn operator_paths_version_and_fleet_identity_are_explicit_not_discovered() {
        let fixture = Fixture::new();
        let baseline = fixture.config.clone();
        let mut config = baseline.clone();
        config.version = 2;
        assert!(config.validate().is_err());
        for fleet_id in [String::new(), "x".repeat(256)] {
            config = baseline.clone();
            config.fleet_id = fleet_id;
            assert!(config.validate().is_err());
        }
        for field in [
            "state_dir", "native_state_dir", "pty_root", "fleet_secret_file",
            "member_key_file", "pty_binary",
        ] {
            let mut value = serde_json::to_value(&baseline).unwrap();
            value[field] = serde_json::json!("relative-operator-path");
            let config: Config = serde_json::from_value(value).unwrap();
            assert!(config.validate().is_err(), "relative {field} was admitted");
        }
        assert!(!baseline.state_dir.exists());
    }

    #[test]
    fn native_roots_are_explicit_existing_physical_and_not_publicly_writable() {
        let fixture = Fixture::new();
        for field in ["native_state_dir", "pty_root"] {
            let original = if field == "native_state_dir" {
                &fixture.config.native_state_dir
            } else {
                &fixture.config.pty_root
            };
            for permissions in [0o700, 0o755] {
                chmod(original, permissions);
                fixture.config.validate().unwrap();
                assert_eq!(mode(original), permissions);
            }
            for permissions in [0o770, 0o757, 0o777] {
                chmod(original, permissions);
                assert!(fixture.config.validate().is_err(), "publicly writable {field} was admitted");
                assert_eq!(mode(original), permissions, "validation must not repair existing native roots");
            }
            chmod(original, 0o755);
            let missing = fixture.root.path().join(format!("{field}-missing"));
            let regular = fixture.root.path().join(format!("{field}-file"));
            write_private(&regular, b"not a native directory");
            let link = fixture.root.path().join(format!("{field}-link"));
            symlink(original, &link).unwrap();
            for path in [&missing, &regular, &link] {
                let mut value = serde_json::to_value(&fixture.config).unwrap();
                value[field] = serde_json::to_value(path).unwrap();
                let config: Config = serde_json::from_value(value).unwrap();
                assert!(config.validate().is_err(), "nonphysical {field} was admitted");
                fs::write(&fixture.config_path, serde_json::to_vec(&config).unwrap()).unwrap();
                assert!(Config::read(&fixture.config_path).is_err());
            }
            assert!(!missing.exists());
            assert_eq!(fs::read(&regular).unwrap(), b"not a native directory");
            assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        }
        assert!(!fixture.config.state_dir.exists());
    }

    #[test]
    fn binary_rejects_nonprivate_config_before_creating_any_state() {
        for permissions in [0o640, 0o604] {
            let fixture = Fixture::new();
            chmod(&fixture.config_path, permissions);
            fixture.startup_failure("open private light-node configuration");
            assert_eq!(mode(&fixture.config_path), permissions);
            assert!(!fixture.config.state_dir.exists());
        }
    }

    #[test]
    fn private_config_rejects_group_world_access_and_final_symlink_without_repair() {
        let fixture = Fixture::new();
        for permissions in [0o640, 0o604, 0o660, 0o644] {
            chmod(&fixture.config_path, permissions);
            assert!(private_file(&fixture.config_path).is_err());
            assert!(Config::read(&fixture.config_path).is_err());
            assert_eq!(mode(&fixture.config_path), permissions);
        }
        chmod(&fixture.config_path, 0o600);
        let link = fixture.root.path().join("config-link.json");
        symlink(&fixture.config_path, &link).unwrap();
        assert!(private_file(&link).is_err());
        assert!(Config::read(&link).is_err());
        assert!(private_file(fixture.root.path()).is_err());
        Config::read(&fixture.config_path).unwrap();
    }

    #[test]
    fn config_json_is_bounded_and_rejects_unknown_or_trailing_content() {
        let fixture = Fixture::new();
        let mut oversized = File::create(&fixture.config_path).unwrap();
        oversized.set_len(1024 * 1024 + 1).unwrap();
        assert!(Config::read(&fixture.config_path).is_err());
        oversized.write_all(b"{}").unwrap();
        drop(oversized);
        let mut value = serde_json::to_value(&fixture.config).unwrap();
        value["graph_store"] = serde_json::json!("forbidden-fallback");
        fs::write(&fixture.config_path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Config::read(&fixture.config_path).is_err());
        let mut bytes = serde_json::to_vec(&fixture.config).unwrap();
        bytes.extend_from_slice(b" {}");
        fs::write(&fixture.config_path, bytes).unwrap();
        assert!(Config::read(&fixture.config_path).is_err());
    }

    #[test]
    fn state_directory_is_private_idempotent_and_never_repairs_unsafe_paths() {
        let fixture = Fixture::new();
        let state = &fixture.config.state_dir;
        prepare_state(state).unwrap();
        assert_eq!(mode(state), 0o700);
        prepare_state(state).unwrap();
        for permissions in [0o750, 0o755, 0o707, 0o600, 0o500] {
            chmod(state, permissions);
            assert!(prepare_state(state).is_err());
            assert_eq!(mode(state), permissions);
        }
        chmod(state, 0o700);
        let link = fixture.root.path().join("state-link");
        symlink(state, &link).unwrap();
        assert!(prepare_state(&link).is_err());
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        let file = fixture.root.path().join("state-file");
        write_private(&file, b"retained");
        assert!(prepare_state(&file).is_err());
        assert_eq!(fs::read(file).unwrap(), b"retained");
    }

    #[test]
    fn journal_and_lock_initialization_preserve_retained_bytes_and_deny_foreign_permissions() {
        let fixture = Fixture::new();
        prepare_state(&fixture.config.state_dir).unwrap();
        for name in ["session.sqlite3", "agent.lock"] {
            let path = fixture.config.state_dir.join(name);
            let mut first = private_state_file(&path).unwrap();
            assert_eq!(mode(&path), 0o600);
            first.write_all(b"retained journal or lock bytes").unwrap();
            drop(first);
            for _ in 0..2 {
                let mut file = private_state_file(&path).unwrap();
                let mut retained = Vec::new();
                file.read_to_end(&mut retained).unwrap();
                assert_eq!(retained, b"retained journal or lock bytes");
            }
            for permissions in [0o640, 0o644, 0o606] {
                chmod(&path, permissions);
                assert!(private_state_file(&path).is_err());
                assert_eq!(mode(&path), permissions);
                assert_eq!(fs::read(&path).unwrap(), b"retained journal or lock bytes");
            }
            chmod(&path, 0o600);
            let link = fixture.root.path().join(format!("{name}-link"));
            symlink(&path, &link).unwrap();
            assert!(private_state_file(&link).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"retained journal or lock bytes");
        }
        assert!(private_state_file(&fixture.config.state_dir).is_err());
    }

    #[test]
    fn help_needs_no_config_secret_pty_or_environment() {
        let fixture = Fixture::new();
        fs::remove_file(&fixture.config_path).unwrap();
        fs::remove_file(&fixture.config.fleet_secret_file).unwrap();
        fs::remove_file(&fixture.config.member_key_file).unwrap();
        for option in ["--help", "-h"] {
            let output = fixture.command().arg(option).output().unwrap();
            assert!(output.status.success(), "help must work without runtime prerequisites");
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(stdout.contains("--config"));
            assert!(stdout.contains("Start is unsupported"));
            assert!(output.stderr.is_empty());
        }
        assert!(!fixture.config.state_dir.exists());
    }

    #[test]
    fn cli_rejects_missing_duplicate_and_unexpected_args_without_config_fallback() {
        let fixture = Fixture::new();
        // A valid private config exists in cwd and HOME; argument rejection must
        // not discover it, create custody state, or attempt a native backend.
        for (args, description) in [
            (vec![], "Usage:"),
            (vec!["--config"], "--config requires"),
            (vec!["--unknown"], "Usage:"),
            (vec!["config.json"], "Usage:"),
            (vec!["--config", "config.json", "--config", "config.json"], "unexpected arguments"),
            (vec!["--config", "config.json", "extra"], "unexpected arguments"),
            (vec!["--help", "--config", "config.json"], "unexpected arguments"),
        ] {
            let output = fixture.command().args(args).output().unwrap();
            assert_failure(&output, description);
            assert!(!fixture.config.state_dir.exists());
        }
        let output = fixture.command().args(["--config", "missing.json"]).output().unwrap();
        assert_failure(&output, "open private light-node configuration");
        assert!(!fixture.config.state_dir.exists());
    }

    #[test]
    fn binary_rejects_owned_fifo_config_and_credentials_without_waiting_for_a_writer() {
        for input in ["config", "fleet-secret", "member-key"] {
            let fixture = Fixture::new();
            let path = match input {
                "config" => &fixture.config_path,
                "fleet-secret" => &fixture.config.fleet_secret_file,
                "member-key" => &fixture.config.member_key_file,
                _ => unreachable!(),
            };
            fs::remove_file(path).unwrap();
            assert!(Command::new("mkfifo").args(["-m", "600"]).arg(path).status().unwrap().success());
            let mut child = fixture.command().arg("--config").arg(&fixture.config_path)
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if child.try_wait().unwrap().is_some() { break; }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{input} FIFO blocked startup instead of rejecting its descriptor type");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert_failure(&child.wait_with_output().unwrap(), "private file must be regular");
            assert!(!fixture.config.socket_path().exists());
        }
    }

    #[test]
    fn binary_rejects_private_auth_permissions_symlinks_and_oversized_files_before_backend() {
        for member_key in [false, true] {
            for permissions in [0o640, 0o604] {
                let fixture = Fixture::new();
                let path = if member_key { &fixture.config.member_key_file } else { &fixture.config.fleet_secret_file };
                chmod(path, permissions);
                fixture.startup_failure("private file must be regular");
                assert_eq!(mode(path), permissions);
            }
            let mut fixture = Fixture::new();
            let target = if member_key { fixture.config.member_key_file.clone() } else { fixture.config.fleet_secret_file.clone() };
            let link = fixture.root.path().join("auth-link");
            symlink(&target, &link).unwrap();
            if member_key { fixture.config.member_key_file = link.clone(); } else { fixture.config.fleet_secret_file = link.clone(); }
            // O_NOFOLLOW returns an OS error; its display text is platform-specific.
            let error = private_file(&link).unwrap_err();
            assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().raw_os_error(),
                Some(libc::ELOOP));
            fixture.save();
            let output = fixture.command().arg("--config").arg(&fixture.config_path).output().unwrap();
            assert_failure(&output, &error.to_string());
            assert!(!fixture.config.socket_path().exists());
            assert_eq!(fs::read_dir(&fixture.config.pty_root).unwrap().count(), 0);
            assert_eq!(fs::read_dir(&fixture.config.native_state_dir).unwrap().count(), 0);

            let fixture = Fixture::new();
            let path = if member_key { &fixture.config.member_key_file } else { &fixture.config.fleet_secret_file };
            OpenOptions::new().write(true).open(path).unwrap().set_len(4097).unwrap();
            fixture.startup_failure(if member_key { "member key file exceeds capacity" } else { "fleet secret file exceeds capacity" });
        }
    }

    #[test]
    fn binary_refuses_a_valid_but_unadmitted_private_member_key() {
        let fixture = Fixture::new();
        let (_, foreign_document) = MemberKey::generate().unwrap();
        fs::write(&fixture.config.member_key_file, foreign_document).unwrap();
        fixture.startup_failure("member signing key differs from the explicit admitted host grant");
    }
}

#[cfg(not(unix))]
#[test]
fn private_operator_files_and_state_explicitly_refuse_unsupported_hosts() {
    use fractal_light_node_agent::config::{prepare_state, private_file};
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("config.json");
    std::fs::write(&file, b"{}").unwrap();
    assert!(private_file(&file).is_err());
    assert!(prepare_state(&directory.path().join("state")).is_err());
}
