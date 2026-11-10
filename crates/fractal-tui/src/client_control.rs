//! The TUI adapter for the existing session-scoped device-commands collection.
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use fractal_client::{Client, device::Device};
use fractal_schema::client_control::{ClientResultBinding, ClientVisibleState};
use serde_json::Value;
use std::{collections::{BTreeMap, BTreeSet}, sync::mpsc};

pub(crate) struct Control {
    pub device: Device,
    pub grant: Option<fractal_client::ClientControlGrantLifecycle>,
    pub stopped: bool,
    used: BTreeSet<String>,
    sequences: BTreeMap<String, u64>,
    terminal: BTreeSet<String>,
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

pub(crate) fn load_fixture_device(path: &std::path::Path, normal: &std::path::Path) -> Result<Device> {
    use anyhow::Context;
    ensure!(path != normal && path.canonicalize()? != normal.canonicalize().unwrap_or_else(|_| normal.to_owned()),
        "Client control must not use the normal paired profile");
    let profile = fractal_client::device::Profile::load(path)?
        .context("Pair the isolated TUI profile first")?;
    ensure!(profile.devices.len() == 1, "Fixture profile must contain exactly one pairing");
    let device = profile.devices.into_iter().next().unwrap();
    let socket = device.endpoint.strip_prefix("unix:").context("Fixture pairing requires a Unix endpoint")?;
    validate_device(&device, std::path::Path::new(socket))?;
    Ok(device)
}

/// Read-only observer output. Never serialize the unredacted device or profile.
pub(crate) fn identity(path: &std::path::Path, normal: &std::path::Path) -> Result<Value> {
    use anyhow::Context;
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let device = load_fixture_device(path, normal)?;
    let public_key = device.signing_key.as_ref().context("Fixture key missing")?.public_key()?;
    let point = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(
        public_key.strip_prefix("p256:").context("Fixture key must be P-256")?)?;
    ensure!(point.len() == 65 && point[0] == 4, "Fixture key must be uncompressed P-256");
    let mut snapshot: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let devices = snapshot["devices"].as_array_mut().context("Invalid fixture profile")?;
    ensure!(devices.len() == 1, "Fixture profile must contain exactly one pairing");
    for device in devices {
        for (section, field) in [("signing_key", "pkcs8"), ("session", "credential")] {
            let secret = device[section][field].as_str().context("Missing fixture secret")?;
            let digest = format!("{:x}", Sha256::digest(secret.as_bytes()));
            device[section][field] = serde_json::json!({"redacted":"[REDACTED]","sha256":digest});
        }
    }
    Ok(serde_json::json!({
        "public_key":public_key,
        "fingerprint":format!("{:x}", Sha256::digest(&point)),
        "device_id":device.session.device_id,
        "session_actor":device.session.session_actor,
        "profile":snapshot,
    }))
}

fn fresh(value: &Value) -> bool {
    value.as_str().is_some_and(fresh_timestamp)
}

fn fresh_timestamp(stamp: &str) -> bool {
    DateTime::parse_from_rfc3339(stamp).ok().is_some_and(|expiry| expiry > Utc::now())
}

impl Control {
    pub fn new(device: Device) -> Self {
        Self { device, grant: None, stopped: false, used: BTreeSet::new(),
            sequences: BTreeMap::new(), terminal: BTreeSet::new() }
    }

    pub fn update_lifecycle(&mut self, frame: Value) {
        use fractal_schema::client_control::ClientControlGrantLifecycleState as State;
        let Ok(frame) = serde_json::from_value::<fractal_client::ClientControlGrantLifecycle>(frame) else { return; };
        if self.stopped || frame.device != self.device.session.device_id
            || frame.session != self.device.session.session_actor
            || frame.sequence == 0 || frame.grant_id.is_empty() || frame.run_id.is_empty() { return; }
        if self.grant.as_ref().is_some_and(|active| active.grant_id == frame.grant_id
            && (active.run_id != frame.run_id || active.item_id != frame.item_id)) { return; }
        match frame.state {
            State::Ended | State::Revoked => {
                self.terminal.insert(frame.grant_id.clone());
                if self.grant.as_ref().is_some_and(|active| active.grant_id == frame.grant_id) {
                    self.grant = None;
                }
            }
            State::Started if !self.terminal.contains(&frame.grant_id)
                && self.sequences.get(&frame.grant_id).is_none_or(|last| frame.sequence > *last) => {
                self.sequences.insert(frame.grant_id.clone(), frame.sequence);
                if self.grant.as_ref().is_none_or(|old| old.grant_id != frame.grant_id) {
                    self.used.clear();
                }
                self.grant = Some(frame);
            }
            _ => {}
        }
    }

