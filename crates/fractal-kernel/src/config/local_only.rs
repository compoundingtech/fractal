//! The fixture instance is an isolation boundary, not an operator-toggleable fleet member.
use std::path::Path;

use serde::Serialize;

use super::Config;

pub const CLIENT_CONTROL_FIXTURE: &str = "client-control-fixture";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkPolicy {
    FleetCapable,
    LocalOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkOperation {
    FleetJoin,
    Replication,
    PeerRelay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LocalOnlyRefusal {
    pub code: &'static str,
    pub operation: NetworkOperation,
}

impl std::fmt::Display for LocalOnlyRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: the client-control-fixture instance is permanently local-only", self.code)
    }
}

impl std::error::Error for LocalOnlyRefusal {}

impl NetworkPolicy {
    /// The instance environment is set before threads start. Recognize its state path too so
    /// an explicit config or service invocation cannot turn the fixture into a fleet member.
    pub fn for_state_dir(state_dir: &Path) -> Self {
        let instance = crate::instance::current();
        Self::for_instance(instance.as_ref().map(crate::instance::Instance::name), state_dir)
    }

    fn for_instance(instance: Option<&str>, state_dir: &Path) -> Self {
        let fixture_path = state_dir.ancestors().any(|path| {
            path.file_name().is_some_and(|name| name == CLIENT_CONTROL_FIXTURE)
                && path.parent().and_then(Path::file_name).is_some_and(|name| {
                    name == ".st-instance" || name == ".fractal-instance"
                })
        });
        if instance == Some(CLIENT_CONTROL_FIXTURE) || fixture_path {
            Self::LocalOnly
        } else {
            Self::FleetCapable
        }
    }

    pub fn is_local_only(self) -> bool {
        self == Self::LocalOnly
    }

    pub fn require(self, operation: NetworkOperation) -> Result<(), LocalOnlyRefusal> {
        if !self.is_local_only() {
            return Ok(());
        }
        Err(LocalOnlyRefusal {
            code: match operation {
                NetworkOperation::FleetJoin => "local-only-fleet-join-refused",
                NetworkOperation::Replication => "local-only-replication-refused",
                NetworkOperation::PeerRelay => "local-only-peer-relay-refused",
            },
            operation,
        })
    }

    pub fn validate_config(self, config: &Config) -> Result<(), LocalOnlyRefusal> {
        if config.fleet_id.is_some() || config.fleet.is_some() {
            self.require(NetworkOperation::FleetJoin)?;
        }
        if config.peer_listen.is_some() || config.shared_secret_file.is_some() {
            self.require(NetworkOperation::Replication)?;
        }
        if !config.peers.is_empty() {
            self.require(NetworkOperation::PeerRelay)?;
        }
        Ok(())
    }
}

impl Config {
    pub fn network_policy(&self) -> NetworkPolicy {
        NetworkPolicy::for_state_dir(&self.state_dir)
    }

    pub fn local_only(&self) -> bool {
        self.network_policy().is_local_only()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_instance_cannot_escape_with_an_explicit_state_directory() {
        assert_eq!(NetworkPolicy::for_instance(Some(CLIENT_CONTROL_FIXTURE), Path::new("/tmp/elsewhere")), NetworkPolicy::LocalOnly);
        for root in [".st-instance", ".fractal-instance"] {
            assert_eq!(NetworkPolicy::for_instance(None, &Path::new("/home/operator").join(root).join(CLIENT_CONTROL_FIXTURE).join("state/fractal")), NetworkPolicy::LocalOnly);
        }
        assert_eq!(NetworkPolicy::for_instance(Some("other"), Path::new("/tmp/state")), NetworkPolicy::FleetCapable);
        assert_eq!(NetworkPolicy::for_instance(None, Path::new("/tmp/client-control-fixture/state")), NetworkPolicy::FleetCapable);
    }

    #[test]
    fn each_network_operation_has_a_typed_refusal() {
        for (operation, code) in [
            (NetworkOperation::FleetJoin, "local-only-fleet-join-refused"),
            (NetworkOperation::Replication, "local-only-replication-refused"),
            (NetworkOperation::PeerRelay, "local-only-peer-relay-refused"),
        ] {
            assert_eq!(NetworkPolicy::LocalOnly.require(operation), Err(LocalOnlyRefusal { code, operation }));
            assert!(NetworkPolicy::FleetCapable.require(operation).is_ok());
        }
    }

    #[test]
    fn stale_or_injected_network_configuration_is_refused() {
        let mut config = Config::default();
        assert!(NetworkPolicy::LocalOnly.validate_config(&config).is_ok());
        config.fleet_id = Some("personal-fleet".into());
        assert_eq!(NetworkPolicy::LocalOnly.validate_config(&config).unwrap_err().operation, NetworkOperation::FleetJoin);
        config.fleet_id = None;
        config.peer_listen = Some("127.0.0.1:31313".into());
        assert_eq!(NetworkPolicy::LocalOnly.validate_config(&config).unwrap_err().operation, NetworkOperation::Replication);
        config.peer_listen = None;
        config.peers.push(super::super::PeerConfig { name: "personal".into(), url: "http://127.0.0.1:31313".into() });
        assert_eq!(NetworkPolicy::LocalOnly.validate_config(&config).unwrap_err().operation, NetworkOperation::PeerRelay);
    }
}
