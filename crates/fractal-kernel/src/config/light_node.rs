//! Explicit opt-in session authority. These grants are not fleet membership or deployment cutover.
use std::collections::BTreeSet;
use std::net::SocketAddr;

use anyhow::{Context as _, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

pub const LIGHT_NODE_SESSION_PATH: &str = "/v1/light-node/session";
pub const LIGHT_NODE_SESSION_MAX_FRAME: usize = 64 * 1024;
pub const LIGHT_NODE_SESSION_CAPABILITIES: [&str; 8] = ["commands-v1", "receipts-v1", "joint-checkpoints-v1", "run-effects-v1", "authority-v1", "signed-channel-v1", "start-authorizations-v1", "retained-withdrawal-v1"];

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LightNodeOperation {
    Start,
    Observe,
    Stop,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LightNodeScope {
    pub subjects: Vec<String>,
    pub operations: Vec<LightNodeOperation>,
}

impl LightNodeScope {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!((1..=256).contains(&self.subjects.len()),
            "a light-node grant needs between 1 and 256 exact subjects");
        anyhow::ensure!((1..=3).contains(&self.operations.len()),
            "a light-node grant needs explicit operations");
        let mut subjects = BTreeSet::new();
        for subject in &self.subjects {
            anyhow::ensure!(subject.len() <= 1024 && !subject.contains('*'),
                "a light-node grant subject must be bounded and exact");
            fractal_schema::registry().validate_subject(subject)
                .map_err(|error| anyhow::anyhow!("invalid light-node subject: {}", error.message))?;
            anyhow::ensure!(subjects.insert(subject), "duplicate light-node grant subject");
        }
        let operations: BTreeSet<_> = self.operations.iter().collect();
        anyhow::ensure!(operations.len() == self.operations.len(), "duplicate light-node grant operation");
        Ok(())
    }
}

/// Operator-selected authority; both ends pin these exact public identities and epoch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LightNodeGrant {
    pub home: String,
    pub home_key: String,
    pub host: String,
    pub host_key: String,
    pub epoch: u64,
    pub grant: LightNodeScope,
}

impl LightNodeGrant {
    pub fn validate(&self) -> Result<()> {
        for identity in [&self.home, &self.host] {
            anyhow::ensure!(!identity.is_empty() && identity.len() <= 255
                && identity.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
                "light-node home and host must be explicit node identities");
        }
        anyhow::ensure!(self.home != self.host, "light-node home and host must differ");
        anyhow::ensure!(self.epoch > 0 && self.epoch <= i64::MAX as u64,
            "light-node epoch must be a positive SQLite integer");
        for key in [&self.home_key, &self.host_key] {
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(key)
                .context("light-node public key must be unpadded base64url")?;
            anyhow::ensure!(bytes.len() == 32, "light-node public key must be Ed25519");
        }
        anyhow::ensure!(self.home_key != self.host_key, "light-node home and host must use distinct private identities");
        self.grant.validate()?;
        self.validate_hello_budget("")
    }

