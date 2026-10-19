//! Agent harness rows maintained by canonical, affected-key edits.
//!
//! The ordered operator handles sparse legacy fields and status episodes without replaying
//! a seat's history. This module does not certify the surrounding declaration, work queue,
//! placement or person joins: the production registry must cover those dependencies before
//! using this source as a complete public card.
use super::*;
use serde::Deserialize;


pub mod ordered;

pub const VIEW: &str = "st3.agents.harness.v1";
const OPTIONAL: &[&str] = &[
    "driver",
    "transport",
    "reason",
    "blocked_on",
    "ask",
    "input_buffer",
    "exit",
];
const STATES: &[&str] = &[
    "",
    "starting",
    "ready",
    "idle",
    "working",
    "blocked",
    "ended",
    "indeterminate",
    "needs-login",
];
const MACHINE_STATES: usize = 1 + STATES.len() * 3 * 2 * 2 * 2 * 2;

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Event {
    pub claim: String,
    pub subject: String,
    pub kind: String,
    pub rank: Vec<u8>,
    pub accepted: u128,
    pub fields: Value,
    pub nested: bool,
}
impl Event {
    fn observed(&self) -> u128 {
        self.fields
            .get("observed_at_ms")
            .and_then(Value::as_u64)
            .map_or(self.accepted, u128::from)
            .min(self.accepted)
    }
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Field {
    rank: Vec<u8>,
    value: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Run {
    first: Option<String>,
    last: Option<String>,
    uniform: bool,
    since: u128,
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Summary {
    state: Option<Event>,
    fields: BTreeMap<String, Field>,
    auth: Option<Event>,
    diagnostics: BTreeMap<String, Event>,
    raw_run: Option<Run>,
    #[serde(skip)]
    machine: Machine,
}
fn newer<'a>(left: Option<&'a Event>, right: Option<&'a Event>) -> Option<&'a Event> {
    match (left, right) {
        (Some(l), Some(r)) => Some(if l.rank > r.rank { l } else { r }),
        (l, None) => l,
        (None, r) => r,
    }
}
pub fn diagnostic(code: &str) -> Option<&'static str> {
    match code {
        "harness-admission-failed" => Some("admission"),
        "provider-update-prompt" | "provider-update-restored" => Some("update"),
        "provider-auth-expired" | "provider-auth-restored" | "provider-trust-prompt" => {
            Some("prompt")
        }
        "claude-channel-unattached" | "claude-channel-attached" => Some("attachment"),
        _ => None,
    }
}
impl Summary {
    fn event(event: &Event, id: u64) -> Result<Self> {
        let mut out = Self::default();
        if event.kind == "harness.observed" {
            let state = event.fields.get("state").and_then(Value::as_str);
            if let Some(state) = state {
                anyhow::ensure!(
                    STATES[1..].contains(&state),
                    "unsupported legacy harness state {state:?}"
                );
            }
            if state.is_some() {
                out.state = Some(event.clone());
            }
            for &name in OPTIONAL {
                if matches!(state, Some("ended" | "indeterminate"))
                    && matches!(name, "blocked_on" | "ask")
                {
                    out.fields.insert(
                        name.into(),
                        Field {
                            rank: event.rank.clone(),
                            value: None,
                        },
                    );
                } else if let Some(value) = event.fields.get(name) {
                    out.fields.insert(
                        name.into(),
                        Field {
                            rank: event.rank.clone(),
                            value: value.as_str().map(str::to_owned),
                        },
                    );
                }
            }
            if event.nested
                && event
                    .fields
                    .get("provider_auth")
                    .and_then(Value::as_bool)
                    .is_some()
            {
                out.auth = Some(event.clone());
            }
            let state = state.map(str::to_owned);
            out.raw_run = Some(Run {
                first: state.clone(),
                last: state,
                uniform: true,
                since: event.observed(),
            });
        } else if event.kind == "harness.diagnostic"
            && event.nested
            && let Some(category) = event
                .fields
                .get("code")
                .and_then(Value::as_str)
                .and_then(diagnostic)
        {
            out.diagnostics.insert(category.into(), event.clone());
        }
        out.machine = Machine::event(event, id)?;
        Ok(out)
    }
    fn join(&self, right: &Self) -> Self {
        let mut fields = self.fields.clone();
        for (name, value) in &right.fields {
            if fields.get(name).is_none_or(|old| old.rank < value.rank) {
                fields.insert(name.clone(), value.clone());
            }
        }
        let mut diagnostics = self.diagnostics.clone();
        for (category, event) in &right.diagnostics {
            if diagnostics
                .get(category)
                .is_none_or(|old| old.rank < event.rank)
            {
                diagnostics.insert(category.clone(), event.clone());
            }
        }
        let raw_run = match (&self.raw_run, &right.raw_run) {
            (Some(l), Some(r)) => Some(Run {
                first: l.first.clone(),
                last: r.last.clone(),
                uniform: l.uniform && r.uniform && l.last == r.first,
                since: if r.uniform && l.last == r.first {
                    l.since
                } else {
                    r.since
                },
            }),
            (l, None) => l.clone(),
            (None, r) => r.clone(),
        };
        Self {
            state: newer(self.state.as_ref(), right.state.as_ref()).cloned(),
            fields,
            auth: newer(self.auth.as_ref(), right.auth.as_ref()).cloned(),
            diagnostics,
            raw_run,
            machine: self.machine.join(&right.machine),
        }
    }
}

// A finite transducer for exactly seat_status::transitions. Each entry records the
// final reducer state and last transition source for one possible incoming state.
// Composition is associative; heartbeats explicitly excluded by the canonical oracle
// are identities. Source pointers are local row IDs, never canonical winner ranks.
#[derive(Clone)]
pub struct Machine(Vec<(u16, u64)>);
impl Default for Machine {
    fn default() -> Self {
        Self((0..MACHINE_STATES).map(|s| (s as u16, 0)).collect())
    }
}
#[derive(Clone, Copy)]
pub struct Status {
    harness: usize,
    prompt: usize,
    auth: bool,
    update: bool,
    human: bool,
    permission: bool,
}
impl Status {
    fn decode(mut index: usize) -> Self {
        index -= 1;
        let permission = index & 1 != 0;
        index /= 2;
        let human = index & 1 != 0;
        index /= 2;
        let update = index & 1 != 0;
        index /= 2;
        let auth = index & 1 != 0;
        index /= 2;
        let prompt = index % 3;
        index /= 3;
        Self {
            harness: index,
            prompt,
            auth,
            update,
            human,
            permission,
        }
    }
    fn encode(self) -> u16 {
        (1 + (((((self.harness * 3 + self.prompt) * 2 + usize::from(self.auth)) * 2
            + usize::from(self.update))
            * 2
            + usize::from(self.human))
            * 2
            + usize::from(self.permission))) as u16
    }
    fn state(self) -> usize {
        if self.update {
            5
        } else if self.auth {
            9
        } else {
            match self.prompt {
                1 => 5,
                2 => 9,
                _ => self.harness,
            }
        }
    }
}
impl Machine {
    fn event(event: &Event, id: u64) -> Result<Self> {
        let observation = event.kind == "harness.observed";
        let fields = &event.fields;
        if observation
            && event.nested
            && fields.get("status_transition").and_then(Value::as_bool) == Some(false)
        {
            return Ok(Self::default());
        }
        if event.kind == "runtime.observed" {
            if fields["status"] != "running" {
                return Ok(Self::default());
            }
            let mut out = Self::default();
            out.0[0] = (1, id);
            return Ok(out);
        }
        let raw = if observation {
            match fields.get("state").and_then(Value::as_str) {
                Some(state) => Some(
                    STATES
                        .iter()
                        .position(|s| *s == state)
                        .context("unsupported legacy harness state")?,
                ),
                None => return Ok(Self::default()),
            }
        } else {
            None
        };
        let code = fields
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !observation
            && !matches!(
                code,
                "provider-auth-expired"
                    | "provider-auth-restored"
                    | "provider-trust-prompt"
                    | "provider-update-prompt"
                    | "provider-update-restored"
            )
        {
            return Ok(Self::default());
        }
        let mut out = Self::default();
        for (incoming, entry) in out.0.iter_mut().enumerate() {
            if incoming == 0 && !observation {
                continue;
            }
            let mut status = Status::decode(incoming.max(1));
            let previous = status.state();
            if let Some(raw) = raw {
                if matches!(raw, 6 | 7) {
                    status.human = false;
                    status.permission = false;
                } else {
                    if let Some(value) = fields.get("blocked_on") {
                        status.human = value.as_str() == Some("human");
                    }
                    if let Some(value) = fields.get("ask") {
                        status.permission = value.as_str() == Some("permission");
                    }
                }
                status.harness = if !matches!(raw, 6 | 7) && status.human && status.permission {
                    5
                } else {
                    raw
                };
                if let Some(auth) = fields.get("provider_auth").and_then(Value::as_bool) {
                    status.auth = !auth;
                }
            } else {
                match code {
                    "provider-auth-expired" => status.prompt = 2,
                    "provider-trust-prompt" => status.prompt = 1,
                    "provider-auth-restored" => status.prompt = 0,
                    "provider-update-prompt" => status.update = true,
                    "provider-update-restored" => status.update = false,
                    _ => {}
                }
            }
            let recorded = observation
                && status.prompt == 0
                && !status.auth
                && !status.update
                && fields.get("status_transition").and_then(Value::as_bool) == Some(true);
            *entry = (
                status.encode(),
                if !(observation
                    && fields.get("status_transition").and_then(Value::as_bool) == Some(false))
                    && (incoming == 0 || previous != status.state() || recorded)
                {
                    id
                } else {
                    0
                },
            );
        }
        Ok(out)
    }
    fn join(&self, right: &Self) -> Self {
        Self(
            self.0
                .iter()
                .map(|(state, transition)| {
                    let (next, new_transition) = right.0[usize::from(*state)];
                    (
                        next,
                        if new_transition != 0 {
                            new_transition
                        } else {
                            *transition
                        },
                    )
                })
                .collect(),
        )
    }
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MACHINE_STATES * 10);
        for (state, source) in &self.0 {
            bytes.extend(state.to_le_bytes());
            bytes.extend(source.to_le_bytes());
        }
        bytes
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        anyhow::ensure!(
            bytes.len() == MACHINE_STATES * 10,
            "agent ordered machine version mismatch"
        );
        let mut entries = Vec::with_capacity(MACHINE_STATES);
        for entry in bytes.chunks_exact(10) {
            let state = u16::from_le_bytes(entry[..2].try_into()?);
            anyhow::ensure!(
                usize::from(state) < MACHINE_STATES,
                "invalid agent reducer state"
            );
            entries.push((state, u64::from_le_bytes(entry[2..].try_into()?)));
        }
        Ok(Self(entries))
    }
}

