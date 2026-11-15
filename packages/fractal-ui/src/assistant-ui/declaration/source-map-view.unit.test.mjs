import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'

const server = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } })
after(() => server.close())
const view = await server.ssrLoadModule('/src/assistant-ui/declaration/source-map-view.ts')

const missionKdl = [
  'version 2',
  'mission "land/code" {',
  '    goal "Land it"',
  '    step "review" {',
  '        goal "Review the diff"',
  '        prompt r#"multi',
  'line { not a node }"#',
  '    }',
  '    step "merge" { needs "review"; }',
  '    gate "ci" kind="check"',
  '    note "a"',
  '    note "b"',
  '}',
  '',
].join('\n')

test('KDL scanner keys agent/mission/step/gate by name and other nodes by sibling index', () => {
  const index = view.kdlIndex(missionKdl)
  assert.deepEqual(index.nodes.map(node => [node.path, node.start, node.end]), [
    ['version[0]', 1, 1],
    ['mission[land/code]', 2, 13],
    ['mission[land/code]/goal[0]', 3, 3],
    ['mission[land/code]/step[review]', 4, 8],
    ['mission[land/code]/step[review]/goal[0]', 5, 5],
    ['mission[land/code]/step[review]/prompt[0]', 6, 7],
    ['mission[land/code]/step[merge]', 9, 9],
    ['mission[land/code]/step[merge]/needs[0]', 9, 9],
    ['mission[land/code]/gate[ci]', 10, 10],
    ['mission[land/code]/note[0]', 11, 11],
    ['mission[land/code]/note[1]', 12, 12],
  ])
  assert.equal(index.lineNode.length, 13)
  // A line belongs to the outermost node starting on it, else the innermost node containing it.
  assert.equal(index.lineNode[8], 'mission[land/code]/step[merge]')
  assert.equal(index.lineNode[6], 'mission[land/code]/step[review]/prompt[0]')
  assert.equal(index.lineNode[7], 'mission[land/code]/step[review]')
})

test('labels and mapped ancestors read the node path', () => {
  assert.deepEqual(view.nodeSegments('mission[land/code]/step[a]b]/goal[0]'), [['mission', 'land/code'], ['step', 'a]b'], ['goal', '0']])
  assert.equal(view.stepNodePath('land/code', 'outer/nested/inner'), 'mission[land/code]/step[outer]/mission[nested]/step[inner]')
  assert.equal(view.nodeLabel('mission[land/code]/step[review]/goal[0]'), 'step review › goal')
  const mapped = new Map([['mission[land/code]', {}], ['mission[land/code]/step[review]', {}]])
  assert.equal(view.mappedAncestor('mission[land/code]/step[review]/prompt[0]', mapped), 'mission[land/code]/step[review]')
  assert.equal(view.mappedAncestor('mission[land/code]/gate[ci]', mapped), 'mission[land/code]')
})

const ops = ops => ops.map(op => `${op.kind[0]}${op.a ?? ''}:${op.b ?? ''}`)

test('line diff is minimal, numbered from 1, and ignores one trailing newline', () => {
  assert.deepEqual(ops(view.diffTexts('a\nb\nc\n', 'a\nx\nc\n')), ['s1:1', 'd2:', 'a:2', 's3:3'])
  assert.deepEqual(ops(view.diffTexts('', 'a\n')), ['d1:', 'a:1'])
  assert.deepEqual(view.diffCounts(view.diffTexts('a\nb\n', 'b\nc\nd\n')), { added: 2, removed: 1 })
  const rows = view.foldContext(view.diffTexts(Array.from({ length: 20 }, (_, at) => `l${at}`).join('\n'), [...Array.from({ length: 19 }, (_, at) => `l${at}`), 'changed'].join('\n')), 3)
  assert.deepEqual(rows[0], { kind: 'gap', folded: 16 })
  assert.equal(rows.length, 6)
})

const file = (path, blob_hash, repository = 'https://github.com/o/dotfiles') => ({ repository, path, blob_hash })
const map = (files, nodes = []) => ({ subject: 'mission/land/code', revision: 'r', repository: 'https://github.com/o/dotfiles', ref: 'main', sha: '3f9c2ab1d', sequence: 1, files, declaration: 0, inputs: [], nodes })

test('Changes names the own file or the helper the change came from', () => {
  const before = map([file('missions/land.ts', 'a'), file('lib/review.ts', 'b')])
  const ownChanged = map([file('missions/land.ts', 'a2'), file('lib/review.ts', 'b2')])
  const helperChanged = map([file('missions/land.ts', 'a'), file('lib/review.ts', 'b2')])
  const own = view.changedFiles(before, ownChanged)
  assert.deepEqual(own.map(change => [change.path, change.own]), [['missions/land.ts', true], ['lib/review.ts', false]])
  assert.equal(view.whyLine({ before, after: ownChanged, files: own, kdl: { added: 1, removed: 1 } }), 'The mission file land.ts changed, plus 1 file it uses.')
  assert.equal(view.whyLine({ before, after: helperChanged, files: view.changedFiles(before, helperChanged), kdl: { added: 0, removed: 0 } }),
    'The mission file land.ts did not change. The change comes from review.ts, which this mission uses. The rendered KDL is identical.')
  assert.equal(view.whyLine({ before, after: before, files: [], kdl: { added: 1, removed: 0 } }), 'No source file changed; the KDL differs because the rendering changed.')
})

test('chips, frames and permalinks', () => {
  const span = (file, start, end) => ({ file, start: { line: start, column: 1 }, end: { line: end, column: 1 } })
  const node = { kdl: 'mission[land/code]/step[review]', span: span(0, 30, 35), frames: [{ ...span(0, 30, 35), role: 'declaration', precision: 'exact' }, { ...span(1, 3, 19), role: { _tag: 'Unknown', raw: 'helper' }, precision: 'enclosing' }] }
  const m = map([file('missions/land.ts', 'a'), file('lib/review.ts', 'b', 'https://github.com/o/other')], [node])
  assert.equal(view.chipLabel(m, node), 'land.ts:30 · via review.ts')
  assert.equal(view.frameLocation(m, view.frameStrip(node)[1]), 'in review.ts:3–19')
  assert.equal(view.githubPermalink(m, 0, 30, 35), 'https://github.com/o/dotfiles/blob/3f9c2ab1d/missions/land.ts#L30-L35')
  assert.equal(view.githubPermalink(m, 1, 3, 19), undefined)
})
