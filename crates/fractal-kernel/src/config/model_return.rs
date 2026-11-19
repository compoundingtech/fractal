//! Automatic return policy. Persona membership is explicit policy data, not render metadata.
use std::collections::BTreeMap;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Overrides {
    pub enabled: Option<bool>,
    /// Minimum continuous drift duration, in seconds. Never shorter than ten minutes.
    pub max_drift: Option<u64>,
    /// Zero disables admission; one is the hard hourly ceiling.
    pub max_per_hour: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// `[model_return.fleet]` supplies defaults before persona and agent overlays.
    pub fleet: Overrides,
    pub personas: BTreeMap<String, Overrides>,
    pub agents: BTreeMap<String, Overrides>,
    /// Render personas do not grant runtime authority. This map explicitly binds policy membership.
    pub agent_personas: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Policy {
    pub enabled: bool,
    pub max_drift_ms: u128,
    pub max_per_hour: u64,
}

impl Config {
    pub fn is_default(&self) -> bool { self == &Self::default() }
    pub fn validate(&self) -> Result<()> {
        for overrides in std::iter::once(&self.fleet).chain(self.personas.values()).chain(self.agents.values()) {
            anyhow::ensure!(overrides.max_drift.is_none_or(|v| v >= 600), "model_return.max_drift must be at least 600 seconds");
            anyhow::ensure!(overrides.max_per_hour.is_none_or(|v| v <= 1), "model_return.max_per_hour cannot exceed one");
        }
        for persona in self.agent_personas.values() {
            anyhow::ensure!(self.personas.contains_key(persona), "model_return agent persona has no policy entry");
        }
        Ok(())
    }
    pub fn resolve(&self, agent: &str, declaration: &Overrides) -> Policy {
        let mut policy = Policy { enabled: true, max_drift_ms: 600_000, max_per_hour: 1 };
        let persona = self.agent_personas.get(agent).and_then(|name| self.personas.get(name));
        for overrides in std::iter::once(&self.fleet).chain(persona).chain(self.agents.get(agent)).chain(std::iter::once(declaration)) {
            if let Some(enabled) = overrides.enabled { policy.enabled = enabled; }
            if let Some(seconds) = overrides.max_drift { policy.max_drift_ms = u128::from(seconds.max(600)) * 1_000; }
            if let Some(count) = overrides.max_per_hour { policy.max_per_hour = count.min(1); }
        }
        policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_on_and_fieldwise_precedence() {
        let mut config = Config::default();
        assert_eq!(config.resolve("agent/example", &Overrides::default()), Policy { enabled: true, max_drift_ms: 600_000, max_per_hour: 1 });
        config.fleet.enabled = Some(false);
        config.fleet.max_drift = Some(900);
        config.personas.insert("worker".into(), Overrides { enabled: Some(true), ..Default::default() });
        config.agent_personas.insert("agent/example".into(), "worker".into());
        config.agents.insert("agent/example".into(), Overrides { max_per_hour: Some(0), ..Default::default() });
        assert_eq!(config.resolve("agent/example", &Overrides { enabled: Some(false), ..Default::default() }), Policy { enabled: false, max_drift_ms: 900_000, max_per_hour: 0 });
    }
    #[test]
    fn rejects_storm_configuration() {
        let mut config = Config::default();
        config.fleet.max_per_hour = Some(2);
        assert!(config.validate().is_err());
        config.fleet.max_per_hour = Some(1);
        config.fleet.max_drift = Some(599);
        assert!(config.validate().is_err());
    }
    #[test]
    fn configuration_tables_round_trip_and_reject_unknown_fields() {
        let config: Config = toml::from_str("[fleet]\nenabled=false\nmax_drift=900\n[personas.worker]\nmax_per_hour=0\n[agent_personas]\n\"agent/example\"=\"worker\"\n[agents.\"agent/example\"]\nenabled=true\n").unwrap();
        config.validate().unwrap();
        assert_eq!(config.resolve("agent/example", &Overrides::default()), Policy { enabled:true, max_drift_ms:900_000, max_per_hour:0 });
        let encoded = toml::to_string(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&encoded).unwrap(), config);
        assert!(toml::from_str::<Config>("[fleet]\nenabeld=false\n").is_err());
    }
}
