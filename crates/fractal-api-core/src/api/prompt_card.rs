//! A harness-prompt card's prompt: the tool calls without a result in the seat's conversation,
//! the answers st offers and whether a client may answer it, or why st cannot read the
//! conversation (docs/clients/ui-contract.md, Attention row).

use super::*;
use std::collections::HashSet;

/// The most calls a card names.
const CALLS: usize = 5;

/// The most of a call's input a card carries, as JSON text.
const INPUT_BYTES: usize = 4 * 1024;

/// How far back st looks for a call without a result: the conversation's newest entries. A
/// harness waits on the calls it made last, so an older one without a result was abandoned.
const TAIL: usize = 256;

/// Reads the tool calls without a result in a seat's conversation, oldest first, each
/// `{call_id, name, arguments}`, or says why st cannot.
pub(crate) type PendingCalls<'a> = &'a dyn Fn(&str) -> Result<Vec<Value>, String>;

/// The prompt on a harness-prompt row's presentation, from the seat's calls without a result.
pub(crate) fn prompt_card(row: &Value, pending: Result<Vec<Value>, String>) -> Value {
    let respond = &row["action_parameters"]["prompt.respond"];
    let answers = respond["answers"].as_array().cloned().unwrap_or_default();
    let calls = match pending {
        Ok(calls) => calls,
        Err(reason) => {
            return json!({"calls": null, "answers": answers, "allow": false, "unknown_reason": reason});
        }
    };
    // The newest calls, in the order the seat made them, and always the one the harness asks
    // about when st knows it.
    let named = respond["call_id"].as_str();
    let asked = named.and_then(|id| calls.iter().position(|call| call["call_id"] == id));
    let mut kept = calls.len().saturating_sub(CALLS);
    // An older asked call takes the place of the oldest of the newest five.
    if asked.is_some_and(|asked| asked < kept) {
        kept += 1;
    }
    let calls = calls
        .iter()
        .enumerate()
        .filter(|(index, _)| *index >= kept || Some(*index) == asked)
        .map(|(_, call)| {
            let input = match &call["arguments"] {
                Value::String(text) => text.clone(),
                arguments => arguments.to_string(),
            };
            let cut = cut(&input, INPUT_BYTES);
            json!({
                "call_id": call["call_id"],
                "tool": call["name"],
                "input": cut,
                "input_complete": cut.len() == input.len(),
            })
        })
        .collect::<Vec<_>>();
    // The call the harness asks about, waiting and whole: a client answers only the exact call
    // its person can read in full. A harness that names none is answered in its terminal.
    let allow = !answers.is_empty()
        && named.is_some_and(|id| {
            calls
                .iter()
                .any(|call| call["call_id"] == id && call["input_complete"] == true)
        });
    json!({"calls": calls, "answers": answers, "allow": allow})
}

/// `text` within `bytes`, cut at a character boundary.
fn cut(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        return text;
    }
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The tool calls without a result among a conversation's entries, newest first as a slice
/// reads them: oldest first, each once.
fn pending_calls(items: &[Value]) -> Vec<Value> {
    let answered = items
        .iter()
        .filter(|item| item["type"] == "tool_result")
        .filter_map(|item| item["body"]["call_id"].as_str())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut calls = items
        .iter()
        .filter(|item| item["type"] == "tool_call")
        .filter(|item| {
            item["body"]["call_id"]
                .as_str()
                .is_some_and(|id| !answered.contains(id) && seen.insert(id))
        })
        .map(|item| {
            let body = &item["body"];
            json!({"call_id": body["call_id"], "name": body["name"], "arguments": body["arguments"]})
        })
        .collect::<Vec<_>>();
    calls.reverse();
    calls
}

