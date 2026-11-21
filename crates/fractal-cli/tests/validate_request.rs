//! The real executable must validate without entering any online initialization path.
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

use fractal_daemon::person_request::StructuredRequest;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Write as _, os::unix::{fs::MetadataExt as _, net::UnixListener}, path::{Path, PathBuf}, process::{Command, Output, Stdio}};

// Include directory change times: even a create-then-delete or same-content rewrite is observable.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (u64, i64, i64, i64, i64)> {
    fn visit(path: &Path, result: &mut BTreeMap<PathBuf, (u64, i64, i64, i64, i64)>) {
        let meta = fs::symlink_metadata(path).unwrap();
        result.insert(path.to_owned(), (meta.len(), meta.mtime(), meta.mtime_nsec(), meta.ctime(), meta.ctime_nsec()));
        if meta.is_dir() {
            for entry in fs::read_dir(path).unwrap() { visit(&entry.unwrap().path(), result); }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, &mut result);
    result
}

struct Offline {
    root: tempfile::TempDir,
    listener: UnixListener,
    telemetry: std::net::TcpListener,
}

impl Offline {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(root.path().join("daemon.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        fs::write(root.path().join("config"), "not valid config\n").unwrap();
        fs::write(root.path().join("gate-report"), "unchanged\n").unwrap();
        let telemetry = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        telemetry.set_nonblocking(true).unwrap();
        for product in ["fractal", "st3"] {
            let directory = root.path().join("config-home").join(product);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("config.toml"), "invalid = [").unwrap();
        }
        Self { root, listener, telemetry }
    }

