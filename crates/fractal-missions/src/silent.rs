//! Store-backed refresh of held context, above the store-independent delivery rules.
pub use fractal_delivery::silent::*;
use crate::model::MessageView;

/// Notes are a live view, not a snapshot of the notes present when mail was queued.
/// Keep one digest per recipient so multiple revisions cannot repeat the same live block.
pub fn refresh_notes(store: &crate::store::Store, messages: &mut Vec<MessageView>) -> anyhow::Result<()> {
    let mut newest = std::collections::BTreeMap::<String, (u64, String)>::new();
    for message in messages.iter().filter(|message| is_notes_digest(message)) {
        let candidate = (message.created_index, message.subject.clone());
        newest.entry(message.to.clone()).and_modify(|current| {
            if candidate > *current { *current = candidate.clone(); }
        }).or_insert(candidate);
    }
    let mut rendered = std::collections::BTreeMap::new();
    for recipient in newest.keys() {
        rendered.insert(recipient.clone(), crate::notes_digest::render(
            &crate::notes_digest::current_notes(store, recipient)?)?);
    }
    messages.retain_mut(|message| {
        if !is_notes_digest(message) { return true; }
        let Some(Some(content)) = rendered.get(&message.to) else { return false; };
        if newest.get(&message.to).is_none_or(|(_, subject)| subject != &message.subject) {
            return false;
        }
        message.content.clone_from(content);
        true
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(subject: &str, from: &str, status: &str, tags: &[&str]) -> MessageView {
        MessageView {
            subject: subject.into(),
            from: from.into(),
            to: "agent/example/reader".into(),
            content: "body".into(),
            status: status.into(),
            title: None,
            in_reply_to: None,
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            attachments: Vec::new(),
            body_ref: None,
            body_bytes: None,
            body_origin: None,
            sender_class: None,
            assistant_grant_id: None,
            created_index: 0,
        }
    }
    #[test]
    fn notes_digest_with_no_current_notes_is_dropped() {
        let store = crate::store::Store::open_memory("example").unwrap();
        let digest = message("message/example-notes", "daemon/runtime", "sent",
            &[SILENT_TAG, crate::notes_digest::TAG]);
        let wake = message("message/example-wake", "agent/example/writer", "sent", &[]);
        let mut batch = vec![digest, wake];
        refresh_notes(&store, &mut batch).unwrap();
        release(&mut batch);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].subject, "message/example-wake");
    }

}
