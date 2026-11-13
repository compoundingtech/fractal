import { collapseWhitespaceChanges, diffStats, normalizePatch, patchFromRows, truncatePatch } from './diff-patch.ts'

// Standalone pure TypeScript suite: no DOM, React, runner or Node type dependency.
const assert = (condition: boolean, message: string): void => {
  if (!condition) throw new Error(message)
}
const equal = (actual: unknown, expected: unknown, message: string): void => {
  assert(JSON.stringify(actual) === JSON.stringify(expected), `${message}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`)
}
let tests = 0
const test = (name: string, body: () => void) => { body(); tests++; void name }

const git = [
  'diff --git a/src/a.ts b/src/a.ts',
  'index 1111111..2222222 100644',
  '--- a/src/a.ts',
  '+++ b/src/a.ts',
  '@@ -10,3 +10,3 @@ export function a() {',
  ' keep',
  '-old',
  '+new',
  ' tail',
  'diff --git a/src/b.ts b/src/b.ts',
  'new file mode 100644',
  '--- /dev/null',
  '+++ b/src/b.ts',
  '@@ -0,0 +1,2 @@',
  '+one',
  '+two',
].join('\n')

test('a git patch splits into one normalized file per diff --git, keeping offsets', () => {
  const files = normalizePatch(git)
  equal(files.map(file => file.path), ['src/a.ts', 'src/b.ts'], 'paths')
  equal(files.map(file => [file.added, file.removed]), [[1, 1], [2, 0]], 'counts')
  equal(files.map(file => file.excerpt), [false, false], 'real offsets are not excerpts')
  assert(files[0]!.patch.includes('@@ -10,3 +10,3 @@ export function a() {'), 'hunk header and its context survive')
  assert(files[1]!.patch.includes('--- /dev/null'), 'new-file header survives')
})

test('diffStats counts body lines only, never the ---/+++ headers', () => {
  equal(diffStats(git), { added: 3, removed: 1 }, 'git patch')
  equal(diffStats('--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n'), { added: 1, removed: 1 }, 'single file')
  equal(diffStats(''), { added: 0, removed: 0 }, 'empty')
})

test('a fence of bare +/- lines gets a synthetic header and hunk, marked as an excerpt', () => {
  const [file] = normalizePatch(' a\n-b\n+c\n+d', { path: 'x.ts' })
  equal(file!.path, 'x.ts', 'path from the caller')
  equal(file!.excerpt, true, 'no trusted offsets')
  equal([file!.added, file!.removed], [2, 1], 'counts')
  assert(file!.patch.startsWith('--- a/x.ts\n+++ b/x.ts\n@@ -1,2 +1,3 @@\n'), `synthesized header: ${file!.patch}`)
})

test('a hunk-only patch (DiffPanel rows) gets a file header and keeps its offsets', () => {
  const [file] = normalizePatch(['@@ -4,2 +4,2 @@', ' a', '-b', '+c'].join('\n'), { path: 'p.rs' })
  equal(file!.excerpt, false, 'offsets are real')
  assert(file!.patch.startsWith('--- a/p.rs\n+++ b/p.rs\n@@ -4,2 +4,2 @@'), file!.patch)
})

test('hunk counts are recomputed, so a streaming (truncated) patch still parses', () => {
  const [file] = normalizePatch(['--- a/s.ts', '+++ b/s.ts', '@@ -1,9 +1,9 @@', ' a', '-b'].join('\n'))
  assert(file!.patch.includes('@@ -1,2 +1,1 @@'), file!.patch)
})

test('numberless Codex hunks are excerpts with distinct synthetic ranges', () => {
  const [file] = normalizePatch(['@@', '-a', '+b', '@@ fn main', ' c', '+d'].join('\n'), { path: 'm.rs' })
  equal(file!.excerpt, true, 'excerpt')
  const headers = file!.patch.split('\n').filter(line => line.startsWith('@@'))
  equal(headers, ['@@ -1,1 +1,1 @@', '@@ -3,1 +3,2 @@ fn main'], 'ranges never overlap')
})

