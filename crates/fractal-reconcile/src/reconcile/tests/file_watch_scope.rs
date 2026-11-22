use super::super::{FileChanges, FileWatch};
use crate::incremental::Incremental;
use futures_util::FutureExt as _;
use notify::{Event, EventKind, event::{ModifyKind, RemoveKind, RenameMode}};
use std::{collections::BTreeSet, path::Path};
use tokio::sync::Notify;

#[test]
fn external_inputs_notify_only_their_readers() {
    let incremental = Incremental::default();
    let changes = FileChanges::default();
    let global = Notify::new();
    for index in 0..64 {
        incremental.evaluated(&format!("mission-run/{index}"),
            BTreeSet::from([format!("file/input-{index}")]), None);
    }
    changes.invalidate(&incremental, "file/unrelated", false, &global);
    assert!(global.notified().now_or_never().is_none(), "zero global notifications");
    assert!(changes.notify.notified().now_or_never().is_none(), "zero scoped notifications");
    assert!(changes.runs.lock().unwrap().is_empty());

    changes.invalidate(&incremental, "file/input-7", false, &global);
    assert!(global.notified().now_or_never().is_none(), "file gates do not wake the common loop");
    assert!(changes.notify.notified().now_or_never().is_some());
    assert_eq!(*changes.runs.lock().unwrap(), BTreeSet::from(["mission-run/7".to_owned()]));

    incremental.evaluated("launch-inputs:agent/example", BTreeSet::from(["launch-input/tool".into()]), None);
    changes.invalidate(&incremental, "launch-input/tool", false, &global);
    assert!(global.notified().now_or_never().is_some(), "launch identity invalidates global admission");
    changes.invalidate(&incremental, "file/unrelated", true, &global);
    assert!(global.notified().now_or_never().is_some(), "genuine lost-path rescans fail closed");
}

#[test]
fn relevance_preserves_any_path_referents_and_ancestor_replacements() {
    let target = Path::new("/example/inputs/tool");
    let directory = target.parent().unwrap();
    let referent = Path::new("/example/referents/tool");
    let relevant = |event: &Event| FileWatch::relevant(event, target, directory, Some(referent));
    let modification = || Event::new(EventKind::Modify(ModifyKind::Any));
    assert!(!relevant(&modification().add_path(directory.join("unrelated"))));
    assert!(relevant(&modification().add_path(directory.join("unrelated")).add_path(referent.to_owned())));
    assert!(relevant(&modification().add_path(target.to_owned())));
    for kind in [EventKind::Remove(RemoveKind::Folder), EventKind::Modify(ModifyKind::Name(RenameMode::Both))] {
        assert!(relevant(&Event::new(kind).add_path(Path::new("/example").to_owned())));
        assert!(relevant(&Event::new(kind).add_path(referent.parent().unwrap().to_owned())));
    }
    assert!(!relevant(&Event::new(EventKind::Access(notify::event::AccessKind::Any)).add_path(target.to_owned())));
    assert!(relevant(&Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan)));
    assert!(relevant(&modification()), "pathless mutations fail closed");
}
