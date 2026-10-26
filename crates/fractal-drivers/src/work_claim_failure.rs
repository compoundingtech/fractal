//! The CLI's socket-connect failure receipt, carried by native tool output.
//! Raw tool payloads remain redacted; only this bounded, typed observation is retained.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PREFIX: &str = "st-work-claim-socket-failure: ";
pub const CODE: &str = "work-claim-socket-failed";

/// Known Unix socket permission/path defects cannot improve while waiting for a daemon.
/// Other connect errors retain the shared client's existing outage-wait behavior.
pub fn is_socket_access_defect(errno: i32) -> bool {
    matches!(errno, libc::EPERM | libc::EACCES | libc::EINVAL | libc::ENAMETOOLONG
        | libc::ENOTDIR | libc::ENOTSOCK | libc::ELOOP)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub actor: String,
    pub step_run: String,
    pub socket: String,
    pub os_error: i32,
    pub started_at_unix_ms: u64,
    pub observed_at_unix_ms: u64,
}

impl Receipt {
    pub fn valid(&self) -> bool {
        let bounded = |text: &str| {
            !text.is_empty()
                && text.len() <= 1024
                && !text.chars().any(|ch| ch.is_control() || ch == '`')
        };
        self.actor.starts_with("agent/")
            && bounded(&self.actor)
            && self.step_run.starts_with("step-run/")
            && bounded(&self.step_run)
            && self.socket.starts_with('/')
            && bounded(&self.socket)
            && (1..=4095).contains(&self.os_error)
            && self.started_at_unix_ms > 0
            && self.started_at_unix_ms <= self.observed_at_unix_ms
    }

    pub fn reason(&self) -> String {
        let cause = std::io::Error::from_raw_os_error(self.os_error);
        let advice = if matches!(self.os_error, libc::EPERM | libc::EACCES) {
            "Inspect the seat's sandbox permissions and socket path; no claim request was sent."
        } else if is_socket_access_defect(self.os_error) {
            "Inspect the socket path; no claim request was sent."
        } else {
            "No claim request was sent."
        };
        format!(
            "`{}` reported that `st work claim {}` could not connect to daemon Unix socket `{}`: {cause}. {advice}",
            self.actor, self.step_run, self.socket
        )
    }

    pub fn follows_wake(&self, sent: u128, adapter_observed: u64) -> bool {
        self.valid()
            && self.started_at_unix_ms as u128 >= sent
            && self.observed_at_unix_ms <= adapter_observed
    }
}

/// Only a failed native tool result is inspected, never assistant text or tool arguments.
pub fn from_tool_result(body: &Value) -> Option<Receipt> {
    if body["status"] != "error" {
        return None;
    }
    fn find(value: &Value, depth: usize) -> Option<Receipt> {
        if depth > 4 {
            return None;
        }
        match value {
            Value::String(text) => text.lines().find_map(|line| {
                let encoded = line.strip_prefix(PREFIX)?;
                if encoded.len() > 4096 {
                    return None;
                }
                let receipt: Receipt = serde_json::from_str(encoded).ok()?;
                receipt.valid().then_some(receipt)
            }),
            Value::Array(parts) => parts.iter().find_map(|part| find(part, depth + 1)),
            Value::Object(parts) => ["text", "content", "output"]
                .iter()
                .find_map(|key| parts.get(*key).and_then(|part| find(part, depth + 1))),
            _ => None,
        }
    }
    find(&body["content"], 0)
}

pub fn key(actor: &str, step: &str, attempt: u32, epoch: u32, incarnation: &str) -> String {
    format!("work-claim-socket:{actor}:{step}:{attempt}:{epoch}:{incarnation}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claim_socket_resource_pressure_is_reported_without_path_advice() {
        let receipt = Receipt {
            actor: "agent/node.worker".into(),
            step_run: "step-run/sample/work".into(),
            socket: "/tmp/st.sock".into(),
            os_error: libc::EMFILE,
            started_at_unix_ms: 1233,
            observed_at_unix_ms: 1234,
        };
        let reason = receipt.reason();
        assert!(reason.contains(&std::io::Error::from_raw_os_error(libc::EMFILE).to_string()));
        assert!(!reason.contains("Inspect"));
        assert!(!reason.contains("permissions"));
        let permission = Receipt { os_error: libc::EPERM, ..receipt.clone() }.reason();
        assert!(permission.contains("Inspect the seat's sandbox permissions and socket path"));
        let path = Receipt { os_error: libc::ENAMETOOLONG, ..receipt }.reason();
        assert!(path.contains("Inspect the socket path"));
        assert!(!path.contains("permissions"));
    }

    #[test]
    fn claim_socket_receipt_requires_failed_tool_and_bounded_typed_cli_output() {
        let receipt = Receipt {
            actor: "agent/node.worker".into(),
            step_run: "step-run/sample/work".into(),
            socket: "/tmp/st.sock".into(),
            os_error: libc::EPERM,
            started_at_unix_ms: 1233,
            observed_at_unix_ms: 1234,
        };
        let line = format!("{PREFIX}{}", serde_json::to_string(&receipt).unwrap());
        let body = json!({"status":"error","content":[{"type":"text","text":line}]});
        assert_eq!(from_tool_result(&body), Some(receipt.clone()));
        assert!(receipt.follows_wake(1233, 1234));
        assert!(
            !receipt.follows_wake(1234, 1234),
            "a delayed old command cannot attach to a new wake"
        );
        assert!(
            !receipt.follows_wake(1233, 1233),
            "future receipts are not current evidence"
        );
        for body in [
            json!({"status":"success","content":line}),
            json!({"status":"error","content":"Operation not permitted on the st socket"}),
            json!({"status":"error","content":format!("quoted: {line}")}),
            json!({"status":"error","arguments":line}),
            json!({"status":"error","content":format!("{PREFIX}{{\"actor\":\"agent/node.worker\"}}")}),
        ] {
            assert!(from_tool_result(&body).is_none());
        }
        let mut malformed = receipt.clone();
        malformed.os_error = 0;
        assert!(!malformed.valid());
        malformed = receipt;
        malformed.socket = "https://localhost/secret".into();
        assert!(!malformed.valid());
    }
}