    pub fn ingest(&mut self, frame: &Value) -> Vec<Value> {
        if frame["id"] != "commands" { return Vec::new(); }
        let items = if frame["kind"] == "snapshot" {
            frame["items"].as_array()
        } else if frame["kind"] == "changes" {
            if self.grant.as_ref().is_some_and(|grant| frame["removes"].as_array()
                .is_some_and(|removed| removed.iter().any(|id| id.as_str() == Some(grant.header.id.as_str())))) {
                if let Some(grant) = &self.grant {
                    self.terminal.insert(grant.grant_id.clone());
                }
                self.grant = None;
            }
            frame["upserts"].as_array()
        } else { return Vec::new(); };
        let Some(items) = items else { return Vec::new(); };
        if frame["kind"] == "snapshot" && self.grant.as_ref().is_some_and(|active|
            !items.iter().any(|item| item["id"].as_str() == Some(active.header.id.as_str()))) {
            self.grant = None;
        }
        for item in items.iter().filter(|item| item["kind"] == "client-control-grant-lifecycle") {
            self.update_lifecycle(item.clone());
        }
        items.iter().filter(|item| item["kind"] == "device-command").take(8).cloned().collect()
    }

    pub fn active(&self) -> Option<&fractal_client::ClientControlGrantLifecycle> {
        self.grant.as_ref().filter(|_| !self.stopped
            && fresh_timestamp(&self.device.session.expires_at))
    }

