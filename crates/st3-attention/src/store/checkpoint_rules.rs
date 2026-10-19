//! The part of `store::checkpoint_rules` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use smallclaims::store::checkpoint::DAY_MS;
use smallclaims::store::checkpoint::*;
pub use st3_store::store::checkpoint_rules::staged::plan_staged;
pub use st3_store::store::checkpoint_rules::*;
pub use st3_store::store::checkpoint_rules::{HARNESS_OPTIONAL_FIELDS, PROJECTION_TABLES, REQUEST_CLOSERS, RULES_VERSION, Rule, TimingEvent, USAGE_WINDOW_MS, carriers, cited_by, claim_facts, clear_projections, expired_questions};
pub use st3_store::store::checkpoint_rules::{field_carriers, field_names, field_str, field_text, fields, global_answers, guarded_kind, harness_keep, limits_keep, limits_reading, loop_keep, plan_drops};
pub use st3_store::store::checkpoint_rules::{question_expired, question_kind, replay_from_nothing, rule_keep, rules_description, rules_digest, slot_from_text, slot_of, slot_text, status_keep_positions, status_source, subject_answers};
pub use st3_store::store::checkpoint_rules::{subject_answers_with_sources, timing_answers, timing_event, usage_cumulative_keep, usage_series_keep, witnessed, witnessed_names};

pub trait CheckpointRulesStore {
    /// The first checkpoint after the newest stable one that is still not stable
    /// `CHECKPOINT_ATTENTION_AFTER_MS` after it became due. The participant that sorts first
    /// among those that sealed asks the person to bring the others back or excuse them.
    fn checkpoint_attention_items(
            &self,
            person: Option<&str>,
            as_of: u128,
        ) -> Result<Vec<AttentionItemView>>;
}

impl CheckpointRulesStore for Store {

    fn checkpoint_attention_items(
            &self,
            person: Option<&str>,
            as_of: u128,
        ) -> Result<Vec<AttentionItemView>> {
        if person.is_some_and(|person| person != "person/operator") {
            return Ok(Vec::new());
        }
        let claims = self.checkpoint_claims()?;
        let newest_stable = stable_checkpoints(&claims).keys().next_back().copied();
        let due = newest_due_cut(as_of);
        let cut = claims
            .iter()
            .filter_map(CheckpointClaim::seal_terms)
            .map(|terms| terms.cut_unix_ms)
            .filter(|cut| *cut <= due && newest_stable.is_none_or(|stable| *cut > stable))
            .max();
        let Some(cut) = cut else {
            return Ok(Vec::new());
        };
        let checkpoint = checkpoint_name(cut);
        let seals = newest_seals(&claims, &checkpoint);
        let (current, _) = self.checkpoint_participants(&claims, &[])?;
        let runtime_rules = rules_digest();
        let reference = seals.iter().find(|(writer, terms)| {
            current.contains(*writer) && terms.rules_digest == runtime_rules
        }).or_else(|| seals.iter().find(|(writer, _)| current.contains(*writer)));
        let Some((writer, terms)) = reference else {
            return Ok(Vec::new());
        };
        let first_waiting = newest_stable.map_or_else(
            || {
                claims
                    .iter()
                    .filter_map(CheckpointClaim::seal_terms)
                    .map(|terms| terms.cut_unix_ms)
                    .min()
                    .unwrap_or(cut)
            },
            |stable| stable + DAY_MS,
        );
        let since = first_waiting + 2 * DAY_MS;
        if as_of < since + CHECKPOINT_ATTENTION_AFTER_MS {
            return Ok(Vec::new());
        }
        let waiting = terms
            .participants
            .intersection(&current)
            .filter(|writer| seals.get(*writer) != Some(terms))
            .cloned()
            .collect::<Vec<_>>();
        if waiting.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![AttentionItemView {
            episode: format!("{}:{writer}", checkpoint_name(first_waiting)), priority: "normal".into(), kind: "fault".into(),
            review_mode: None, subject: checkpoint.clone(), person: "person/operator".into(), requester_id: None,
            launch_id: None, variant_id: None, message_id: None, prompt_call: None, answers: Vec::new(), conversation: None, title: format!("Checkpoints are waiting for {}", waiting.join(", ")),
            detail: "Bring the waiting machines back, upgrade them, or excuse a machine that stays away.".into(),
            mission: None, mission_run: None, step: None, targets: vec![checkpoint.clone()], requested_at_unix_ms: since,
            actions: vec![attention_action("inspect checkpoint", &["st", "replication", "checkpoint", "status"])],
            request: None,
        }])
    }
}

