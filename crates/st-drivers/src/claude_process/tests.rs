use super::*;
use std::collections::BTreeMap;

fn fixture() -> (Owner, BTreeMap<u32, Process>) {
    let wrapper = Process {
        pid: 10,
        parent: 2,
        birth: 100,
    };
    let provider = Process {
        pid: 20,
        parent: 10,
        birth: 200,
    };
    let owner = Owner {
        version: 1,
        session: "primary".into(),
        sequence: 3,
        runtime: "seat".into(),
        wrapper,
        provider: Some(provider),
        closed: false,
    };
    let nodes = [
        wrapper,
        provider,
        Process {
            pid: 30,
            parent: 20,
            birth: 300,
        }, // primary channel/hook
        Process {
            pid: 40,
            parent: 20,
            birth: 400,
        }, // nested provider
        Process {
            pid: 50,
            parent: 40,
            birth: 500,
        }, // nested channel/hook
        Process {
            pid: 60,
            parent: 20,
            birth: 600,
        }, // generated hook shell
        Process {
            pid: 70,
            parent: 60,
            birth: 700,
        },
    ]
    .into_iter()
    .map(|p| (p.pid, p))
    .collect();
    (owner, nodes)
}

fn read(nodes: &BTreeMap<u32, Process>, pid: u32) -> Result<Process> {
    nodes.get(&pid).copied().context("no such live process")
}

#[test]
fn primary_and_nested_channels_cannot_replace_each_other_in_either_order() {
    let (owner, nodes) = fixture();
    for order in [[30, 50], [50, 30]] {
        let mut epoch = 0;
        let mut admitted = Vec::new();
        for pid in order {
            if prove(
                &owner,
                pid,
                &Caller::Channel,
                |p| read(&nodes, p),
                |_| Ok(vec![]),
            )
            .is_ok()
            {
                epoch += 1;
                admitted.push(pid);
            }
        }
        assert_eq!(epoch, 1);
        assert_eq!(admitted, [30]);
        // Secondary EOF has no admitted epoch or callback and therefore no primary close.
        assert!(
            prove(
                &owner,
                50,
                &Caller::Channel,
                |p| read(&nodes, p),
                |_| Ok(vec![])
            )
            .is_err()
        );
        assert!(
            prove(
                &owner,
                30,
                &Caller::Channel,
                |p| read(&nodes, p),
                |_| Ok(vec![])
            )
            .is_ok()
        );
    }
}

#[test]
fn rewritten_record_cannot_pass_an_independent_wrapper_generation_or_run_a_callback() {
    let dir = tempfile::tempdir().unwrap();
    let sequence = claim(dir.path(), "fixture", "fixture", "current").unwrap();
    let mut owner: Owner = serde_json::from_slice(&read_owner(dir.path()).unwrap()).unwrap();
    let independently_captured = owner.wrapper;
    owner.wrapper.birth += 1;
    write(dir.path(), &owner).unwrap();
    let mut mutated = false;
    assert!(
        with_caller(
            dir.path(),
            "current",
            sequence,
            std::process::id(),
            Some(independently_captured),
            Caller::Wrapper,
            |_| {
                mutated = true;
                Ok(())
            }
        )
        .is_err()
    );
    assert!(!mutated);
}

#[test]
fn hook_trampoline_is_exact_and_nested_hook_cannot_rebind_native_session() {
    let (owner, nodes) = fixture();
    let command = "\"$ST_HOOKS/claude-observe.sh\" SessionStart";
    let role = Caller::Hook {
        trampoline: command,
    };
    let argv = |_: u32| Ok(vec!["/bin/sh".into(), "-c".into(), command.into()]);
    assert!(prove(&owner, 70, &role, |p| read(&nodes, p), argv).is_ok());
    assert!(
        prove(
            &owner,
            50,
            &role,
            |p| read(&nodes, p),
            |_| Ok(vec!["claude".into(), "-p".into()])
        )
        .is_err()
    );
    for argv in [
        vec!["sh", "-c", "unrelated"],
        vec!["python", "-c", command],
        vec!["sh", "-c", command, "extra"],
    ] {
        assert!(
            prove(
                &owner,
                70,
                &role,
                |p| read(&nodes, p),
                |_| Ok(argv.iter().map(|s| s.to_string()).collect())
            )
            .is_err()
        );
    }
}

#[test]
fn pid_reuse_reparenting_and_primary_exit_refuse_before_the_publication_callback() {
    let (owner, nodes) = fixture();
    let proof = prove(
        &owner,
        30,
        &Caller::Channel,
        |p| read(&nodes, p),
        |_| Ok(vec![]),
    )
    .unwrap();
    for pid in [10, 20, 30] {
        for mutate in ["birth", "parent", "exit"] {
            let mut changed = nodes.clone();
            match mutate {
                "birth" => changed.get_mut(&pid).unwrap().birth += 1,
                "parent" => changed.get_mut(&pid).unwrap().parent = 1,
                _ => {
                    changed.remove(&pid);
                }
            }
            assert!(
                validate_proof(&proof, |p| read(&changed, p)).is_err(),
                "{pid}/{mutate}"
            );
        }
    }
}