    /// No navigation, signature or result is constructed on any refusal path.
    pub fn allows(&self, command: &Value, current_screen: &str) -> bool {
        let Some(lifecycle) = self.active() else { return false; };
        let grant = &command["control_grant"];
        let item_id = grant["item_id"].as_str();
        if grant["grant_id"] != lifecycle.grant_id || grant["run_id"] != lifecycle.run_id
            || grant["device"] != self.device.session.device_id
            || grant["session"] != self.device.session.session_actor
            || !fresh(&grant["expires_at"])
            || item_id != lifecycle.item_id.as_deref()
        { return false; }
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
            && (screen != "Attention" || item_id.is_some_and(|item| !item.is_empty()
                && (verb == "phone.inspect" || command["args"]["params"]["id"].as_str() == Some(item))))
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
        let grant = &command["control_grant"];
        if visible_state.screen == "Attention" && visible_state.opened_item.as_ref()
            .is_none_or(|item| grant["item_id"].as_str() != Some(item.id.as_str())) {
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

    #[test]
    fn identity_is_read_only_redacted_and_detects_secret_changes() {
        use fractal_client::device::Profile;
        use sha2::{Digest, Sha256};
        use base64::Engine;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.json");
        let normal = dir.path().join("normal.json");
        let mut device = fixture().0.device;
        device.signing_key = Some(SigningKey::generate(KeyAlgorithm::P256).unwrap());
        let mut profile = Profile { devices: vec![device] };
        profile.save(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let raw: Value = serde_json::from_slice(&before).unwrap();
        let output = identity(&path, &normal).unwrap();
        assert_eq!(output.as_object().unwrap().len(), 5);
        let point = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(output["public_key"].as_str().unwrap().strip_prefix("p256:").unwrap()).unwrap();
        assert_eq!(point.len(), 65);
        assert_eq!(output["fingerprint"], format!("{:x}", Sha256::digest(&point)));
        assert_eq!(output["device_id"], raw["devices"][0]["session"]["device_id"]);
        assert_eq!(output["session_actor"], raw["devices"][0]["session"]["session_actor"]);
        let printed = serde_json::to_string(&output).unwrap();
        for (section, field) in [("signing_key", "pkcs8"), ("session", "credential")] {
            let secret = raw["devices"][0][section][field].as_str().unwrap();
            assert!(!printed.contains(secret));
            assert_eq!(output["profile"]["devices"][0][section][field]["redacted"], "[REDACTED]");
            assert_eq!(output["profile"]["devices"][0][section][field]["sha256"],
                format!("{:x}", Sha256::digest(secret.as_bytes())));
        }
        let mut restored = output["profile"].clone();
        restored["devices"][0]["signing_key"]["pkcs8"] = raw["devices"][0]["signing_key"]["pkcs8"].clone();
        restored["devices"][0]["session"]["credential"] = raw["devices"][0]["session"]["credential"].clone();
        assert_eq!(restored, raw);
        assert_eq!(Sha256::digest(std::fs::read(&path).unwrap()), Sha256::digest(&before));
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert!(identity(&path, &path).is_err());
        #[cfg(unix)] {
            std::os::unix::fs::symlink(&path, &normal).unwrap();
            assert!(identity(&normal, &path).is_err());
        }
        profile.devices[0].session.credential.push('x');
        profile.devices[0].signing_key = Some(SigningKey::generate(KeyAlgorithm::P256).unwrap());
        profile.save(&path).unwrap();
        let changed = identity(&path, &dir.path().join("other.json")).unwrap();
        for (section, field) in [("signing_key", "pkcs8"), ("session", "credential")] {
            assert_ne!(changed["profile"]["devices"][0][section][field]["sha256"],
                output["profile"]["devices"][0][section][field]["sha256"]);
        }
        profile.devices.push(profile.devices[0].clone());
        profile.save(&path).unwrap();
        assert!(identity(&path, &dir.path().join("other.json")).is_err());
    }

    fn fixture() -> (Control, Value) {
        let device: Device = serde_json::from_value(json!({
            "endpoint":"unix:/fixture/fractal-client.sock", "allow_public_http":false,
            "session":{"kind":"paired-session","device_id":"device/fixture",
                "person_id":"person/client-control-fixture","session_actor":"person/client-control-fixture/session/tui",
                "credential":"fixture-only-credential-00000000000000000000000000",
                "scopes":["read.projections","control.commands"],"expires_at":"2099-01-01T00:00:00Z"}
        })).unwrap();
        let mut control = Control::new(device);
        control.update_lifecycle(json!({"kind":"client-control-grant-lifecycle","id":"grant/one",
            "revision":"1","updated_at":"2026-11-10T12:00:00Z","reason":"granted",
            "grant_id":"grant/one","run_id":"run/one","sequence":1,"state":"started",
            "device":control.device.session.device_id,"session":control.device.session.session_actor}));
        let command = json!({"id":"device-command/one","kind":"device-command",
            "device_id":control.device.session.device_id,"session_actor":control.device.session.session_actor,
            "issued_by":"agent/fixture","grant_id":"grant/one","run_id":"run/one",
            "command_kind":"phone.navigate","args":{"screen":"NowRoot"},
            "control_grant":{"grant_id":"grant/one","run_id":"run/one",
                "device":control.device.session.device_id,"session":control.device.session.session_actor,
                "actor_agent":"agent/fixture","verbs":["phone.inspect","phone.navigate"],
                "screens":["NowRoot","Attention"],"max_commands":50,"expires_at":"2099-01-01T00:00:00Z"},
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
                "verb" => command["control_grant"]["verbs"] = json!(["phone.inspect"]),
                "screen" => command["control_grant"]["screens"] = json!(["Attention"]),
                "expired grant" => command["control_grant"]["expires_at"] = json!("2000-01-01T00:00:00Z"),
                "expired command" => command["expires_at"] = json!("2000-01-01T00:00:00Z"),
                "budget" => command["control_grant"]["max_commands"] = json!(0),
                "read mutation" => command["command_kind"] = json!("phone.message.read"),
                "conversation" => command["args"] = json!({"screen":"Conversation","params":{"target":"agent/fixture"}}),
                "answer" => command["command_kind"] = json!("work.answer"),
                "close" => command["command_kind"] = json!("work.close"),
                "wrong item" => {
                    command["control_grant"]["item_id"] = json!("attention/seed");
                    control.grant.as_mut().unwrap().lifecycle.item_id = Some("attention/seed".into());
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
        control.update_lifecycle(serde_json::to_value(grant).unwrap());
        assert!(control.active().is_none());
        assert!(control.result(&command, visible()).unwrap().is_none());
    }

    #[test]
    fn shared_lifecycle_traces_drive_the_shipped_reducer() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../fixtures/clients/client-control-lifecycle-v1.json")).unwrap();
        for trace in vectors["traces"].as_array().unwrap() {
            let (mut control, mut command) = fixture();
            control.grant = None;
            control.sequences.clear();
            control.device.session.device_id = "device/test-tui".into();
            control.device.session.session_actor = "client-session/test-tui".into();
            command["device_id"] = json!(control.device.session.device_id);
            command["session_actor"] = json!(control.device.session.session_actor);
            command["grant_id"] = trace["grant_id"].clone();
            command["run_id"] = json!("mission-run/golden");
            command["control_grant"]["device"] = command["device_id"].clone();
            command["control_grant"]["session"] = command["session_actor"].clone();
            command["control_grant"]["grant_id"] = command["grant_id"].clone();
            command["control_grant"]["run_id"] = command["run_id"].clone();
            command["control_grant"]["item_id"] = trace["item_id"].clone();
            for step in trace["steps"].as_array().unwrap() {
                let before = control.grant.clone();
                if let Some(resource) = step.get("resource") {
                    control.ingest(&json!({"kind":"changes","id":"commands","upserts":[resource],"removes":[]}));
                }
                match step["expected"].as_str().unwrap() {
                    "activate" | "allow-lifecycle-gate-only" => assert!(control.allows(&command, "NowRoot")),
                    "terminate" | "remain-terminal" | "refuse-terminal-lifecycle" | "refuse-no-active-lifecycle" => {
                        assert!(control.active().is_none());
                        assert!(control.result(&command, visible()).unwrap().is_none());
                    }
                    "refuse-invalid-frame" | "ignore-stale-started" | "ignore-item-mismatch" | "ignore-final-grant" => {
                        assert_eq!(control.grant, before);
                    }
                    expected => panic!("Unhandled shared trace expectation: {expected}"),
                }
            }
        }
        let (mut control, command) = fixture();
        let mut revoked = serde_json::to_value(control.grant.as_ref().unwrap()).unwrap();
        revoked["state"] = json!("revoked");
        revoked["sequence"] = json!(2);
        control.update_lifecycle(revoked.clone());
        revoked["state"] = json!("started");
        revoked["sequence"] = json!(99);
        control.update_lifecycle(revoked);
        assert!(control.active().is_none());
        assert!(control.result(&command, visible()).unwrap().is_none());
    }

    #[test]
    fn bound_item_cannot_be_removed_or_changed_before_shipped_navigation_or_signing() {
        use fractal_schema::client_control::ClientVisibleItem;
        fn native_asks() -> crate::ui::Ui {
            use crate::ui::view::{AttentionKind, Load};
            let mut world = crate::ui::demo::test_world();
            let Load::Ready(items) = &mut world.attention else { panic!("attention must be loaded"); };
            let item = items.first_mut().unwrap();
            item.id = "attention/A".into();
            item.title = "Ask A".into();
            item.kind = AttentionKind::Request { from:"Fixture producer".into(),from_id:"agent/fixture".into(),
                question:"Question A".into(),structured:None,images:Vec::new() };
            let mut other = item.clone();
            other.id = "attention/B".into();
            other.title = "Ask B".into();
            items.push(other);
            crate::ui::Ui::new(world)
        }
        fn bind_a(control: &mut Control) {
            let mut frame = serde_json::to_value(control.grant.as_ref().unwrap()).unwrap();
            frame["item_id"] = json!("attention/A");
            control.grant = None;
            control.sequences.clear();
            control.ingest(&json!({"kind":"snapshot","id":"commands","items":[frame]}));
        }
        for supplied in [None, Some(Value::Null), Some(json!("attention/B"))] {
            let (mut control, mut command) = fixture();
            bind_a(&mut control);
            if let Some(item) = supplied { command["control_grant"]["item_id"] = item; }
            command["args"] = json!({"screen":"Attention","params":{"id":"attention/B"}});
            let mut ui = native_asks();
            let before = ui.control_visible();
            assert!(!ui.control_apply(&control, &command));
            assert_eq!(ui.control_visible(), before);
            let rendered = ClientVisibleState { route:"home:attention/B".into(),screen:"Attention".into(),
                visible_item_ids:vec!["attention/B".into()],opened_item:Some(ClientVisibleItem {
                    id:"attention/B".into(),title:"Ask B".into(),body:"Question B".into() }) };
            // Deliberately no key: a signer call would error, not return None.
            assert!(control.result(&command, rendered).unwrap().is_none());
        }
        let (mut control, mut command) = fixture();
        bind_a(&mut control);
        command["control_grant"]["item_id"] = json!("attention/A");
        command["args"] = json!({"screen":"Attention","params":{"id":"attention/A"}});
        let mut ui = native_asks();
        assert!(ui.control_apply(&control, &command));
        let rendered = ClientVisibleState { route:"home:attention/A".into(),screen:"Attention".into(),
            visible_item_ids:vec!["attention/A".into()],opened_item:Some(ClientVisibleItem {
                id:"attention/A".into(),title:"Ask A".into(),body:"Question A".into() }) };
        control.device.signing_key = Some(SigningKey::generate(KeyAlgorithm::P256).unwrap());
        assert!(control.result(&command, rendered).unwrap().unwrap().signature.is_some());
        let (mut control, mut command) = fixture();
        assert!(control.allows(&command, "NowRoot")); // both unbound
        command["args"] = json!({"screen":"Attention","params":{"id":"attention/B"}});
        let mut ui = native_asks();
        let before = ui.control_visible();
        assert!(!ui.control_apply(&control, &command)); // Attention always bound
        assert_eq!(ui.control_visible(), before);
        assert!(control.result(&command, visible()).unwrap().is_none());
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
