//! Native message notification rendering and retained catalog relationship helpers.
//!
//! Current harnesses share bounded PING/envelope formatting, attachment and assistant metadata,
//! and the durable tag readers below. Catalog callers also use the relationship marker formatter.

use std::collections::HashSet;
use std::path::Path;

use crate::message::{self, Message};
#[cfg(test)]
use crate::supervisor_chain::SUPERVISOR_CHAIN_LIMIT;
use crate::supervisor_chain::{chain_bus_ids, resolve_spec};

const SUBJECT_MAX_CHARS: usize = 160;
const SENDER_MAX_CHARS: usize = 80;
/// Envelope identities are whole graph subjects, so they keep a larger bound than the PING header.
const ADDRESS_MAX_CHARS: usize = 256;
const FRACTAL_BODY_MAX_CHARS: usize = 2048;
/// The rule wrapped around a message from a person: replies stay in the conversation,
/// while new questions and decisions use native work asks, even when the person is present.
/// Views also recognize earlier delivery notes in historical transcripts.
pub const PERSON_REPLY_INSTRUCTION: &str =
    "Reply to the person in this conversation; any new question or decision for the person goes through a native work ask.";
/// The `instructions` both Claude channel servers send at initialize, and the rule the st skill
/// repeats in the same words (backticks aside).
pub const CHANNEL_INSTRUCTIONS: &str = "Reply to the person in this conversation; any new question or decision for the person goes through a native work ask. This applies even when the person is present. Use work update only for information the person asked for. A message from an agent is answered with conversations reply.";
/// The marker for a declared non-agent event source. A fixed st2-chosen literal — never
/// producer-supplied text — so the bounded-notice proofs are unaffected.
const SOURCE_MARKER: &str = "»";
/// The `<rand6>` of a `<unix-ms>-<rand6>.md` filename — the stable id an agent dedups re-pokes on.
/// Falls back to the `.md`-stripped stem for anything off-grammar.
pub fn poke_id(filename: &str) -> &str {
    if message::is_message_filename(filename) {
        // `13 digits` + `-` = 14 bytes of prefix, then the 6 rand chars.
        &filename[14..20]
    } else {
        filename.strip_suffix(".md").unwrap_or(filename)
    }
}

/// Prefer the canonical st3 message subject carried across the compatibility inbox boundary.
/// Native st2 messages retain their stable filename-derived identifier.
fn poke_reference(msg: &Message) -> &str {
    fractal_message_reference(msg).unwrap_or_else(|| poke_id(&msg.filename))
}

pub fn fractal_message_reference(msg: &Message) -> Option<&str> {
    fractal_tag(msg, "st3-message:")
        .filter(|reference| reference.starts_with("message/") && reference.len() > "message/".len())
}

/// The st3 bridge records the graph recipient and the SHA-256 of the exact graph body on each
/// projected inbox file, because the inbox round trip may add a trailing newline to the body.
pub const FRACTAL_TO_TAG: &str = "st3-to:";
/// A trusted topic context carried between the seat delivery client and its file-backed driver.
pub const FRACTAL_DELIVERY_THREAD_TAG: &str = "st3-delivery-thread:";
pub const FRACTAL_SHA256_TAG: &str = "st3-sha256:";
/// One tag per attachment on a projected message: [`AttachmentNotice`] as percent-escaped JSON.
pub const FRACTAL_ATTACHMENT_TAG: &str = "st3-attachment:";

// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
/// Choose one durable tag family before parsing; malformed legacy tags still win.
pub fn fractal_tag_family<'a>(tags: &[String], legacy: &'a str, renamed: &'a str) -> &'a str {
    if tags.iter().any(|tag| tag.starts_with(legacy)) {
        legacy
    } else {
        renamed
    }
}

