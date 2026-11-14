//! Line diffs between two stored source texts: which lines two versions share (to carry a span
//! recorded against one version into another) and unified hunks (to show what changed).
//! Myers' O((N+M)·D) algorithm with the linear-space middle-snake split.
use std::collections::HashMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};

/// The pairs of 0-based line indices `(old, new)` the two texts share, ascending in both.
pub fn common_lines(old: &str, new: &str) -> Vec<(usize, usize)> {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    common_line_indices(&old, &new)
}

fn intern<'a>(ids: &mut HashMap<&'a str, usize>, lines: &[&'a str]) -> Vec<usize> {
    lines
        .iter()
        .map(|line| {
            let next = ids.len();
            *ids.entry(line).or_insert(next)
        })
        .collect()
}

fn common_line_indices(old: &[&str], new: &[&str]) -> Vec<(usize, usize)> {
    let mut ids = HashMap::new();
    let old = intern(&mut ids, old);
    let new = intern(&mut ids, new);
    let max_d = (old.len() + new.len()).div_ceil(2) + 1;
    let mut forward = V::new(max_d);
    let mut backward = V::new(max_d);
    let mut pairs = Vec::new();
    conquer(&old, 0..old.len(), &new, 0..new.len(), &mut forward, &mut backward, &mut pairs);
    pairs
}

/// For each 1-based line of `old`, its 1-based line in `new` when that line is unchanged.
pub fn line_map(old: &str, new: &str) -> Vec<Option<u32>> {
    let mut map = vec![None; old.lines().count()];
    for (old, new) in common_lines(old, new) {
        map[old] = Some(new as u32 + 1);
    }
    map
}

/// Lines `start..=end` (1-based) of `old` carried into `new`: the first and last of them that
/// are unchanged there, or `None` when every line of the span changed.
pub fn carry_span(map: &[Option<u32>], start: u32, end: u32) -> Option<[u32; 2]> {
    let lines = (start.max(1)..=end).filter_map(|line| map.get(line as usize - 1).copied().flatten());
    let mut first = None;
    let mut last = None;
    for line in lines {
        first.get_or_insert(line);
        last = Some(line);
    }
    Some([first?, last?])
}

/// One unified-diff hunk. `lines` start with ` `, `-` or `+`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

/// A whole-text diff: line counts and hunks with `context` unchanged lines around changes.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TextDiff {
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<Hunk>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Equal(usize, usize),
    Delete(usize),
    Insert(usize),
}

pub fn unified(old: &str, new: &str, context: usize) -> TextDiff {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let mut ops = Vec::with_capacity(old.len().max(new.len()));
    let (mut o, mut n) = (0, 0);
    for (old_index, new_index) in common_line_indices(&old, &new).into_iter().chain([(old.len(), new.len())]) {
        ops.extend((o..old_index).map(Op::Delete));
        ops.extend((n..new_index).map(Op::Insert));
        if old_index < old.len() {
            ops.push(Op::Equal(old_index, new_index));
        }
        (o, n) = (old_index + 1, new_index + 1);
    }
    let mut diff = TextDiff::default();
    let changed: Vec<usize> = ops.iter().enumerate().filter(|(_, op)| !matches!(op, Op::Equal(..))).map(|(i, _)| i).collect();
    let mut index = 0;
    while index < changed.len() {
        let first = changed[index];
        let mut last = first;
        while index + 1 < changed.len() && changed[index + 1] <= last + 2 * context + 1 {
            index += 1;
            last = changed[index];
        }
        index += 1;
        let range = first.saturating_sub(context)..(last + context + 1).min(ops.len());
        diff.hunks.push(hunk(&ops[range], &old, &new, o_before(&ops, first.saturating_sub(context)), n_before(&ops, first.saturating_sub(context)), &mut diff.added, &mut diff.removed));
    }
    diff
}

fn o_before(ops: &[Op], at: usize) -> usize {
    ops[..at].iter().filter(|op| !matches!(op, Op::Insert(_))).count()
}

fn n_before(ops: &[Op], at: usize) -> usize {
    ops[..at].iter().filter(|op| !matches!(op, Op::Delete(_))).count()
}

