//! Structured person requests: a typed question with named answers, carried on a person ask.
//!
//! A request is one of four types. A `decision` proposes one action and names what accepting,
//! declining and (optionally) requesting changes each do. A `choice` names two to five options.
//! `feedback` asks for text. Each named answer has a stable ID, so the asker dispatches on that
//! ID instead of reading prose. An `update` asks nothing: it brings the person information they
//! asked for, names the work or message where they asked (`about`), and clears once read. An
//! ask without a request stays a free-text ask.
//!
//! A request may carry up to four images, as a message does (`docs/st3/attachments.md`): the
//! asker names its uploads (`{blob, media_type, name}`), the daemon completes each with its size
//! and the member that holds the bytes, and the person reads them by the ask's step.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use crate::model::{AnswerInput, ChoiceInput};
use crate::model::FractalError;

pub const REQUEST_VERSION: u32 = 1;
const MAX_ANSWERS: usize = 5;
const MAX_REASONS: usize = 8;
const MAX_SUBJECTS: usize = 12;
const MAX_CONDITIONS: usize = 6;
const MAX_LINE: usize = 600;
const MAX_TEXT: usize = 8_000;
const SUBJECT_KINDS: &[&str] = &[
    "pull_request",
    "issue",
    "document",
    "mission",
    "run",
    "step",
    "agent",
    "host",
    "commit",
    "link",
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestType {
    Decision,
    Choice,
    Feedback,
    Update,
}

impl RequestType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Choice => "choice",
            Self::Feedback => "feedback",
            Self::Update => "update",
        }
    }
}

/// Whether a request takes the person's own words beside their answer.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WordsPolicy {
    None,
    Optional,
    Required,
}

/// What a decision answer does to the proposed action.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    Accept,
    Decline,
    RequestChanges,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StructuredRequest {
    pub version: u32,
    #[serde(rename = "type")]
    pub request_type: RequestType,
    /// The one concrete question. Required, except on an update, which asks nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// Why only this person can answer: no runtime fact or standing instruction settles it.
    /// Required, except on an update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why_person: Option<String>,
    /// An update's proof that the person asked: their mission run or step run, or their
    /// message to the agent posting the update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    /// Absent means the asker makes no recommendation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<Recommendation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<RequestSubject>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<AnswerOption>,
    /// A choice may also be answered with the person's words and no option (`custom`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub custom: bool,
    /// Whether the person's words go with an answer: `none`, `optional` or `required`. Absent
    /// means required for feedback and optional for a decision or choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<WordsPolicy>,
    /// Images the person looks at to answer, completed by the daemon from the asker's uploads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<crate::model::MessageAttachment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Recommendation {
    pub answer: String,
    pub reason: String,
}

/// A thing the question is about. `revision` pins what was reviewed, such as a PR head.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RequestSubject {
    pub kind: String,
    pub label: String,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnswerOption {
    pub id: String,
    pub label: String,
    /// What happens next when the person gives this answer.
    pub consequence: String,
    /// Required on a decision answer; absent on a choice option.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<DecisionOutcome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<String>,
}

fn invalid(message: impl Into<String>) -> FractalError {
    FractalError::new("invalid-person-request", message.into())
}

fn invalid_answer(message: impl Into<String>) -> FractalError {
    FractalError::new("invalid-person-answer", message.into())
}

fn answer_required(request: &StructuredRequest) -> FractalError {
    FractalError::new(
        "answer-required",
        format!(
            "this {} needs one of its named answers: {}",
            request.request_type.as_str(),
            request.answer_ids()
        ),
    )
}

/// The person's text, trimmed; absent when blank, refused (`too_long`) beyond `max` characters.
fn bounded<'a>(text: Option<&'a str>, max: usize, name: &str) -> Result<Option<&'a str>, FractalError> {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if text.chars().count() > max {
        return Err(invalid_answer(format!("too_long: {name} must be at most {max} characters")));
    }
    Ok(Some(text))
}

fn line(name: &str, value: &str) -> Result<(), FractalError> {
    if value.trim().is_empty() {
        return Err(invalid(format!("{name} must not be empty")));
    }
    if value.chars().count() > MAX_LINE {
        return Err(invalid(format!(
            "{name} must be at most {MAX_LINE} characters"
        )));
    }
    Ok(())
}

fn answer_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 48
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// A request as clients read it: each image also names its `blob/<sha256>` reference, as a
/// message's attachments do, so a client reads it with the same blob route and the ask's step.
pub fn client_view(request: &Value) -> Value {
    let mut view = request.clone();
    if let Some(attachments) = view.get_mut("attachments").and_then(Value::as_array_mut) {
        for attachment in attachments {
            if let Some(sha256) = attachment.get("sha256").and_then(Value::as_str) {
                attachment["blob"] = json!(format!("blob/{sha256}"));
            }
        }
    }
    view
}

impl StructuredRequest {
    /// Parses and checks a request, returning its canonical JSON for the ask claim.
    pub fn parse(value: &Value) -> Result<(Self, Value), FractalError> {
        let request: Self = serde_json::from_value(value.clone())
            .map_err(|error| invalid(format!("the request is malformed: {error}")))?;
        request.validate()?;
        let canonical =
            serde_json::to_value(&request).map_err(|error| invalid(error.to_string()))?;
        Ok((request, canonical))
    }