pub fn observed(
    connection: &Connection,
    subject: &str,
) -> Result<Option<crate::model::CurrentHarnessView>> {
    let runtime: Option<String> = connection
        .query_row(
            "SELECT value FROM ivm_heads WHERE view=?1 AND key=?2 AND register='runtime'",
            params![VIEW, subject],
            |r| r.get(0),
        )
        .optional()?;
    let Some(runtime) = runtime else {
        return Ok(None);
    };
    let runtime: Event = serde_json::from_str(&runtime)?;
    let Some(incarnation) = runtime.fields.get("incarnation_id").and_then(Value::as_str) else {
        return Ok(None);
    };
    let named = ordered::all(connection, subject, Some(incarnation))?;
    let diag = |category: &str| named.diagnostics.get(category);
    let text = |event: &Event, name: &str| {
        event
            .fields
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let fence = |event: &Event,
                 state: &str,
                 driver: Option<String>,
                 transport: Option<String>,
                 reason: Option<String>,
                 blocked_on: Option<&str>| crate::model::CurrentHarnessView {
        state: state.into(),
        driver,
        incarnation_id: incarnation.into(),
        transport,
        reason,
        blocked_on: blocked_on.map(str::to_owned),
        ask: None,
        input_buffer: None,
        exit: None,
        claim: event.claim.clone(),
        since_unix_ms: event.accepted,
        observed_at_unix_ms: event.accepted,
    };
    let mut view = if let Some(event) = diag("admission") {
        Some(fence(
            event,
            "indeterminate",
            None,
            None,
            Some(text(event, "reason").context("non-text harness admission reason")?),
            None,
        ))
    } else if runtime.fields["status"] != "running" {
        None
    } else if let Some(event) =
        diag("update").filter(|e| e.fields["code"] == "provider-update-prompt")
    {
        Some(fence(
            event,
            "blocked",
            text(event, "driver"),
            Some("native".into()),
            text(event, "reason"),
            Some("human"),
        ))
    } else if let Some(event) = named
        .auth
        .as_ref()
        .filter(|e| e.fields["provider_auth"] == false)
    {
        Some(fence(
            event,
            "needs-login",
            text(event, "driver"),
            text(event, "transport"),
            Some("providerAuth".into()),
            Some("human"),
        ))
    } else if let Some(event) =
        diag("prompt").filter(|e| e.fields["code"] != "provider-auth-restored")
    {
        let trust = event.fields["code"] == "provider-trust-prompt";
        let driver = text(event, "driver").unwrap_or_else(|| "claude".into());
        let transport = if driver == "claude" {
            "claude-channel"
        } else {
            "native"
        };
        Some(fence(
            event,
            if trust { "blocked" } else { "needs-login" },
            Some(driver),
            Some(transport.into()),
            Some(
                if trust {
                    "providerTrustPrompt"
                } else {
                    "providerAuth"
                }
                .into(),
            ),
            Some("human"),
        ))
    } else if let Some(event) =
        diag("attachment").filter(|e| e.fields["code"] == "claude-channel-unattached")
    {
        Some(fence(
            event,
            if event.fields["status"] == "starting" {
                "starting"
            } else {
                "blocked"
            },
            Some("claude".into()),
            Some("claude-channel".into()),
            Some("claude-channel-unattached".into()),
            Some("channel"),
        ))
    } else {
        let unnamed = ordered::unnamed_after(connection, subject, &runtime.rank)?;
        let merged = named.join(&unnamed);
        let Some(event) = merged.state.as_ref() else {
            return Ok(None);
        };
        let field = |name: &str| merged.fields.get(name).and_then(|f| f.value.clone());
        let mut state = text(event, "state").context("harness state missing")?;
        let mut reason = field("reason");
        let terminal = matches!(state.as_str(), "ended" | "indeterminate");
        if seat_status::permission_blocked(
            Some(&state),
            field("blocked_on").as_deref(),
            field("ask").as_deref(),
        ) {
            state = "blocked".into();
        }
        if reason.as_deref() == Some("providerAuth") {
            if named
                .auth
                .as_ref()
                .is_some_and(|e| e.fields["provider_auth"] == true)
            {
                reason = None;
            } else {
                state = "needs-login".into();
            }
        }
        Some(crate::model::CurrentHarnessView {
            state,
            driver: field("driver"),
            incarnation_id: incarnation.into(),
            transport: field("transport"),
            reason,
            blocked_on: if terminal { None } else { field("blocked_on") },
            ask: if terminal { None } else { field("ask") },
            input_buffer: field("input_buffer"),
            exit: field("exit"),
            claim: event.claim.clone(),
            since_unix_ms: event.accepted,
            observed_at_unix_ms: event.accepted,
        })
    };
    if let Some(view) = view.as_mut() {
        // Since uses the selected claim's stamp first, then the named raw-state run;
        // diagnostics/auth can replace that with the final canonical transition.
        let event =
            claim_by_id_tx(connection, &view.claim)?.context("agent source claim unavailable")?;
        if let Some(since) = event
            .body
            .pointer("/fields/observed_since_ms")
            .and_then(Value::as_u64)
        {
            view.since_unix_ms = u128::from(since).min(event.accepted_at_unix_ms);
        } else if event.kind == "harness.observed"
            && let Some(run) = named
                .raw_run
                .as_ref()
                .filter(|r| r.last.as_deref() == Some(view.state.as_str()))
        {
            view.since_unix_ms = run.since;
        }
        let prompts =
            named.diagnostics.contains_key("prompt") || named.diagnostics.contains_key("update");
        let unstamped_auth = event
            .body
            .pointer("/fields/provider_auth")
            .and_then(Value::as_bool)
            .is_some()
            && event.body.pointer("/fields/observed_since_ms").is_none();
        if prompts || view.state == "needs-login" || unstamped_auth {
            let (state, transition) = named.machine.0[0];
            let state = if state == 0 {
                0
            } else {
                Status::decode(usize::from(state)).state()
            };
            let name = if state == 9 {
                "unauthenticated"
            } else {
                STATES[state]
            };
            let target = if view.state == "needs-login" {
                "unauthenticated"
            } else {
                &view.state
            };
            if transition != 0 && name == target {
                let event = ordered::event(connection, transition)?;
                view.since_unix_ms = if event.nested {
                    event.observed()
                } else {
                    event.accepted
                };
            }
        }
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::agent_view_tests_shared_attention_tests::{SEAT};

    
    
}
