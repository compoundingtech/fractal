import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { assertStrictEvidence } from './kit-play-gate-runner.mjs'

// A fixture build with the strict policy's aliases and kept bases. Like Storybook's addon-a11y,
// it reports an a11y result unless the a11y.manual global is set; `report` overrides that report.
const iframe = (report, probes = [], navigation) => `<!doctype html><meta charset="utf-8"><title>Gate fixture</title>
<script>
  ${navigation?.startsWith('reload-') ? `
  if (!sessionStorage.getItem('reloaded')) {
    sessionStorage.setItem('reloaded', 'true')
    ${navigation === 'reload-before-dom' ? "location.reload()" : "document.addEventListener('DOMContentLoaded', () => location.reload(), { once: true })"}
  }` : ''}
  const listeners = new Map()
  const manual = new URLSearchParams(location.search).get('globals') === 'a11y.manual:!true'
  const id = new URLSearchParams(location.search).get('id')
  const emit = (name, payload) => { for (const listener of listeners.get(name) ?? []) listener(payload) }
  window.__STORYBOOK_ADDONS_CHANNEL__ = {
    on(name, listener) {
      if (!listeners.has(name)) listeners.set(name, [])
      listeners.get(name).push(listener)
      if (name === 'storyErrored') setTimeout(async () => {
        const phase = newPhase => emit('storyRenderPhaseChanged', { storyId: id, newPhase })
        phase('playing')
        await new Promise(resolve => setTimeout(resolve, 2))
        phase('played')
        phase('completing')
        ${navigation === 'document' ? "location.assign('/away.html'); return" : ['push', 'replace'].includes(navigation) ? `history.${navigation}State({}, '', '/w/fixture/missions')` : ''}
        if (!manual) {
          window.dispatchEvent(new CustomEvent('kit-a11y-timing', { detail: { storyId: id, phase: 'start', at: performance.now() } }))
          await new Promise(resolve => setTimeout(resolve, 2))
          window.dispatchEvent(new CustomEvent('kit-a11y-timing', { detail: { storyId: id, phase: 'end', at: performance.now() } }))
        }
        for (const probe of ${JSON.stringify(probes)}) window.dispatchEvent(new CustomEvent('kit-filter-benchmark', { detail: probe }))
        emit('storyFinished', {
          storyId: id, status: ${JSON.stringify(report?.some(({ status }) => status === 'failed') ? 'error' : 'success')},
          reporters: ${report === undefined ? "manual ? [] : [{ type: 'a11y', status: 'passed' }]" : JSON.stringify(report)},
        })
      }, 0)
    },
  }
</script>`

// These untagged stories satisfy the complete-index policy without satisfying the per-commit
// critical inventory: that test must still fail at its own missing-critical-story check.
const policyFixtureIds = [
  'fractal-ui-transcript--no-green-all-states-light',
  'fractal-ui-transcript--no-green-all-states',
  'fractal-ui-composer--footer-width-budget-light',
  'fractal-ui-composer--footer-width-budget',
  'fractal-ui-sidebar-tree--filter-keystroke-to-paint',
]

const runGate = async ({ env = {}, report, probes, navigation, fixtureOverrides } = {}) => {
  const directory = await mkdtemp(join(tmpdir(), 'kit-play-gate-test-'))
  try {
    const build = join(directory, 'build'), output = join(directory, 'receipt')
    await mkdir(build)
    await writeFile(join(build, 'index.json'), JSON.stringify({ entries: {
      critical: { type: 'story', id: 'kit--critical', importPath: './Kit.stories.tsx', tags: ['dev', 'critical'] },
      other: { type: 'story', id: 'kit--other', importPath: './Kit.stories.tsx', tags: ['dev'] },
      ...Object.fromEntries(policyFixtureIds.map(id => [id, {
        type: 'story', id, importPath: './Kit.stories.tsx', tags: ['dev'],
      }])),
    } }))
    await writeFile(join(build, 'iframe.html'), iframe(report, probes, navigation))
    await writeFile(join(build, 'away.html'), '<!doctype html><title>Not Storybook</title>')
    // The job's own KIT_PLAY_* options (CI sets them per event) must not reach the fixture runs.
    const inherited = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('KIT_PLAY_')))
    const args = fixtureOverrides === undefined
      ? [fileURLToPath(new URL('./kit-play-gate.mjs', import.meta.url)), build, output]
      : ['--input-type=module', '--eval', `
          import { runKitPlayGate } from ${JSON.stringify(new URL('./kit-play-gate-runner.mjs', import.meta.url).href)}
          const receipt = await runKitPlayGate({
            buildDirectory: ${JSON.stringify(build)}, outputDirectory: ${JSON.stringify(output)},
            overrides: new Map(${JSON.stringify(fixtureOverrides)}),
          })
          if (!receipt.pass) process.exitCode = 1
        `]
    const child = spawn(process.execPath, args, {
      env: { ...inherited, ...env },
      stdio: 'ignore',
    })
    const code = await new Promise((resolve, reject) => {
      child.once('error', reject)
      child.once('close', resolve)
    })
    return { code, receipt: JSON.parse(await readFile(join(output, 'receipt.json'), 'utf8')) }
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
}

