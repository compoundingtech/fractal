use super::*;

#[test]
fn seed_recovery_uses_agent_control_cli_and_requires_reason() {
    let cli = Cli::try_parse_from([
        "st", "agents", "acknowledge-seed", "example",
        "--reason", "accept fresh", "--as", "person/operator",
    ]).unwrap();
    let Command::Agents { command: AgentsCommand::AcknowledgeSeed(args) } = cli.command else {
        panic!("agent seed acknowledgement expected");
    };
    assert_eq!(args.subject, "example");
    assert_eq!(args.reason, "accept fresh");
    assert_eq!(args.actor.as_deref(), Some("person/operator"));
    assert!(Cli::try_parse_from(["st", "agents", "acknowledge-seed", "example"]).is_err());
    assert!(Cli::try_parse_from(["st", "agents", "acknowledge-seed", "example", "--reason", ""]).is_err());
}

#[test]
fn rejects_effective_resume_environment_before_receipt() {
    const CHILD: &str = "ST3_TEST_NATIVE_SEED_CONFLICT";
    if let Some(variable) = std::env::var_os(CHILD) {
        let root = PathBuf::from(std::env::var_os("ST3_TEST_NATIVE_SEED_ROOT").unwrap());
        let client = Client::new(Endpoint::Unix(root.join("no-daemon.sock")));
        let args = DriverArgs {
            driver: "omp".into(),
            subject: Some("agent/example".into()),
            identity: None,
            initial_message: None,
            initial_message_id: None,
            seed: Some(root.join("missing-seed.jsonl")),
            argv: vec!["omp".into()],
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all().build().unwrap();
        let error = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(1), run_driver(&client, args, None))
                .await.expect("seed conflict must fail before daemon access")
                .unwrap_err()
        });
        assert_eq!(error.to_string(), format!(
            "native seed cannot accompany resume environment ({})", variable.to_string_lossy()
        ));
        assert!(!root.join("drivers").exists(), "no inventory or first-launch setup may occur");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let variables = [
        fractal_daemon::suspension::RESUME_ENV,
        fractal_daemon::suspension::CONTINUE_ENV,
        fractal_daemon::suspension::CONTINUE_PATH_ENV,
    ];
    for variable in variables {
        for value in ["native", ""] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child.args([
                "--exact",
                "native_seed_driver_tests::rejects_effective_resume_environment_before_receipt",
                "--nocapture",
            ]);
            for variable in variables {
                child.env_remove(variable);
            }
            let output = child.env(variable, value).env(CHILD, variable)
                .env("ST3_TEST_NATIVE_SEED_ROOT", root.path())
                .env("ST3_DRIVER_STATE_DIR", root.path().join("drivers"))
                .env("HOME", root.path()).output().unwrap();
            assert!(output.status.success(), "{variable}={value:?}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr));
        }
    }
}

