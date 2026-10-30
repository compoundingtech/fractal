//! Real hook wire contract against the bootstrap handler and physical role admission.
//! The fixture supplies the trusted supervisor launch record from live kernel generations;
//! it does not certify provider launch/version admission or run a live daemon.
use super::*;
use axum::{body::Body, routing::post};
use serde_json::json;
use std::{collections::BTreeMap, fs, time::Duration};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tower::ServiceExt as _;

struct PhysicalFixture {
    runtime: fractal_runtime::PtyRuntime,
    name: String,
    incarnation: String,
}
impl Drop for PhysicalFixture {
    fn drop(&mut self) {
        let _ = self.runtime.stop_if(&self.name, Some(&self.incarnation));
    }
}

// Run the shipped helper in its real provider process, with its actual spawned FD3 child.
// A separate control socket lets the fixture install trusted launch evidence before HTTP.
const PROVIDER: &str = r#"
import fs from 'node:fs';
import net from 'node:net';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {stripTypeScriptTypes} from 'node:module';
import {Duplex} from 'node:stream';
const [asset, controlPath, connectorPath, httpPath, identity, harness] = process.argv.slice(2);
const source = fs.readFileSync(asset, 'utf8');
const start = source.indexOf('const registerConnector = (');
if (start < 0) throw Error('registration helper absent');
const end = source.indexOf('\n};', start) + 3;
const implementation = stripTypeScriptTypes(source.slice(start, end));
const register = new Function('http', 'Duplex', 'HELLO_TIMEOUT_MS',
  `${implementation}\nreturn registerConnector;`)(http, Duplex, 5000);
const child = spawn(process.execPath, ['-e', `
  const net = require('node:net');
  const fs = require('node:fs');
  const socket = net.connect(process.argv[1], () => {
    socket.write('paused\\n');
    const byte = Buffer.alloc(1);
    if (fs.readSync(3, byte, 0, 1, null) !== 1 || byte[0] !== 1) process.exit(2);
    socket.write('ready\\n');
  });
`, connectorPath], {stdio:['ignore', 'inherit', 'inherit', 'pipe']});
const control = net.connect(controlPath, () => control.write(`${child.pid}\n`));
let input = '';
control.on('data', (chunk) => {
  input += chunk;
  if (!input.includes('\n')) return;
  const facts = JSON.parse(input.trim());
  control.removeAllListeners('data');
  // Older provider integrations may still send their bare launch identity directly.
  // Exercise the daemon's canonicalization independently, then the actual asset helper.
  const body = JSON.stringify({...facts, subject:identity, harness, connector_pid:child.pid});
  const request = http.request({socketPath:httpPath, path:'/v1/native-connectors/bootstrap',
    method:'POST', agent:false, headers:{'content-type':'application/json',
      'content-length':Buffer.byteLength(body)}}, (response) => {
    response.resume();
    response.on('end', () => {
      if (response.statusCode < 200 || response.statusCode >= 300) {
        throw Error(`direct bootstrap refused: ${response.statusCode}`);
      }
      register(child, httpPath, {...facts, subject:identity, harness});
    });
  });
  request.on('error', (error) => { throw error; });
  request.end(body);
});
process.on('exit', () => child.kill());
"#;

