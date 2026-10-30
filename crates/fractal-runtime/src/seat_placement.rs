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
    memory_max: u64,
    memory_swap_max: u64,
    oom_policy: OomPolicy,
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
        if fields.memory_max == 0 {
            return Err("memoryMax must be a positive byte count");
        }
        Ok(Self(fields))
    }
}


impl UnlanedSeatPlacement {
    pub(crate) fn apply(&self, command: &mut Command) {
        let fields = &self.0;
        let oom = match fields.oom_policy {
            OomPolicy::Continue => "continue",
            OomPolicy::Stop => "stop",
            OomPolicy::Kill => "kill",
        };
        command.arg(format!("--slice={}", fields.slice))
            .args(["-p", &format!("MemoryMax={}", fields.memory_max)])
            .args(["-p", &format!("MemorySwapMax={}", fields.memory_swap_max)])
            .args(["-p", &format!("OOMPolicy={oom}")]);
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
        for key in ["slice", "memoryMax", "memorySwapMax", "oomPolicy"] {
            let mut invalid = valid.clone();
            invalid.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<UnlanedSeatPlacement>(invalid).is_err());
        }
    }
}