#[cfg(unix)]
#[test]
fn managed_startup_reads_its_declaration_over_authenticated_unix() {
    use std::os::unix::fs::PermissionsExt as _;

    const CHILD: &str = "ST3_TEST_MANAGED_UNIX_ROOT";
    let Some(root) = std::env::var_os(CHILD) else {
        // Both the profile environment and the resolver's process-wide provider PATH belong
        // to one driver. Isolate them from other tests, just like a real driver process.
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("steward/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let wrapper = bin.join("codex");
        std::fs::write(&wrapper, "#!/bin/sh\nexit 99\n").unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let profile = root.path().join("profile.json");
        // Deliberately omit generalist: the environment must select the steward wrapper.
        std::fs::write(&profile, serde_json::to_vec(&json!({
            "personas": {"launch": {"steward": {"harnessBin": bin}}},
        })).unwrap()).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native_seed_driver_tests::managed_startup_reads_its_declaration_over_authenticated_unix",
                "--nocapture",
            ])
            .env(CHILD, root.path())
            .env("AGENT_RUNTIME_PROFILE", &profile)
            .env("AGENT_PERSONA", "steward")
            .env("HOME", root.path())
            .output().unwrap();
        assert!(output.status.success(), "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr));
        return;
    };
    let root = PathBuf::from(root);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            const NODE: &str = "managed-startup";
            const FLEET: &str = "3c9a1f2e-8b7d-4e6c-a5f4-1d2e3c4b5a69";
            const SUBJECT: &str = "agent/example/managed";
            let store = Arc::new(Store::open_memory(NODE).unwrap());
            let key = Arc::new(fractal_daemon::fleet::MemberKey::generate().unwrap().0);
            store.bind_fleet(FLEET).unwrap();
            store.pin_fleet_anchor(key.public()).unwrap();
            store.set_member_key(Some(key.clone())).unwrap();
            store.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
            let source = format!(
                "version 2\nagent \"example/managed\" {{ accountable \"person/operator\"; workspace {:?}; harness \"codex\" {{}}; }}",
                root,
            );
            let intent = fractal_daemon::graph::parse_intent(&source, NODE).unwrap();
            store.apply_as(
                &intent,
                &BTreeMap::from([(SUBJECT.into(), Vec::new())]),
                "managed-startup-declaration",
                Some("person/operator"),
            ).unwrap();
            let state = AppState {
                browser_pairing_login: None,
                seat_admission: Default::default(),
                owning_person: Some("person/operator".into()),
                store,
                notify: Arc::new(Notify::new()),
                event_notify: watch::channel(0_u64).0,
                node: NODE.into(),
                api_socket: root.join("api.sock"),
                state_dir: root.clone(),
                pty_root: root.join("pty"),
                pty_binary: PathBuf::from("pty"),
                fleet_id: Some(FLEET.into()),
                configured_peers: vec![],
                client_relay: None,
                native_session_home: None,
                planner_default: PlannerSpec::default(),
                harness_question_sink: None,
                assistant_receipts: Default::default(),
                private_notes: Default::default(),
            };
            let socket = root.join("managed.sock");
            #[cfg(target_os = "linux")]
            let _startup = {
                let startup = fractal_daemon::startup::Startup::begin(&socket).unwrap();
                startup.phase("full-replay/base-claims");
                let Err(error) = generated_client(&Endpoint::Unix(socket.clone()), Some(SUBJECT)) else {
                    panic!("full-replay startup must refuse client construction before listening");
                };
                assert!(daemon_is_unreachable(&error));
                startup
            };
            let server_socket = socket.clone();
            let server = tokio::spawn(async move {
                // Managed adoption may arrive before the restarted daemon accepts connections.
                tokio::time::sleep(Duration::from_millis(50)).await;
                serve_unix(&server_socket, router(state)).await.unwrap();
            });
            let mut client = Client::new(Endpoint::Unix(socket));
            let mut argv = vec!["/raw/bin/codex".into()];
            driver_cli::prepare_managed_provider(&client, SUBJECT, "codex", &mut argv)
                .await.expect("managed startup must retry client construction during full replay");
            assert_eq!(Path::new(&argv[0]), root.join("steward/bin/codex"));
            let anonymous = generated_client(&client.endpoint, None).unwrap()
                .agent_declaration_get(SUBJECT, None, false).await.unwrap_err();
            assert!(matches!(anonymous,
                GeneratedClientError::Api(ClientErrorCode::Forbidden, _, _)
            ), "anonymous declaration read must be forbidden: {anonymous}");

            // Declaration GET requires read.declarations, not a live runtime incarnation:
            // it is needed before startup can launch or bind the provider.
            for person in [Some("person/operator"), None] {
                client.person = person.map(str::to_owned);
                let mut argv = vec!["/raw/bin/codex".into(), "--model".into(), "example".into()];
                driver_cli::prepare_managed_provider(&client, SUBJECT, "codex", &mut argv)
                    .await.unwrap();
                assert_eq!(Path::new(&argv[0]), root.join("steward/bin/codex"));
                assert_eq!(&argv[1..], ["--model", "example"]);
            }
            server.abort();
        }).await.expect("managed Unix startup must finish without launching a provider");
    });
}
