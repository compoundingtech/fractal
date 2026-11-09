//! Local answer visibility is independent of the daemon's action/stream latency.
use super::{Attention, AttentionKind, Effect, Load, Ui};

pub(super) struct PendingAnswer {
    item: Attention,
    position: usize,
    words: Option<String>,
    answer: Option<String>,
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
        let AttentionKind::Request { .. } = &item.kind else { return };
        self.pending_answers.insert(id.clone(), PendingAnswer {
            item: item.clone(),
            position,
            words: reason.clone(),
            answer: answer.clone(),
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
        if let Some(answer) = pending.answer {
            self.note_answers.insert(id.to_owned(), answer);
        } else {
            self.note_answers.remove(id);
        }
        if let Some(words) = pending.words {
            let key = self.answer_draft_key(id);
            self.conversation_state.drafts.entry(key).or_insert(words);
        }
        self.answer_errors.insert(id.to_owned(),
            "Could not confirm answer · request restored; check before sending again".into());
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
        // The refusal belongs to this card until the person acts, not to the flash timer.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{demo, doc::Hit, view::Tier};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn press(ui: &mut Ui, code: KeyCode) {
        ui.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn note_request(outcome: Option<&str>, custom: bool) -> Ui {
        let mut ui = requests();
        if let Load::Ready(items) = &mut ui.world.attention {
            for item in items {
                if let AttentionKind::Request { structured: Some(request), .. } = &mut item.kind {
                    request.custom = custom;
                    request.entry_type = if outcome.is_some() { "decision" } else { "choice" }.into();
                    request.answers[0].outcome = outcome.map(str::to_owned);
                    let mut second = request.answers[0].clone();
                    second.id = "other".into();
                    second.label = "Other".into();
                    request.answers.push(second);
                }
            }
        }
        ui
    }

    #[test]
    fn every_named_outcome_sends_its_note_with_keyboard_and_mouse() {
        for outcome in [Some("accept"), Some("decline"), Some("request_changes"), None] {
            for mouse in [false, true] {
                let mut ui = note_request(outcome, false);
                ui.click(Hit::Answer(0));
                if !ui.editing {
                    press(&mut ui, KeyCode::Char('c'));
                }
                for letter in "with this note".chars() {
                    press(&mut ui, KeyCode::Char(letter));
                }
                ui.answering = Some(1);
                if mouse { ui.click(Hit::Enter) } else { press(&mut ui, KeyCode::Enter) }
                assert!(matches!(&ui.effects[..], [Effect::Attention {
                    id, answer: Some(answer), reason: Some(text), ..
                }] if id == "attention/first" && answer == "yes" && text == "with this note"));
            }
        }
    }

    #[test]
    fn notes_survive_highlight_changes_and_send_only_the_bound_options_note() {
        let mut ui = note_request(None, false);
        ui.click(Hit::Answer(0));
        press(&mut ui, KeyCode::Char('c'));
        press(&mut ui, KeyCode::Char('A'));
        ui.click(Hit::Answer(1));
        assert_eq!(ui.note_answers["attention/first"], "yes");
        ui.click(Hit::Key('c'));
        press(&mut ui, KeyCode::Char('B'));
        assert_eq!(ui.conversation_state.drafts["answer:attention/first:yes"], "A");
        assert_eq!(ui.conversation_state.drafts["answer:attention/first:other"], "B");
        ui.click(Hit::Enter);
        assert!(matches!(&ui.effects[..], [Effect::Attention {
            answer: Some(answer), reason: Some(text), ..
        }] if answer == "other" && text == "B"));
    }

    #[test]
    fn custom_disabled_without_a_highlight_does_not_open_an_editor() {
        let mut ui = note_request(None, false);
        press(&mut ui, KeyCode::Char('c'));
        assert!(!ui.editing);
        assert!(ui.note_answers.is_empty());
        assert!(ui.effects.is_empty());
        ui.click(Hit::Key('c'));
        assert!(!ui.editing);
    }

    #[test]
    fn custom_answer_requires_its_distinct_entry() {
        let mut ui = note_request(None, true);
        press(&mut ui, KeyCode::Char('c'));
        assert!(!ui.editing);
        ui.click(Hit::Answer(2));
        press(&mut ui, KeyCode::Enter);
        assert!(ui.editing);
        press(&mut ui, KeyCode::Char('X'));
        ui.click(Hit::Enter);
        assert!(matches!(&ui.effects[..], [Effect::Attention {
            answer: None, reason: Some(text), ..
        }] if text == "X"));
    }

    #[test]
    fn required_note_refusal_stays_inline_until_the_person_acts() {
        let mut ui = note_request(Some("request_changes"), false);
        ui.click(Hit::Answer(0));
        ui.click(Hit::Enter);
        assert!(ui.effects.is_empty());
        let error = ui.answer_errors["attention/first"].clone();
        ui.flash = None;
        ui.set_world(ui.world.clone());
        assert_eq!(ui.answer_errors["attention/first"], error);
        assert!(ui.editing);
        press(&mut ui, KeyCode::Char('x'));
        ui.click(Hit::Enter);
        assert!(!ui.answer_errors.contains_key("attention/first"));
        assert_eq!(ui.effects.len(), 1);
    }

    #[test]
    fn refused_named_note_is_restored_to_its_option_and_error_is_durable() {
        let mut ui = note_request(Some("accept"), false);
        let effect = Effect::Attention {
            id: "attention/first".into(), action: "work.done".into(),
            answer: Some("yes".into()), reason: Some("retain this".into()),
        };
        ui.note_acted(&effect);
        ui.finish_answer("attention/first", false);
        ui.flash = None;
        assert_eq!(ui.note_answers["attention/first"], "yes");
        assert_eq!(ui.conversation_state.drafts["answer:attention/first:yes"], "retain this");
        assert!(ui.answer_errors["attention/first"].contains("Could not confirm answer"));
        ui.set_world(ui.world.clone());
        assert!(ui.answer_errors.contains_key("attention/first"));
    }

    #[test]
    fn request_composer_and_send_ignore_an_inactive_chat() {
        let mut ui = note_request(None, false);
        ui.chat = Some(super::super::ChatState {
            item: "attention/first".into(),
            to: "agent/example/asker".into(),
            to_name: "Asker".into(),
            editing: false,
        });
        ui.click(Hit::Answer(0));
        ui.click(Hit::Composer);
        assert!(ui.editing);
        assert!(!ui.chat.as_ref().unwrap().editing);
        press(&mut ui, KeyCode::Char('N'));
        ui.click(Hit::Enter);
        assert!(matches!(&ui.effects[..], [Effect::Attention {
            answer: Some(answer), reason: Some(text), ..
        }] if answer == "yes" && text == "N"));
    }

    #[test]
    fn unstructured_requests_keep_the_reply_editor_and_field_reply_action() {
        for action in ["work.done", "custom.reply"] {
            for mouse in [false, true] {
                let mut ui = requests();
                if let Load::Ready(items) = &mut ui.world.attention {
                    for item in items {
                        if let AttentionKind::Request { structured, .. } = &mut item.kind {
                            *structured = None;
                            item.actions = vec![action.into()];
                        }
                    }
                }
                if mouse { ui.click(Hit::Key('c')) } else { press(&mut ui, KeyCode::Char('c')) }
                assert!(ui.editing);
                for letter in "field=value".chars() {
                    press(&mut ui, KeyCode::Char(letter));
                }
                ui.click(Hit::Enter);
                assert!(matches!(&ui.effects[..], [Effect::Attention {
                    action: sent_action, reason: Some(text), answer: None, ..
                }] if sent_action == action && text == "field=value"));
            }
        }
    }

    #[test]
    fn visible_send_submits_a_note_after_escape_or_a_refused_answer() {
        for restored in [false, true] {
            let mut ui = note_request(Some("accept"), false);
            ui.click(Hit::Answer(0));
            press(&mut ui, KeyCode::Char('c'));
            for letter in "keep this note".chars() {
                press(&mut ui, KeyCode::Char(letter));
            }
            if restored {
                ui.click(Hit::Enter);
                let effect = ui.effects.pop().unwrap();
                ui.note_acted(&effect);
                ui.finish_answer("attention/first", false);
                let first = ui.listing_for(0, 40).ids.iter()
                    .position(|id| id == "attention/first").unwrap();
                ui.select(first);
            } else {
                press(&mut ui, KeyCode::Esc);
            }
            ui.editing = false;
            ui.answering = None;
            ui.click(Hit::Enter);
            assert!(matches!(&ui.effects[..], [Effect::Attention {
                id, answer: Some(answer), reason: Some(note), ..
            }] if id == "attention/first" && answer == "yes" && note == "keep this note"));
        }
    }

    #[test]
    fn named_answer_without_a_note_passes_the_work_done_transport_boundary() {
        for outcome in [None, Some("accept"), Some("decline")] {
            let mut ui = note_request(outcome, false);
            let (_, request) = ui.structured_request().unwrap();
            ui.click(Hit::Answer(0));
            ui.click(Hit::Enter);
            let Effect::Attention { answer, reason, .. } = ui.effects.pop().unwrap() else {
                panic!("expected answer effect");
            };
            let (summary, answer) =
                crate::person_step_response(Some(&request), answer, reason).unwrap();
            assert_eq!(summary, "Yes");
            let answer = answer.unwrap();
            assert_eq!(answer.id.as_deref(), Some("yes"));
            assert_eq!(answer.text, None);
        }
    }

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
        assert!(ui.answer_errors["attention/first"].contains("Could not confirm answer"));
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
