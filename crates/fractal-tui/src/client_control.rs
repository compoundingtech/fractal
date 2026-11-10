//! The TUI adapter for the existing session-scoped device-commands collection.
//! Wire fields live here until the generated SDK exposes grant lifecycle resources.
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use fractal_client::{Client, device::Device};
use fractal_schema::client_control::{ClientResultBinding, ClientVisibleState};
use serde_json::Value;
use std::{collections::BTreeSet, sync::mpsc};

pub(crate) struct Control {
    pub device: Device,
    pub grant: Option<Value>,
    pub stopped: bool,
    used: BTreeSet<String>,
}

pub(crate) fn validate_device(device: &Device, client_socket: &std::path::Path) -> Result<()> {
    ensure!(device.session.person_id == "person/client-control-fixture"
        && device.endpoint == format!("unix:{}", client_socket.display())
        && device.session.scopes.iter().all(|scope| matches!(scope.as_str(), "read.projections" | "control.commands"))
        && device.session.scopes.iter().any(|scope| scope == "read.projections")
        && device.session.scopes.iter().any(|scope| scope == "control.commands")
        && device.signing_key.as_ref().is_some_and(|key| key.algorithm == fractal_client::device::KeyAlgorithm::P256),
        "Fixture pairing must have its own P-256 key and read plus control.commands only");
    Ok(())
}

fn fresh(value: &Value) -> bool {
    value.as_str().is_some_and(fresh_timestamp)
}

fn fresh_timestamp(stamp: &str) -> bool {
    DateTime::parse_from_rfc3339(stamp).ok().is_some_and(|expiry| expiry > Utc::now())
}

impl Control {
    pub fn new(device: Device) -> Self {
        Self { device, grant: None, stopped: false, used: BTreeSet::new() }
    }

    pub fn update_grant(&mut self, grant: Value) {
        if !self.stopped && grant["device"] == self.device.session.device_id
            && grant["session"] == self.device.session.session_actor
            && ["grant_id", "run_id", "actor_agent"].iter()
                .all(|field| grant[*field].as_str().is_some_and(|value| !value.is_empty()))
        {
            if grant["state"] == "active" && fresh(&grant["expires_at"]) {
                if self.grant.as_ref().is_none_or(|old| old["grant_id"] != grant["grant_id"]) {
                    self.used.clear();
                }
                self.grant = Some(grant);
            } else {
                self.grant = None;
            }
        }
    }

    pub fn ingest(&mut self, frame: &Value) -> Vec<Value> {
        if frame["id"] != "commands" { return Vec::new(); }
        let items = if frame["kind"] == "snapshot" {
            self.grant = None;
            frame["items"].as_array()
        } else if frame["kind"] == "changes" {
            if self.grant.as_ref().is_some_and(|grant| frame["removes"].as_array()
                .is_some_and(|removed| removed.contains(&grant["grant_id"]))) {
                self.grant = None;
            }
            frame["upserts"].as_array()
        } else { return Vec::new(); };
        let Some(items) = items else { return Vec::new(); };
        for item in items.iter().filter(|item| item["kind"] == "client-control-grant") {
            self.update_grant(item.clone());
        }
        items.iter().filter(|item| item["kind"] == "device-command").take(8).cloned().collect()
    }

    pub fn active(&self) -> Option<&Value> {
        self.grant.as_ref().filter(|grant| !self.stopped && fresh(&grant["expires_at"])
            && fresh_timestamp(&self.device.session.expires_at))
    }

    /// No navigation, signature or result is constructed on any refusal path.
    pub fn allows(&self, command: &Value, current_screen: &str) -> bool {
        let Some(grant) = self.active() else { return false; };
        let Some(id) = command["id"].as_str() else { return false; };
        let Some(verb) = command["command_kind"].as_str() else { return false; };
        let screen = if verb == "phone.inspect" { current_screen }
            else if verb == "phone.navigate" { command["args"]["screen"].as_str().unwrap_or("") }
            else { return false; };
        matches!(screen, "NowRoot" | "Attention")
            && command["device_id"] == self.device.session.device_id
            && command["session_actor"] == self.device.session.session_actor
            && command["grant_id"] == grant["grant_id"]
            && command["run_id"] == grant["run_id"]
            && command["issued_by"] == grant["actor_agent"]
            && matches!(command["state"].as_str(), Some("pending" | "delivered"))
            && fresh(&command["expires_at"])
            && grant["verbs"].as_array().is_some_and(|verbs| verbs.iter().any(|v| v == verb))
            && grant["screens"].as_array().is_some_and(|screens| screens.iter().any(|s| s == screen))
            && (screen != "Attention" || verb == "phone.inspect"
                || grant.get("item_id").is_none_or(|id| command["args"]["params"]["id"] == *id))
            && grant["max_commands"].as_u64().is_some_and(|max| self.used.len() < max as usize)
            && !self.used.contains(id)
    }

