//! Exact accepted resolution proofs for original native attention locators.
use super::*;
use super::attention_controls::AttentionControlSource;
use super::attention_history::SourceResolution;

impl Store {
    pub fn attention_source_resolution(
        &self, source: &AttentionControlSource<'_>, person: &str, native_id: &str,
    ) -> Result<Option<SourceResolution>, St3Error> {
        if !person.starts_with("person/") || [source.source_id, source.kind, source.episode, native_id]
            .iter().any(|value| value.is_empty() || value.len() > 4096)
            || crate::client_values::client_attention_id(source.source_id, person, source.episode).map_err(internal)? != native_id {
            return Err(St3Error::new("validation-failed", "Attention locator does not match the native item identity"));
        }
        let connection = self.readers.get();
        match source.kind {
            "person-step" | "agent-request" => attention_work_resolution::person_step_resolution_at(&connection, source.source_id, person, source.episode),
            "human-gate" => attention_work_resolution::human_gate_resolution_at(&connection, source.source_id, person, source.episode),
            "launch-approval" => attention_launch_resolution::launch_resolution_at(&connection, source.source_id, person, source.episode),
            "revision-approval" => attention_launch_resolution::revision_resolution_at(&connection, source.source_id, person, source.episode),
            "harness-prompt" => attention_prompt_resolution::prompt_resolution_at(&connection, source.source_id, person, source.episode),
            _ => Ok(None),
        }
    }
}
