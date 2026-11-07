//! Thread interests use the ordinary authenticated collection socket.

use fractal_client::{ClientError, CollectionStream, ThreadListScope};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ThreadInterest {
    Agent(String),
    Ask { id: String, episode: String },
    Mission(String),
    RecentlyClosed,
    Timeline(String),
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
            ThreadInterest::Ask { id, episode } => ThreadListScope::Ask { id, episode },
            ThreadInterest::Mission(id) => ThreadListScope::Mission(id),
            ThreadInterest::RecentlyClosed => ThreadListScope::RecentlyClosed,
            ThreadInterest::Timeline(id) => {
                return stream.subscribe_thread_timeline(&self.id, id, 50).await;
            }
        };
        stream.subscribe_thread_list(&self.id, scope, 30).await
    }
}

pub(super) const MAX_THREADS: usize = 3;