    pub fn result(&mut self, command: &Value, visible_state: ClientVisibleState) -> Result<Option<fractal_client::DeviceResult>> {
        if !self.allows(command, &visible_state.screen) { return Ok(None); }
        if command["command_kind"] == "phone.navigate"
            && (command["args"]["screen"] != visible_state.screen
                || (visible_state.screen == "Attention"
                    && visible_state.route != format!("home:{}", command["args"]["params"]["id"].as_str().unwrap_or(""))))
        {
            return Ok(None);
        }
        let Some(grant) = self.active() else { return Ok(None); };
        if visible_state.screen == "Attention" && grant.get("item_id").is_some_and(|id|
            visible_state.opened_item.as_ref().is_none_or(|item| id.as_str() != Some(item.id.as_str()))) {
            return Ok(None);
        }
        let binding = ClientResultBinding {
            purpose: "client.device-result.v1".into(),
            command_kind: command["command_kind"].as_str().unwrap().into(),
            device: self.device.session.device_id.clone(), session: self.device.session.session_actor.clone(),
            grant_id: grant["grant_id"].as_str().unwrap().into(), run_id: grant["run_id"].as_str().unwrap().into(),
            command_id: command["id"].as_str().unwrap().into(), outcome: "completed".into(),
            error_code: None, visible_state,
        };
        let signature = self.device.sign_client_result(&binding)?;
        let result = fractal_client::DeviceResult {
            command_id: binding.command_id.clone(),
            outcome: fractal_client::DeviceCommandOutcome::Completed,
            result: Some(serde_json::to_value(&binding.visible_state)?),
            error_code: None,
            binding: Some(binding),
            signature: Some(signature),
        };
        self.used.insert(command["id"].as_str().unwrap().into());
        Ok(Some(result))
    }

    pub fn stop(&mut self) { self.stopped = true; self.grant = None; }
}