fn hunk(ops: &[Op], old: &[&str], new: &[&str], old_before: usize, new_before: usize, added: &mut u32, removed: &mut u32) -> Hunk {
    let mut hunk = Hunk { old_start: old_before as u32 + 1, old_lines: 0, new_start: new_before as u32 + 1, new_lines: 0, lines: Vec::with_capacity(ops.len()) };
    for op in ops {
        match *op {
            Op::Equal(o, _) => {
                hunk.old_lines += 1;
                hunk.new_lines += 1;
                hunk.lines.push(format!(" {}", old[o]));
            }
            Op::Delete(o) => {
                hunk.old_lines += 1;
                *removed += 1;
                hunk.lines.push(format!("-{}", old[o]));
            }
            Op::Insert(n) => {
                hunk.new_lines += 1;
                *added += 1;
                hunk.lines.push(format!("+{}", new[n]));
            }
        }
    }
    // An empty side starts at the line before, as in `diff -u`.
    if hunk.old_lines == 0 {
        hunk.old_start -= 1;
    }
    if hunk.new_lines == 0 {
        hunk.new_start -= 1;
    }
    hunk
}

/// Furthest-reaching x per diagonal k, indexable by negative k.
struct V {
    offset: isize,
    v: Vec<usize>,
}

impl V {
    fn new(max_d: usize) -> Self {
        // Diagonals reach ±(max_d + 1) at the last step.
        Self { offset: max_d as isize + 1, v: vec![0; 2 * max_d + 3] }
    }
}

impl std::ops::Index<isize> for V {
    type Output = usize;
    fn index(&self, index: isize) -> &usize {
        &self.v[(index + self.offset) as usize]
    }
}

impl std::ops::IndexMut<isize> for V {
    fn index_mut(&mut self, index: isize) -> &mut usize {
        &mut self.v[(index + self.offset) as usize]
    }
}

fn prefix_len(old: &[usize], old_range: Range<usize>, new: &[usize], new_range: Range<usize>) -> usize {
    old[old_range].iter().zip(&new[new_range]).take_while(|(a, b)| a == b).count()
}

fn suffix_len(old: &[usize], old_range: Range<usize>, new: &[usize], new_range: Range<usize>) -> usize {
    old[old_range].iter().rev().zip(new[new_range].iter().rev()).take_while(|(a, b)| a == b).count()
}

/// The start of a snake on an optimal path through the two ranges, which share neither their
/// first nor their last element.
fn middle_snake(old: &[usize], old_range: Range<usize>, new: &[usize], new_range: Range<usize>, forward: &mut V, backward: &mut V) -> (usize, usize) {
    let n = old_range.len();
    let m = new_range.len();
    let delta = n as isize - m as isize;
    let odd = delta & 1 == 1;
    forward[1] = 0;
    backward[1] = 0;
    let max_d = (n + m).div_ceil(2) + 1;
    for d in 0..max_d as isize {
        for k in (-d..=d).rev().step_by(2) {
            let mut x = if k == -d || (k != d && forward[k - 1] < forward[k + 1]) { forward[k + 1] } else { forward[k - 1] + 1 };
            let y = (x as isize - k) as usize;
            let (x0, y0) = (x, y);
            if x < n && y < m {
                x += prefix_len(old, old_range.start + x..old_range.end, new, new_range.start + y..new_range.end);
            }
            forward[k] = x;
            if odd && (k - delta).abs() < d && forward[k] + backward[-(k - delta)] >= n {
                return (x0 + old_range.start, y0 + new_range.start);
            }
        }
        for k in (-d..=d).rev().step_by(2) {
            let mut x = if k == -d || (k != d && backward[k - 1] < backward[k + 1]) { backward[k + 1] } else { backward[k - 1] + 1 };
            let mut y = (x as isize - k) as usize;
            if x < n && y < m {
                let advance = suffix_len(old, old_range.start..old_range.start + n - x, new, new_range.start..new_range.start + m - y);
                x += advance;
                y += advance;
            }
            backward[k] = x;
            if !odd && (k - delta).abs() <= d && backward[k] + forward[-(k - delta)] >= n {
                return (n - x + old_range.start, m - y + new_range.start);
            }
        }
    }
    unreachable!("two finite ranges always meet within (n + m) / 2 + 1 steps")
}

