use serde::{Deserialize, Serialize};
use std::process::Command;

/// Daemon-owned placement for seats without a declared lane. Memory values are bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "PlacementFields")]
pub struct UnlanedSeatPlacement(PlacementFields);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlacementFields {
    slice: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_max: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_swap_max: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oom_policy: Option<OomPolicy>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum OomPolicy {
    Continue,
    Stop,
    Kill,
}

impl TryFrom<PlacementFields> for UnlanedSeatPlacement {
    type Error = &'static str;

    fn try_from(fields: PlacementFields) -> Result<Self, Self::Error> {
        let stem = fields.slice.strip_suffix(".slice").ok_or("slice must end in .slice")?;
        if stem.is_empty()
            || stem.starts_with('-')
            || stem.ends_with('-')
            || stem.contains("--")
            || !stem.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err("slice must be a non-root systemd slice name using letters, digits, underscores and single hyphens");
        }
        if fields.memory_max == Some(0) {
            return Err("memoryMax must be a positive byte count");
        }
        Ok(Self(fields))
    }
}


impl UnlanedSeatPlacement {
    pub(crate) fn apply(&self, command: &mut Command) {
        let fields = &self.0;
        command.arg(format!("--slice={}", fields.slice));
        if let Some(memory) = fields.memory_max {
            command.args(["-p", &format!("MemoryMax={memory}")]);
        }
        if let Some(swap) = fields.memory_swap_max {
            command.args(["-p", &format!("MemorySwapMax={swap}")]);
        }
        if let Some(policy) = fields.oom_policy {
            let oom = match policy {
                OomPolicy::Continue => "continue",
                OomPolicy::Stop => "stop",
                OomPolicy::Kill => "kill",
            };
            command.args(["-p", &format!("OOMPolicy={oom}")]);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn policy() -> UnlanedSeatPlacement {
        serde_json::from_str(r#"{"slice":"app-seats.slice","memoryMax":1073741824,"memorySwapMax":0,"oomPolicy":"kill"}"#).unwrap()
    }

    #[test]
    fn invalid_unlaned_seat_placement_is_rejected() {
        let valid = serde_json::to_value(policy()).unwrap();
        for (key, value) in [
            ("slice", serde_json::json!("-.slice")),
            ("slice", serde_json::json!("app--seat.slice")),
            ("slice", serde_json::json!("/app.slice")),
            ("slice", serde_json::json!("app.service")),
            ("memoryMax", serde_json::json!(0)),
            ("memoryMax", serde_json::json!("1G")),
            ("memorySwapMax", serde_json::json!(-1)),
            ("oomPolicy", serde_json::json!("ignore")),
            ("unknown", serde_json::json!(true)),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(serde_json::from_value::<UnlanedSeatPlacement>(invalid).is_err(), "accepted invalid {key}");
        }
        let mut missing_slice = valid;
        missing_slice.as_object_mut().unwrap().remove("slice");
        assert!(serde_json::from_value::<UnlanedSeatPlacement>(missing_slice).is_err());
    }

    #[test]
    fn unlaned_properties_are_independently_optional() {
        for (key, value, argument) in [
            ("memoryMax", serde_json::json!(1024), "MemoryMax=1024"),
            ("memorySwapMax", serde_json::json!(0), "MemorySwapMax=0"),
            ("oomPolicy", serde_json::json!("stop"), "OOMPolicy=stop"),
        ] {
            let mut fields = serde_json::json!({"slice": "app-seats.slice"});
            fields[key] = value;
            let policy: UnlanedSeatPlacement = serde_json::from_value(fields).unwrap();
            let mut command = Command::new("systemd-run");
            policy.apply(&mut command);
            assert_eq!(command.get_args().collect::<Vec<_>>(),
                ["--slice=app-seats.slice", "-p", argument].map(std::ffi::OsStr::new));
        }
        let policy: UnlanedSeatPlacement =
            serde_json::from_str(r#"{"slice":"app-seats.slice"}"#).unwrap();
        let mut command = Command::new("systemd-run");
        policy.apply(&mut command);
        assert_eq!(command.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new("--slice=app-seats.slice")]);
    }
}