/// One foreground subscription on the existing collections socket; it never opens a privileged socket.
pub(crate) async fn subscribe(client: Client, tx: mpsc::Sender<Value>) -> Result<()> {
    let capabilities = client.capabilities().await?;
    ensure!(capabilities.value.grants("phone-commands", 1), "Device commands unavailable");
    let mut stream = client.collection_stream().await?;
    stream.subscribe("commands", "commands", 8, None, None).await?;
    while let Some(frame) = stream.next().await? {
        if tx.send(frame).is_err() { break; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::*;
    use fractal_client::device::{KeyAlgorithm, SigningKey};

    fn fixture() -> (Control, Value) {
        let device: Device = serde_json::from_value(json!({
            "endpoint":"unix:/fixture/fractal-client.sock", "allow_public_http":false,
            "session":{"kind":"paired-session","device_id":"device/fixture",
                "person_id":"person/client-control-fixture","session_actor":"person/client-control-fixture/session/tui",
                "credential":"fixture-only-credential-00000000000000000000000000",
                "scopes":["read.projections","control.commands"],"expires_at":"2099-01-01T00:00:00Z"}
        })).unwrap();
        let mut control = Control::new(device);
        control.update_grant(json!({"kind":"client-control-grant","grant_id":"grant/one","run_id":"run/one",
            "device":control.device.session.device_id,"session":control.device.session.session_actor,
            "actor_agent":"agent/fixture","verbs":["phone.inspect","phone.navigate"],
            "screens":["NowRoot","Attention"],"max_commands":50,"expires_at":"2099-01-01T00:00:00Z","state":"active"}));
        let command = json!({"id":"device-command/one","kind":"device-command",
            "device_id":control.device.session.device_id,"session_actor":control.device.session.session_actor,
            "issued_by":"agent/fixture","grant_id":"grant/one","run_id":"run/one",
            "command_kind":"phone.navigate","args":{"screen":"NowRoot"},
            "state":"delivered","expires_at":"2099-01-01T00:00:00Z"});
        (control, command)
    }

    fn visible() -> ClientVisibleState {
        ClientVisibleState { route:"list:home".into(),screen:"NowRoot".into(),
            visible_item_ids:Vec::new(),opened_item:None }
    }

    #[test]
    fn refusals_never_navigate_construct_results_or_sign() {
        for case in ["no grant", "wrong run", "wrong device", "wrong session", "wrong actor",
            "verb", "screen", "expired grant", "expired command", "budget", "read mutation",
            "conversation", "answer", "close", "wrong item"] {
            let (mut control, mut command) = fixture();
            match case {
                "no grant" => control.grant = None,
                "wrong run" => command["run_id"] = json!("run/other"),
                "wrong device" => command["device_id"] = json!("device/other"),
                "wrong session" => command["session_actor"] = json!("session/other"),
                "wrong actor" => command["issued_by"] = json!("agent/other"),
                "verb" => control.grant.as_mut().unwrap()["verbs"] = json!(["phone.inspect"]),
                "screen" => control.grant.as_mut().unwrap()["screens"] = json!(["Attention"]),
                "expired grant" => control.grant.as_mut().unwrap()["expires_at"] = json!("2000-01-01T00:00:00Z"),
                "expired command" => command["expires_at"] = json!("2000-01-01T00:00:00Z"),
                "budget" => control.grant.as_mut().unwrap()["max_commands"] = json!(0),
                "read mutation" => command["command_kind"] = json!("phone.message.read"),
                "conversation" => command["args"] = json!({"screen":"Conversation","params":{"target":"agent/fixture"}}),
                "answer" => command["command_kind"] = json!("work.answer"),
                "close" => command["command_kind"] = json!("work.close"),
                "wrong item" => {
                    control.grant.as_mut().unwrap()["item_id"] = json!("attention/seed");
                    command["args"] = json!({"screen":"Attention","params":{"id":"attention/other"}});
                }
                _ => unreachable!(),
            }
            let mut ui = crate::ui::Ui::new(crate::ui::demo::test_world());
            let before = ui.control_visible();
            assert!(!ui.control_apply(&control, &command), "{case}");
            assert_eq!(ui.control_visible(), before, "{case}: navigation changed");
            // No key is installed: reaching the signer would return an error, not None.
            assert!(control.result(&command, visible()).unwrap().is_none(), "{case}");
        }
    }

    #[test]
    fn active_result_is_signed_once_and_revoke_latches_closed() {
        let (mut control, command) = fixture();
        control.device.signing_key = Some(SigningKey::generate(KeyAlgorithm::P256).unwrap());
        let result = control.result(&command, visible()).unwrap().unwrap();
        assert!(result.signature.is_some());
        assert_eq!(result.binding.unwrap().run_id, "run/one");
        assert!(control.result(&command, visible()).unwrap().is_none());
        let grant = control.grant.clone().unwrap();
        control.stop();
        control.update_grant(grant);
        assert!(control.active().is_none());
        assert!(control.result(&command, visible()).unwrap().is_none());
    }

    #[test]
    fn lifecycle_snapshot_revocation_and_removal_end_authority() {
        let (mut control, command) = fixture();
        let mut grant = control.grant.clone().unwrap();
        control.grant = None;
        let commands = control.ingest(&json!({"kind":"snapshot","id":"commands",
            "items":[command.clone(),grant.clone()]}));
        assert_eq!(commands.len(), 1);
        assert!(control.allows(&command, "NowRoot"));
        grant["state"] = json!("revoked");
        control.ingest(&json!({"kind":"changes","id":"commands","upserts":[grant.clone()],"removes":[]}));
        assert!(!control.allows(&command, "NowRoot"));
        grant["state"] = json!("active");
        control.update_grant(grant);
        control.ingest(&json!({"kind":"changes","id":"commands","upserts":[],"removes":["grant/one"]}));
        assert!(!control.allows(&command, "NowRoot"));
    }

    #[test]
    fn fixture_profile_refuses_other_principals_endpoints_and_message_authority() {
        let (mut control, _) = fixture();
        control.device.signing_key = Some(SigningKey::generate(KeyAlgorithm::P256).unwrap());
        let socket = std::path::Path::new("/fixture/fractal-client.sock");
        assert!(validate_device(&control.device, socket).is_ok());
        assert!(validate_device(&control.device, std::path::Path::new("/fixture/privileged.sock")).is_err());
        let mut other = control.device.clone();
        other.session.person_id = "person/ordinary".into();
        assert!(validate_device(&other, socket).is_err());
        other = control.device.clone();
        other.session.scopes.push("control.messages".into());
        assert!(validate_device(&other, socket).is_err());
        other = control.device;
        other.signing_key = None;
        assert!(validate_device(&other, socket).is_err());
    }
}
