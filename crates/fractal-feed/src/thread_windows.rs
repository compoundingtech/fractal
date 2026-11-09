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