fn fractal_tag<'a>(msg: &'a Message, prefix: &str) -> Option<&'a str> {
    let renamed = match prefix {
        "st3-message:" => "fractal-message:",
        FRACTAL_TO_TAG => "fractal-to:",
        FRACTAL_SHA256_TAG => "fractal-sha256:",
        FRACTAL_DELIVERY_THREAD_TAG => FRACTAL_DELIVERY_THREAD_TAG,
        _ => return None,
    };
    let prefix = fractal_tag_family(&msg.tags, prefix, renamed);
    msg.tags.iter().find_map(|tag| tag.strip_prefix(prefix))
}
// LIVE-MIGRATION END

/// Lowercase hex SHA-256 of one exact graph message body.
pub fn fractal_body_sha256(body: &str) -> String {
    use sha2::{Digest as _, Sha256};
    format!("{:x}", Sha256::digest(body.as_bytes()))
}

fn fractal_body_preview(body: &str) -> (String, bool) {
    let normalized = normalize_line(body);
    let mut characters = normalized.chars();
    let preview: String = characters.by_ref().take(FRACTAL_BODY_MAX_CHARS).collect();
    let truncated = characters.next().is_some();
    (preview, truncated)
}

/// The plain ST3 notice. Claude receives it inside its own channel tag.
/// The graph remains the source of the complete message when the preview is bounded.
pub fn fractal_ping_text(
    reference: &str,
    from: &str,
    subject: Option<&str>,
    body: &str,
    delivery_thread: Option<&str>,
) -> String {
    let person_message = from.starts_with("person/");
    let from = normalize_field(Some(from), "unknown", SENDER_MAX_CHARS);
    let subject = normalize_field(subject, "(no subject)", SUBJECT_MAX_CHARS);
    let header = format!("[PING from st3] {reference} from {from}: {subject}");
    let (preview, truncated) = fractal_body_preview(body);
    let mut notice = if preview.is_empty() {
        header
    } else if truncated {
        format!("{header}\n\n{preview}… [read the full message in st3]")
    } else {
        format!("{header}\n\n{preview}")
    };
    if person_message {
        append_reply_instruction(&mut notice, reference, delivery_thread);
    }
    notice
}

/// One `<smalltalk-message>` envelope, shared by the Codex, OpenCode, pi, and omp drivers.
///
/// `graph` is the message subject, `message/ID`; `id` is its ID; `sha256` hashes the complete
/// graph body. Every value is escaped, so no sender-chosen text can close the element or add an
/// attribute. The body is a bounded one-line preview; truncation is noted after the element.
pub fn fractal_notification_text(
    reference: &str,
    from: &str,
    to: &str,
    subject: Option<&str>,
    body: &str,
    body_sha256: &str,
) -> String {
    fractal_notification_with_attachments(reference, from, to, subject, body, body_sha256, &[])
}

/// A file a message carries, as the seat's harness is told about it: where this machine keeps
/// it, so the harness can open it. A file that cannot be had says why instead.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentNotice {
    pub path: Option<String>,
    pub media_type: String,
    pub name: Option<String>,
    pub size: u64,
    /// Why there is no `path`, such as `expired`.
    pub unavailable: Option<String>,
}

impl AttachmentNotice {
    /// The tag a projected message carries this notice in. Tags are comma-separated text, so
    /// every byte outside a plain word is escaped.
    pub fn to_tag(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        let mut tag = String::from(FRACTAL_ATTACHMENT_TAG);
        for byte in json.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/') {
                tag.push(char::from(byte));
            } else {
                tag.push_str(&format!("%{byte:02x}"));
            }
        }
        tag
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        let escaped = tag
            .strip_prefix(FRACTAL_ATTACHMENT_TAG)
            .or_else(|| tag.strip_prefix("fractal-attachment:"))?
            .as_bytes();
        let mut bytes = Vec::with_capacity(escaped.len());
        let mut index = 0;
        while index < escaped.len() {
            if escaped[index] == b'%' {
                let pair = escaped.get(index + 1..index + 3)?;
                bytes.push(u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?);
                index += 3;
            } else {
                bytes.push(escaped[index]);
                index += 1;
            }
        }
        serde_json::from_slice(&bytes).ok()
    }
}

