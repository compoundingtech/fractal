//! Visit-scoped unread reminders, independent of natural read history.
//!
//! Only marks captured on entry can clear, and each needs its own visible anchor.
//! Explicitly marking unread discards that capture until the next deliberate visit.

use std::collections::HashSet;

use fractal_client::{ThreadUnreadMark, ThreadUnreadReminder};

#[derive(Default)]
pub(super) struct Visit {
    thread: Option<String>,
    captured: Vec<ThreadUnreadMark>,
}

impl Visit {
    /// Capture once on entry, never on a render or reminder refresh of this visit.
    pub(super) fn begin(&mut self, thread: &str, reminder: Option<&ThreadUnreadReminder>) {
        if self.thread.as_deref() == Some(thread) {
            return;
        }
        self.thread = Some(thread.to_owned());
        self.captured = reminder.map_or_else(Vec::new, |reminder| reminder.marks.clone());
    }

    #[cfg(test)]
    pub(super) fn leave(&mut self) {
        self.thread = None;
        self.captured.clear();
    }

    /// Keep this visit sticky, including reminders already present when marked.
    pub(super) fn marked_here(&mut self, thread: &str) {
        if self.thread.as_deref() != Some(thread) {
            self.thread = Some(thread.to_owned());
        }
        self.captured.clear();
    }

