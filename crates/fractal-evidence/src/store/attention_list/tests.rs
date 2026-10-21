#![allow(unused_imports)]
use crate::PublishedViewsAttentionStore;
use crate::PublishedViewsStore;
use super::*;


#[test]
fn a_view_failing_for_its_limit_is_withdrawn_until_it_publishes_again() {
    let store = Store::open_memory("alder").unwrap();
    assert!(!store.published_view_serving("attention"), "nothing serves without a refresher");
    store.start_attention_list_refresher().unwrap();
    for view in PUBLISHED_VIEWS {
        assert!(store.published_view_serving(view));
    }
    assert!(!store.published_view_serving("missions"), "not this refresher's view");
    // A first failure is not yet a withdrawal; one that has lasted the limit is.
    store.note_view_refreshed("summary", false);
    assert!(store.published_view_serving("summary"));
    store.smalltalk.attention_list.health[3]
        .failing_since
        .store(now_ms() as u64 - WITHDRAW_AFTER_MS - 1, AtomicOrdering::Release);
    store.note_view_refreshed("summary", false);
    assert!(!store.published_view_serving("summary"));
    assert!(!store.collection_view_published("summary"), "windows follow commits again");
    assert!(store.published_view_serving("attention"), "the other views keep serving");
    store.note_view_refreshed("summary", true);
    assert!(store.published_view_serving("summary"));
    assert!(store.collection_view_published("summary"), "windows reread the publication");
    // A stopped refresher serves nothing, whatever its views' health.
    store.publish_collection_view("attention");
    store.stop_attention_list_refresher();
    assert!(!store.attention_list_refresher_running());
    for view in PUBLISHED_VIEWS {
        assert!(!store.published_view_serving(view));
        assert!(!store.collection_view_published(view));
    }
}
