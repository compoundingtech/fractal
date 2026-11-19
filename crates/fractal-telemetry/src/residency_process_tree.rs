//! A bounded observation of one birth-fenced Linux process tree, not an RSS sum.
//! Enumerate every thread's children: the leader's children alone omit descendants
//! forked by other threads. Any unreadable edge or changing census is unknown.
use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};

#[derive(Debug, PartialEq, Eq)]
struct Process {
    parent: u32,
    birth: u64,
    children_by_thread: BTreeMap<u32, BTreeSet<u32>>,
}

fn identity(proc: &Path, pid: u32) -> Option<(u32, u64)> {
    let text = fs::read_to_string(proc.join(pid.to_string()).join("stat")).ok()?;
    let (head, tail) = text.rsplit_once(") ")?;
    if head.split_once(" (")?.0.parse::<u32>().ok()? != pid { return None; }
    let mut fields = tail.split_whitespace();
    let state = fields.next()?;
    if state.len() != 1 || !state.as_bytes()[0].is_ascii_alphabetic()
        || matches!(state, "Z" | "X" | "x") { return None; }
    let parent = fields.next()?.parse::<u32>().ok()?;
    // The tail begins at field 3 (state); starttime is field 22.
    let birth = fields.nth(17)?.parse::<u64>().ok()?;
    (birth > 0).then_some((parent, birth))
}

fn threads(proc: &Path, pid: u32) -> Option<BTreeMap<u32, BTreeSet<u32>>> {
    let task = proc.join(pid.to_string()).join("task");
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(&task).ok()? {
        let entry = entry.ok()?;
        let tid = entry.file_name().to_str()?.parse::<u32>().ok()?;
        if tid == 0 { return None; }
        let text = fs::read_to_string(entry.path().join("children")).ok()?;
        let mut children = BTreeSet::new();
        for word in text.split_whitespace() {
            let child = word.parse::<u32>().ok()?;
            if child == 0 || !children.insert(child) { return None; }
        }
        if result.insert(tid, children).is_some() { return None; }
    }
    result.contains_key(&pid).then_some(result)
}

fn census(proc: &Path, root: (u32, u64)) -> Option<BTreeMap<u32, Process>> {
    if root.0 == 0 || root.1 == 0 { return None; }
    let mut pending = vec![(root.0, None)];
    let mut result = BTreeMap::new();
    while let Some((pid, expected_parent)) = pending.pop() {
        // Duplicate edges and cycles are not a complete tree.
        if result.contains_key(&pid) { return None; }
        let (parent, birth) = identity(proc, pid)?;
        if expected_parent.is_some_and(|(expected_pid, parent_birth)| parent != expected_pid || birth < parent_birth)
            || (pid == root.0 && birth != root.1) { return None; }
        let children_by_thread = threads(proc, pid)?;
        for children in children_by_thread.values() {
            pending.extend(children.iter().map(|child| (*child, Some((pid, birth)))));
        }
        result.insert(pid, Process { parent, birth, children_by_thread });
    }
    Some(result)
}

pub(super) fn private_bytes(proc: &Path, root: (u32, u64)) -> Option<u64> {
    // Re-read the complete topology and every birth after smaps. Fork, exit,
    // reparent, PID reuse or a vanished/unreadable task invalidates the pair.
    // This is a stable-endpoint observation, not an atomic kernel snapshot.
    measure(proc, root, || {})
}

