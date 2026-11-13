//! Local answer visibility is independent of the daemon's action/stream latency.
use super::{Attention, AttentionKind, Effect, Load, Ui};

pub(super) struct PendingAnswer {
    item: Attention,
    position: usize,
    words: Option<String>,
    answer: Option<String>,
    /// The other answers whose notes went with this one: confirmed, their drafts are done.
    noted: Vec<String>,
}

impl Ui {
    /// Called by the live dispatcher before spawning the RPC, never after awaiting it.
    pub(super) fn begin_answer(&mut self, effect: &Effect) {
        let Effect::Attention { id, action, reason, answer, option_notes } = effect else { return };
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
            noted: option_notes.iter().map(|(answer, _)| answer.clone()).collect(),
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
    /// A refusal restores the card without stealing focus from the next request's draft; notes
    /// on other answers were never taken off the card, so they are all still there.
    pub(super) fn finish_answer(&mut self, id: &str, accepted: bool) {
        let Some(pending) = self.pending_answers.remove(id) else { return };
        if accepted {
            for answer in &pending.noted {
                self.conversation_state.drafts.remove(&format!("answer:{id}:{answer}"));
            }
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

    /// Writes note “A” on “Yes”, then note “B” on “Other”, which stays open in the editor, as a
    /// person does before choosing.
    fn two_notes() -> Ui {
        let mut ui = note_request(None, false);
        ui.click(Hit::Answer(0));
        press(&mut ui, KeyCode::Char('c'));
        press(&mut ui, KeyCode::Char('A'));
        ui.click(Hit::Answer(1));
        assert_eq!(ui.note_answers.get("attention/first").map(String::as_str), Some("yes"));
        ui.click(Hit::Key('c'));
        press(&mut ui, KeyCode::Char('B'));
        assert_eq!(ui.conversation_state.drafts.get("answer:attention/first:yes").map(String::as_str), Some("A"));
        assert_eq!(ui.conversation_state.drafts.get("answer:attention/first:other").map(String::as_str), Some("B"));
        ui
    }

    /// Notes saved on both named answers, with no editor open.
    fn saved_notes(custom: bool) -> Ui {
        let mut ui = note_request(None, custom);
        for (answer, text) in [("yes", "  A  "), ("other", "B")] {
            ui.conversation_state.drafts.insert(format!("answer:attention/first:{answer}"), text.into());
        }
        ui
    }

    #[test]
    fn the_chosen_answer_goes_with_every_note_on_the_other_answers() {
        let mut ui = two_notes();
        ui.click(Hit::Enter);
        assert!(matches!(&ui.effects[..], [Effect::Attention {
            answer: Some(answer), reason: Some(text), option_notes, ..
        }] if answer == "other" && text == "B" && option_notes == &[("yes".to_owned(), "A".to_owned())]));
        // Chosen from the list without opening its note, the highlighted answer carries them too.
        let mut ui = saved_notes(false);
        ui.answering = Some(0);
        press(&mut ui, KeyCode::Enter);
        assert!(matches!(&ui.effects[..], [Effect::Attention {
            answer: Some(answer), reason: Some(text), option_notes, ..
        }] if answer == "yes" && text == "  A  " && option_notes == &[("other".to_owned(), "B".to_owned())]),
            "{:?}", ui.effects);
    }

    #[test]
    fn dismissal_uses_its_named_note_and_preserves_every_other_note() {
        let mut ui = saved_notes(false);
        if let Load::Ready(items) = &mut ui.world.attention {
            for item in items {
                if let AttentionKind::Request { structured: Some(request), .. } = &mut item.kind {
                    let mut dismiss = request.answers[0].clone();
                    dismiss.id = "dismiss".into();
                    dismiss.label = "Dismiss".into();
                    request.answers.push(dismiss);
                }
            }
        }
        ui.conversation_state.drafts.insert(
            "answer:attention/first:dismiss".into(), "  No action needed  ".into(),
        );
        press(&mut ui, KeyCode::Char('x'));
        assert_eq!(ui.confirm, Some('r'));
        press(&mut ui, KeyCode::Char('y'));
        assert!(matches!(&ui.effects[..], [Effect::Attention {
            answer: Some(answer), reason: Some(text), option_notes, ..
        }] if answer == "dismiss" && text == "  No action needed  " && option_notes == &[
            ("yes".to_owned(), "A".to_owned()), ("other".to_owned(), "B".to_owned()),
        ]), "{:?}", ui.effects);
    }

    #[test]
    fn synthetic_dismissal_preserves_every_named_note() {
        let mut ui = saved_notes(false);
        press(&mut ui, KeyCode::Char('x'));
        assert_eq!(ui.confirm, Some('r'));
        press(&mut ui, KeyCode::Char('y'));
        assert!(matches!(&ui.effects[..], [Effect::Attention {
            answer: Some(answer), reason: Some(text), option_notes, ..
        }] if answer == "dismiss" && text == "Dismissed without an answer." && option_notes == &[
            ("yes".to_owned(), "A".to_owned()), ("other".to_owned(), "B".to_owned()),
        ]), "{:?}", ui.effects);
    }

    #[test]
    fn a_custom_answer_carries_the_notes_on_named_answers() {
        let mut ui = saved_notes(true);
        ui.click(Hit::Answer(2));
        press(&mut ui, KeyCode::Enter);
        assert!(ui.editing);
        press(&mut ui, KeyCode::Char('X'));
        ui.click(Hit::Enter);
        assert!(matches!(&ui.effects[..], [Effect::Attention {
            answer: None, reason: Some(text), option_notes, ..
        }] if text == "X" && option_notes == &[
            ("yes".to_owned(), "A".to_owned()), ("other".to_owned(), "B".to_owned()),
        ]), "{:?}", ui.effects);
    }

    /// An older Fractal, without `person-answer-option-notes`, is refused before anything is
    /// sent (`crate::attention_action`); the card and every note come back for the person.
    #[test]
    fn notes_refused_by_an_older_fractal_all_stay_on_the_card() {
        let mut ui = two_notes();
        ui.click(Hit::Enter);
        let effect = ui.effects.pop().unwrap();
        ui.note_acted(&effect);
        assert!(!ui.world.attention.items().iter().any(|item| item.id == "attention/first"));
        // What the live loop does with the refusal (`Fetched::AlertOutcome`).
        ui.finish_answer("attention/first", false);
        ui.answer_errors.insert("attention/first".into(), crate::OptionNotesUnsupported.to_string());
        assert!(ui.world.attention.items().iter().any(|item| item.id == "attention/first"));
        assert_eq!(ui.note_answers["attention/first"], "other");
        assert_eq!(ui.conversation_state.drafts["answer:attention/first:other"], "B");
        assert_eq!(ui.conversation_state.drafts["answer:attention/first:yes"], "A");
        assert!(ui.answer_errors["attention/first"].contains("person-answer-option-notes"));
        // Sending again, once Fractal is updated, tries the same answer with the same notes.
        ui.answer_errors.clear();
        ui.select(0);
        ui.editing = true;
        ui.click(Hit::Enter);
        assert_eq!(ui.effects.last(), Some(&effect), "{:?}", ui.effects);
    }

    #[test]
    fn a_confirmed_answer_retires_the_notes_it_carried() {
        let mut ui = two_notes();
        ui.click(Hit::Enter);
        let effect = ui.effects.pop().unwrap();
        ui.note_acted(&effect);
        ui.finish_answer("attention/first", true);
        assert!(!ui.conversation_state.drafts.contains_key("answer:attention/first:yes"));
        assert!(!ui.conversation_state.drafts.contains_key("answer:attention/first:other"));
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
        let error = ui.answer_errors.get("attention/first").cloned();
        assert!(error.is_some(), "the refusal must have an inline error");
        ui.flash = None;
        ui.set_world(ui.world.clone());
        assert_eq!(ui.answer_errors.get("attention/first"), error.as_ref());
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
            answer: Some("yes".into()), reason: Some("retain this".into()), option_notes: Vec::new(),
        };
        ui.note_acted(&effect);
        ui.finish_answer("attention/first", false);
        ui.flash = None;
        assert_eq!(ui.note_answers.get("attention/first").map(String::as_str), Some("yes"));
        assert_eq!(ui.conversation_state.drafts.get("answer:attention/first:yes").map(String::as_str), Some("retain this"));
        assert!(ui.answer_errors.get("attention/first").is_some_and(|error| error.contains("Could not confirm answer")));
        ui.set_world(ui.world.clone());
        assert!(ui.answer_errors.contains_key("attention/first"));
    }

    #[test]
    fn request_composer_and_send_preserve_a_saved_thread_draft() {
        let mut ui = note_request(None, false);
        let thread_draft = "chat:attention/first";
        ui.conversation_state.drafts.insert(thread_draft.into(), "Unsent thread message".into());
        ui.click(Hit::Answer(0));
        ui.click(Hit::Composer);
        assert!(ui.editing);
        assert_eq!(ui.conversation_state.drafts[thread_draft], "Unsent thread message");
        press(&mut ui, KeyCode::Char('N'));
        ui.click(Hit::Enter);
        assert!(matches!(&ui.effects[..], [Effect::Attention {
            answer: Some(answer), reason: Some(text), ..
        }] if answer == "yes" && text == "N"));
        assert_eq!(ui.conversation_state.drafts[thread_draft], "Unsent thread message");
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
            let Effect::Attention { answer, reason, option_notes, .. } = ui.effects.pop().unwrap() else {
                panic!("expected answer effect");
            };
            let (summary, answer) =
                crate::person_step_response(Some(&request), answer, reason, option_notes).unwrap();
            assert_eq!(summary, "Yes");
            let answer = answer.unwrap();
            assert_eq!(answer.id.as_deref(), Some("yes"));
            assert_eq!(answer.text, None);
            assert!(answer.option_notes.is_empty());
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
        assert_eq!(ui.conversation_state.drafts.get("attention/next").map(String::as_str), Some("new draft"));
        assert!(ui.pending_answers.is_empty());
        assert!(!ui.acted.contains("attention/first"));
        assert!(ui.answer_errors.get("attention/first").is_some_and(|error| error.contains("Could not confirm answer")));
    }

    #[test]
    fn confirmed_answer_stays_gone_and_custom_words_survive_failure() {
        let mut ui = requests();
        let effect = Effect::Attention {
            id: "attention/first".into(), action: "work.done".into(),
            reason: Some("my answer".into()), answer: None, option_notes: Vec::new(),
        };
        ui.note_acted(&effect);
        ui.finish_answer("attention/first", false);
        assert_eq!(ui.conversation_state.drafts.get("attention/first").map(String::as_str), Some("my answer"));
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
