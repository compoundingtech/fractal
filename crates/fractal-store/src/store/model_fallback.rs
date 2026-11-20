//! Sanitized native retry-fallback evidence, fenced by incarnation, native session and the
//! latest native model-change entry. Provider error strings are never part of this contract.

use super::*;

pub const KIND: &str = "harness.model-fallback.observed";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelFallbackCause {
    Transient,
    Quota,
    Auth,
    Unknown,
}

impl ModelFallbackCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Quota => "quota",
            Self::Auth => "auth",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFallbackObservation {
    pub cause: ModelFallbackCause,
    pub from: String,
    pub to: String,
    pub observed_at_ms: u128,
    pub native_session: String,
    pub episode_id: String,
}

impl Store {
    /// At most 64 indexed claims. Missing selection or binding evidence is unknown, never an
    /// invitation to borrow an older session's cause. A manual switch gets a new source event
    /// even when its destination happens to be the same model as the previous fallback.
    pub fn latest_model_fallback(&self, agent: &str, incarnation: &str) -> Result<Option<ModelFallbackObservation>> {
        let Some(binding) = self.latest_observation(agent, "harness.session-file")? else { return Ok(None) };
        let Some(binding) = binding.body.get("fields") else { return Ok(None) };
        if binding["incarnation_id"].as_str() != Some(incarnation)
            || binding["harness"].as_str() != Some("omp")
        { return Ok(None); }
        if matches!(binding["status"].as_str(), Some("inactive" | "missing" | "unknown")) {
            return Ok(None);
        }
        let Some(native) = binding["session_id"].as_str().filter(|id| !id.is_empty()) else { return Ok(None) };
        let page = self.claims_for_subject_kind_at(agent, KIND, None, true, 64)?;
        let current = |claim: &&ClaimRecord| claim.body.pointer("/fields/incarnation_id").and_then(Value::as_str) == Some(incarnation)
            && claim.body.pointer("/fields/native_session").and_then(Value::as_str) == Some(native);
        let Some(selection) = page.claims.iter().filter(current)
            .find(|claim| claim.body.pointer("/fields/observation").and_then(Value::as_str) == Some("selected"))
            .and_then(|claim| claim.body.get("fields")) else { return Ok(None) };
        let applied = page.claims.iter().filter(current)
            .find(|claim| claim.body.pointer("/fields/observation").and_then(Value::as_str) == Some("applied")
                && claim.body.pointer("/fields/source_event") == selection.get("source_event")
                && claim.body.pointer("/fields/to") == selection.get("to"));
        Ok(applied.and_then(|claim| {
            let fields = claim.body.get("fields")?;
            Some(ModelFallbackObservation {
                cause: match fields.get("cause")?.as_str()? {
                    "transient" => ModelFallbackCause::Transient,
                    "quota" => ModelFallbackCause::Quota,
                    "auth" => ModelFallbackCause::Auth,
                    "unknown" => ModelFallbackCause::Unknown,
                    _ => return None,
                },
                from: fields.get("from")?.as_str()?.into(),
                to: fields.get("to")?.as_str()?.into(),
                observed_at_ms: u128::from(fields.get("observed_at_unix_ms")?.as_u64()?),
                native_session: native.into(),
                episode_id: fields.get("episode_id")?.as_str()?.into(),
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append(store: &Store, kind: &str, fields: Value, key: Option<&str>) {
        store.append_claim(&ClaimInput {
            subject: "agent/example".into(), kind: kind.into(), actor: Some("agent/example".into()),
            fields: fields.as_object().unwrap().iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: key.map(str::to_owned),
        }).unwrap();
    }

    fn binding(store: &Store, incarnation: &str, session: &str) {
        append(store, "harness.session-file", json!({"harness":"omp", "session_id":session,
            "incarnation_id":incarnation, "path":"/session/example.jsonl"}), None);
    }

    fn observation(store: &Store, state: &str, source: &str, cause: &str) {
        let mut fields = json!({"observation":state, "incarnation_id":"one", "native_session":"native-one",
            "source_event":source, "episode_id":source, "to":"provider/model-b", "observed_at_unix_ms":100,
            "reason":if state == "applied" { "native-retry-fallback" } else { "native-model-selection" }});
        if state == "applied" { fields["from"] = json!("provider/model-a"); fields["cause"] = json!(cause); }
        append(store, KIND, fields, Some(&format!("{source}:{state}")));
    }

    #[test]
    fn model_fallback_projection_is_session_incarnation_and_selection_fenced() {
        let store = Store::open_memory("node").unwrap();
        binding(&store, "one", "native-one");
        observation(&store, "selected", "change-one", "unknown");
        observation(&store, "applied", "change-one", "transient");
        let first = store.latest_model_fallback("agent/example", "one").unwrap().unwrap();
        assert_eq!(first.cause, ModelFallbackCause::Transient);
        observation(&store, "applied", "change-one", "transient");
        assert_eq!(store.latest_model_fallback("agent/example", "one").unwrap(), Some(first));
        assert_eq!(store.latest_model_fallback("agent/example", "two").unwrap(), None);
        // Same model, different native model-change entry: never attach the old cause.
        observation(&store, "selected", "manual-change", "unknown");
        assert_eq!(store.latest_model_fallback("agent/example", "one").unwrap(), None);
        binding(&store, "one", "native-two");
        assert_eq!(store.latest_model_fallback("agent/example", "one").unwrap(), None);
    }

    #[test]
    fn model_fallback_projection_preserves_all_closed_causes() {
        for cause in ["transient", "quota", "auth", "unknown"] {
            let store = Store::open_memory("node").unwrap();
            binding(&store, "one", "native-one");
            observation(&store, "selected", "change", cause);
            observation(&store, "applied", "change", cause);
            assert_eq!(store.latest_model_fallback("agent/example", "one").unwrap().unwrap().cause.as_str(), cause);
        }
    }
}
