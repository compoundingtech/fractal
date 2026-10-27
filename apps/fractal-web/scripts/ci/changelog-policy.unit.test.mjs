import assert from 'node:assert/strict'
import { test } from 'node:test'
import { checkChangelogRequirement, isChangelogEntryPath, isUiPath, parseGitNameStatus } from './changelog-policy.mjs'

const uiFile = { status: 'modified', filename: 'apps/fractal-web/src/web/Example.tsx' }
const entryFile = { status: 'added', filename: 'apps/fractal-web/.changelog/2030-01-02-example-change.ts' }
const check = (files, labels = []) => checkChangelogRequirement({ files, labels })

test('UI changes fail without a new entry', () => {
  assert.equal(check([uiFile]).ok, false)
  assert.equal(check([uiFile]).reason, 'missing-entry')
})

test('new source, no UI, and explicit label each pass', () => {
  assert.equal(check([uiFile, entryFile]).reason, 'new-entry')
  assert.equal(check([]).reason, 'no-ui-change')
  assert.equal(check([{ status: 'modified', filename: 'docs/development.md' }]).ok, true)
  assert.equal(check([uiFile], ['no-changelog']).reason, 'opt-out')
  assert.equal(check([uiFile], ['No-Changelog']).ok, false)
})

test('editing, deleting, copying and renaming an old entry do not count as new', () => {
  for (const status of ['modified', 'removed', 'copied', 'renamed']) {
    assert.equal(check([uiFile, { ...entryFile, status, previousFilename: 'apps/fractal-web/.changelog/2029-01-02-old.ts' }]).ok, false)
  }
})

test('valid filename date and slug are required', () => {
  for (const path of ['apps/fractal-web/.changelog/2030-02-30-example.ts', 'apps/fractal-web/.changelog/2030-13-01-example.ts', 'apps/fractal-web/.changelog/2030-01-02-Example.ts', 'apps/fractal-web/.changelog/2030-01-02-example.json', '.changelog/2030-01-02-example.ts', 'apps/fractal-web/.changelog/mod.ts']) {
    assert.equal(isChangelogEntryPath(path), false, path)
    assert.equal(check([uiFile, { ...entryFile, filename: path }]).ok, false)
  }
  assert.equal(isChangelogEntryPath('apps/fractal-web/.changelog/2032-02-29-example.ts'), true)
  assert.equal(isChangelogEntryPath('apps/fractal-web/private/.changelog/2030-01-02-example.ts'), true)
})

test('runtime TS and styles count conservatively in both UI roots', () => {
  for (const path of ['apps/fractal-web/src/web/Example.tsx', 'apps/fractal-web/src/data/example.ts', 'apps/fractal-web/src/web/theme.css', 'packages/fractal-ui/src/theme.ts', 'packages/fractal-ui/src/Example.tsx', 'apps/fractal-web/private/monitor/Example.tsx']) {
    assert.equal(isUiPath(path), true, path)
  }
})

test('tests, declarations, stories, fixtures and non-product scripts alone do not count', () => {
  for (const path of ['apps/fractal-web/src/Example.unit.test.ts', 'apps/fractal-web/src/Example.integration.test.ts', 'apps/fractal-web/src/Example.pw.test.ts', 'apps/fractal-web/src/Example.test.tsx', 'apps/fractal-web/src/Example.spec.ts', 'apps/fractal-web/src/Example.stories.tsx', 'apps/fractal-web/src/example-fixtures.ts', 'apps/fractal-web/src/example-story-harness.tsx', 'apps/fractal-web/src/vite-env.d.ts', 'apps/fractal-web/src/web/example.d.ts', 'apps/fractal-web/src/__tests__/Example.tsx', 'packages/fractal-ui/src/stories/Example.tsx', 'apps/fractal-web/server/core.mts', 'apps/fractal-web/private/live-server.mts', 'apps/fractal-web/private/vite.config.ts', 'apps/fractal-web/scripts/example.tsx', 'packages/fractal-ui/.storybook/main.ts', 'apps/other/src/Example.tsx']) {
    assert.equal(isUiPath(path), false, path)
    assert.equal(check([{ status: 'modified', filename: path }]).ok, true, path)
  }
})

test('UI deletion and renames into or out of UI all require an entry', () => {
  assert.equal(check([{ ...uiFile, status: 'removed' }]).ok, false)
  assert.equal(check([{ status: 'renamed', filename: 'archived/Example.tsx', previousFilename: uiFile.filename }]).ok, false)
  assert.equal(check([{ status: 'renamed', filename: uiFile.filename, previousFilename: 'archived/Example.tsx' }]).ok, false)
})

test('NUL diff parser preserves whitespace and both rename paths', () => {
  assert.deepEqual(parseGitNameStatus('M\0apps/fractal-web/src/with\tspace\n.tsx\0D\0packages/fractal-ui/src/Old.tsx\0R100\0apps/fractal-web/src/Before.tsx\0archived/After.tsx\0A\0apps/fractal-web/.changelog/2030-01-02-example.ts\0'), [
    { status: 'modified', filename: 'apps/fractal-web/src/with\tspace\n.tsx' },
    { status: 'removed', filename: 'packages/fractal-ui/src/Old.tsx' },
    { status: 'renamed', previousFilename: 'apps/fractal-web/src/Before.tsx', filename: 'archived/After.tsx' },
    { status: 'added', filename: 'apps/fractal-web/.changelog/2030-01-02-example.ts' },
  ])
  assert.deepEqual(parseGitNameStatus('C90\0old.ts\0new.ts\0T\0type.ts\0'), [
    { status: 'copied', previousFilename: 'old.ts', filename: 'new.ts' },
    { status: 'changed', filename: 'type.ts' },
  ])
  assert.deepEqual(parseGitNameStatus(''), [])
})

test('complete diff is not capped at a single API page or 3000 files', () => {
  const diff = Array.from({ length: 3100 }, (_, index) => `M\0docs/example-${index}.md\0`).join('') + `D\0${uiFile.filename}\0`
  const files = parseGitNameStatus(diff)
  assert.equal(files.length, 3101)
  assert.equal(check(files).ok, false)
})

test('malformed input and incomplete diffs fail closed', () => {
  for (const input of [{}, { files: [], labels: [1] }, { files: [{ status: 'unknown', filename: 'file' }], labels: [] }, { files: [{ status: 'renamed', filename: 'file' }], labels: [] }]) {
    assert.throws(() => checkChangelogRequirement(input))
  }
  for (const diff of ['M\0file', 'R100\0old\0', 'U\0file\0', 'M\0\0']) assert.throws(() => parseGitNameStatus(diff))
})