test('whitespace-only change pairs collapse into context lines, keeping hunk counts valid', () => {
  const [file] = normalizePatch(['--- a/w.ts', '+++ b/w.ts', '@@ -1,3 +1,3 @@', '-  a', '-b', '+a', '+c', ' z'].join('\n'))
  const collapsed = collapseWhitespaceChanges(file!)
  equal([collapsed.added, collapsed.removed], [1, 1], 'only the real change remains')
  assert(collapsed.patch.includes('@@ -1,3 +1,3 @@\n a\n-b\n+c\n z'), collapsed.patch)
})

test('an unpaired blank addition is hidden; the rest of the hunk keeps its real new-side numbers', () => {
  const [file] = normalizePatch(['--- a/w.ts', '+++ b/w.ts', '@@ -1,2 +1,3 @@ fn w', ' a', '+', ' b', '@@ -10,2 +11,2 @@', ' c', '-d', '+e'].join('\n'))
  const collapsed = collapseWhitespaceChanges(file!)
  equal([collapsed.added, collapsed.removed], [1, 1], 'only the later real change remains')
  equal(collapsed.patch.split('\n').filter(line => line.startsWith('@@')), ['@@ -1,1 +1,1 @@ fn w', '@@ -2,1 +3,1 @@', '@@ -10,2 +11,2 @@'], 'split ranges, later hunk untouched')
  assert(!collapsed.patch.split('\n').includes('+'), collapsed.patch)
})

test('an unpaired whitespace-only deletion is hidden; the rest of the hunk keeps its real old-side numbers', () => {
  const [file] = normalizePatch(['--- a/w.ts', '+++ b/w.ts', '@@ -1,3 +1,2 @@', ' a', '-  ', ' b', '@@ -10,2 +9,2 @@', ' c', '-d', '+e'].join('\n'))
  const collapsed = collapseWhitespaceChanges(file!)
  equal([collapsed.added, collapsed.removed], [1, 1], 'only the later real change remains')
  equal(collapsed.patch.split('\n').filter(line => line.startsWith('@@')), ['@@ -1,1 +1,1 @@', '@@ -3,1 +2,1 @@', '@@ -10,2 +9,2 @@'], 'split ranges, later hunk untouched')
  assert(!collapsed.patch.split('\n').includes('-  '), collapsed.patch)
})

test('a file whose only changes are blank lines keeps no hunks and counts nothing', () => {
  const [file] = normalizePatch(['--- a/w.ts', '+++ b/w.ts', '@@ -1,1 +1,1 @@', '-', '+  '].join('\n'))
  const paired = collapseWhitespaceChanges(file!)
  equal([paired.added, paired.removed], [0, 0], 'a blank pair collapses to context')
  const [only] = normalizePatch(['--- a/w.ts', '+++ b/w.ts', '@@ -0,0 +1,1 @@', '+ '].join('\n'))
  const hidden = collapseWhitespaceChanges(only!)
  equal([hidden.added, hidden.removed, hidden.patch], [0, 0, '--- a/w.ts\n+++ b/w.ts\n'], 'nothing left to draw')
})

const noNewline = '\\ No newline at end of file'
const finalLine = (removed: readonly string[], added: readonly string[]) => normalizePatch(['--- a/n.ts', '+++ b/n.ts', '@@ -1,2 +1,2 @@', ' z', ...removed, ...added].join('\n'))[0]!

test('no-newline markers on both sides do not interrupt pairing: a whitespace-only final-line edit is hidden', () => {
  const collapsed = collapseWhitespaceChanges(finalLine(['-a ', noNewline], ['+a', noNewline]))
  equal([collapsed.added, collapsed.removed], [0, 0], 'hidden')
  equal(collapsed.patch.split('\n').slice(2), ['@@ -1,2 +1,2 @@', ' z', ' a', noNewline, ''], 'one context line, still without a final newline')
})