    fn run(&self, args: &[&str], input: &[u8]) -> Output {
        let root = self.root.path();
        let before = snapshot(root);
        let mut child = Command::new(test_env!("CARGO_BIN_EXE_fractal"))
            .args(args).current_dir(root).env_clear()
            .env("HOME", root).env("XDG_CONFIG_HOME", root.join("config-home"))
            .env("XDG_STATE_HOME", root.join("state-home"))
            .env("XDG_CACHE_HOME", root.join("cache-home"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("ST3_ENDPOINT", root.join("daemon.sock"))
            .env("FRACTAL_ENDPOINT", root.join("daemon.sock"))
            .env("FRACTAL_INSTANCE", "../invalid-instance")
            .env("ST_INSTANCE", "../invalid-instance")
            .env("FRACTAL_AGENT", "person/hostile")
            .env("ST_AGENT", "agent/conflicting")
            .env("ST3_CONFIG", root.join("config"))
            .env("ST_CONFIG", root.join("config"))
            .env("ST_GATE_REPORT", root.join("gate-report"))
            .env("COMPLETE", "bash")
            .env("OTEL_EXPORTER_OTLP_ENDPOINT", format!("http://{}", self.telemetry.local_addr().unwrap()))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().unwrap();
        // Some malformed forms exit before reading stdin.
        let _ = child.stdin.take().unwrap().write_all(input);
        let output = child.wait_with_output().unwrap();
        assert_eq!(snapshot(root), before, "offline command wrote files: {output:?}");
        assert_eq!(self.listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock,
            "offline command connected to the daemon");
        assert_eq!(self.telemetry.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock,
            "offline command connected to telemetry");
        output
    }

    fn canonical(&self, args: &[&str], input: &[u8]) -> Value {
        let output = self.run(args, input);
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON value")
    }
}

fn requests() -> Vec<Value> {
    let common = json!({"version": 1, "type": "feedback", "question": "What should change?",
        "why_person": "Only the owner can judge the result.",
        "subjects": [{"kind": "document", "label": "Proposal", "ref": "doc/proposal"}]});
    let mut decision = common.clone();
    decision["type"] = json!("decision");
    decision["answers"] = json!([
        {"id": "approve", "label": "Approve", "consequence": "Apply the proposal.", "outcome": "accept"},
        {"id": "reject", "label": "Reject", "consequence": "Leave it unchanged.", "outcome": "decline"}]);
    let mut choice = common.clone();
    choice["type"] = json!("choice");
    choice["custom"] = json!(true);
    choice["summary"] = json!("Choose the presentation.");
    choice["reasons"] = json!(["Both alternatives are implementable."]);
    choice["recommendation"] = json!({"answer": "compact", "reason": "Less scrolling."});
    choice["answers"] = json!([
        {"id": "compact", "label": "Compact", "consequence": "Use compact layout.", "conditions": ["Keep labels readable."]},
        {"id": "wide", "label": "Wide", "consequence": "Use wide layout."}]);
    vec![common, decision, choice, json!({"version": 1, "type": "update", "about": "step-run/owner/request",
        "summary": "The requested change is complete."})]
}

#[test]
fn native_types_defaults_roundtrip_and_file_stdin_equivalence_are_offline() {
    let offline = Offline::new();
    for input in requests() {
        let (typed, expected) = StructuredRequest::parse(&input).unwrap();
        let bytes = serde_json::to_vec(&input).unwrap();
        let stdin = offline.canonical(&["work", "validate-request", "-"], &bytes);
        assert_eq!(stdin, expected);
        assert_eq!(StructuredRequest::parse(&stdin).unwrap().0, typed);
        assert_eq!(offline.canonical(&["work", "validate-request", "-"], &serde_json::to_vec(&stdin).unwrap()), stdin);
        fs::write(offline.root.path().join("request.json"), &bytes).unwrap();
        assert_eq!(offline.canonical(&["work", "validate-request", "request.json"], b""), stdin);
    }
    let mut explicit = requests().remove(0);
    for (key, value) in [("custom", json!(false)), ("answers", json!([])), ("attachments", json!([])),
        ("reasons", json!([])), ("recommendation", Value::Null), ("summary", Value::Null)] {
        explicit[key] = value;
    }
    let canonical = offline.canonical(&["work", "validate-request", "-"], &serde_json::to_vec(&explicit).unwrap());
    assert_eq!(canonical, requests().remove(0));
}

#[test]
fn input_errors_preserve_native_diagnostics_and_empty_stdout() {
    let offline = Offline::new();
    for (input, diagnostic) in [
        (b"not JSON".as_slice(), "the request is not JSON"),
        (b"{}".as_slice(), "the request is malformed"),
        (b"{\"version\":2,\"type\":\"feedback\"}".as_slice(), "this daemon reads request version 1"),
        (b"{\"version\":1,\"type\":\"feedback\"}".as_slice(), "question must not be empty"),
        (b"\xff".as_slice(), "stream did not contain valid UTF-8"),
    ] {
        let output = offline.run(&["work", "validate-request", "-"], input);
        assert!(!output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains(diagnostic), "{output:?}");
    }
    let invalid = |mut value: Value, key: &str, replacement: Value| { value[key] = replacement; value };
    for input in [
        invalid(requests().remove(0), "why_person", Value::Null),
        invalid(requests().remove(0), "subjects", json!([{"kind":"document","label":"Missing address"}])),
        invalid(requests().remove(1), "answers", json!([])),
        invalid(requests().remove(0), "unknown", json!(true)),
    ] {
        let native = StructuredRequest::parse(&input).unwrap_err().to_string();
        let output = offline.run(&["work", "validate-request", "-"], &serde_json::to_vec(&input).unwrap());
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains(&native), "{output:?}");
    }
    let missing = offline.run(&["work", "validate-request", "missing.json"], b"");
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("read the request missing.json"));
}

#[test]
fn recognized_malformed_arguments_exit_offline_without_gate_report_writes() {
    let offline = Offline::new();
    for args in [
        vec!["work", "validate-request"],
        vec!["work", "validate-request", "-", "extra"],
        vec!["work", "validate-request", "--unknown"],
        vec!["work", "validate-request", "-", "--as", "person/owner"],
        vec!["--instance", "../invalid", "work", "validate-request", "-"],
        vec!["--endpoint", "hostile", "work", "validate-request", "-"],
        vec!["work", "validate-request", "-", "--config", "config"],
        vec!["--json", "work", "validate-request", "-"],
        vec!["work", "--json", "validate-request", "-"],
        vec!["work", "--daemon-wait=invalid", "validate-request", "-"],
        vec!["--daemon-wait", "invalid", "work", "validate-request", "-"],
    ] {
        let output = offline.run(&args, b"{}");
        assert!(!output.status.success(), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        assert!(!output.stderr.is_empty(), "{args:?}: {output:?}");
    }
}
