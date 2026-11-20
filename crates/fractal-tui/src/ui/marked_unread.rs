//! Owner-wide unread reminders use the ordinary authenticated list seam.
use std::{collections::HashSet, future::Future};
use fractal_client::{Client, ClientError, Envelope, ThreadListScope, ThreadSummary, ThreadSummaryPage};

const MAX_PAGES: usize = 1000;

#[derive(Debug)]
pub(super) enum Failure {
    Fetch(ClientError),
    RepeatedCursor,
    PageBoundExhausted,
}

impl Failure {
    pub(super) fn human(&self) -> String {
        match self {
            Self::Fetch(error) => error.human(),
            Self::RepeatedCursor => "Unread reminder listing returned a repeated continuation cursor.".into(),
            Self::PageBoundExhausted => "Unread reminder listing exceeded its page bound.".into(),
        }
    }
}

pub(super) async fn read(client: &Client) -> Result<Vec<Envelope<ThreadSummary>>, Failure> {
    pages(MAX_PAGES, |cursor| async move {
        client.thread_list_in(ThreadListScope::MarkedUnread, cursor.as_deref(), Some(100)).await
    }).await
}

async fn pages<F, Fut>(max_pages: usize, mut fetch: F) -> Result<Vec<Envelope<ThreadSummary>>, Failure>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<Envelope<ThreadSummaryPage>, ClientError>>,
{
    let mut cursor = None;
    let mut seen = HashSet::new();
    let mut seen_cursors = HashSet::new();
    let mut items = Vec::new();
    for _ in 0..max_pages {
        let page = fetch(cursor).await.map_err(Failure::Fetch)?;
        for value in page.value.items {
            if seen.insert(value.id.clone()) {
                items.push(Envelope {
                    api_version: page.api_version.clone(), request_id: page.request_id.clone(),
                    snapshot: page.snapshot.clone(), value,
                });
            }
        }
        // An empty page can represent filtered/dependency-pending rows, not exhaustion.
        let Some(next) = page.value.next_cursor else {
            return Ok(items);
        };
        if !seen_cursors.insert(next.clone()) {
            return Err(Failure::RepeatedCursor);
        }
        cursor = Some(next);
    }
    Err(Failure::PageBoundExhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque, future::ready};

    fn page(ids: &[&str], cursor: Option<&str>) -> Envelope<ThreadSummaryPage> {
        serde_json::from_value(serde_json::json!({
            "api_version": fractal_client::API_VERSION, "request_id": "request/test",
            "snapshot": { "id": "snapshot/test", "host_id": "host/test", "store_index": 1,
                "projection_version": "1", "created_at": "2026-11-16T00:00:00Z" },
            "value": { "items": ids.iter().map(|id| serde_json::json!({
                "id": id, "agent_id": "agent/test", "title": id, "state": "closed",
                "revision": "claim/current", "title_revision": "claim/title",
                "message_count": 1, "unread": false, "unread_reminder": null
            })).collect::<Vec<_>>(), "next_cursor": cursor }
        })).unwrap()
    }

    #[tokio::test]
    async fn unread_empty_page_with_continuation_is_not_an_empty_list() {
        let replies = RefCell::new(VecDeque::from([
            page(&[], Some("cursor/filtered")),
            page(&["thread/marked"], Some("cursor/last")),
            page(&[], None),
        ]));
        let cursors = RefCell::new(Vec::new());
        let rows = pages(MAX_PAGES, |cursor| {
            cursors.borrow_mut().push(cursor);
            ready(Ok(replies.borrow_mut().pop_front().unwrap()))
        }).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value.id, "thread/marked");
        assert_eq!(*cursors.borrow(), [None, Some("cursor/filtered".into()), Some("cursor/last".into())]);
        assert!(replies.borrow().is_empty());
    }

    #[tokio::test]
    async fn unread_empty_list_is_known_only_after_the_last_cursor() {
        let replies = RefCell::new(VecDeque::from([page(&[], Some("cursor/next")), page(&[], None)]));
        let rows = pages(MAX_PAGES, |_| ready(Ok(replies.borrow_mut().pop_front().unwrap()))).await.unwrap();
        assert!(rows.is_empty());
        assert!(replies.borrow().is_empty());
    }

    #[tokio::test]
    async fn unread_backdated_receipt_pagination_does_not_duplicate_threads() {
        let replies = RefCell::new(VecDeque::from([
            page(&["thread/one"], Some("cursor/next")), page(&["thread/one", "thread/two"], None)
        ]));
        let rows = pages(MAX_PAGES, |_| ready(Ok(replies.borrow_mut().pop_front().unwrap()))).await.unwrap();
        assert_eq!(rows.iter().map(|row| row.value.id.as_str()).collect::<Vec<_>>(), ["thread/one", "thread/two"]);
    }

    #[tokio::test]
    async fn unread_repeated_cursor_fails_without_fetching_it_again() {
        let replies = RefCell::new(VecDeque::from([
            page(&[], Some("cursor/loop")),
            page(&[], Some("cursor/loop")),
        ]));
        let cursors = RefCell::new(Vec::new());
        let result = pages(MAX_PAGES, |cursor| {
            cursors.borrow_mut().push(cursor);
            ready(Ok(replies.borrow_mut().pop_front().unwrap()))
        }).await;
        assert!(matches!(result, Err(Failure::RepeatedCursor)));
        assert_eq!(*cursors.borrow(), [None, Some("cursor/loop".into())]);
        assert!(replies.borrow().is_empty());
    }

    #[tokio::test]
    async fn unread_page_bound_exhaustion_is_not_an_empty_list() {
        let replies = RefCell::new(VecDeque::from([
            page(&[], Some("cursor/one")),
            page(&[], Some("cursor/two")),
            page(&["thread/marked"], None),
        ]));
        let result = pages(2, |_| {
            ready(Ok(replies.borrow_mut().pop_front().unwrap()))
        }).await;
        assert!(matches!(result, Err(Failure::PageBoundExhausted)));
        assert_eq!(replies.borrow().len(), 1);
    }

    #[tokio::test]
    async fn unread_fetch_failure_preserves_the_error_instead_of_partial_results() {
        let replies = RefCell::new(VecDeque::from([
            Ok(page(&["thread/marked"], Some("cursor/next"))),
            Err(ClientError::Transport("connection closed".into())),
        ]));
        let result = pages(MAX_PAGES, |_| {
            ready(replies.borrow_mut().pop_front().unwrap())
        }).await;
        match result {
            Err(Failure::Fetch(ClientError::Transport(message))) => {
                assert_eq!(message, "connection closed");
            }
            other => panic!("expected the original fetch failure, got {other:?}"),
        }
        assert!(replies.borrow().is_empty());
    }
}