    pub(super) fn viewed(&mut self, thread: &str, visible: &HashSet<String>) -> Vec<String> {
        if self.thread.as_deref() != Some(thread) {
            return Vec::new();
        }
        let mut cleared = Vec::new();
        self.captured.retain_mut(|mark| {
            if visible.contains(&mark.anchor_message_id) {
                cleared.push(std::mem::take(&mut mark.mark_id));
                false
            } else {
                true
            }
        });
        cleared
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(key: &str, anchor: &str) -> ThreadUnreadMark {
        ThreadUnreadMark {
            mark_id: smallclaims::store::operation_id_for_key(key),
            anchor_message_id: anchor.to_owned(),
        }
    }

    fn reminder(marks: &[ThreadUnreadMark]) -> ThreadUnreadReminder {
        ThreadUnreadReminder {
            marks: marks.to_vec(),
            anchor_message_id: marks[0].anchor_message_id.clone(),
        }
    }

    fn visible(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn three_concurrent_marks_clear_only_at_their_own_visible_anchors() {
        let marks = [
            mark("unread-visit/first", "message/first"),
            mark("unread-visit/second", "message/second"),
            mark("unread-visit/third", "message/third"),
        ];
        let mut visit = Visit::default();
        visit.begin("thread/a", Some(&reminder(&marks)));

        assert!(visit.viewed("thread/a", &visible(&["message/other"])).is_empty());
        assert_eq!(
            visit.viewed("thread/a", &visible(&["message/second"])),
            vec![marks[1].mark_id.clone()]
        );
        assert!(visit.viewed("thread/a", &visible(&["message/second"])).is_empty());
        assert_eq!(
            visit.viewed("thread/a", &visible(&["message/first", "message/third"])),
            vec![marks[0].mark_id.clone(), marks[2].mark_id.clone()]
        );
        assert!(visit.viewed("thread/a", &visible(&["message/first", "message/third"])).is_empty());
    }

    #[test]
    fn same_visit_reminder_refresh_preserves_concurrent_new_marks() {
        let old = mark("unread-visit/old", "message/old");
        let new = mark("unread-visit/new", "message/new");
        let mut visit = Visit::default();
        visit.begin("thread/a", Some(&reminder(std::slice::from_ref(&old))));
        visit.begin("thread/a", Some(&reminder(&[new.clone(), old.clone()])));

        assert_eq!(
            visit.viewed("thread/a", &visible(&["message/old", "message/new"])),
            vec![old.mark_id]
        );
        assert!(visit.viewed("thread/a", &visible(&["message/new"])).is_empty());

        visit.leave();
        visit.begin("thread/a", Some(&reminder(std::slice::from_ref(&new))));
        assert_eq!(
            visit.viewed("thread/a", &visible(&["message/new"])),
            vec![new.mark_id]
        );
    }

    #[test]
    fn explicitly_marked_visit_keeps_current_and_new_reminders_sticky() {
        let old = mark("unread-visit/sticky-old", "message/old");
        let new = mark("unread-visit/sticky-new", "message/new");
        let marks = [old, new];
        let mut visit = Visit::default();
        visit.begin("thread/a", Some(&reminder(&marks[..1])));
        visit.marked_here("thread/a");
        visit.begin("thread/a", Some(&reminder(&marks)));
        let anchors = visible(&["message/old", "message/new"]);
        assert!(visit.viewed("thread/a", &anchors).is_empty());

        visit.leave();
        visit.begin("thread/a", Some(&reminder(&marks)));
        assert_eq!(
            visit.viewed("thread/a", &anchors),
            marks.iter().map(|mark| mark.mark_id.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn same_thread_rerender_does_not_recapture_emitted_marks() {
        let marks = [mark("unread-visit/rerender", "message/anchor")];
        let reminder = reminder(&marks);
        let anchors = visible(&["message/anchor"]);
        let mut visit = Visit::default();
        visit.begin("thread/a", Some(&reminder));
        assert_eq!(visit.viewed("thread/a", &anchors), vec![marks[0].mark_id.clone()]);
        visit.begin("thread/a", Some(&reminder));
        assert!(visit.viewed("thread/a", &anchors).is_empty());
    }

    #[test]
    fn entering_without_reminders_does_not_capture_later_refreshes() {
        let marks = [mark("unread-visit/late", "message/anchor")];
        let mut visit = Visit::default();
        visit.begin("thread/a", None);
        visit.begin("thread/a", Some(&reminder(&marks)));
        assert!(visit.viewed("thread/a", &visible(&["message/anchor"])).is_empty());
    }

    #[test]
    fn thread_navigation_and_leave_isolate_captures() {
        let a = [mark("unread-visit/thread-a", "message/a")];
        let b = [mark("unread-visit/thread-b", "message/b")];
        let anchors = visible(&["message/a", "message/b"]);
        let mut visit = Visit::default();
        visit.begin("thread/a", Some(&reminder(&a)));
        assert!(visit.viewed("thread/b", &anchors).is_empty());
        visit.begin("thread/b", Some(&reminder(&b)));
        assert!(visit.viewed("thread/a", &anchors).is_empty());
        assert_eq!(visit.viewed("thread/b", &anchors), vec![b[0].mark_id.clone()]);
        visit.begin("thread/a", Some(&reminder(&a)));
        visit.leave();
        assert!(visit.viewed("thread/a", &anchors).is_empty());
        visit.begin("thread/a", Some(&reminder(&a)));
        assert_eq!(visit.viewed("thread/a", &anchors), vec![a[0].mark_id.clone()]);
    }

    #[test]
    fn explicitly_marking_before_begin_establishes_a_sticky_visit() {
        let marks = [mark("unread-visit/before-begin", "message/anchor")];
        let reminder = reminder(&marks);
        let anchors = visible(&["message/anchor"]);
        let mut visit = Visit::default();
        visit.marked_here("thread/a");
        visit.begin("thread/a", Some(&reminder));
        assert!(visit.viewed("thread/a", &anchors).is_empty());
        visit.leave();
        visit.begin("thread/a", Some(&reminder));
        assert_eq!(visit.viewed("thread/a", &anchors), vec![marks[0].mark_id.clone()]);
    }

    #[test]
    fn distinct_marks_sharing_an_anchor_are_both_emitted_once() {
        let marks = [
            mark("unread-visit/shared-first", "message/shared"),
            mark("unread-visit/shared-second", "message/shared"),
        ];
        let mut visit = Visit::default();
        visit.begin("thread/a", Some(&reminder(&marks)));
        let anchors = visible(&["message/shared"]);
        assert_eq!(
            visit.viewed("thread/a", &anchors),
            marks.iter().map(|mark| mark.mark_id.clone()).collect::<Vec<_>>()
        );
        assert!(visit.viewed("thread/a", &anchors).is_empty());
    }
}
