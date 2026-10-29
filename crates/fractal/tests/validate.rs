use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/validate-revisions.kdl");
// A mission revision is SHA-256 over the serialized normalized mission. Any change to that
// serialization changes every published revision; update this literal only on purpose. The
// fixture's schedule pins the same revision.
const PINNED_REVISION: &str = "b283ff12e887f06fbeb2da3094b10f8fd2b9b7840f7af8031445b124829f25f7";

#[test]
fn declaration_mission_revision_is_pinned() {
    let intent = fractal_store::graph::parse_intent(FIXTURE, "local").unwrap();
    assert_eq!(intent.subjects.len(), 3);
    assert_eq!(intent.missions.len(), 1);
    let mission = &intent.missions["docs/daily-review"];
    assert_eq!(
        mission.revision, PINNED_REVISION,
        "mission docs/daily-review revision changed; actual revision: {}",
        mission.revision
    );
}

#[tokio::test]
async fn validate_reads_files_without_a_daemon_or_configuration() {
    let scratch = tempfile::tempdir().unwrap();
    let fixture = scratch.path().join("declarations.kdl");
    std::fs::write(&fixture, FIXTURE).unwrap();
    // A malformed config and a missing endpoint make accidental online dispatch fail.
    let config = scratch.path().join("config/st3");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("config.toml"), "not valid TOML [").unwrap();
    let expected = fractal_store::graph::parse_intent(FIXTURE, "local").unwrap();
    for json_output in [false, true] {
        let mut command = fractal_daemon::test_support::async_command(test_bin!("fractal-fixture"));
        command
            .env("XDG_CONFIG_HOME", scratch.path().join("config"))
            .env("TMPDIR", scratch.path())
            .arg("--endpoint")
            .arg(scratch.path().join("absent.sock"))
            .arg("validate")
            .arg(&fixture)
            .kill_on_drop(true);
        if json_output {
            command.arg("--json");
        }
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
            .await
            .expect("offline validation must finish")
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let revision = &expected.missions["docs/daily-review"].revision;
        if json_output {
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["subjects"], json!([
                {"kind": "agent", "id": "agent/docs/editor"},
                {"kind": "agent", "id": "agent/docs/writer"},
                {"kind": "schedule", "id": "schedule/docs/daily-review"},
                {"kind": "mission", "id": "mission/docs/daily-review"}
            ]));
            assert_eq!(value["missions"], json!([
                {"id": "docs/daily-review", "revision": revision}
            ]));
        } else {
            let text = String::from_utf8(output.stdout).unwrap();
            assert_eq!(text, format!(
                "agent agent/docs/editor\nagent agent/docs/writer\nschedule schedule/docs/daily-review\nmission mission/docs/daily-review\ndocs/daily-review@{revision}\n"
            ));
        }
    }
    let invalid = scratch.path().join("invalid.kdl");
    std::fs::write(&invalid, "version 2\nagent \"broken\" {\n").unwrap();
    let output = fractal_daemon::test_support::async_command(test_bin!("fractal-fixture"))
        .env("XDG_CONFIG_HOME", scratch.path().join("config"))
        .env("TMPDIR", scratch.path())
        .arg("validate")
        .arg(&invalid)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("invalid.kdl"), "{diagnostic}");
    assert!(diagnostic.contains("invalid-kdl"), "{diagnostic}");
}
