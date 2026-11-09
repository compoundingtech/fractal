//! Thread interests use the ordinary authenticated collection socket.

use fractal_client::{ClientError, CollectionStream, ThreadListScope};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ThreadInterest {
    Agent(String),
    Inbox { source: String, episode: String },
    Mission(String),
    RecentlyClosed,
    Timeline(String),
    Summary(String),
}

pub(super) struct ThreadSubscription {
    pub(super) interest: ThreadInterest,
    pub(super) id: String,
}

impl ThreadSubscription {
    pub(super) fn new(interest: ThreadInterest) -> Self {
        Self {
            interest,
            id: format!("thread-window/{}", uuid::Uuid::now_v7()),
        }
    }

    pub(super) async fn subscribe(&self, stream: &mut CollectionStream) -> Result<(), ClientError> {
        let scope = match &self.interest {
            ThreadInterest::Agent(id) => ThreadListScope::Agent(id),
            ThreadInterest::Inbox { source, episode } => ThreadListScope::Inbox { source, episode },
            ThreadInterest::Mission(id) => ThreadListScope::Mission(id),
            ThreadInterest::RecentlyClosed => ThreadListScope::RecentlyClosed,
            ThreadInterest::Summary(id) => {
                return stream.subscribe_thread_summary(&self.id, id).await;
            }
            ThreadInterest::Timeline(id) => {
                return stream.subscribe_thread_timeline(&self.id, id, 50).await;
            }
        };
        stream.subscribe_thread_list(&self.id, scope, 30).await
    }
}

pub(super) const MAX_THREADS: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_inbox_windows_fit_the_bounded_thread_budget() {
        // An expanded Inbox item needs both lists and both live detail windows.
        let required = [
            ThreadInterest::Inbox {
                source: "agent/example".into(),
                episode: "claim/example".into(),
            },
            ThreadInterest::RecentlyClosed,
            ThreadInterest::Summary("thread/example".into()),
            ThreadInterest::Timeline("thread/example".into()),
        ];
        assert_eq!(required.iter().take(MAX_THREADS).count(), required.len());
        assert_eq!(MAX_THREADS, 4);
    }
}

/// Current connection grants. Read access never implies permission to mutate a thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThreadAccess {
    pub start: bool,
    pub reply: bool,
    pub rename: bool,
    pub close: bool,
    pub reopen: bool,
    pub mark_read: bool,
}

pub(super) fn access(
    capabilities: &fractal_client::Capabilities,
    has_signer: bool,
) -> ThreadAccess {
    use fractal_client::{CapabilityState, TransportKind};
    let granted = |id: &str| {
        capabilities
            .capabilities
            .iter()
            .any(|capability| capability.id == id && capability.state == CapabilityState::Granted)
    };
    let native_unsigned =
        capabilities.transport == TransportKind::Unix && capabilities.grant_id.is_none();
    let browser_send = capabilities.capabilities.iter().any(|capability| {
        capability.id == "thread-browser-send"
            && capability.version >= 1
            && capability.state == CapabilityState::Granted
    });
    let can_message = has_signer || native_unsigned || browser_send;
    ThreadAccess {
        start: can_message && granted("thread.start"),
        reply: can_message && granted("thread.reply"),
        rename: granted("thread.rename"),
        close: granted("thread.close"),
        reopen: granted("thread.reopen"),
        mark_read: granted("thread.read"),
    }
}

#[cfg(test)]
mod access_tests {
    use super::*;

    fn capabilities(paired: bool) -> fractal_client::Capabilities {
        serde_json::from_value(serde_json::json!({
            "kind":"capabilities","session_actor":"person/operator","transport":"unix",
            "capabilities":[],
            "limits":{"max_page_items":50,"max_event_items":50,"max_response_bytes":100000,"max_wait_ms":1000},
            "event_cursor":"","oldest_event_cursor":"","schemas":[],
            "grant_id":if paired { Some("claim/grant") } else { None },
        })).unwrap()
    }

    #[test]
    fn a_read_grant_never_becomes_a_send_or_metadata_grant() {
        let mut caps = capabilities(false);
        caps.capabilities = vec![fractal_client::Capability {
            id: "threads-v1".into(),
            version: 2,
            state: fractal_client::CapabilityState::Granted,
        }];
        assert_eq!(access(&caps, false), ThreadAccess::default());
    }

    #[test]
    fn paired_native_messages_need_a_signer_or_the_exact_browser_assertion() {
        let mut caps = capabilities(true);
        caps.capabilities = [
            "thread.start",
            "thread.reply",
            "thread.rename",
            "thread.close",
            "thread.reopen",
            "thread.read",
        ]
        .into_iter()
        .map(|id| fractal_client::Capability {
            id: id.into(),
            version: 0,
            state: fractal_client::CapabilityState::Granted,
        })
        .collect();
        let unsigned = access(&caps, false);
        assert!(!unsigned.start && !unsigned.reply);
        assert!(unsigned.rename && unsigned.close && unsigned.reopen && unsigned.mark_read);
        let signed = access(&caps, true);
        assert!(signed.start && signed.reply);
        caps.capabilities.push(fractal_client::Capability {
            id: "thread-browser-send".into(),
            version: 1,
            state: fractal_client::CapabilityState::Granted,
        });
        assert!(access(&caps, false).reply);
    }
}