test('full set with a11y requires one passed a11y report per case', { timeout: 60000 }, async () => {
  // Empty values are unset, as a workflow expression with no value yields them.
  const { code, receipt } = await runGate({ env: { KIT_PLAY_SET: '', KIT_PLAY_A11Y: '', KIT_PLAY_MAX_SECONDS: '' } })
  assert.equal(code, 0)
  assert.equal(receipt.pass, true)
  assert.deepEqual([receipt.set, receipt.a11y, receipt.maxSeconds, receipt.stories, receipt.cases], ['all', 'on', null, 7, 17])
  assert.ok(receipt.cpu.count > 0 && receipt.cpu.busyCoreMinutes >= 0 && receipt.cpu.loadavgEnd.length === 3)
  for (const row of receipt.rows) {
    assert.ok(row.timings.playMilliseconds > 0)
    assert.ok(row.timings.a11yMilliseconds > 0)
    const sum = ['bootstrapMilliseconds', 'playMilliseconds', 'a11yMilliseconds', 'lifecycleMilliseconds', 'completionObservationMilliseconds'].reduce((sum, name) => sum + row.timings[name], 0)
    assert.ok(Math.abs(sum - row.timings.readinessAfterDomMilliseconds) < 0.001)
  }
  assert.equal(receipt.phaseMilliseconds.a11yMilliseconds.samples, receipt.cases)
  assert.equal(receipt.topCases.length, 17)
  assert.equal(receipt.storyFamilies.reduce((sum, family) => sum + family.cases, 0), receipt.cases)
})

test('fixture code can inject its policy through the runner parameter, not process environment', { timeout: 60000 }, async () => {
  const { code, receipt } = await runGate({ fixtureOverrides: [] })
  assert.equal(code, 0)
  assert.equal(receipt.pass, true)
  assert.deepEqual(receipt.caseOverrides, {})
  assert.equal(receipt.cases, 28)
})

test('empty, duplicate, and wrong-story strict evidence fail closed', () => {
  const event = { payload: { storyId: 'kit--critical', status: 'success', reporters: [{ type: 'a11y', status: 'passed' }] } }
  for (const records of [[], [{ finished: [] }], [{ finished: [event, event] }], [{ finished: [event] }, { finished: [event] }]]) {
    assert.throws(() => assertStrictEvidence(records, 'kit--critical', 'on'), /exactly one completed strict story record/)
  }
  assert.throws(() => assertStrictEvidence([{ finished: [event] }], 'kit--other', 'on'), /requested story/)
  const duplicateReports = { ...event.payload, reporters: [...event.payload.reporters, ...event.payload.reporters] }
  assert.throws(() => assertStrictEvidence([{ finished: [{ payload: duplicateReports }] }], 'kit--critical', 'on'), /expected one passed a11y report/)
})

test('History API navigation preserves one completed record at the final URL', { timeout: 60000 }, async () => {
  for (const navigation of ['push', 'replace']) {
    const { code, receipt } = await runGate({ navigation, env: { KIT_PLAY_SET: 'critical' } })
    assert.equal(code, 0)
    assert.ok(receipt.rows.every(row => row.strict.length === 1 && row.strict[0].url.endsWith('/w/fixture/missions')))
    assert.ok(receipt.rows.every(row => row.strict[0].finished.length === 1))
  }
})

test('in-app navigation without axe evidence fails', { timeout: 60000 }, async () => {
  const { code, receipt } = await runGate({ navigation: 'push', report: [], env: { KIT_PLAY_SET: 'critical' } })
  assert.equal(code, 1)
  assert.equal(receipt.pass, false)
  assert.ok(receipt.rows.every(row => /expected one passed a11y report/.test(row.error)))
  assert.ok(receipt.rows.every(row => row.strict[0].url.endsWith('/w/fixture/missions')))
})

test('cross-document navigation away from Storybook fails closed', { timeout: 60000 }, async () => {
  const { code, receipt } = await runGate({ navigation: 'document', env: { KIT_PLAY_SET: 'critical' } })
  assert.equal(code, 1)
  assert.equal(receipt.pass, false)
  assert.ok(receipt.rows.every(row => /story document was replaced|Execution context was destroyed/.test(row.error)))
})

for (const navigation of ['reload-before-dom', 'reload-after-dom']) {
  test(`same-URL ${navigation} fails closed`, { timeout: 60000 }, async () => {
    const { code, receipt } = await runGate({ navigation, env: { KIT_PLAY_SET: 'critical' } })
    assert.equal(code, 1)
    assert.equal(receipt.pass, false)
    assert.ok(receipt.rows.every(row => /story document was replaced/.test(row.error)), JSON.stringify(receipt.rows.map(row => row.error)))
  })
}

