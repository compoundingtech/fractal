use super::*;
use crate::config::{LocalOnlyRefusal, NetworkOperation};

fn fixture_config(root: &Path) -> Config {
    Config {
        state_dir: root.join(".st-instance/client-control-fixture/state/fractal"),
        ..Config::default()
    }
}

#[test]
fn fixture_never_constructs_a_peer_relay_even_without_fleet_configuration() {
    let root = tempfile::tempdir().unwrap();
    let mut config = fixture_config(root.path());
    assert!(ClientRelay::from_config(&config).unwrap().is_none());
    config.peers.push(PeerConfig { name: "personal".into(), url: "http://127.0.0.1:31313".into() });
    let error = match ClientRelay::from_config(&config) {
        Err(error) => error,
        Ok(_) => panic!("fixture peer configuration must be refused"),
    };
    let refusal = error.downcast_ref::<LocalOnlyRefusal>().unwrap();
    assert_eq!(refusal.operation, NetworkOperation::PeerRelay);
    assert_eq!(refusal.code, "local-only-peer-relay-refused");
    assert!(!config.state_dir.exists());
}

#[tokio::test]
async fn fixture_worker_refuses_before_creating_state_or_loading_keys() {
    fn never_splice(_: axum::extract::ws::WebSocket, _: tokio::net::UnixStream)
        -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>>
    {
        panic!("a refused worker cannot start a peer transport");
    }
    let root = tempfile::tempdir().unwrap();
    let config = fixture_config(root.path());
    let state_dir = config.state_dir.clone();
    let error = run_worker(config, never_splice).await.unwrap_err();
    let refusal = error.downcast_ref::<LocalOnlyRefusal>().unwrap();
    assert_eq!(refusal.operation, NetworkOperation::Replication);
    assert_eq!(refusal.code, "local-only-replication-refused");
    assert!(!state_dir.exists());
}
