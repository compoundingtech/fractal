use super::*;
use std::collections::BTreeMap;

fn generation(pid: u32, parent: u32) -> Process {
    Process { pid, parent, birth: u64::from(pid) * 100 }
}

fn fixture(harness: &str) -> (Launch, BTreeMap<u32, Process>) {
    let wrapper = generation(10, 2);
    let provider = generation(20, 10);
    let launch = Launch {
        version: 1, harness: harness.into(), session: "session-one".into(), sequence: 7,
        wrapper, provider: Some(provider), connector: Some(generation(30, 20)),
    };
    let table = [wrapper, provider, generation(30, 20), generation(40, 30),
        generation(50, 10), generation(60, 50), generation(70, 2), generation(80, 20)]
        .into_iter().map(|process| (process.pid, process)).collect();
    (launch, table)
}

fn read_table(table: &BTreeMap<u32, Process>, pid: u32) -> Result<Process> {
    table.get(&pid).copied().context("process exited")
}

fn admit(launch: &Launch, table: &BTreeMap<u32, Process>, pid: u32, role: Role) -> Result<Process> {
    prove(launch, launch.wrapper, pid, &launch.harness, role, &launch.session, launch.sequence,
        |pid| read_table(table, pid))
}

#[test]
fn pi_family_delivery_requires_the_registered_providers_exact_child() {
    for harness in ["omp", "pi"] {
        let (launch, table) = fixture(harness);
        assert!(admit(&launch, &table, 30, Role::Delivery).is_ok());
        for pid in [10, 20, 40, 50, 60, 70, 80] {
            assert!(admit(&launch, &table, pid, Role::Delivery).is_err(), "{harness}: {pid}");
        }
    }
}

#[test]
fn wrapper_roles_and_wrapper_delivery_require_the_exact_generation() {
    for harness in ["omp", "pi", "codex", "opencode"] {
        let (launch, table) = fixture(harness);
        assert!(admit(&launch, &table, 10, Role::Wrapper).is_ok());
        for pid in [20, 30, 40, 50, 60, 70] {
            assert!(admit(&launch, &table, pid, Role::Wrapper).is_err());
        }
        if matches!(harness, "codex" | "opencode") {
            assert!(admit(&launch, &table, 10, Role::Delivery).is_ok());
            for pid in [20, 30, 40, 50, 60, 70] {
                assert!(admit(&launch, &table, pid, Role::Delivery).is_err());
            }
        }
    }
}

#[test]
fn process_labels_and_forged_argv_cannot_nominate_a_provider() {
    let (launch, table) = fixture("omp");
    // Every label, including an exact copy of the provider's command line, is outside the
    // proof interface. Only the recorded spawn and the process table are consulted.
    for _argv in [vec!["omp"], vec!["st", "driver", "omp", "--session", "session-one"],
        vec!["node", "channel.js"]] {
        assert!(admit(&launch, &table, 60, Role::Delivery).is_err());
        assert!(admit(&launch, &table, 30, Role::Delivery).is_ok());
    }
}

#[test]
fn exited_and_reused_wrapper_provider_or_connector_fail_closed() {
    let (launch, table) = fixture("pi");
    for pid in [10, 20, 30] {
        let mut exited = table.clone();
        exited.remove(&pid);
        assert!(admit(&launch, &exited, 30, Role::Delivery).is_err());
        let mut reused = table.clone();
        reused.get_mut(&pid).unwrap().birth += 1;
        if pid != 30 {
            assert!(admit(&launch, &reused, 30, Role::Delivery).is_err());
        }
        let witness = Witness { dir: PathBuf::new(), launch: launch.clone(), peer: table[&30], role: Role::Delivery };
        assert!(witness.validate_launch(&launch, |pid| read_table(&reused, pid)).is_err());
    }
}

#[test]
fn captured_witness_revalidates_the_record_and_every_generation() {
    let (launch, table) = fixture("omp");
    let witness = Witness { dir: PathBuf::new(), launch: launch.clone(), peer: table[&30], role: Role::Delivery };
    assert!(witness.validate_launch(&launch, |pid| read_table(&table, pid)).is_ok());
    for pid in [10, 20, 30] {
        let mut reparented = table.clone();
        reparented.get_mut(&pid).unwrap().parent = 70;
        assert!(witness.validate_launch(&launch, |pid| read_table(&reparented, pid)).is_err());
    }
    let mut changed = launch.clone();
    changed.sequence += 1;
    assert!(witness.validate_launch(&changed, |pid| read_table(&table, pid)).is_err());
    changed = launch.clone();
    changed.provider = Some(table[&50]);
    assert!(witness.validate_launch(&changed, |pid| read_table(&table, pid)).is_err());
}

