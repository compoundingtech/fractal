use super::*;

#[cfg(test)]
fn file(root: &Path, category: &str, name: &str, bytes: usize) -> std::path::PathBuf {
    let directory = root.join(category);
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    fs::write(&path, vec![b'x'; bytes]).unwrap();
    path
}

#[cfg(test)]
#[test]
fn archive_capacity_is_disjoint_and_hardlinks_count_once() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let segment = file(root, "segments", &format!("{}.wire", "a".repeat(64)), 10);
    file(root, "index", "catalogue", 20);
    file(root, "overlays", "delta", 30);
    file(root, "staging", "pending", 40);
    file(root, "orphans", "unreferenced", 50);
    file(root, "backups", "copy", 60);
    fs::hard_link(segment, root.join("backups/linked-segment")).unwrap();
    file(root, "segments", "not-a-sealed-name", 7);
    file(root, "segments", ".private.wire-segment", 8);
    let bytes = measure(root).unwrap().unwrap();
    assert_eq!(bytes, ArchiveBytes { segment_file_bytes: 10, index_bytes: 20, overlay_bytes: 30,
        staging_bytes: 48, orphan_bytes: 57, backup_bytes: 60, files: 8 });
    assert_eq!(totals(bytes).unwrap(), (60, 225));
}

#[cfg(test)]
#[test]
fn absent_empty_and_missing_previously_measured_roots_are_distinct() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("claims.sqlite3"), "alder").unwrap();
    let root = directory.path().join("wire-archive");
    let first = store.record_database_archive_size(100 * DAY);
    assert_eq!(first.status, ArchiveSizeStatus::NotActivated);
    assert_eq!(first.archive_bytes, None);
    assert!(!root.exists(), "sampling must not create or activate the archive");
    fs::create_dir(&root).unwrap();
    let empty = store.record_database_archive_size(100 * DAY + HOUR);
    assert_eq!(empty.status, ArchiveSizeStatus::Available);
    assert_eq!(empty.archive_bytes, Some(0));
    assert_eq!(empty.growth_bytes_per_day, None);
    fs::remove_dir(&root).unwrap();
    let missing = store.record_database_archive_size(103 * DAY);
    assert_eq!(missing.status, ArchiveSizeStatus::Unavailable);
    assert_eq!(missing.archive_bytes, None);
    assert_eq!(missing.growth_bytes_per_day, None);
}

#[cfg(test)]
#[test]
fn moved_store_does_not_inherit_activation_from_the_old_archive_scope() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original");
    let moved = directory.path().join("moved");
    fs::create_dir(&original).unwrap();
    fs::create_dir(original.join("wire-archive")).unwrap();
    let store = Store::open(&original.join("claims.sqlite3"), "alder").unwrap();
    let old = store.record_database_archive_size(100 * DAY);
    assert_eq!(old.status, ArchiveSizeStatus::Available);
    drop(store);
    fs::rename(&original, &moved).unwrap();
    fs::remove_dir(moved.join("wire-archive")).unwrap();

    let store = Store::open(&moved.join("claims.sqlite3"), "alder").unwrap();
    let new = store.record_database_archive_size(100 * DAY + HOUR);
    assert_ne!(old.scope, new.scope);
    assert_eq!(new.status, ArchiveSizeStatus::NotActivated);
    assert_eq!(new.archive_bytes, None);
    assert_eq!(new.growth_bytes_per_day, None);
    assert!(!moved.join("wire-archive").exists());

    fs::create_dir(moved.join("wire-archive")).unwrap();
    assert_eq!(store.record_database_archive_size(100 * DAY + 2 * HOUR).status, ArchiveSizeStatus::Available);
    fs::remove_dir(moved.join("wire-archive")).unwrap();
    assert_eq!(store.record_database_archive_size(100 * DAY + 3 * HOUR).status, ArchiveSizeStatus::Unavailable);
}

#[cfg(test)]
#[test]
fn unknown_files_and_symlinks_refuse_partial_capacity_results() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("unknown"), b"evidence").unwrap();
    assert!(measure(directory.path()).is_err());
    fs::remove_file(directory.path().join("unknown")).unwrap();
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), directory.path().join("segments")).unwrap();
        assert!(measure(directory.path()).is_err());
        fs::remove_file(directory.path().join("segments")).unwrap();
        fs::create_dir(directory.path().join("segments")).unwrap();
        std::os::unix::fs::symlink(outside.path(), directory.path().join("segments/link")).unwrap();
        assert!(measure(directory.path()).is_err());
    }
}

#[cfg(test)]
fn samples() -> Vec<Sample> {
    (0..=24).map(|hour| Sample { at: 100 * DAY + hour * HOUR, scope: "scope".into(), bytes: Some(1_000 + hour as u64 * 10) }).collect()
}

#[cfg(test)]
#[test]
fn daily_growth_requires_a_covered_same_scope_day_without_unknown_samples() {
    let mut series = samples();
    let now = series.last().unwrap().at;
    assert_eq!(growth(&series, now, "scope"), Some((240, DAY, 100 * DAY)));
    assert_eq!(growth(&series[..24], now - HOUR, "scope"), None);
    series[12].bytes = None;
    assert_eq!(growth(&series, now, "scope"), None);
    let mut gap = samples();
    gap.drain(10..14);
    assert_eq!(growth(&gap, now, "scope"), None);
    let mut missed_write = samples();
    missed_write.remove(12);
    assert_eq!(growth(&missed_write, now, "scope"), None);
    let mut moved = samples();
    moved[12].scope = "other".into();
    assert_eq!(growth(&moved, now, "scope"), None);
    let mut shrunk = samples();
    shrunk.last_mut().unwrap().bytes = Some(400);
    assert_eq!(growth(&shrunk, now, "scope"), Some((-600, DAY, 100 * DAY)));
}

#[cfg(test)]
#[test]
fn archive_growth_survives_restart_but_not_a_clock_rollback() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("claims.sqlite3");
    let root = directory.path().join("wire-archive");
    let mut store = Store::open(&database, "alder").unwrap();
    for hour in 0..=24 {
        file(&root, "segments", &format!("{}.wire", "b".repeat(64)), 100 + hour * 10);
        let size = store.record_database_archive_size(100 * DAY + hour as u128 * HOUR);
        assert_eq!(size.status, ArchiveSizeStatus::Available);
        if hour == 24 { assert_eq!(size.growth_bytes_per_day, Some(240)); }
        if hour == 12 {
            drop(store);
            store = Store::open(&database, "alder").unwrap();
        }
    }
    let rolled_back = store.record_database_archive_size(100 * DAY);
    assert_eq!(rolled_back.growth_bytes_per_day, None);
    assert_eq!(rolled_back.growth_span_ms, None);
}