/// Recipient-facing class marker for a message the authorized external-assistant path sent,
/// independent of sender-authored body text.
pub const EXTERNAL_ASSISTANT_WARNING: &str =
    "WARNING: Untrusted external-assistant request; not owner authorization.";

/// Verified stored provenance supplied by st from the claim, never parsed from sender-authored
/// text. Only the authorized assistant send path can produce it.
pub struct AssistantMessageMetadata<'a> {
    pub assistant_grant_id: Option<&'a str>,
}

/// Supplemental envelope data, kept separate from sender-authored message text.
pub struct NotificationMetadata<'a> {
    pub attachments: &'a [AttachmentNotice],
    pub assistant: Option<&'a AssistantMessageMetadata<'a>>,
    pub delivery_thread: Option<&'a str>,
}

/// The attachment envelope for an ordinary message, without assistant metadata.
pub fn fractal_notification_with_attachments(
    reference: &str,
    from: &str,
    to: &str,
    subject: Option<&str>,
    body: &str,
    body_sha256: &str,
    attachments: &[AttachmentNotice],
) -> String {
    fractal_notification_with_metadata(
        reference, from, to, subject, body, body_sha256,
        NotificationMetadata { attachments, assistant: None, delivery_thread: None },
    )
}

/// Include separately supplied, verified assistant provenance outside the bounded body. The
/// warning prefix and `sender_class` attribute come only from `assistant`; body text cannot forge
/// either, and the prefix stands even when the body preview truncates.
pub fn fractal_notification_with_metadata(
    reference: &str,
    from: &str,
    to: &str,
    subject: Option<&str>,
    body: &str,
    body_sha256: &str,
    metadata: NotificationMetadata<'_>,
) -> String {
    let NotificationMetadata { attachments, assistant, delivery_thread } = metadata;
    let person_message = assistant.is_none() && from.starts_with("person/");
    let graph = normalize_field(Some(reference), "message/unknown", ADDRESS_MAX_CHARS);
    let id = graph.strip_prefix("message/").unwrap_or(&graph);
    let from = normalize_field(Some(from), "unknown", ADDRESS_MAX_CHARS);
    let to = normalize_field(Some(to), "unknown", ADDRESS_MAX_CHARS);
    let subject = normalize_field(subject, "(no subject)", SUBJECT_MAX_CHARS);
    let sha256 = normalize_field(Some(body_sha256), "unknown", 64);
    let mut envelope = format!(
        "<smalltalk-message id=\"{}\" from=\"{}\" to=\"{}\" subject=\"{}\" sha256=\"{}\" graph=\"{}\">\n",
        xml_escape(id, true),
        xml_escape(&from, true),
        xml_escape(&to, true),
        xml_escape(&subject, true),
        xml_escape(&sha256, true),
        xml_escape(&graph, true),
    );
    if assistant.is_some() {
        // Attributes precede the escaped, bounded body; body text cannot forge this boundary.
        envelope.truncate(envelope.len() - 2);
        envelope.push_str(" sender_class=\"external-assistant\"");
        if let Some(grant) = assistant.and_then(|metadata| metadata.assistant_grant_id) {
            envelope.push_str(&format!(" assistant_grant_id=\"{}\"", xml_escape(grant, true)));
        }
        envelope.push_str(">\n");
        envelope = format!(
            "[PING from st3] {graph} from {from} sender_class=external-assistant: {subject}\n{EXTERNAL_ASSISTANT_WARNING}\n{envelope}"
        );
    }
    let (preview, truncated) = fractal_body_preview(body);
    if !preview.is_empty() {
        envelope.push_str(&xml_escape(&preview, false));
        if truncated {
            envelope.push('…');
        }
        envelope.push('\n');
    }
    for attachment in attachments {
        envelope.push_str("<attachment");
        if let Some(path) = &attachment.path {
            envelope.push_str(&format!(" path=\"{}\"", xml_escape(&normalize_line(path), true)));
        }
        envelope.push_str(&format!(
            " media_type=\"{}\" bytes=\"{}\"",
            xml_escape(&normalize_line(&attachment.media_type), true),
            attachment.size
        ));
        if let Some(name) = attachment.name.as_deref().map(normalize_line).filter(|name| !name.is_empty()) {
            envelope.push_str(&format!(" name=\"{}\"", xml_escape(&name, true)));
        }
        if let Some(reason) = &attachment.unavailable {
            envelope.push_str(&format!(" unavailable=\"{}\"", xml_escape(&normalize_line(reason), true)));
        }
        envelope.push_str("/>\n");
    }
    envelope.push_str("</smalltalk-message>");
    if truncated {
        envelope.push_str("\n[preview truncated; read the full message in st3]");
    }
    if person_message {
        append_reply_instruction(&mut envelope, reference, delivery_thread);
    }
    envelope
}