#[test]
fn wrong_session_sequence_harness_wrapper_and_incomplete_launch_are_refused() {
    let (launch, table) = fixture("omp");
    for (wrapper, harness, session, sequence) in [
        (launch.wrapper, "omp", "other-session", 7),
        (launch.wrapper, "omp", "session-one", 8),
        (launch.wrapper, "pi", "session-one", 7),
        (table[&50], "omp", "session-one", 7),
    ] {
        assert!(prove(&launch, wrapper, 30, harness, Role::Delivery, session, sequence,
            |pid| read_table(&table, pid)).is_err());
    }
    let mut incomplete = launch;
    incomplete.provider = None;
    assert!(admit(&incomplete, &table, 30, Role::Delivery).is_err());
}

#[test]
fn provider_registration_precedes_connector_admission_and_is_immutable() {
    for harness in ["omp", "pi"] {
        let (mut launch, table) = fixture(harness);
        launch.connector = None;
        assert!(admit(&launch, &table, 30, Role::Delivery).is_err());
        let connector = establish_connector(
            &launch, 20, 30, harness, &launch.session, launch.sequence,
            |pid| read_table(&table, pid),
        ).unwrap();
        launch.connector = Some(connector);
        assert!(admit(&launch, &table, 30, Role::Delivery).is_ok());
        assert!(admit(&launch, &table, 80, Role::Delivery).is_err());
        assert!(establish_connector(
            &launch, 20, 30, harness, &launch.session, launch.sequence,
            |pid| read_table(&table, pid),
        ).is_ok());
        assert!(establish_connector(
            &launch, 20, 80, harness, &launch.session, launch.sequence,
            |pid| read_table(&table, pid),
        ).is_err());
    }
}

#[test]
fn registration_requires_the_exact_live_provider_and_current_incarnation() {
    let (mut launch, table) = fixture("omp");
    launch.connector = None;
    for provider_pid in [10, 30, 40, 50, 60, 70, 80] {
        assert!(establish_connector(
            &launch, provider_pid, 30, "omp", &launch.session, launch.sequence,
            |pid| read_table(&table, pid),
        ).is_err());
    }
    for connector_pid in [10, 20, 40, 50, 60, 70] {
        assert!(establish_connector(
            &launch, 20, connector_pid, "omp", &launch.session, launch.sequence,
            |pid| read_table(&table, pid),
        ).is_err());
    }
    for (harness, session, sequence) in [
        ("pi", "session-one", 7), ("omp", "other", 7), ("omp", "session-one", 8),
    ] {
        assert!(establish_connector(
            &launch, 20, 30, harness, session, sequence, |pid| read_table(&table, pid),
        ).is_err());
    }
    for pid in [10, 20] {
        let mut reused = table.clone();
        reused.get_mut(&pid).unwrap().birth += 1;
        assert!(establish_connector(
            &launch, 20, 30, "omp", &launch.session, launch.sequence,
            |pid| read_table(&reused, pid),
        ).is_err());
    }
}

#[test]
fn provider_may_replace_only_a_provably_departed_connector() {
    let (launch, table) = fixture("omp");
    let captured = Witness {
        dir: PathBuf::new(), launch: launch.clone(), peer: table[&30], role: Role::Delivery,
    };
    // A metadata failure is not proof that the former connector has departed.
    assert!(establish_connector(
        &launch, 20, 80, "omp", &launch.session, launch.sequence,
        |pid| if pid == 30 { anyhow::bail!("process metadata unavailable") } else { read_table(&table, pid) },
    ).is_err());
    // Absence and a different kernel birth establish that the former generation is gone.
    let absent = |pid| if pid == 30 {
        Err(std::io::Error::from(std::io::ErrorKind::NotFound).into())
    } else {
        read_table(&table, pid)
    };
    let successor = establish_connector(
        &launch, 20, 80, "omp", &launch.session, launch.sequence, absent,
    ).unwrap();
    let mut replacement = launch.clone();
    replacement.connector = Some(successor);
    assert!(captured.validate_launch(&replacement, |pid| read_table(&table, pid)).is_err());
    assert!(admit(&replacement, &table, 80, Role::Delivery).is_ok());
    assert!(admit(&replacement, &table, 30, Role::Delivery).is_err());
    let mut reused = table.clone();
    reused.get_mut(&30).unwrap().birth += 1;
    assert!(establish_connector(
        &launch, 20, 30, "omp", &launch.session, launch.sequence,
        |pid| read_table(&reused, pid),
    ).is_ok());
    assert!(captured.validate_launch(&launch, |pid| read_table(&reused, pid)).is_err());
}

