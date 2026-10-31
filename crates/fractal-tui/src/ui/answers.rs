//! Local answer visibility is independent of the daemon's action/stream latency.
use super::{Attention, AttentionKind, Effect, Load, Ui};

pub(super) struct PendingAnswer {
    item: Attention,
    position: usize,
    words: Option<String>,
}

impl Ui {
    /// Called by the live dispatcher before spawning the RPC, never after awaiting it.
    pub(super) fn begin_answer(&mut self, effect: &Effect) {
        let Effect::Attention { id, action, reason, answer } = effect else { return };
        if action != "work.done" || self.pending_answers.contains_key(id) {
            return;
        }
        let Some((position, item)) = self.world.attention.items().iter().enumerate()
            .find(|(_, item)| &item.id == id) else { return };
        let AttentionKind::Request { structured, .. } = &item.kind else { return };
        // A named option's label is not a draft; custom answers and requested changes are.
        let needs_words = answer.is_none() || structured.as_ref().is_some_and(|request| {
            request.answers.iter().any(|option| Some(&option.id) == answer.as_ref()
                && option.outcome.as_deref() == Some("request_changes"))
        });
        self.pending_answers.insert(id.clone(), PendingAnswer {
            item: item.clone(),
            position,
            words: needs_words.then(|| reason.clone()).flatten(),
        });
        self.hide_pending_answers();
        self.refresh_tab_statuses();
        if self.tab() == 0 {
            self.select(self.selected[0]);
        }
    }

    /// A stale stream frame must not put an in-flight answer back on screen.
    pub(super) fn hide_pending_answers(&mut self) {
        if let Load::Ready(items) = &mut self.world.attention {
            items.retain(|item| !self.pending_answers.contains_key(&item.id));
        }
    }

    /// Success means BOTH action acceptance and a complete Attention window without the card.
    /// A refusal restores the card without stealing focus from the next request's draft.
    pub(super) fn finish_answer(&mut self, id: &str, accepted: bool) {
        let Some(pending) = self.pending_answers.remove(id) else { return };
        if accepted {
            return;
        }
        self.acted.remove(id);
        if let Some(words) = pending.words {
            self.conversation_state.drafts.entry(id.to_owned()).or_insert(words);
        }
        let chosen = self.list_selection(0);
        match &mut self.world.attention {
            Load::Ready(items) => {
                if !items.iter().any(|item| item.id == id) {
                    items.insert(pending.position.min(items.len()), pending.item);
                }
            }
            attention => *attention = Load::Ready(vec![pending.item]),
        }
        self.refresh_tab_statuses();
        if let Some(chosen) = chosen
            && let Some(position) = self.listing_for(0, 40).ids.iter().position(|id| id == &chosen)
        {
            self.selected[0] = position;
        }
        self.focus_seen = self.focus();
        self.flash("Could not confirm answer · request restored; check before sending again");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{demo, doc::Hit, view::Tier};

    fn requests() -> Ui {
        let mut world = demo::test_world();
        let kind = AttentionKind::Request {
            from: "Asker".into(), from_id: "agent/example/asker".into(),
            question: "Choose".into(), images: Vec::new(),
            structured: Some(Box::new(fractal_client::StructuredRequest {
                version: 1, entry_type: "choice".into(), question: "Choose".into(),
                why_person: String::new(), summary: None, reasons: Vec::new(),
                recommendation: None, subjects: Vec::new(), custom: true,
                attachments: Vec::new(),
                answers: vec![fractal_client::RequestAnswerOption {
                    id: "yes".into(), label: "Yes".into(), ..Default::default()
                }],
            })),
        };
        let item = Attention {
            deadline: None,
            requested_at: None,
            id: "attention/first".into(),
            tier: Tier::Stopped,
            title: "Choose".into(),
            waiting: None,
            age: "1h".into(),
            mission: None,
            agent: Some("agent/example/asker".into()),
            kind,
            actions: vec!["work.done".into()],
            related: Vec::new(),
            raised_by: None,
            blocked: None,
            conversations: Vec::new(),
        };
        let mut next = item.clone();
        next.id = "attention/next".into();
        world.attention = Load::Ready(vec![item, next]);
        let mut ui = Ui::new(world);
        ui.live = true;
        ui.open_home();
        ui.select(0);
        ui
    }

    #[test]
    fn answer_advances_before_rpc_completion_and_rolls_back_on_failure() {
        let mut ui = requests();
        let stale = ui.world.clone();
        ui.click(Hit::Answer(0));
        ui.click(Hit::Enter);
        let effect = ui.effects.pop().expect("Enter queued an answer");
        // The live dispatcher calls this BEFORE spawning the RPC. Withhold its completion:
        // there is deliberately no daemon response or authoritative window to advance on.
        ui.note_acted(&effect);
        assert_eq!(ui.selected_id().as_deref(), Some("attention/next"));
        assert_eq!(ui.world.attention.items().len(), 1);
        assert_eq!(ui.pending_answers.len(), 1);
        ui.set_world(stale);
        assert_eq!(ui.selected_id().as_deref(), Some("attention/next"));
        assert_eq!(ui.world.attention.items().len(), 1, "old stream frames stay hidden");
        ui.conversation_state.drafts.insert("attention/next".into(), "new draft".into());
        ui.finish_answer("attention/first", false);
        assert_eq!(ui.world.attention.items().len(), 2);
        assert_eq!(ui.selected_id().as_deref(), Some("attention/next"), "rollback does not steal focus");
        assert_eq!(ui.conversation_state.drafts["attention/next"], "new draft");
        assert!(ui.pending_answers.is_empty());
        assert!(!ui.acted.contains("attention/first"));
        assert!(ui.flash.as_ref().unwrap().0.contains("Could not confirm answer"));
    }

    #[test]
    fn confirmed_answer_stays_gone_and_custom_words_survive_failure() {
        let mut ui = requests();
        let effect = Effect::Attention {
            id: "attention/first".into(), action: "work.done".into(),
            reason: Some("my answer".into()), answer: None,
        };
        ui.note_acted(&effect);
        ui.finish_answer("attention/first", false);
        assert_eq!(ui.conversation_state.drafts["attention/first"], "my answer");
        ui.note_acted(&effect);
        // Live calls finish(true) only after BOTH the RPC and complete window agree.
        let confirmed = ui.world.clone();
        ui.finish_answer("attention/first", true);
        ui.set_world(confirmed);
        assert!(ui.pending_answers.is_empty());
        assert!(!ui.world.attention.items().iter().any(|item| item.id == "attention/first"));
        assert!(!ui.closed.contains("attention/first"));
    }
}