fn append_reply_instruction(output: &mut String, reference: &str, delivery_thread: Option<&str>) {
    output.push('\n');
    if let Some(thread) = delivery_thread {
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
        output.push_str(&format!(
            "Reply in this topic with `fractal conversations reply-in-thread {} --quote {} --body TEXT`. Any new question or decision goes through a native work ask.",
            quote(thread),
            quote(reference),
        ));
    } else {
        output.push_str(PERSON_REPLY_INSTRUCTION);
    }
}

pub(crate) fn fractal_delivery_thread(msg: &Message) -> Option<&str> {
    fractal_tag(msg, FRACTAL_DELIVERY_THREAD_TAG)
}

/// Annotate a dictated or silent delivery without changing the durable message or its body digest.
pub fn with_tag_notices(notification: String, tags: &[String]) -> String {
    let mut notification = notification;
    // st3's silent tag: the message was held, without waking the seat, until this turn.
    if tags.iter().any(|tag| tag == "st3-silent") {
        notification = format!("(silent: held without waking you until this turn)\n{notification}");
    }
    if let Some(count) = tags.iter().find_map(|tag| {
        tag.strip_prefix("st3-silent-remaining:")?
            .parse::<usize>()
            .ok()
    }) {
        notification = format!("({count} older silent held; st conversations ls)\n{notification}");
    }
    if tags.iter().any(|tag| tag == "dictated") {
        notification =
            format!("(dictated by voice; it may contain transcription mistakes)\n{notification}");
    }
    notification
}

/// Escape markup characters so sender text cannot start or close an element. Attribute values
/// also escape both quote characters.
fn xml_escape(text: &str, attribute: bool) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' if attribute => escaped.push_str("&quot;"),
            '\'' if attribute => escaped.push_str("&apos;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Convert arbitrary text into one printable line.
///
/// Every control or whitespace run becomes at most one ordinary space. Removing terminal control
/// bytes before bracketed-paste framing makes it impossible for an untrusted field to inject the
/// closing marker.
fn normalize_line(input: &str) -> String {
    let mut normalized = String::with_capacity(input.len());
    let mut pending_space = false;

    for ch in input.chars() {
        if ch.is_control() || ch.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            normalized.push(' ');
            pending_space = false;
        }
        normalized.push(ch);
    }

    normalized
}

fn normalize_field(value: Option<&str>, fallback: &str, max_chars: usize) -> String {
    let normalized = normalize_line(value.unwrap_or_default());
    let bounded: String = normalized.chars().take(max_chars).collect();
    let bounded = bounded.trim_end();
    if bounded.is_empty() {
        fallback.to_string()
    } else {
        bounded.to_string()
    }
}

struct RelationshipResolver {
    specs: Vec<crate::AgentSpec>,
    valid: bool,
}

impl RelationshipResolver {
    fn read(catalog_root: &Path) -> Self {
        let discovered = crate::discover_strict(catalog_root);
        Self {
            specs: discovered.specs,
            valid: discovered.errors.is_empty(),
        }
    }
}