#[test]
fn matching_start_token_without_a_boot_id_does_not_replace_the_connector_role_proof() {
    // A daemon may select a legacy physical wrapper by its public kernel start token alone.
    // That selection supplies only Process: no boot-ID or argv label enters this boundary.
    let (launch, table) = fixture("omp");
    let selected_wrapper = Process {
        pid: 10, parent: 2, birth: table[&10].birth,
    };
    for (pid, eligible) in [(30, true), (40, false), (80, false)] {
        assert_eq!(prove(
            &launch, selected_wrapper, pid, "omp", Role::Delivery,
            &launch.session, launch.sequence, |pid| read_table(&table, pid),
        ).is_ok(), eligible);
    }
    let mut unwitnessed = launch;
    unwitnessed.connector = None;
    assert!(prove(
        &unwitnessed, selected_wrapper, 30, "omp", Role::Delivery,
        &unwitnessed.session, unwitnessed.sequence, |pid| read_table(&table, pid),
    ).is_err());
}

#[test]
fn exact_wrapper_role_covers_startup_and_terminal_publication_without_a_live_provider() {
    for harness in ["omp", "pi", "codex", "opencode"] {
        let (mut launch, mut table) = fixture(harness);
        let provider = launch.provider.take();
        launch.connector = None;
        assert!(admit(&launch, &table, 10, Role::Wrapper).is_ok());
        assert!(admit(&launch, &table, 50, Role::Wrapper).is_err());
        if matches!(harness, "codex" | "opencode") {
            assert!(admit(&launch, &table, 10, Role::Delivery).is_ok());
        } else {
            assert!(admit(&launch, &table, 30, Role::Delivery).is_err());
        }
        launch.provider = provider;
        launch.connector = Some(table[&30]);
        table.remove(&20);
        assert!(admit(&launch, &table, 10, Role::Wrapper).is_ok());
        if matches!(harness, "codex" | "opencode") {
            assert!(admit(&launch, &table, 10, Role::Delivery).is_ok());
        } else {
            assert!(admit(&launch, &table, 30, Role::Delivery).is_err());
        }
        let witness = Witness {
            dir: PathBuf::new(), launch: launch.clone(), peer: table[&10], role: Role::Wrapper,
        };
        assert!(witness.validate_launch(&launch, |pid| read_table(&table, pid)).is_ok());
        let mut changed = launch.clone();
        changed.sequence += 1;
        assert!(witness.validate_launch(&changed, |pid| read_table(&table, pid)).is_err());
        table.get_mut(&10).unwrap().birth += 1;
        assert!(witness.validate_launch(&launch, |pid| read_table(&table, pid)).is_err());
    }
}

#[test]
fn legacy_observer_adoption_reuses_canonical_sequence_without_claiming_or_minting_witness() {
    let dir = tempfile::tempdir().unwrap();
    let sequence = crate::harness_state::claim(dir.path(), "seat", "codex", "same-incarnation").unwrap();
    let path = crate::harness_state::harness_state_path(dir.path());
    let before = fs::read(&path).unwrap();
    for _ in 0..2 {
        assert_eq!(observer_sequence(dir.path(), "codex", "same-incarnation").unwrap(), sequence);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!recorded(dir.path()).unwrap());
    }
    assert!(observer_sequence(dir.path(), "codex", "different-incarnation").is_err());
    assert!(observer_sequence(dir.path(), "pi", "same-incarnation").is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn present_invalid_witness_never_downgrades_to_legacy_canonical_ownership() {
    let dir = tempfile::tempdir().unwrap();
    crate::harness_state::claim(dir.path(), "seat", "codex", "same-incarnation").unwrap();
    let path = crate::harness_state::harness_state_path(dir.path());
    let before = fs::read(&path).unwrap();
    fs::write(dir.path().join(FILE), b"not a launch record").unwrap();
    assert!(observer_sequence(dir.path(), "codex", "same-incarnation").is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(dir.path().join(FILE)).unwrap(), b"not a launch record");
}