/// The tool calls without a result in `seat`'s conversation on this node, or why st cannot
/// read it: the seat runs elsewhere, has no session yet, or its harness keeps none st reads.
pub(crate) fn seat_pending_calls(state: &AppState, seat: &str) -> Result<Vec<Value>, String> {
    let unreadable = |error: ApiError| format!("st cannot read this seat's conversation: {}", error.message);
    let session = super::client_v0::conversation_session_id(state, seat).map_err(unreadable)?;
    let index = state.store.index().map_err(|error| format!("st cannot read this seat's conversation: {error}"))?;
    let managed = managed_session_owner_at(&state.store, index, &session)
        .map_err(|error| format!("st cannot read this seat's conversation: {error}"))?;
    let source = super::client_v0::conversation_blocks::source(state, &session, managed.as_ref())
        .map_err(unreadable)?;
    let timeline = crate::external_sessions::LineTimeline::acquire(&source);
    let slice = timeline
        .hold()
        .slice(&source, crate::external_sessions::TimelineOrder::Sequence, None, TAIL)
        .map_err(|error| format!("st cannot read this seat's conversation: {error}"))?;
    Ok(pending_calls(&slice.items))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(answers: Value, call: Value) -> Value {
        json!({"action_parameters": {"prompt.respond": {"answers": answers, "call_id": call}}})
    }

    fn entry(sequence: u64, kind: &str, body: Value) -> Value {
        json!({"sequence": sequence, "type": kind, "body": body})
    }

    #[test]
    fn a_prompt_names_the_calls_without_a_result_and_allows_an_answer_to_one_whole_call() {
        // Newest first, as a slice reads them.
        let items = [
            entry(5, "tool_call", json!({"call_id": "call-c", "name": "Bash", "arguments": {"command": "make"}})),
            entry(4, "tool_result", json!({"call_id": "call-b"})),
            entry(3, "tool_call", json!({"call_id": "call-b", "name": "Read", "arguments": {"path": "a"}})),
            entry(2, "content", json!({"text": "Building."})),
        ];
        let calls = pending_calls(&items);
        assert_eq!(calls, [json!({"call_id": "call-c", "name": "Bash", "arguments": {"command": "make"}})]);

        let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-c")), Ok(calls.clone()));
        assert_eq!(
            prompt,
            json!({
                "calls": [{"call_id": "call-c", "tool": "Bash", "input": "{\"command\":\"make\"}", "input_complete": true}],
                "answers": ["allow", "deny"],
                "allow": true,
            })
        );
        // The decoded card is the client's type.
        serde_json::from_value::<fractal_client::AttentionPrompt>(prompt).unwrap();

        // A call other than the one the harness asks about, a harness that names none, or no
        // answers: not answered here.
        assert_eq!(prompt_card(&row(json!(["allow", "deny"]), json!("call-z")), Ok(calls.clone()))["allow"], false);
        assert_eq!(prompt_card(&row(json!(["allow", "deny"]), Value::Null), Ok(calls.clone()))["allow"], false);
        assert_eq!(prompt_card(&row(json!([]), json!("call-c")), Ok(calls.clone()))["allow"], false);

        // Beside another call without a result (a parent's open Task call), the named call is
        // still the one a client allows.
        let task = json!({"call_id": "call-task", "name": "Task", "arguments": {"prompt": "build"}});
        let both = vec![task, calls[0].clone()];
        assert_eq!(prompt_card(&row(json!(["allow", "deny"]), json!("call-c")), Ok(both))["allow"], true);
    }

    #[test]
    fn a_prompt_with_several_calls_or_a_cut_input_is_answered_in_the_terminal() {
        let calls = (0..7)
            .map(|n| json!({"call_id": format!("call-{n}"), "name": "Bash", "arguments": {"n": n}}))
            .collect::<Vec<_>>();
        let ids = |prompt: &Value| {
            prompt["calls"].as_array().unwrap().iter().map(|call| call["call_id"].as_str().unwrap().to_owned()).collect::<Vec<_>>()
        };
        let prompt = prompt_card(&row(json!(["allow", "deny"]), Value::Null), Ok(calls.clone()));
        // The newest five, in the order the seat made them; with no call named, none is allowed.
        assert_eq!(ids(&prompt), ["call-2", "call-3", "call-4", "call-5", "call-6"]);
        assert_eq!(prompt["allow"], false);
        // The call the harness asks about is always among them, however old.
        let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-0")), Ok(calls));
        assert_eq!(ids(&prompt), ["call-0", "call-3", "call-4", "call-5", "call-6"]);
        assert_eq!(prompt["allow"], true);

        let long = "é".repeat(INPUT_BYTES);
        let prompt = prompt_card(
            &row(json!(["allow", "deny"]), json!("call-a")),
            Ok(vec![json!({"call_id": "call-a", "name": "Write", "arguments": {"text": long}})]),
        );
        let input = prompt["calls"][0]["input"].as_str().unwrap();
        assert!(input.len() <= INPUT_BYTES && input.len() > INPUT_BYTES - 2, "{}", input.len());
        assert_eq!(prompt["calls"][0]["input_complete"], false);
        assert_eq!(prompt["allow"], false);
    }

    #[test]
    fn a_prompt_st_cannot_read_says_why_and_allows_no_answer() {
        let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-a")), Err("the seat runs elsewhere".into()));
        assert_eq!(
            prompt,
            json!({"calls": null, "answers": ["allow", "deny"], "allow": false, "unknown_reason": "the seat runs elsewhere"})
        );
        serde_json::from_value::<fractal_client::AttentionPrompt>(prompt).unwrap();
    }
}