#[test]
fn legacy_generation_stays_legacy_and_malformed_witness_never_downgrades() {
    let dir = tempfile::tempdir().unwrap();
    let admit = || {
        with_caller(
            dir.path(),
            "old",
            1,
            std::process::id(),
            None,
            Caller::Channel,
            Ok,
        )
    };
    assert_eq!(admit().unwrap(), Admission::Legacy);
    assert!(!dir.path().join(FILE).exists());
    assert!(!dir.path().join(LOCK).exists());
    let _ = lock(dir.path(), true).unwrap();
    fs::write(dir.path().join(FILE), b"malformed").unwrap();
    assert!(admit().is_err());
    // Old daemons ignore this additive local witness. New daemons preserve absence for old
    // wrappers; they do not rewrite it into an allegedly protected generation.
}

#[test]
fn stale_session_sequence_and_closed_generation_never_run_the_callback() {
    let dir = tempfile::tempdir().unwrap();
    let seq = claim(dir.path(), "fixture", "fixture", "current").unwrap();
    for (session, sequence) in [("previous", seq), ("current", seq + 1)] {
        let mut ran = false;
        assert!(
            with_caller(
                dir.path(),
                session,
                sequence,
                std::process::id(),
                None,
                Caller::Wrapper,
                |_| {
                    ran = true;
                    Ok(())
                }
            )
            .is_err()
        );
        assert!(!ran);
    }
    assert_eq!(
        with_caller(
            dir.path(),
            "current",
            seq,
            std::process::id(),
            None,
            Caller::Wrapper,
            Ok
        )
        .unwrap(),
        Admission::Primary
    );
    ended(dir.path(), "current", seq).unwrap();
    assert!(
        with_caller(
            dir.path(),
            "current",
            seq,
            std::process::id(),
            None,
            Caller::Wrapper,
            Ok
        )
        .is_err()
    );
}

/// Ordinary hosted kernel control only. Every child is this test executable; no Claude,
/// daemon, provider, network call, terminal or live seat participates.
#[test]
fn kernel_primary_and_nested_helpers_with_identical_credentials_are_isolated() {
    use std::process::{Command, Stdio};
    const ROLE: &str = "ST_TEST_CLAUDE_PROCESS_ROLE";
    const TEST: &str = "claude_process::tests::kernel_primary_and_nested_helpers_with_identical_credentials_are_isolated";
    let role = std::env::var(ROLE).ok();
    let command = |role: &str| {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", TEST, "--nocapture"]).env(ROLE, role);
        cmd
    };
    if let Some(role) = role {
        let dir = std::path::PathBuf::from(std::env::var_os("ST_TEST_CLAUDE_PROCESS_DIR").unwrap());
        let seq = std::env::var("ST_CLAUDE_SESSION_SEQ")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(std::env::var("ST_CLAUDE_SESSION").unwrap(), "kernel");
        match role.as_str() {
            "provider" => {
                use std::io::Read as _;
                std::io::stdin().read_exact(&mut [0u8; 1]).unwrap();
                assert!(command("primary").status().unwrap().success());
                assert!(command("primary-hook").status().unwrap().success());
                assert!(command("nested").status().unwrap().success());
            }
            "nested" => {
                assert!(command("secondary").status().unwrap().success());
                assert!(command("secondary-hook").status().unwrap().success());
            }
            _ => {
                let hook = role.ends_with("-hook");
                let caller = if hook {
                    Caller::Hook {
                        trampoline: "\"$ST_HOOKS/claude-observe.sh\" SessionStart",
                    }
                } else {
                    Caller::Channel
                };
                let result = with_caller(
                    &dir,
                    "kernel",
                    seq,
                    std::process::id(),
                    None,
                    caller,
                    |admission| {
                        if hook {
                            fs::write(dir.join("native-binding-control"), role.as_bytes())?;
                        }
                        Ok(admission)
                    },
                );
                if role.starts_with("primary") {
                    assert_eq!(result.unwrap(), Admission::Primary);
                } else {
                    assert!(result.is_err());
                }
            }
        }
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let seq = claim(dir.path(), "fixture", "fixture", "kernel").unwrap();
    let mut child = command("provider")
        .env("ST_TEST_CLAUDE_PROCESS_DIR", dir.path())
        .env("ST_CLAUDE_SESSION_SEQ", seq.to_string())
        .env("ST_CLAUDE_SESSION", "kernel")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    spawned(dir.path(), "kernel", seq, child.id()).unwrap();
    use std::io::Write as _;
    child.stdin.take().unwrap().write_all(b"g").unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read(dir.path().join("native-binding-control")).unwrap(),
        b"primary-hook"
    );
    ended(dir.path(), "kernel", seq).unwrap();
}

#[test]
fn startup_and_reexec_keep_the_exact_primary_and_never_admit_a_nested_candidate() {
    let (mut owner, nodes) = fixture();
    owner.provider = None;
    assert!(
        prove(
            &owner,
            30,
            &Caller::Channel,
            |p| read(&nodes, p),
            |_| Ok(vec![])
        )
        .is_ok()
    );
    assert!(
        prove(
            &owner,
            50,
            &Caller::Channel,
            |p| read(&nodes, p),
            |_| Ok(vec![])
        )
        .is_err()
    );
    owner.provider = Some(nodes[&20]);
    // A driver reexec retains PID, birth, provider and session sequence. A different image
    // does not fabricate a new parent; a replacement generation must claim a new witness.
    let retained = prove(
        &owner,
        30,
        &Caller::Channel,
        |p| read(&nodes, p),
        |_| Ok(vec![]),
    )
    .unwrap();
    validate_proof(&retained, |p| read(&nodes, p)).unwrap();
    owner.provider.as_mut().unwrap().birth += 1;
    assert!(
        prove(
            &owner,
            30,
            &Caller::Channel,
            |p| read(&nodes, p),
            |_| Ok(vec![])
        )
        .is_err()
    );
}
