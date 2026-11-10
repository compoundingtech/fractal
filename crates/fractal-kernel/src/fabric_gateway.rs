//! The client gateway over Fabric: `st devices fabric enable` exposes this member's paired-only
//! client socket as one Fabric service, and from then on pairing a device that sends its Fabric
//! NodeID grants that NodeID exactly that service, inbound-only. Revoking the pairing removes the
//! grant. st names every peer it adds `st-device-…` and never changes or removes a peer it did not
//! add, so a grant someone made by hand stays theirs.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::fleet::transport::Fabric;

/// The service name `enable` uses when none is given. Both native client URI
/// spellings dial this service; accepting `fractal-client-v0` does not change exposure.
pub const DEFAULT_SERVICE: &str = "st3-client-v0";
/// D-10's isolated instance and its exact paired-client Fabric service.
pub const FIXTURE_SERVICE: &str = crate::config::CLIENT_CONTROL_FIXTURE;
const PEER_PREFIX: &str = "st-device-";
const FIXTURE_PEER_PREFIX: &str = "st-fixture-device-";

/// Resolve the exposure without allowing the fixture to replace production's service.
pub fn service_for_instance(instance: Option<&str>, requested: Option<&str>) -> Result<String> {
    let fixture = instance == Some(FIXTURE_SERVICE);
    let service = requested.unwrap_or(if fixture { FIXTURE_SERVICE } else { DEFAULT_SERVICE });
    anyhow::ensure!(!service.trim().is_empty(), "the Fabric service name must not be empty");
    anyhow::ensure!(
        if fixture { service == FIXTURE_SERVICE } else { service != FIXTURE_SERVICE },
        "client-control-fixture must use its own Fabric service, never the production service"
    );
    Ok(service.to_owned())
}

/// Select only the paired gateway, preserving production's legacy-name discovery.
pub fn client_socket(config: &crate::config::Config) -> Result<PathBuf> {
    config.validate()?;
    let socket = if config.local_only() {
        config.client_gateway_socket.clone()
    } else {
        fractal_socket_discovery::default_unix_socket(
            &config.client_gateway_socket,
            &config.client_gateway_socket,
        )
    };
    let resolve = |path: &Path| fs::canonicalize(path).or_else(|_| std::path::absolute(path));
    let resolved = resolve(&socket)?;
    for privileged in [&config.socket, &crate::config::state_socket(&config.state_dir)] {
        anyhow::ensure!(resolved != resolve(privileged)?, "Fabric must never expose a privileged socket");
    }
    Ok(socket)
}

/// `STATE/fabric-gateway.toml`, written by `st devices fabric enable`. Its presence is the switch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayExposure {
    /// The Fabric service the client socket is exposed under.
    pub service: String,
    /// The `fabric` executable `enable` found, so the daemon does not depend on its own PATH.
    pub fabric: PathBuf,
}

impl GatewayExposure {
    pub fn path(state_dir: &Path) -> PathBuf {
        state_dir.join("fabric-gateway.toml")
    }