test('a11y on rejects a case whose axe report is missing', { timeout: 60000 }, async () => {
  const { code, receipt } = await runGate({ report: [] })
  assert.equal(code, 1)
  assert.equal(receipt.pass, false)
  assert.ok(receipt.rows.every(row => /expected one passed a11y report/.test(row.error)))
})

test('a failed axe report names its rule and the node that broke it', { timeout: 60000 }, async () => {
  const violation = { id: 'label-content-name-mismatch', impact: 'serious', nodes: [{ target: ['button[aria-label="Needs me only"]'], html: '<button aria-label="Needs me only">Needs me · 2</button>', failureSummary: 'Text inside the element is not included in the accessible name' }] }
  const { code, receipt } = await runGate({ report: [{ type: 'a11y', status: 'failed', result: { violations: [violation] } }] })
  assert.equal(code, 1)
  assert.equal(receipt.pass, false)
  for (const row of receipt.rows) {
    assert.match(row.error, /label-content-name-mismatch/)
    assert.match(row.error, /Needs me only/)
    assert.match(row.error, /not included in the accessible name/)
  }
  const crashed = await runGate({ report: [{ type: 'a11y', status: 'failed', result: { error: { message: 'axe could not run' } } }] })
  assert.ok(crashed.receipt.rows.every(row => /axe could not run/.test(row.error)))
})

test('critical set without a11y selects tagged stories and asks addon-a11y not to run', { timeout: 60000 }, async () => {
  const { code, receipt } = await runGate({ env: { KIT_PLAY_SET: 'critical', KIT_PLAY_A11Y: 'off' } })
  assert.equal(code, 0)
  assert.deepEqual([receipt.set, receipt.a11y, receipt.stories, receipt.cases], ['critical', 'off', 1, 4])
  assert.ok(receipt.rows.every(row => row.id === 'kit--critical' && row.strict.every(record => record.url.includes('globals=a11y.manual'))))
  assert.ok(receipt.rows.every(row => row.timings.a11yMilliseconds === 0))
})

test('the per-commit set refuses a build whose critical stories are not the ones it names', { timeout: 60000 }, async () => {
  const { code, receipt } = await runGate({ env: { KIT_PLAY_SET: 'commit', KIT_PLAY_A11Y: 'off' } })
  assert.equal(code, 1)
  assert.equal(receipt.pass, false)
  assert.equal(receipt.cases, 0)
  assert.match(receipt.fatalError, /names fractal-ui-.*, which is not a critical story/)
})

test('a11y off rejects a case where axe still ran', { timeout: 60000 }, async () => {
  const { code, receipt } = await runGate({ env: { KIT_PLAY_A11Y: 'off' }, report: [{ type: 'a11y', status: 'passed' }] })
  assert.equal(code, 1)
  assert.ok(receipt.rows.every(row => /a11y off: axe still ran/.test(row.error)))
})

test('a run over its wall-time budget fails after running every case', { timeout: 60000 }, async () => {
  const { code, receipt } = await runGate({ env: { KIT_PLAY_SET: 'critical', KIT_PLAY_MAX_SECONDS: '0.001' } })
  assert.equal(code, 1)
  assert.equal(receipt.pass, false)
  assert.equal(receipt.overBudget, true)
  assert.equal(receipt.maxSeconds, 0.001)
  assert.equal(receipt.cases, 4)
  assert.ok(receipt.rows.every(row => row.pass))
})

test('invalid options fail before any case runs', { timeout: 60000 }, async () => {
  for (const env of [{ KIT_PLAY_SET: 'some' }, { KIT_PLAY_A11Y: 'no' }, { KIT_PLAY_MAX_SECONDS: '-1' }, { KIT_PLAY_MAX_SECONDS: 'soon' },
    // Only the full set is sharded.
    { KIT_PLAY_SET: 'commit', KIT_PLAY_SHARD_COUNT: '2' }, { KIT_PLAY_SET: 'critical', KIT_PLAY_SHARD_INDEX: '1', KIT_PLAY_SHARD_COUNT: '2' }]) {
    await assert.rejects(() => runGate({ env }), /ENOENT/)
  }
})

test('structured benchmark events survive successful and failed case receipts', { timeout: 60000 }, async () => {
  const probes = [240, 440].flatMap(width => ['dark', 'light'].map(theme => ({
    width, theme, filterP90Milliseconds: 30, referenceP90Milliseconds: 20,
    pooledRatio: 1.5, busyControlRatio: 4.1, threshold: 3,
  })))
  for (const status of ['passed', 'failed']) {
    const { code, receipt } = await runGate({
      env: { KIT_PLAY_SET: 'critical' },
      report: [{ type: 'a11y', status, result: { violations: [] } }],
      probes,
    })
    assert.equal(code, status === 'passed' ? 0 : 1)
    assert.ok(receipt.rows.every(row => JSON.stringify(row.filterBenchmark) === JSON.stringify(probes)))
  }
})