fn conquer(old: &[usize], mut old_range: Range<usize>, new: &[usize], mut new_range: Range<usize>, forward: &mut V, backward: &mut V, pairs: &mut Vec<(usize, usize)>) {
    let prefix = prefix_len(old, old_range.clone(), new, new_range.clone());
    pairs.extend((0..prefix).map(|i| (old_range.start + i, new_range.start + i)));
    old_range.start += prefix;
    new_range.start += prefix;
    let suffix = suffix_len(old, old_range.clone(), new, new_range.clone());
    old_range.end -= suffix;
    new_range.end -= suffix;
    if !old_range.is_empty() && !new_range.is_empty() {
        let (x, y) = middle_snake(old, old_range.clone(), new, new_range.clone(), forward, backward);
        conquer(old, old_range.start..x, new, new_range.start..y, forward, backward, pairs);
        conquer(old, x..old_range.end, new, y..new_range.end, forward, backward, pairs);
    }
    pairs.extend((0..suffix).map(|i| (old_range.end + i, new_range.end + i)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(old: &str, diff: &TextDiff) -> String {
        let old: Vec<&str> = old.lines().collect();
        let mut out = Vec::new();
        let mut next = 0usize;
        for hunk in &diff.hunks {
            let start = if hunk.old_lines == 0 { hunk.old_start as usize } else { hunk.old_start as usize - 1 };
            out.extend(old[next..start].iter().map(|line| line.to_string()));
            for line in &hunk.lines {
                if let Some(text) = line.strip_prefix(' ').or_else(|| line.strip_prefix('+')) {
                    out.push(text.to_owned());
                }
            }
            next = start + hunk.old_lines as usize;
        }
        out.extend(old[next..].iter().map(|line| line.to_string()));
        out.join("\n")
    }

    #[test]
    fn unified_hunks_rebuild_the_new_text_and_count_changes() {
        let cases = [
            ("a\nb\nc\nd\ne\nf\ng\nh\ni\nj", "a\nb\nX\nd\ne\nf\ng\nh\nY\nj\nk"),
            ("", "one\ntwo"),
            ("one\ntwo", ""),
            ("same\nsame", "same\nsame"),
            ("x\ny\nz", "z\ny\nx"),
            ("a\nb\na\nb\na", "b\na\nb\na\nb"),
        ];
        for (old, new) in cases {
            let diff = unified(old, new, 3);
            assert_eq!(apply(old, &diff), new, "{old:?} -> {new:?}: {diff:?}");
            let shared = common_lines(old, new).len() as u32;
            assert_eq!(diff.removed, old.lines().count() as u32 - shared);
            assert_eq!(diff.added, new.lines().count() as u32 - shared);
        }
        let diff = unified("a\nb\nc\nd\ne\nf\ng\nh\ni\nj", "a\nb\nX\nd\ne\nf\ng\nh\nY\nj", 1);
        assert_eq!(diff.hunks.len(), 2);
        assert_eq!(diff.hunks[0], Hunk { old_start: 2, old_lines: 3, new_start: 2, new_lines: 3, lines: vec![" b".into(), "-c".into(), "+X".into(), " d".into()] });
    }

    #[test]
    fn common_lines_are_a_longest_common_subsequence() {
        assert_eq!(common_lines("a\nb\nc\nd", "a\nx\nc\nd\ne"), vec![(0, 0), (2, 2), (3, 3)]);
        assert_eq!(common_lines("p\nq\nr\ns\nt", "q\nr\nz\nt").len(), 3);
    }

    #[test]
    fn spans_carry_through_moved_lines_and_vanish_when_rewritten() {
        let old = "head\nconst policy = {\n  goal: 'x',\n}\n";
        let new = "head\n// added\n\nconst policy = {\n  goal: 'x',\n}\n";
        let map = line_map(old, new);
        assert_eq!(carry_span(&map, 2, 4), Some([4, 6]));
        let rewritten = "head\nconst policy = {\n  goal: 'y',\n}\n";
        assert_eq!(carry_span(&line_map(old, rewritten), 3, 3), None);
        assert_eq!(carry_span(&line_map(old, rewritten), 2, 4), Some([2, 4]));
    }
}