fn measure(proc: &Path, root: (u32, u64), after_smaps: impl FnOnce()) -> Option<u64> {
    let before = census(proc, root)?;
    let bytes = before.keys().try_fold(0u64, |total, pid| {
        let text = fs::read_to_string(proc.join(pid.to_string()).join("smaps_rollup")).ok()?;
        total.checked_add(super::private_bytes(&text)?)
    })?;
    after_smaps();
    (census(proc, root)? == before).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(tempfile::TempDir);
    impl Fixture {
        fn new() -> Self { Self(tempfile::tempdir().unwrap()) }
        fn path(&self) -> &Path { self.0.path() }
        fn process(&self, pid: u32, parent: u32, birth: u64, children: &[u32]) {
            let path = self.path().join(pid.to_string());
            fs::create_dir_all(path.join("task").join(pid.to_string())).unwrap();
            self.stat(pid, parent, birth);
            self.children(pid, pid, children);
            fs::write(path.join("smaps_rollup"), "Rss: 999999 kB\nPrivate_Clean: 2 kB\nPrivate_Dirty: 3 kB\nPrivate_Hugetlb: 99 kB\n").unwrap();
        }
        fn stat(&self, pid: u32, parent: u32, birth: u64) {
            // A command containing ')' exercises stat's rightmost delimiter.
            let middle = std::iter::repeat_n("0", 17).collect::<Vec<_>>().join(" ");
            fs::write(self.path().join(pid.to_string()).join("stat"),
                format!("{pid} (seat ) worker) S {parent} {middle} {birth} 0\n")).unwrap();
        }
        fn children(&self, pid: u32, tid: u32, children: &[u32]) {
            let task = self.path().join(pid.to_string()).join("task").join(tid.to_string());
            fs::create_dir_all(&task).unwrap();
            fs::write(task.join("children"), children.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")).unwrap();
        }
        fn tree() -> Self {
            let fixture = Self::new();
            fixture.process(10, 1, 100, &[20]);
            fixture.process(20, 10, 200, &[30]);
            fixture.process(30, 20, 300, &[]);
            // A child of a nonleader thread still belongs to the process tree.
            fixture.children(10, 11, &[40]);
            fixture.process(40, 10, 400, &[]);
            fixture.process(90, 1, 50, &[]); // unrelated neighbor is not included
            fixture
        }
    }

    #[test]
    fn sums_private_clean_and_dirty_for_all_threads_and_descendants_only() {
        let fixture = Fixture::tree();
        assert_eq!(private_bytes(fixture.path(), (10, 100)), Some(20 * 1024));
    }

    #[test]
    fn wrong_birth_and_reparented_or_reused_children_are_unknown() {
        let fixture = Fixture::tree();
        assert_eq!(private_bytes(fixture.path(), (10, 101)), None);
        fixture.stat(20, 1, 200);
        assert_eq!(private_bytes(fixture.path(), (10, 100)), None);
        fixture.stat(20, 10, 99);
        assert_eq!(private_bytes(fixture.path(), (10, 100)), None);
    }


    #[test]
    fn completed_stop_pairs_known_private_bytes_with_positive_stop_evidence() {
        use super::super::{MemoryDelta, read_memory_tree};
        let fixture = Fixture::tree();
        fs::write(fixture.path().join("meminfo"), "MemAvailable: 100 kB\n").unwrap();
        let before = read_memory_tree(fixture.path(), Some((10, 100)));
        assert_eq!(before.private_bytes, Some(20 * 1024));
        fs::remove_dir_all(fixture.path().join("10")).unwrap();
        fs::write(fixture.path().join("meminfo"), "MemAvailable: 110 kB\n").unwrap();
        let after = read_memory_tree(fixture.path(), Some((10, 100)));
        assert_eq!(after.private_bytes, None);
        // Mere disappearance is not proof of reclamation.
        assert_eq!(MemoryDelta::between(before, after).private_reclaimed_bytes, None);
        // Only the caller's completed whole-seat-stop receipt permits this.
        let delta = MemoryDelta::after_completed_stop(before, after);
        assert_eq!(delta.private_reclaimed_bytes, Some(20 * 1024));
        assert_eq!(delta.host_available_delta_bytes, Some(10 * 1024));
        let unknown = read_memory_tree(fixture.path(), None);
        assert_eq!(MemoryDelta::after_completed_stop(unknown, after).private_reclaimed_bytes, None);
    }
    #[test]
    fn changed_birth_reparent_fork_or_vanished_process_invalidates_measurement() {
        for change in 0..5 {
            let fixture = Fixture::tree();
            assert_eq!(measure(fixture.path(), (10, 100), || match change {
                0 => fixture.stat(20, 10, 201),
                1 => fixture.stat(20, 1, 200),
                2 => { fixture.process(50, 10, 500, &[]); fixture.children(10, 10, &[20, 50]); }
                3 => fs::remove_dir_all(fixture.path().join("30")).unwrap(),
                _ => fixture.stat(10, 1, 101),
            }), None);
        }
    }

    #[test]
    fn incomplete_or_malformed_tree_never_reports_partial_private_total() {
        for damage in 0..9 {
            let fixture = Fixture::tree();
            match damage {
                0 => fs::remove_file(fixture.path().join("30/smaps_rollup")).unwrap(),
                1 => fs::write(fixture.path().join("30/smaps_rollup"), "Private_Clean: 1 kB\n").unwrap(),
                2 => fs::write(fixture.path().join("20/stat"), "malformed").unwrap(),
                3 => fs::remove_file(fixture.path().join("10/task/11/children")).unwrap(),
                4 => fs::write(fixture.path().join("10/task/11/children"), "bad").unwrap(),
                5 => fixture.children(10, 11, &[20]), // duplicate edge
                6 => fixture.children(30, 30, &[10]), // cycle
                7 => fs::remove_dir_all(fixture.path().join("30")).unwrap(),
                _ => fs::remove_dir_all(fixture.path().join("20/task/20")).unwrap(),
            }
            assert_eq!(private_bytes(fixture.path(), (10, 100)), None, "damage {damage}");
        }
    }
}