    pub fn load(state_dir: &Path) -> Result<Option<Self>> {
        let path = Self::path(state_dir);
        match fs::read_to_string(&path) {
            Ok(text) => {
                let exposure: Self = toml::from_str(&text)
                    .with_context(|| format!("parse {}", path.display()))?;
                let fixture = crate::config::NetworkPolicy::for_state_dir(state_dir).is_local_only();
                service_for_instance(fixture.then_some(FIXTURE_SERVICE), Some(&exposure.service))?;
                Ok(Some(exposure))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn save(&self, state_dir: &Path) -> Result<()> {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        fs::create_dir_all(state_dir).with_context(|| format!("create {}", state_dir.display()))?;
        let path = Self::path(state_dir);
        let temporary = path.with_extension(format!("toml.{}.tmp", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .with_context(|| format!("write {}", temporary.display()))?;
        file.write_all(toml::to_string(self)?.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, &path).with_context(|| format!("write {}", path.display()))
    }

    pub fn remove(state_dir: &Path) -> Result<()> {
        match fs::remove_file(Self::path(state_dir)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }

    fn fabric(&self) -> Fabric {
        Fabric::new(self.fabric.clone())
    }
}

/// A Fabric NodeID as the phone sends it: 64 lowercase hex characters.
pub fn valid_node_id(node: &str) -> bool {
    node.len() == 64
        && node
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// The local peer name st gives `node`. One name per NodeID, so re-pairing the same phone
/// updates its one entry.
pub fn peer_name(node: &str) -> String {
    format!("{PEER_PREFIX}{}", &node[..12])
}

fn service_peer_name(service: &str, node: &str) -> String {
    if service == FIXTURE_SERVICE {
        format!("{FIXTURE_PEER_PREFIX}{}", &node[..12])
    } else {
        peer_name(node)
    }
}

/// What pairing did about the device's Fabric grant, as the paired session reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Grant {
    /// The device may now reach the client gateway over Fabric as `peer`.
    Granted {
        service: String,
        peer: String,
    },
    /// Someone already trusts this NodeID under `peer`; st left that entry alone.
    AlreadyTrusted {
        peer: String,
    },
    /// This member does not expose its client gateway over Fabric.
    NotEnabled,
    /// The person who made this pairing code did not ask for a Fabric grant
    /// (`st devices pair --fabric-gateway`).
    NotRequested,
    Failed {
        detail: String,
    },
}

impl Grant {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Granted { service, peer } => {
                json!({"state": "granted", "service": service, "peer": peer})
            }
            Self::AlreadyTrusted { peer } => json!({"state": "already-trusted", "peer": peer}),
            Self::NotEnabled => json!({"state": "not-enabled"}),
            Self::NotRequested => json!({"state": "not-requested"}),
            Self::Failed { detail } => json!({"state": "failed", "detail": detail}),
        }
    }
}

fn failed(error: anyhow::Error) -> Grant {
    let mut detail = format!("{error:#}");
    if detail.len() > 300 {
        let mut end = 300;
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
    }
    Grant::Failed { detail }
}

/// Grant `node` the client gateway service, inbound-only, if this member exposes it.
pub async fn grant(state_dir: &Path, node: &str) -> Grant {
    let exposure = match GatewayExposure::load(state_dir) {
        Ok(Some(exposure)) => exposure,
        Ok(None) => return Grant::NotEnabled,
        Err(error) => return failed(error),
    };
    let fabric = exposure.fabric();
    let name = service_peer_name(&exposure.service, node);
    let peers = match fabric.peers().await {
        Ok(peers) => peers,
        Err(error) => return failed(error),
    };
    if let Some((_, existing)) = peers.iter().find(|(id, _)| id == node)
        && *existing != name
    {
        return Grant::AlreadyTrusted {
            peer: existing.clone(),
        };
    }
    // `fabric add` replaces a peer of the same name: never let it take another NodeID's entry.
    if peers
        .iter()
        .any(|(id, existing)| *existing == name && id != node)
    {
        return Grant::Failed {
            detail: format!("the Fabric peer name {name} already belongs to another NodeID"),
        };
    }
    match fabric
        .add_inbound_only(node, &name, &exposure.service)
        .await
        .context("fabric add --inbound-only failed; it needs fabric 0.2.41 or later")
    {
        Ok(()) => Grant::Granted {
            service: exposure.service,
            peer: name,
        },
        Err(error) => failed(error),
    }
}

/// Remove the grant st made for `node`. Leaves any peer st did not add. `Ok(None)` when there
/// was nothing of st's to remove.
pub async fn revoke(state_dir: &Path, node: &str) -> Result<Option<String>> {
    let exposure = GatewayExposure::load(state_dir)?;
    let fabric = match &exposure {
        Some(exposure) => exposure.fabric(),
        None => match crate::fleet::transport::resolve_tool(None, "fabric") {
            Some(program) => Fabric::new(program),
            None => return Ok(None),
        },
    };
    let fallback = if crate::config::NetworkPolicy::for_state_dir(state_dir).is_local_only() {
        FIXTURE_SERVICE
    } else {
        DEFAULT_SERVICE
    };
    let service = exposure.as_ref().map_or(fallback, |exposure| exposure.service.as_str());
    let name = service_peer_name(service, node);
    if !fabric
        .peers()
        .await?
        .iter()
        .any(|(id, existing)| id == node && *existing == name)
    {
        return Ok(None);
    }
    fabric.remove(&name).await?;
    Ok(Some(name))
}

/// Every peer st added for this gateway's devices, never another instance's.
pub async fn granted_peers(fabric: &Fabric, service: &str) -> Result<Vec<String>> {
    Ok(fabric
        .peers()
        .await?
        .into_iter()
        .filter(|(node, name)| valid_node_id(node) && *name == service_peer_name(service, node))
        .map(|(_, name)| name)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    const PHONE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const OTHER: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    #[test]
    fn fixture_and_production_services_cannot_replace_each_other() {
        assert_eq!(service_for_instance(None, None).unwrap(), DEFAULT_SERVICE);
        assert_eq!(service_for_instance(Some(FIXTURE_SERVICE), None).unwrap(), FIXTURE_SERVICE);
        assert_eq!(service_for_instance(Some(FIXTURE_SERVICE), Some(FIXTURE_SERVICE)).unwrap(), FIXTURE_SERVICE);
        assert!(service_for_instance(Some(FIXTURE_SERVICE), Some(DEFAULT_SERVICE)).is_err());
        assert!(service_for_instance(Some(FIXTURE_SERVICE), Some("fractal-client-v0")).is_err());
        assert!(service_for_instance(None, Some(FIXTURE_SERVICE)).is_err());
        assert!(service_for_instance(Some("other"), Some(FIXTURE_SERVICE)).is_err());
    }

    #[test]
    fn fixture_exposure_rejects_privileged_socket_aliases() {
        let root = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::default();
        config.state_dir = root.path().join(".st-instance/client-control-fixture/state/st3");
        config.socket = root.path().join("private.sock");
        config.client_gateway_socket = root.path().join("paired.sock");
        assert_eq!(client_socket(&config).unwrap(), config.client_gateway_socket);
        fs::write(&config.socket, "").unwrap();
        std::os::unix::fs::symlink(&config.socket, &config.client_gateway_socket).unwrap();
        assert!(client_socket(&config).is_err());
        fs::remove_file(&config.client_gateway_socket).unwrap();
        fs::create_dir_all(&config.state_dir).unwrap();
        let state_socket = crate::config::state_socket(&config.state_dir);
        fs::write(&state_socket, "").unwrap();
        std::os::unix::fs::symlink(&state_socket, &config.client_gateway_socket).unwrap();
        assert!(client_socket(&config).is_err());
    }

    #[test]
    fn fixture_settings_cannot_load_a_production_exposure() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".st-instance/client-control-fixture/state/st3");
        GatewayExposure { service: DEFAULT_SERVICE.into(), fabric: "/unused/fabric".into() }.save(&state).unwrap();
        assert!(GatewayExposure::load(&state).is_err());
    }

    #[tokio::test]
    async fn fixture_grants_and_cleanup_never_modify_normal_device_peers() {
        let root = tempfile::tempdir().unwrap();
        let log = enable(root.path(), &[(OTHER, "st-device-fedcba987654")]);
        let mut exposure = GatewayExposure::load(root.path()).unwrap().unwrap();
        exposure.service = FIXTURE_SERVICE.into();
        let fixture_state = root.path().join(".st-instance/client-control-fixture/state/st3");
        exposure.save(&fixture_state).unwrap();
        assert_eq!(grant(&fixture_state, PHONE).await, Grant::Granted {
            service: FIXTURE_SERVICE.into(),
            peer: "st-fixture-device-0123456789ab".into(),
        });
        assert!(calls(&log).ends_with(&format!(
            "add {PHONE} st-fixture-device-0123456789ab --inbound-only --allow {FIXTURE_SERVICE}\n"
        )));
        assert_eq!(revoke(&fixture_state, OTHER).await.unwrap(), None);
        assert!(granted_peers(&exposure.fabric(), FIXTURE_SERVICE).await.unwrap().is_empty());
        assert_eq!(granted_peers(&exposure.fabric(), DEFAULT_SERVICE).await.unwrap(), ["st-device-fedcba987654"]);
        assert_eq!(grant(&fixture_state, OTHER).await, Grant::AlreadyTrusted {
            peer: "st-device-fedcba987654".into(),
        });
        assert!(!calls(&log).contains("remove "));
        let log = enable(root.path(), &[
            (PHONE, "st-fixture-device-0123456789ab"),
            (OTHER, "st-device-fedcba987654"),
        ]);
        assert_eq!(granted_peers(&exposure.fabric(), DEFAULT_SERVICE).await.unwrap(), ["st-device-fedcba987654"]);
        assert_eq!(revoke(&fixture_state, PHONE).await.unwrap().as_deref(), Some("st-fixture-device-0123456789ab"));
        assert!(calls(&log).ends_with("remove st-fixture-device-0123456789ab\n"));
        assert!(!calls(&log).contains("remove st-device-"));
    }

    /// A fake `fabric` whose `peers --json` lists `peers`, as NodeID and name, and which logs
    /// every call.
    fn enable(root: &Path, peers: &[(&str, &str)]) -> PathBuf {
        let program = root.join("fabric");
        let log = root.join("calls");
        let peers = json!({
            "peers": peers
                .iter()
                .map(|(node, name)| json!({"node": node, "name": name, "allow": ["demo-client/0"]}))
                .collect::<Vec<_>>()
        });
        fs::write(
            &program,
            format!(
                "#!/bin/sh\necho \"$@\" >> {log}\ncase \"$1\" in peers) echo '{peers}';; esac\n",
                log = log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        GatewayExposure {
            service: "demo-client/0".into(),
            fabric: program,
        }
        .save(root)
        .unwrap();
        log
    }

    fn calls(log: &Path) -> String {
        fs::read_to_string(log).unwrap_or_default()
    }

    #[test]
    fn gateway_settings_read_both_service_names_without_rewriting() {
        let root = tempfile::tempdir().unwrap();
        for service in ["st3-client-v0", "fractal-client-v0"] {
            let text = format!("service = '{service}'\nfabric = '/opt/fabric'\n");
            fs::write(GatewayExposure::path(root.path()), &text).unwrap();
            let exposure = GatewayExposure::load(root.path()).unwrap().unwrap();
            assert_eq!(exposure.service, service);
            assert_eq!(exposure.fabric, PathBuf::from("/opt/fabric"));
            assert_eq!(fs::read_to_string(GatewayExposure::path(root.path())).unwrap(), text);
        }
        assert_eq!(DEFAULT_SERVICE, "st3-client-v0");
    }

    #[test]
    fn node_ids_and_peer_names() {
        assert!(valid_node_id(PHONE));
        assert!(!valid_node_id(&PHONE.to_uppercase()));
        assert!(!valid_node_id(&PHONE[1..]));
        assert_eq!(peer_name(PHONE), "st-device-0123456789ab");
    }

    #[tokio::test]
    async fn nothing_happens_until_the_gateway_is_enabled() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(grant(root.path(), PHONE).await, Grant::NotEnabled);
    }

    #[tokio::test]
    async fn pairing_grants_exactly_the_gateway_inbound_only_and_revoking_removes_it() {
        let root = tempfile::tempdir().unwrap();
        let log = enable(root.path(), &[]);
        assert_eq!(
            grant(root.path(), PHONE).await,
            Grant::Granted {
                service: "demo-client/0".into(),
                peer: "st-device-0123456789ab".into()
            }
        );
        assert_eq!(
            calls(&log),
            format!(
                "peers --json\nadd {PHONE} st-device-0123456789ab --inbound-only --allow demo-client/0\n"
            )
        );

        let log = enable(root.path(), &[(PHONE, "st-device-0123456789ab")]);
        fs::remove_file(&log).ok();
        assert_eq!(
            revoke(root.path(), PHONE).await.unwrap().as_deref(),
            Some("st-device-0123456789ab")
        );
        assert_eq!(calls(&log), "peers --json\nremove st-device-0123456789ab\n");
    }

    #[tokio::test]
    async fn a_peer_st_did_not_add_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let log = enable(root.path(), &[(PHONE, "demo-phone")]);
        assert_eq!(
            grant(root.path(), PHONE).await,
            Grant::AlreadyTrusted {
                peer: "demo-phone".into()
            }
        );
        assert_eq!(revoke(root.path(), PHONE).await.unwrap(), None);
        assert_eq!(calls(&log), "peers --json\npeers --json\n");
    }

    #[tokio::test]
    async fn a_name_taken_by_another_node_is_never_replaced() {
        let root = tempfile::tempdir().unwrap();
        let log = enable(root.path(), &[(OTHER, "st-device-0123456789ab")]);
        assert!(matches!(
            grant(root.path(), PHONE).await,
            Grant::Failed { .. }
        ));
        assert_eq!(calls(&log), "peers --json\n");
    }
}