    pub fn validate(&self) -> Result<(), FractalError> {
        if self.version != REQUEST_VERSION {
            return Err(invalid(format!(
                "this daemon reads request version {REQUEST_VERSION}"
            )));
        }
        if self.request_type == RequestType::Update {
            if self.question.is_some() || self.why_person.is_some() {
                return Err(invalid(
                    "an update asks nothing: it has no question or why_person",
                ));
            }
            line(
                "about (the run, step or message where the person asked)",
                self.about.as_deref().unwrap_or_default(),
            )?;
        } else {
            line("question", self.question.as_deref().unwrap_or_default())?;
            line("why_person", self.why_person.as_deref().unwrap_or_default())?;
            if self.about.is_some() {
                return Err(invalid("only an update names what it is about"));
            }
        }
        if let Some(summary) = &self.summary {
            line("summary", summary)?;
        }
        if self.reasons.len() > MAX_REASONS {
            return Err(invalid(format!("at most {MAX_REASONS} reasons")));
        }
        for reason in &self.reasons {
            line("a reason", reason)?;
        }
        if self.subjects.len() > MAX_SUBJECTS {
            return Err(invalid(format!("at most {MAX_SUBJECTS} subjects")));
        }
        for subject in &self.subjects {
            if !SUBJECT_KINDS.contains(&subject.kind.as_str()) {
                return Err(invalid(format!(
                    "subject kind `{}` is not one of {}",
                    subject.kind,
                    SUBJECT_KINDS.join(", ")
                )));
            }
            line("a subject label", &subject.label)?;
            if subject.reference.is_none() && subject.url.is_none() {
                return Err(invalid(format!(
                    "subject `{}` needs a ref or a url",
                    subject.label
                )));
            }
            if subject
                .url
                .as_deref()
                .is_some_and(|url| !(url.starts_with("https://") || url.starts_with("http://")))
            {
                return Err(invalid(format!(
                    "subject `{}` url must be http or https",
                    subject.label
                )));
            }
            for (name, value) in [("ref", &subject.reference), ("revision", &subject.revision)] {
                if let Some(value) = value {
                    line(&format!("subject {name}"), value)?;
                }
            }
        }
        if self.request_type == RequestType::Update && !self.attachments.is_empty() {
            return Err(invalid("an update carries no images; ask with them instead"));
        }
        if self.attachments.len() > crate::blobs::MAX_ATTACHMENTS {
            return Err(invalid(format!(
                "at most {} images",
                crate::blobs::MAX_ATTACHMENTS
            )));
        }
        for attachment in &self.attachments {
            if !crate::blobs::is_sha256(&attachment.sha256)
                || !crate::blobs::MEDIA_TYPES.contains(&attachment.media_type.as_str())
                || attachment.size == 0
                || attachment.size > crate::blobs::MAX_BLOB_BYTES as u64
                || attachment.origin.trim().is_empty()
            {
                return Err(invalid(
                    "an image names an uploaded PNG, JPEG, GIF or WebP of at most 10 MiB",
                ));
            }
            if let Some(name) = &attachment.name {
                line("an image name", name)?;
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        for answer in &self.answers {
            if !answer_id(&answer.id) {
                return Err(invalid(format!(
                    "answer id `{}` must be lowercase letters, digits and dashes, starting with a letter",
                    answer.id
                )));
            }
            if !ids.insert(answer.id.as_str()) {
                return Err(invalid(format!("answer id `{}` repeats", answer.id)));
            }
            line("an answer label", &answer.label)?;
            line("an answer consequence", &answer.consequence)?;
            if answer.conditions.len() > MAX_CONDITIONS {
                return Err(invalid(format!(
                    "at most {MAX_CONDITIONS} conditions per answer"
                )));
            }
            for condition in &answer.conditions {
                line("a condition", condition)?;
            }
        }
        match self.request_type {
            RequestType::Decision => {
                let count = |outcome| {
                    self.answers
                        .iter()
                        .filter(|answer| answer.outcome == Some(outcome))
                        .count()
                };
                if self.answers.iter().any(|answer| answer.outcome.is_none())
                    || count(DecisionOutcome::Accept) != 1
                    || count(DecisionOutcome::Decline) != 1
                    || count(DecisionOutcome::RequestChanges) > 1
                {
                    return Err(invalid(
                        "a decision names one accept answer, one decline answer and at most one request_changes answer",
                    ));
                }
                if self.custom {
                    return Err(invalid("only a choice takes a custom answer"));
                }
            }
            RequestType::Choice => {
                if !(2..=MAX_ANSWERS).contains(&self.answers.len()) {
                    return Err(invalid(format!(
                        "a choice names 2 to {MAX_ANSWERS} options"
                    )));
                }
                if self.answers.iter().any(|answer| answer.outcome.is_some()) {
                    return Err(invalid("choice options have no outcome"));
                }
            }
            RequestType::Feedback => {
                if self.words.is_some_and(|words| words != WordsPolicy::Required) {
                    return Err(invalid("feedback asks for words: its words are required"));
                }
                if !self.answers.is_empty() || self.custom || self.recommendation.is_some() {
                    return Err(invalid(
                        "feedback asks for text: it has no answers, custom answer or recommendation",
                    ));
                }
            }
            RequestType::Update => {
                if !self.answers.is_empty()
                    || self.words.is_some()
                    || self.custom
                    || self.recommendation.is_some()
                    || !self.reasons.is_empty()
                {
                    return Err(invalid(
                        "an update asks nothing: it has no answers, reasons or recommendation",
                    ));
                }
            }
        }
        if self.custom && self.words == Some(WordsPolicy::None) {
            return Err(invalid("a custom answer is the person's words: it needs words"));
        }
        if let Some(recommendation) = &self.recommendation {
            if !ids.contains(recommendation.answer.as_str()) {
                return Err(invalid(format!(
                    "the recommendation names `{}`, which is not one of the answers",
                    recommendation.answer
                )));
            }
            line("the recommendation reason", &recommendation.reason)?;
        }
        Ok(())
    }

    /// Whether this request takes the person's words: `words` when the asker set it, else
    /// required for feedback, none for an update, and optional for a decision or choice.
    pub fn words_policy(&self) -> WordsPolicy {
        self.words.unwrap_or(match self.request_type {
            RequestType::Feedback => WordsPolicy::Required,
            RequestType::Update => WordsPolicy::None,
            RequestType::Decision | RequestType::Choice => WordsPolicy::Optional,
        })
    }

    /// Resolves a submitted answer to the typed answer the done claim records, in the one answer
    /// schema: `{type, outcome, choices: [{id, label, note?}], words?}`. Every note and every word
    /// the person typed is recorded or refused with its reason; nothing is folded or dropped. A
    /// feedback request also accepts a bare summary as its words, so a client that only sends a
    /// summary can still answer it.
    pub fn answer(&self, input: Option<&AnswerInput>, summary: &str) -> Result<Value, FractalError> {
        let kind = self.request_type.as_str();
        if self.request_type == RequestType::Update {
            // Opening an update or pressing read clears it; any summary is only history.
            let read = input.is_none_or(|input| {
                input.words.is_none()
                    && (input.choices.is_empty()
                        || (input.choices.len() == 1 && input.choices[0].id == "read" && input.choices[0].note.is_none()))
            });
            if !read {
                return Err(invalid_answer(
                    "an update takes no answer; it is only read. Reply in a conversation",
                ));
            }
            return Ok(json!({"type": "update", "outcome": "read", "choices": []}));
        }
        let fallback;
        let input = match input {
            Some(input) => input,
            None if self.request_type == RequestType::Feedback && !summary.trim().is_empty() => {
                fallback = AnswerInput::words(summary);
                &fallback
            }
            None => return Err(answer_required(self)),
        };
        let words = bounded(input.words.as_deref(), MAX_TEXT, "words")?;
        let mut notes = Vec::with_capacity(input.choices.len());
        for choice in &input.choices {
            notes.push(bounded(choice.note.as_deref(), MAX_LINE, "a note")?);
        }
        // The person may always set a request aside without answering it: `dismiss` is not one of
        // the asker's answers, it is the person's own, and it tells the asker so (Nathan,
        // 2026-10-11). Their words say why. A request that names its own `dismiss` keeps it.
        if input.choices.len() == 1
            && input.choices[0].id == DISMISS
            && !self.answers.iter().any(|option| option.id == DISMISS)
        {
            if notes[0].is_some() {
                return Err(invalid_answer("note_not_allowed: dismissing takes words, not a note"));
            }
            let mut answer = json!({"type": kind, "outcome": "dismissed", "choices": []});
            if let Some(words) = words {
                answer["words"] = json!(words);
            }
            return Ok(answer);
        }
        if input.choices.len() > 1 {
            return Err(invalid_answer("too_many: this request takes one answer"));
        }
        if words.is_some() && self.words_policy() == WordsPolicy::None {
            return Err(invalid_answer("words_not_allowed: this request takes no words"));
        }
        let mut answer = json!({"type": kind});
        match input.choices.first() {
            None => {
                let words_alone = self.request_type == RequestType::Feedback
                    || (self.request_type == RequestType::Choice && self.custom);
                if !words_alone || words.is_none() {
                    return Err(answer_required(self));
                }
                answer["outcome"] = json!(if self.request_type == RequestType::Feedback { "feedback" } else { "custom" });
                answer["choices"] = json!([]);
            }
            Some(choice) => {
                if self.request_type == RequestType::Feedback {
                    return Err(invalid_answer("unknown_choice: feedback has no named answers; send words"));
                }
                let option = self.answers.iter().find(|option| option.id == choice.id).ok_or_else(|| {
                    invalid_answer(format!(
                        "unknown_choice: `{}` is not one of this request's answers: {}",
                        choice.id,
                        self.answer_ids()
                    ))
                })?;
                let outcome = match option.outcome {
                    Some(DecisionOutcome::Accept) => "accept",
                    Some(DecisionOutcome::Decline) => "decline",
                    Some(DecisionOutcome::RequestChanges) => {
                        if words.is_none() {
                            return Err(invalid_answer(
                                "words_required: requesting changes needs words naming the changes",
                            ));
                        }
                        "request_changes"
                    }
                    None => "selected",
                };
                let mut chosen = json!({"id": option.id, "label": option.label});
                if let Some(note) = notes[0] {
                    chosen["note"] = json!(note);
                }
                answer["outcome"] = json!(outcome);
                answer["choices"] = json!([chosen]);
            }
        }
        if self.words_policy() == WordsPolicy::Required && words.is_none() {
            return Err(invalid_answer("words_required: this request needs the person's words"));
        }
        if let Some(words) = words {
            answer["words"] = json!(words);
        }
        Ok(answer)
    }

    /// Checks that `id` can stand in for the person at the deadline: one of this request's
    /// named answers that needs no words of theirs.
    pub fn check_default(&self, id: &str) -> Result<(), FractalError> {
        let invalid_default = |message: String| FractalError::new("invalid-default-answer", message);
        if matches!(self.request_type, RequestType::Feedback | RequestType::Update) {
            return Err(invalid_default(format!(
                "a {} has no named answers to default to",
                self.request_type.as_str()
            )));
        }
        let option = self
            .answers
            .iter()
            .find(|option| option.id == id)
            .ok_or_else(|| {
                invalid_default(format!(
                    "`{id}` is not one of this request's answers: {}",
                    self.answer_ids()
                ))
            })?;
        if option.outcome == Some(DecisionOutcome::RequestChanges) {
            return Err(invalid_default(format!(
                "`{id}` requests changes, which needs the person's words, so it cannot apply by itself"
            )));
        }
        Ok(())
    }

    /// The answer an ask records when its deadline passes unanswered: its default answer, exactly
    /// as if the person had chosen it, or none. The response's `resolved_by` says the deadline
    /// gave it, so the outcome stays one that every client already reads.
    pub fn at_deadline(&self, default: Option<&str>) -> Option<Value> {
        let id = default?;
        self.check_default(id).ok()?;
        self.answer(Some(&AnswerInput::choose(id)), "").ok()
    }

    fn answer_ids(&self) -> String {
        let mut ids = self
            .answers
            .iter()
            .map(|answer| answer.id.as_str())
            .collect::<Vec<_>>();
        if self.request_type == RequestType::Choice && self.custom {
            ids.push("or custom text");
        }
        ids.join(", ")
    }
}

/// The answer id by which a person sets a request aside; the asker reads it as a dismissal.
pub const DISMISS: &str = "dismiss";

/// The human-readable history line for an ask its deadline resolved, with the default answer
/// that applied, if any.
pub fn deadline_summary(answer: Option<&Value>) -> String {
    let chosen = answer.map(read_answer).and_then(|answer| {
        let choice = answer["choices"].get(0)?;
        Some(format!("{} ({})", choice["label"].as_str()?, choice["id"].as_str()?))
    });
    match chosen {
        Some(chosen) => format!("Timed out without an answer; the default applied: {chosen}"),
        None => "Timed out without an answer".to_owned(),
    }
}

/// The human-readable history line for a typed answer: each choice with its note, then the
/// person's words. Older peers and every client read this line, so it carries everything typed.
pub fn answer_summary(answer: &Value) -> String {
    let answer = read_answer(answer);
    let words = answer["words"].as_str();
    if answer["outcome"] == "dismissed" {
        return match words {
            Some(words) => format!("Dismissed without an answer: {words}"),
            None => "Dismissed without an answer".to_owned(),
        };
    }
    let chosen = answer["choices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|choice| match (choice["label"].as_str(), choice["note"].as_str()) {
            (Some(label), Some(note)) => format!("{label}: {note}"),
            (Some(label), None) => label.to_owned(),
            (None, _) => choice["id"].as_str().unwrap_or_default().to_owned(),
        })
        .collect::<Vec<_>>()
        .join("; ");
    match (chosen.is_empty(), words) {
        (false, Some(words)) => format!("{chosen} — {words}"),
        (false, None) => chosen,
        (true, Some(words)) => words.to_owned(),
        (true, None) if answer["outcome"] == "read" => "Read".into(),
        (true, None) => String::new(),
    }
}

/// A stored answer in the one answer schema. Records are immutable and replicated, so they are
/// never rewritten: a record from before the schema (`{id, label, text}`) reads here, and so does
/// one that also carries those fields for an older member (see [`with_legacy_mirror`]). Legacy
/// `text` was always the person's own words, so it reads as `words`.
pub fn read_answer(answer: &Value) -> Value {
    let mut read = json!({"type": answer["type"], "outcome": answer["outcome"]});
    if let Some(choices) = answer.get("choices").filter(|choices| choices.is_array()) {
        read["choices"] = choices.clone();
        if let Some(words) = answer.get("words").filter(|words| words.is_string()) {
            read["words"] = words.clone();
        }
        return read;
    }
    read["choices"] = match answer["id"].as_str() {
        Some(id) => json!([{"id": id, "label": answer["label"]}]),
        None => json!([]),
    };
    if let Some(text) = answer.get("text").filter(|text| text.is_string()) {
        read["words"] = text.clone();
    }
    read
}

/// The answer with the fields a member older than the one answer schema reads: the chosen `id`
/// and `label`, and its note and the words as `text`. Written only while such a member is in the
/// fleet (R49, `docs/st3/compatibility.md`), so its agents still dispatch on `id`.
pub fn with_legacy_mirror(mut answer: Value) -> Value {
    let choice = answer["choices"].get(0).cloned();
    if let Some(choice) = &choice {
        answer["id"] = choice["id"].clone();
        answer["label"] = choice["label"].clone();
    }
    let text = [choice.as_ref().and_then(|choice| choice["note"].as_str()), answer["words"].as_str()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n\n");
    if !text.is_empty() {
        answer["text"] = json!(text);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision() -> Value {
        json!({
            "version": 1,
            "type": "decision",
            "question": "Change the browser sign-in, then review the gateway again?",
            "why_person": "Only the owner decides how browsers authenticate.",
            "recommendation": {"answer": "revise-auth", "reason": "Native clients already pair this way."},
            "subjects": [{"kind": "pull_request", "label": "#41", "url": "https://example.com/pull/41", "revision": "abc123"}],
            "answers": [
                {"id": "land", "label": "Land the gateway", "outcome": "accept", "consequence": "The gateway merges as it is."},
                {"id": "keep-open", "label": "Keep it open", "outcome": "decline", "consequence": "Nothing merges."},
                {"id": "revise-auth", "label": "Revise auth and review", "outcome": "request_changes", "consequence": "The author changes sign-in and asks again."}
            ]
        })
    }

    #[test]
    fn a_decision_records_its_named_outcome_and_needs_text_for_changes() {
        let (request, canonical) = StructuredRequest::parse(&decision()).unwrap();
        assert_eq!(canonical["subjects"][0]["revision"], "abc123");
        let accept = request.answer(Some(&AnswerInput::choose("land")), "").unwrap();
        assert_eq!(
            accept,
            json!({"type": "decision", "outcome": "accept", "choices": [{"id": "land", "label": "Land the gateway"}]})
        );
        assert_eq!(answer_summary(&accept), "Land the gateway");
        let changes = AnswerInput::choose("revise-auth");
        assert_eq!(
            request.answer(Some(&changes), "").unwrap_err().code,
            "invalid-person-answer"
        );
        let changes = AnswerInput { words: Some("Use device pairing.".into()), ..changes };
        let answer = request.answer(Some(&changes), "").unwrap();
        assert_eq!(answer["outcome"], "request_changes");
        assert_eq!(
            answer_summary(&answer),
            "Revise auth and review — Use device pairing."
        );
        // Prose never selects an answer.
        assert_eq!(
            request.answer(None, "yes").unwrap_err().code,
            "answer-required"
        );
        let unknown = AnswerInput::choose("yes");
        assert_eq!(
            request.answer(Some(&unknown), "").unwrap_err().code,
            "invalid-person-answer"
        );
    }

    #[test]
    fn a_default_is_a_named_answer_that_needs_no_words_and_applies_as_itself() {
        let (request, _) = StructuredRequest::parse(&decision()).unwrap();
        request.check_default("land").unwrap();
        request.check_default("keep-open").unwrap();
        assert_eq!(request.check_default("revise-auth").unwrap_err().code, "invalid-default-answer");
        assert_eq!(request.check_default("yes").unwrap_err().code, "invalid-default-answer");
        assert_eq!(request.check_default(DISMISS).unwrap_err().code, "invalid-default-answer");
        // The default applies as the person's own choice of it would: an outcome every client
        // already reads, never a new one.
        let answer = request.at_deadline(Some("keep-open")).unwrap();
        assert_eq!(
            answer,
            json!({"type": "decision", "outcome": "decline", "choices": [{"id": "keep-open", "label": "Keep it open"}]})
        );
        let chosen = AnswerInput::choose("keep-open");
        assert_eq!(answer, request.answer(Some(&chosen), "").unwrap());
        assert_eq!(
            deadline_summary(Some(&answer)),
            "Timed out without an answer; the default applied: Keep it open (keep-open)"
        );
        assert_eq!(request.at_deadline(None), None);
        assert_eq!(request.at_deadline(Some("revise-auth")), None);
        assert_eq!(deadline_summary(None), "Timed out without an answer");
        let mut feedback = decision();
        feedback["type"] = json!("feedback");
        feedback.as_object_mut().unwrap().remove("answers");
        feedback.as_object_mut().unwrap().remove("recommendation");
        let (request, _) = StructuredRequest::parse(&feedback).unwrap();
        assert_eq!(request.check_default("land").unwrap_err().code, "invalid-default-answer");
    }

    #[test]
    fn a_person_can_always_dismiss_a_request_and_the_asker_is_told() {
        for kind in ["decision", "choice"] {
            let mut value = decision();
            value["type"] = json!(kind);
            if kind == "choice" {
                for answer in value["answers"].as_array_mut().unwrap() {
                    answer.as_object_mut().unwrap().remove("outcome");
                }
            }
            let (request, _) = StructuredRequest::parse(&value).unwrap();
            let dismiss = AnswerInput::choose("dismiss");
            let answer = request.answer(Some(&dismiss), "").unwrap();
            assert_eq!(answer, json!({"type": kind, "outcome": "dismissed", "choices": []}));
            assert_eq!(answer_summary(&answer), "Dismissed without an answer");
            let with_reason = AnswerInput { words: Some("Not now.".into()), ..dismiss };
            let answer = request.answer(Some(&with_reason), "").unwrap();
            assert_eq!(
                answer_summary(&answer),
                "Dismissed without an answer: Not now."
            );
        }
        // Prose alone still never selects or dismisses.
        let (request, _) = StructuredRequest::parse(&decision()).unwrap();
        assert_eq!(request.answer(None, "dismiss").unwrap_err().code, "answer-required");
        // A request that names its own `dismiss` answer keeps its meaning.
        let mut own = decision();
        own["answers"][1]["id"] = json!("dismiss");
        let (request, _) = StructuredRequest::parse(&own).unwrap();
        let answer = request
            .answer(Some(&AnswerInput::choose("dismiss")), "")
            .unwrap();
        assert_eq!(answer["outcome"], "decline");
    }

    #[test]
    fn malformed_requests_are_refused() {
        let mut two_accepts = decision();
        two_accepts["answers"][1]["outcome"] = json!("accept");
        let mut bad_recommendation = decision();
        bad_recommendation["recommendation"]["answer"] = json!("merge");
        let mut unknown_field = decision();
        unknown_field["deadline"] = json!("tomorrow");
        let mut no_reason = decision();
        no_reason["why_person"] = json!(" ");
        let mut bad_id = decision();
        bad_id["answers"][0]["id"] = json!("Land");
        let mut feedback_with_answers = decision();
        feedback_with_answers["type"] = json!("feedback");
        let mut one_option = json!({
            "version": 1, "type": "choice", "question": "Which?", "why_person": "Taste.",
            "answers": [{"id": "a", "label": "A", "consequence": "A happens."}]
        });
        let mut newer = decision();
        newer["version"] = json!(2);
        for request in [
            two_accepts,
            bad_recommendation,
            unknown_field,
            no_reason,
            bad_id,
            feedback_with_answers,
            one_option.take(),
            newer,
        ] {
            assert_eq!(
                StructuredRequest::parse(&request).unwrap_err().code,
                "invalid-person-request",
                "{request}"
            );
        }
    }

    #[test]
    fn an_update_names_where_it_was_asked_for_and_is_only_read() {
        let update = json!({
            "version": 1, "type": "update", "about": "mission-run/example/report/1",
            "summary": "The nightly build is green again.",
            "subjects": [{"kind": "document", "label": "Report", "ref": "doc/example/report"}]
        });
        let (request, canonical) = StructuredRequest::parse(&update).unwrap();
        assert_eq!(canonical, update);
        let read = json!({"type": "update", "outcome": "read", "choices": []});
        assert_eq!(request.answer(None, "").unwrap(), read);
        assert_eq!(answer_summary(&read), "Read");
        let pressed = AnswerInput::choose("read");
        assert_eq!(request.answer(Some(&pressed), "").unwrap(), read);
        for input in [
            AnswerInput::choose("yes"),
            AnswerInput::words("Thanks, now do the next one"),
        ] {
            assert_eq!(
                request.answer(Some(&input), "").unwrap_err().code,
                "invalid-person-answer"
            );
        }
        let mut no_about = update.clone();
        no_about.as_object_mut().unwrap().remove("about");
        let mut asks = update.clone();
        asks["question"] = json!("Is this fine?");
        let mut answers = update.clone();
        answers["answers"] =
            json!([{"id": "ok", "label": "OK", "consequence": "Nothing changes."}]);
        let mut about_on_decision = decision();
        about_on_decision["about"] = json!("mission-run/example/report/1");
        let mut decision_without_question = decision();
        decision_without_question
            .as_object_mut()
            .unwrap()
            .remove("question");
        for request in [
            no_about,
            asks,
            answers,
            about_on_decision,
            decision_without_question,
        ] {
            assert_eq!(
                StructuredRequest::parse(&request).unwrap_err().code,
                "invalid-person-request",
                "{request}"
            );
        }
    }

    #[test]
    fn choice_takes_an_option_or_allowed_custom_text_and_feedback_takes_text() {
        let choice = json!({
            "version": 1, "type": "choice", "question": "How should an oversized tree load?",
            "why_person": "It changes what every client shows.", "custom": true,
            "answers": [
                {"id": "paged", "label": "Page it", "consequence": "Clients read pages with explicit metadata."},
                {"id": "limit", "label": "Fail past a limit", "consequence": "Large trees return an error."}
            ]
        });
        let (request, _) = StructuredRequest::parse(&choice).unwrap();
        let selected = request
            .answer(Some(&AnswerInput::choose("paged")), "")
            .unwrap();
        assert_eq!(selected["outcome"], "selected");
        let custom = request
            .answer(Some(&AnswerInput::words("Page it, but warn")), "")
            .unwrap();
        assert_eq!(
            custom,
            json!({"type": "choice", "outcome": "custom", "choices": [], "words": "Page it, but warn"})
        );
        let feedback = json!({
            "version": 1, "type": "feedback", "question": "What should the page say?",
            "why_person": "It is the owner's voice.",
            "subjects": [{"kind": "document", "label": "Draft", "ref": "doc/example/draft"}]
        });
        let (request, _) = StructuredRequest::parse(&feedback).unwrap();
        assert_eq!(
            request.answer(None, "Shorter, please.").unwrap(),
            json!({"type": "feedback", "outcome": "feedback", "choices": [], "words": "Shorter, please."})
        );
        assert_eq!(
            request
                .answer(Some(&AnswerInput::choose("ok")), "")
                .unwrap_err()
                .code,
            "invalid-person-answer"
        );
    }
}

/// One answer schema for every question a person answers (`docs/st3/person-answers.md`): the
/// choices they made, each with an optional note, and their own words beside them. Clean cut
/// approved by the user as HIGH: "Clean cut, I approve HIGH (Recommended)".
#[cfg(test)]
mod one_answer_schema {
    use super::*;

    fn request(value: Value) -> StructuredRequest {
        StructuredRequest::parse(&value).unwrap().0
    }
    fn decision() -> StructuredRequest {
        request(json!({
            "version": 1, "type": "decision",
            "question": "Land the gateway?", "why_person": "Only the owner decides.",
            "answers": [
                {"id": "land", "label": "Land the gateway", "outcome": "accept", "consequence": "It merges."},
                {"id": "keep-open", "label": "Keep it open", "outcome": "decline", "consequence": "Nothing merges."},
                {"id": "revise", "label": "Revise it", "outcome": "request_changes", "consequence": "The author revises."}
            ]
        }))
    }
    fn input(value: Value) -> Result<AnswerInput, serde_json::Error> {
        serde_json::from_value(value)
    }
    fn answer(request: &StructuredRequest, value: Value) -> Result<Value, FractalError> {
        request.answer(Some(&input(value).unwrap()), "")
    }

    #[test]
    fn a_choice_carries_its_note_and_the_words_stand_beside_it() {
        let recorded = answer(&decision(), json!({"choices": [{"id": "land", "note": " after the cut "}], "words": " Ship Friday. "})).unwrap();
        assert_eq!(recorded, json!({
            "type": "decision", "outcome": "accept",
            "choices": [{"id": "land", "label": "Land the gateway", "note": "after the cut"}],
            "words": "Ship Friday."
        }));
        assert_eq!(answer_summary(&recorded), "Land the gateway: after the cut — Ship Friday.");
        // A blank note or blank words are absent, not empty.
        let plain = answer(&decision(), json!({"choices": [{"id": "land", "note": "  "}], "words": ""})).unwrap();
        assert_eq!(plain, json!({"type": "decision", "outcome": "accept", "choices": [{"id": "land", "label": "Land the gateway"}]}));
    }

    #[test]
    fn the_old_input_is_refused_and_nothing_typed_is_dropped() {
        assert!(input(json!({"id": "land", "text": "x"})).is_err());
        let refused = |value: Value, reason: &str| {
            let error = answer(&decision(), value).unwrap_err();
            assert_eq!(error.code, "invalid-person-answer");
            assert!(error.message.contains(reason), "{} does not say {reason}", error.message);
        };
        refused(json!({"choices": [{"id": "land"}, {"id": "keep-open"}]}), "too_many");
        refused(json!({"choices": [{"id": "land"}, {"id": "land"}]}), "too_many");
        refused(json!({"choices": [{"id": "merge"}]}), "unknown_choice");
        refused(json!({"choices": [{"id": "revise"}]}), "words_required");
        refused(json!({"choices": [{"id": "land", "note": "n".repeat(MAX_LINE + 1)}]}), "too_long");
        refused(json!({"choices": [{"id": "land"}], "words": "w".repeat(MAX_TEXT + 1)}), "too_long");
        let quiet = request(json!({
            "version": 1, "type": "choice", "question": "Which?", "why_person": "Taste.", "words": "none",
            "answers": [{"id": "a", "label": "A", "consequence": "A."}, {"id": "b", "label": "B", "consequence": "B."}]
        }));
        let error = answer(&quiet, json!({"choices": [{"id": "a"}], "words": "and also"})).unwrap_err();
        assert!(error.message.contains("words_not_allowed"), "{}", error.message);
        // Words alone answer only a request that takes them without a choice.
        assert_eq!(answer(&decision(), json!({"words": "maybe"})).unwrap_err().code, "answer-required");
    }

    #[test]
    fn every_kind_of_answer_is_one_shape() {
        let changes = answer(&decision(), json!({"choices": [{"id": "revise"}], "words": "Pair devices."})).unwrap();
        assert_eq!(changes, json!({"type": "decision", "outcome": "request_changes", "choices": [{"id": "revise", "label": "Revise it"}], "words": "Pair devices."}));
        let choice = request(json!({
            "version": 1, "type": "choice", "question": "Which?", "why_person": "Taste.", "custom": true,
            "answers": [{"id": "a", "label": "A", "consequence": "A."}, {"id": "b", "label": "B", "consequence": "B."}]
        }));
        assert_eq!(answer(&choice, json!({"choices": [{"id": "b"}]})).unwrap()["outcome"], "selected");
        assert_eq!(answer(&choice, json!({"words": "C"})).unwrap(), json!({"type": "choice", "outcome": "custom", "choices": [], "words": "C"}));
        let feedback = request(json!({"version": 1, "type": "feedback", "question": "What?", "why_person": "Yours."}));
        assert_eq!(answer(&feedback, json!({"words": "Blue"})).unwrap(), json!({"type": "feedback", "outcome": "feedback", "choices": [], "words": "Blue"}));
        assert_eq!(answer(&feedback, json!({"choices": [{"id": "x"}], "words": "Blue"})).unwrap_err().code, "invalid-person-answer");
        let dismissed = answer(&decision(), json!({"choices": [{"id": "dismiss"}], "words": "Not now."})).unwrap();
        assert_eq!(dismissed, json!({"type": "decision", "outcome": "dismissed", "choices": [], "words": "Not now."}));
        assert_eq!(answer_summary(&dismissed), "Dismissed without an answer: Not now.");
        assert_eq!(decision().at_deadline(Some("keep-open")).unwrap(), json!({"type": "decision", "outcome": "decline", "choices": [{"id": "keep-open", "label": "Keep it open"}]}));
    }

    /// Stored records are never rewritten: every legacy shape reads as the one answer schema.
    #[test]
    fn every_stored_legacy_answer_reads_as_the_one_schema() {
        for (stored, read) in [
            (json!({"type": "decision", "outcome": "accept", "id": "land", "label": "Land the gateway"}),
             json!({"type": "decision", "outcome": "accept", "choices": [{"id": "land", "label": "Land the gateway"}]})),
            (json!({"type": "decision", "outcome": "accept", "id": "land", "label": "Land the gateway", "text": "after the cut"}),
             json!({"type": "decision", "outcome": "accept", "choices": [{"id": "land", "label": "Land the gateway"}], "words": "after the cut"})),
            (json!({"type": "decision", "outcome": "request_changes", "id": "revise", "label": "Revise it", "text": "Pair devices."}),
             json!({"type": "decision", "outcome": "request_changes", "choices": [{"id": "revise", "label": "Revise it"}], "words": "Pair devices."})),
            (json!({"type": "choice", "outcome": "selected", "id": "b", "label": "B"}),
             json!({"type": "choice", "outcome": "selected", "choices": [{"id": "b", "label": "B"}]})),
            (json!({"type": "choice", "outcome": "custom", "text": "C"}),
             json!({"type": "choice", "outcome": "custom", "choices": [], "words": "C"})),
            (json!({"type": "feedback", "outcome": "feedback", "text": "Blue"}),
             json!({"type": "feedback", "outcome": "feedback", "choices": [], "words": "Blue"})),
            (json!({"type": "decision", "outcome": "dismissed"}),
             json!({"type": "decision", "outcome": "dismissed", "choices": []})),
            (json!({"type": "decision", "outcome": "dismissed", "text": "Not now."}),
             json!({"type": "decision", "outcome": "dismissed", "choices": [], "words": "Not now."})),
            (json!({"type": "update", "outcome": "read"}),
             json!({"type": "update", "outcome": "read", "choices": []})),
            // A record written while an older member was in the fleet also carries the legacy
            // fields; the one schema wins and the mirror is dropped.
            (json!({"type": "decision", "outcome": "accept", "choices": [{"id": "land", "label": "Land the gateway", "note": "n"}], "words": "w", "id": "land", "label": "Land the gateway", "text": "w"}),
             json!({"type": "decision", "outcome": "accept", "choices": [{"id": "land", "label": "Land the gateway", "note": "n"}], "words": "w"})),
        ] {
            assert_eq!(read_answer(&stored), read, "{stored}");
            assert_eq!(read_answer(&read), read, "reading is idempotent");
        }
        assert_eq!(answer_summary(&json!({"type": "decision", "outcome": "accept", "id": "land", "label": "Land the gateway", "text": "x"})), "Land the gateway — x");
    }

    /// While a member that predates the one schema is in the fleet, a record also carries the
    /// fields it reads (R49): its agents still dispatch on `id`.
    #[test]
    fn a_record_for_an_older_fleet_mirrors_the_fields_it_reads() {
        let recorded = answer(&decision(), json!({"choices": [{"id": "land", "note": "after the cut"}], "words": "Friday"})).unwrap();
        assert_eq!(with_legacy_mirror(recorded.clone()), json!({
            "type": "decision", "outcome": "accept",
            "choices": [{"id": "land", "label": "Land the gateway", "note": "after the cut"}], "words": "Friday",
            "id": "land", "label": "Land the gateway", "text": "after the cut\n\nFriday"
        }));
        assert_eq!(read_answer(&with_legacy_mirror(recorded.clone())), recorded);
    }
}