test('a no-newline marker on one side only pairs too, and the collapsed line drops it', () => {
  for (const [removed, added] of [[['-a ', noNewline], ['+a']], [['-a '], ['+a', noNewline]]] as const) {
    const collapsed = collapseWhitespaceChanges(finalLine(removed, added))
    equal([collapsed.added, collapsed.removed], [0, 0], `hidden: ${JSON.stringify([removed, added])}`)
    equal(collapsed.patch.split('\n').slice(2), ['@@ -1,2 +1,2 @@', ' z', ' a', ''], 'a missing final newline on one side is whitespace too')
  }
})

test('a kept final-line change keeps its no-newline markers on each side', () => {
  const collapsed = collapseWhitespaceChanges(finalLine(['-a', noNewline], ['+b', noNewline]))
  equal([collapsed.added, collapsed.removed], [1, 1], 'real change')
  equal(collapsed.patch.split('\n').slice(2), ['@@ -1,2 +1,2 @@', ' z', '-a', noNewline, '+b', noNewline, ''], 'markers stay with their lines')
  const added = collapseWhitespaceChanges(finalLine(['-a'], ['+b', noNewline]))
  equal(added.patch.split('\n').slice(2), ['@@ -1,2 +1,2 @@', ' z', '-a', '+b', noNewline, ''], 'addition side only')
  const removed = collapseWhitespaceChanges(finalLine(['-a', noNewline], ['+b']))
  equal(removed.patch.split('\n').slice(2), ['@@ -1,2 +1,2 @@', ' z', '-a', noNewline, '+b', ''], 'removal side only')
})

test('a dropped blank final line takes its no-newline marker with it', () => {
  const [file] = normalizePatch(['--- a/n.ts', '+++ b/n.ts', '@@ -1,1 +1,2 @@', ' z', '+ ', noNewline].join('\n'))
  const collapsed = collapseWhitespaceChanges(file!)
  equal([collapsed.added, collapsed.removed], [0, 0], 'hidden')
  equal(collapsed.patch.split('\n').slice(2), ['@@ -1,1 +1,1 @@', ' z', ''], 'no orphan marker')
})

test('truncatePatch keeps the first body lines and reports what it hid', () => {
  const [file] = normalizePatch(['--- a/t.ts', '+++ b/t.ts', '@@ -1,4 +1,4 @@', ' a', '-b', '+c', ' d', ' e'].join('\n'))
  const { file: short, hidden } = truncatePatch(file!, 3)
  equal(hidden, 2, 'hidden lines')
  assert(short.patch.endsWith('@@ -1,2 +1,2 @@\n a\n-b\n+c\n'), short.patch)
})

test('patchFromRows turns kit diff rows into a normalized patch', () => {
  const file = patchFromRows('r.ts', [{ kind: 'context', text: 'a' }, { kind: 'removed', text: 'b' }, { kind: 'added', text: 'c' }], { excerpt: true })
  equal([file.added, file.removed, file.excerpt], [1, 1, true], 'counts')
  assert(file.patch.includes('\n a\n-b\n+c\n'), file.patch)
})

test('a removed line that looks like a file header stays inside its counted hunk', () => {
  const files = normalizePatch(['--- a/h.md', '+++ b/h.md', '@@ -1,2 +1,0 @@', '--- x', '-+++ y'].join('\n'))
  equal(files.length, 1, 'one file')
  equal([files[0]!.added, files[0]!.removed], [0, 2], 'counts')
})

test('text without any diff body yields no files', () => {
  equal(normalizePatch('just prose\n').length, 1, 'prose is context in a bare fence')
  equal(normalizePatch('').length, 0, 'empty')
})

console.log(`diff-patch: ${tests} tests passed`)