fn relationship_marker(
    resolver: &RelationshipResolver,
    this_host: &str,
    recipient: &str,
    claimed_sender: Option<&str>,
) -> String {
    if !resolver.valid {
        return "?".to_string();
    }
    let Some(sender) = claimed_sender.and_then(|id| resolve_spec(&resolver.specs, id, this_host))
    else {
        return "?".to_string();
    };
    let Some(recipient) = resolve_spec(&resolver.specs, recipient, this_host) else {
        return "?".to_string();
    };
    let sender_id = sender.bus_id(this_host);
    let recipient_id = recipient.bus_id(this_host);
    if sender_id == recipient_id {
        return "↺".to_string();
    }
    let Ok(recipient_chain) = chain_bus_ids(&resolver.specs, recipient, this_host) else {
        return "?".to_string();
    };
    let Ok(sender_chain) = chain_bus_ids(&resolver.specs, sender, this_host) else {
        return "?".to_string();
    };

    if let Some(depth) = recipient_chain.iter().position(|id| id == &sender_id)
        && depth > 0
    {
        return "↓".repeat(depth);
    }
    if let Some(depth) = sender_chain.iter().position(|id| id == &recipient_id)
        && depth > 0
    {
        return "↑".repeat(depth);
    }
    let recipient_ancestors = recipient_chain.iter().collect::<HashSet<_>>();
    if sender_chain
        .iter()
        .any(|ancestor| recipient_ancestors.contains(ancestor))
    {
        return "←".to_string();
    }
    "?".to_string()
}

/// The `[PING] …` line an agent sees for one newly arrived message. Consumers must key on the
/// prefix and stable id rather than descriptive words. Subject and sender are bounded, normalized
/// untrusted fields. The marker describes the relationship implied by the claimed sender identity;
/// it does not authenticate that identity.
pub fn poke_text(catalog_root: &Path, this_host: &str, recipient: &str, msg: &Message) -> String {
    poke_text_with_resolver(
        &RelationshipResolver::read(catalog_root),
        this_host,
        recipient,
        msg,
    )
}

fn poke_text_with_resolver(
    resolver: &RelationshipResolver,
    this_host: &str,
    recipient: &str,
    msg: &Message,
) -> String {
    if let Some(reference) = fractal_message_reference(msg) {
        let sha256 = fractal_tag(msg, FRACTAL_SHA256_TAG)
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .map_or_else(|| fractal_body_sha256(&msg.body), str::to_owned);
        let attachment_prefix =
            fractal_tag_family(&msg.tags, FRACTAL_ATTACHMENT_TAG, "fractal-attachment:");
        let attachments: Vec<AttachmentNotice> = msg
            .tags
            .iter()
            .filter(|tag| tag.starts_with(attachment_prefix))
            .filter_map(|tag| AttachmentNotice::from_tag(tag))
            .collect();
        return with_tag_notices(
            fractal_notification_with_metadata(
                reference,
                msg.from.as_deref().unwrap_or_default(),
                fractal_tag(msg, FRACTAL_TO_TAG).unwrap_or(recipient),
                msg.subject.as_deref(),
                &msg.body,
                &sha256,
                NotificationMetadata {
                    attachments: &attachments,
                    assistant: None,
                    delivery_thread: fractal_delivery_thread(msg),
                },
            ),
            &msg.tags,
        );
    }
    let subject = normalize_field(msg.subject.as_deref(), "(no subject)", SUBJECT_MAX_CHARS);
    let from = normalize_field(msg.from.as_deref(), "unknown", SENDER_MAX_CHARS);
    let marker = if msg.stream.is_some() && msg.event_id.is_some() {
        SOURCE_MARKER.to_string()
    } else {
        relationship_marker(resolver, this_host, recipient, msg.from.as_deref())
    };
    format!(
        "[PING] {marker} {from}: {subject} [id:{}]",
        poke_reference(msg)
    )
}


#[cfg(test)]
mod tests;