async fn declared_bootstrap(crate_name: &str, harness: &'static str, canonical_identity: bool) {
    let root = tempfile::tempdir().unwrap();
    let mut state = super::super::tests_shared::state(root.path());
    let intent = crate::graph::parse_intent(
        &format!("version 2\nagent \"worker\" {{ host \"node\"; workspace \"/tmp\"; harness {harness:?} {{}} }}"),
        "node",
    ).unwrap();
    // No subject rewriting: normal declaration must resolve to the production address.
    let subject = "agent/node.worker";
    assert!(intent.subjects.contains_key(subject));
    state.store.apply_internal(&intent, "declared-bootstrap").unwrap();
    let member = state.store.desired_subjects_named(&[subject.into()]).unwrap()
        .remove(0).member.unwrap();
    state.pty_binary = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|directory| directory.join("pty")).find(|path| path.is_file())
        .and_then(|path| fs::canonicalize(path).ok()).expect("pinned devShell pty must exist");
    let runtime = fractal_runtime::PtyRuntime::new(state.pty_root.clone())
        .with_binary(state.pty_binary.to_string_lossy());
    let control_path = root.path().join("control.sock");
    let connector_path = root.path().join("connector.sock");
    let http_path = root.path().join("bootstrap.sock");
    let control_listener = tokio::net::UnixListener::bind(&control_path).unwrap();
    let connector_listener = tokio::net::UnixListener::bind(&connector_path).unwrap();
    let http_listener = tokio::net::UnixListener::bind(&http_path).unwrap();
    let provider_path = root.path().join("provider.mjs");
    fs::write(&provider_path, PROVIDER).unwrap();
    let asset = std::path::Path::new(test_env!("CARGO_MANIFEST_DIR"))
        .join("..").join(crate_name).join("hooks").join(format!("{harness}-channel.ts"));
    let identity = if canonical_identity { subject } else { "node.worker" };
    let argv = vec!["sh".into(), "-c".into(), "\"$@\" & wait".into(), "bootstrap-wrapper".into(),
        "node".into(), provider_path.to_string_lossy().into_owned(), asset.to_string_lossy().into_owned(),
        control_path.to_string_lossy().into_owned(), connector_path.to_string_lossy().into_owned(),
        http_path.to_string_lossy().into_owned(), identity.into(), harness.into()];
    runtime.spawn(&member.runtime_id, &fractal_runtime::Launch::Argv(argv), root.path(),
        &BTreeMap::new(), None, &BTreeMap::new(), None).unwrap();
    let physical = runtime.snapshot().unwrap().into_iter()
        .find(|observation| observation.name == member.runtime_id).unwrap();
    let incarnation = format!("{}:{}", physical.pid.unwrap(), physical.created_at.unwrap());
    let _physical = PhysicalFixture { runtime, name: member.runtime_id.clone(), incarnation: incarnation.clone() };
    let (control, _) = control_listener.accept().await.unwrap();
    let provider_kernel = super::super::local_peer_process(&control).unwrap();
    let provider = process(provider_kernel.pid()).unwrap();
    let mut control = BufReader::new(control);
    let mut line = String::new();
    control.read_line(&mut line).await.unwrap();
    let connector_pid: u32 = line.trim().parse().unwrap();
    let (connector, _) = connector_listener.accept().await.unwrap();
    let connector_kernel = super::super::local_peer_process(&connector).unwrap();
    assert_eq!(connector_kernel.pid(), connector_pid);
    let connector_kernel = if canonical_identity {
        connector_kernel
    } else {
        // Exercise fresh (not retained) custody with the older-kernel generation proof too.
        fractal_runtime::PeerProcessIdentity::capture_generation(connector_pid).unwrap()
    };
    let mut connector = BufReader::new(connector);
    line.clear();
    connector.read_line(&mut line).await.unwrap();
    assert_eq!(line, "paused\n");
    let stats = pty_client::query_stats_in_with_timeout(&state.pty_root, &member.runtime_id, pty_client::STATS_TIMEOUT).unwrap();
    let wrapper = process(u32::try_from(stats.process.pid.unwrap()).unwrap()).unwrap();
    assert_eq!(provider.parent, wrapper.pid);
    assert_eq!(process(connector_pid).unwrap().parent, provider.pid);
    let fence = Fence::new(subject, &incarnation, "delivery");
    let dir = source_agent_dir(&state, &fence);
    fractal_drivers::harness_events::enable(&dir, &incarnation).unwrap();
    let session = "declared-bootstrap-session";
    let sequence = fractal_drivers::harness_state::claim(&dir, "node.worker", harness, session).unwrap();
    // Model only the trusted launcher output, using actual physical process generations.
    // Neither the hook nor its HTTP payload can nominate these wrapper/provider roles.
    fs::write(dir.join("native-connector-process.json"), serde_json::to_vec(&json!({
        "version":1, "harness":harness, "session":session, "sequence":sequence,
        "wrapper":wrapper, "provider":provider, "connector":null,
    })).unwrap()).unwrap();
    fs::File::create(dir.join("native-connector-process.lock")).unwrap();
    let transport = if harness == "omp" { "omp-channel" } else { "pi-channel" };
    let peer = NativeDeliveryPeer { agent:subject.into(), transport, pid:connector_pid,
        archives_inbox:false, peer_process:Some(connector_kernel) };
    assert!(capture(&state, &fence, &peer).is_err(), "unregistered child has no delivery role");
    assert!(state.store.current_mailbox_fence(subject, &incarnation, "delivery").unwrap().is_none());
    let facts = json!({"runtime_incarnation":incarnation,"session":session,"sequence":sequence});
    control.get_mut().write_all(format!("{facts}\n").as_bytes()).await.unwrap();
    let mut servers = Vec::new();
    for _ in 0..2 {
        let (http, _) = http_listener.accept().await.unwrap();
        let accepted = super::super::local_peer_process(&http).unwrap();
        assert_eq!(accepted.pid(), provider_kernel.pid());
        let accepted = if canonical_identity {
            accepted
        } else {
            fractal_runtime::PeerProcessIdentity::capture_generation(accepted.pid()).unwrap()
        };
        let local_peer = super::super::LocalPeer { pid:accepted.pid(), peer_process:Some(accepted), ancestor:None };
        let app = axum::Router::new().route("/v1/native-connectors/bootstrap", post(bootstrap))
            .with_state(state.clone()).layer(axum::Extension(local_peer));
        let service = hyper::service::service_fn(move |request: axum::http::Request<hyper::body::Incoming>| {
            app.clone().oneshot(request.map(Body::new))
        });
        servers.push(tokio::spawn(async move {
            hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(http), service).await.unwrap();
        }));
    }
    line.clear();
    connector.read_line(&mut line).await.unwrap();
    assert_eq!(line, "ready\n", "only successful real bootstrap releases the hook's FD3 gate");
    let proof = capture(&state, &fence, &peer).unwrap();
    proof.validate().unwrap();
    assert_eq!(proof.wrapper(), Some(wrapper));
    // Registration proves custody, not an implicit mailbox lease or readiness report.
    assert!(state.store.current_mailbox_fence(subject, &incarnation, "delivery").unwrap().is_none());
    for server in servers { server.abort(); }
}

#[tokio::test]
async fn normally_declared_omp_and_pi_hooks_register_and_gain_delivery_custody() {
    tokio::time::timeout(Duration::from_secs(90), async {
        for crate_name in ["fractal-drivers", "fractal-daemon"] {
            for harness in ["omp", "pi"] {
                for canonical_identity in [false, true] {
                    declared_bootstrap(crate_name, harness, canonical_identity).await;
                }
            }
        }
    }).await.expect("isolated bootstrap fixtures must finish");
}