    /// Include JSON escaping, the actual fleet identity and the signed envelope, not just
    /// subject counts. Maximum integer widths make this an upper bound for the first Hello.
    /// A real signed-frame regression in the peer crate fences this wire-layout calculation.
    pub fn signed_hello_max_bytes(&self, fleet: &str) -> Result<usize> {
        #[derive(Serialize)]
        struct Hello<'a> {
            #[serde(rename = "type")]
            kind: &'static str,
            version: u32,
            selection: &'a LightNodeGrant,
            capabilities: &'static [&'static str],
        }
        #[derive(Serialize)]
        struct SignedHello<'a> {
            sequence: u64,
            frame: Hello<'a>,
            authentication: [(); 0],
        }
        let empty = SignedHello { sequence: u64::MAX, frame: Hello {
            kind: "hello", version: u32::MAX, selection: self,
            capabilities: &LIGHT_NODE_SESSION_CAPABILITIES,
        }, authentication: [] };
        let authentication = smallclaims::sync::signed_request_authentication_json_size(
            fleet, &self.host, &self.host_key)?;
        Ok(serde_json::to_vec(&empty)?.len() + authentication - 2)
    }

    pub fn validate_hello_budget(&self, fleet: &str) -> Result<()> {
        anyhow::ensure!(fleet.len() <= LIGHT_NODE_SESSION_MAX_FRAME
            && self.signed_hello_max_bytes(fleet)? <= LIGHT_NODE_SESSION_MAX_FRAME,
            "light-node grant exceeds the encoded signed Hello frame budget");
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum LightNodeSessionConfig {
    Home {
        listen: SocketAddr,
        grants: Vec<LightNodeGrant>,
    },
    Agent {
        home_endpoint: String,
        grant: LightNodeGrant,
    },
}

impl LightNodeSessionConfig {
    pub fn validate_for(&self, node: &str) -> Result<()> {
        match self {
            Self::Home { listen, grants } => {
                anyhow::ensure!(crate::fleet::transport::is_permitted_route_address(&listen.ip()),
                    "light-node session listener must be loopback or Tailscale");
                anyhow::ensure!((1..=256).contains(&grants.len()),
                    "light-node home needs between 1 and 256 explicit host grants");
                let mut hosts = BTreeSet::new();
                for grant in grants {
                    grant.validate()?;
                    anyhow::ensure!(grant.home == node, "light-node home grant names another home");
                    anyhow::ensure!(hosts.insert(&grant.host), "duplicate light-node host grant");
                }
            }
            Self::Agent { home_endpoint, grant } => {
                grant.validate()?;
                anyhow::ensure!(grant.host == node, "light-node agent grant names another host");
                let endpoint = reqwest::Url::parse(home_endpoint).context("parse light-node home endpoint")?;
                anyhow::ensure!(matches!(endpoint.scheme(), "ws" | "wss")
                    && endpoint.username().is_empty() && endpoint.password().is_none()
                    && endpoint.query().is_none() && endpoint.fragment().is_none()
                    && endpoint.path() == LIGHT_NODE_SESSION_PATH,
                    "light-node endpoint must be a credential-free websocket session URL");
                anyhow::ensure!(endpoint.host_str().is_some(), "light-node endpoint needs a host");
                if endpoint.scheme() == "ws" {
                    let address = endpoint.host_str().and_then(|host| host.trim_matches(['[', ']']).parse().ok());
                    anyhow::ensure!(address.as_ref().is_some_and(crate::fleet::transport::is_permitted_route_address),
                        "unencrypted light-node endpoint must be a literal loopback or Tailscale address");
                }
            }
        }
        Ok(())
    }

    pub fn validate_hello_budget(&self, fleet: &str) -> Result<()> {
        match self {
            Self::Home { grants, .. } => {
                for grant in grants { grant.validate_hello_budget(fleet)?; }
                Ok(())
            }
            Self::Agent { grant, .. } => grant.validate_hello_budget(fleet),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> LightNodeGrant {
        LightNodeGrant {
            home: "example-home".into(),
            home_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1; 32]),
            host: "example-seat".into(),
            host_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([2; 32]),
            epoch: 1,
            grant: LightNodeScope {
                subjects: vec!["agent/example/worker".into()],
                operations: vec![LightNodeOperation::Observe, LightNodeOperation::Stop],
            },
        }
    }

    #[test]
    fn session_configuration_pins_identity_scope_and_transport() {
        let mut authority = grant();
        authority.validate().unwrap();
        authority.grant.subjects.push(authority.grant.subjects[0].clone());
        assert!(authority.validate().is_err());
        for endpoint in [
            "ws://192.0.2.10/v1/light-node/session",
            "ws://example.invalid/v1/light-node/session",
            "wss://secret@example.invalid/v1/light-node/session",
            "wss://example.invalid/other",
        ] {
            assert!(LightNodeSessionConfig::Agent { home_endpoint: endpoint.into(), grant: grant() }
                .validate_for("example-seat").is_err());
        }
        for endpoint in ["ws://127.0.0.1:1234/v1/light-node/session", "wss://example.invalid/v1/light-node/session"] {
            let config = LightNodeSessionConfig::Agent { home_endpoint: endpoint.into(), grant: grant() };
            config.validate_for("example-seat").unwrap();
            assert!(config.validate_for("another-seat").is_err());
        }
        assert!(LightNodeSessionConfig::Home {
            listen: "0.0.0.0:1234".parse().unwrap(), grants: vec![grant()],
        }.validate_for("example-home").is_err());
    }

    #[test]
    fn start_grants_are_explicit_scopes_for_both_configuration_roles() {
        let mut authority = grant();
        authority.grant.operations.push(LightNodeOperation::Start);
        authority.validate().unwrap();
        for (node, config) in [
            ("example-home", LightNodeSessionConfig::Home {
                listen: "127.0.0.1:1234".parse().unwrap(), grants: vec![authority.clone()],
            }),
            ("example-seat", LightNodeSessionConfig::Agent {
                home_endpoint: "ws://127.0.0.1:1234/v1/light-node/session".into(), grant: authority,
            }),
        ] {
            let decoded: LightNodeSessionConfig = serde_json::from_value(serde_json::to_value(config).unwrap()).unwrap();
            decoded.validate_for(node).unwrap();
        }
    }

    #[test]
    fn individually_valid_subjects_cannot_exceed_the_encoded_hello_budget() {
        let mut authority = grant();
        authority.grant.subjects = (0..256).map(|index| {
            let prefix = format!("agent/budget-{index:03}-");
            format!("{prefix}{}", "x".repeat(300 - prefix.len()))
        }).collect();
        authority.grant.validate().unwrap();
        assert!(authority.validate().is_err());
        for (node, config) in [
            ("example-home", LightNodeSessionConfig::Home {
                listen: "127.0.0.1:1234".parse().unwrap(), grants: vec![authority.clone()],
            }),
            ("example-seat", LightNodeSessionConfig::Agent {
                home_endpoint: "ws://127.0.0.1:1234/v1/light-node/session".into(), grant: authority,
            }),
        ] {
            assert!(config.validate_for(node).is_err());
        }
    }

    #[test]
    fn both_roles_validate_actual_signed_hello_at_the_exact_encoded_boundary() {
        let fleet = "1f91ca65-7793-48cc-866e-ac15690130e1";
        let mut authority = grant();
        authority.grant.subjects.clear();
        while authority.signed_hello_max_bytes(fleet).unwrap() < LIGHT_NODE_SESSION_MAX_FRAME {
            let prefix = format!("agent/budget-{:03}-", authority.grant.subjects.len());
            authority.grant.subjects.push(prefix);
            let remaining = LIGHT_NODE_SESSION_MAX_FRAME - authority.signed_hello_max_bytes(fleet).unwrap();
            let subject = authority.grant.subjects.last_mut().unwrap();
            let padding = (512 - subject.len()).min(remaining);
            subject.push_str(&"x".repeat(padding));
        }
        assert!(authority.grant.subjects.last().unwrap().len() < 512);
        assert_eq!(authority.signed_hello_max_bytes(fleet).unwrap(), LIGHT_NODE_SESSION_MAX_FRAME);
        authority.validate_hello_budget(fleet).unwrap();
        for at_boundary in [true, false] {
            if !at_boundary { authority.grant.subjects.last_mut().unwrap().push('x'); }
            for (node, config) in [
                ("example-home", LightNodeSessionConfig::Home {
                    listen: "127.0.0.1:1234".parse().unwrap(), grants: vec![authority.clone()],
                }),
                ("example-seat", LightNodeSessionConfig::Agent {
                    home_endpoint: "ws://127.0.0.1:1234/v1/light-node/session".into(), grant: authority.clone(),
                }),
            ] {
                config.validate_for(node).unwrap();
                assert_eq!(config.validate_hello_budget(fleet).is_ok(), at_boundary);
                let complete = super::super::Config {
                    node: node.to_owned(), fleet_id: Some(fleet.to_owned()),
                    shared_secret_file: Some("unused-test-secret".into()),
                    fleet: Some(super::super::FleetFile {
                        mode: super::super::FleetMode::DialOut, ..Default::default()
                    }),
                    light_node_session: Some(config), ..Default::default()
                };
                let result = complete.validate();
                assert_eq!(result.is_ok(), at_boundary, "{result:?}");
            }
        }
    }
}
