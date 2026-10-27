import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawn } from 'node:child_process'

// This test requires a real build. It runs the real indexed story, including its
// unchanged sampling, negative controls, ratio assertion and Storybook exception path.
test('real filter benchmark retains completed ratios when its threshold assertion throws', { timeout: 600000 }, async () => {
  const source = fileURLToPath(new URL('../storybook-static/', import.meta.url))
  const index = JSON.parse(await readFile(join(source, 'index.json'), 'utf8'))
  const id = 'fractal-ui-sidebar-tree--filter-keystroke-to-paint'
  assert.equal(index.entries[id]?.type, 'story')
  const directory = await mkdtemp(join(tmpdir(), 'kit-filter-benchmark-test-'))
  try {
    for (const regression of [false, true]) {
      const build = join(directory, regression ? 'regressed' : 'normal')
      const output = join(directory, regression ? 'failed-receipt' : 'passed-receipt')
      await mkdir(build)
      for (const name of await readdir(source)) {
        if (!['index.json', 'iframe.html'].includes(name)) await symlink(join(source, name), join(build, name))
      }
      await writeFile(join(build, 'index.json'), JSON.stringify({ ...index, entries: { [id]: index.entries[id] } }))
      const iframe = await readFile(join(source, 'iframe.html'), 'utf8')
      // Arm the story's existing synchronous-regression control before React's
      // input capture/onChange. Ten times each paired reference duration guarantees
      // a pooled ratio above 3, without changing samples or replacing assertions.
      const control = `<script>document.addEventListener('input', event => {
        if (event.target.type !== 'search') return
        const root = event.target.closest('[data-testid^="filter-probe-"]')
        if (!root) return
        const reference = JSON.parse(root.dataset.referenceSamples || '[]').at(-1)
        if (reference) root.dataset.synchronousCostMs = String(Math.max(Number(root.dataset.synchronousCostMs), 10 * reference.durationMs))
      }, true)</script>`
      await writeFile(join(build, 'iframe.html'), regression ? iframe.replace('<head>', `<head>${control}`) : iframe)
      const inherited = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('KIT_PLAY_')))
      const child = spawn(process.execPath, ['--input-type=module', '--eval', `
        import { runKitPlayGate } from ${JSON.stringify(new URL('./kit-play-gate-runner.mjs', import.meta.url).href)}
        const receipt = await runKitPlayGate({
          buildDirectory: ${JSON.stringify(build)}, outputDirectory: ${JSON.stringify(output)},
          overrides: new Map([[${JSON.stringify(id)}, [['dark', 1440]]]]),
        })
        if (!receipt.pass) process.exitCode = 1
      `], { env: inherited, stdio: ['ignore', 'pipe', 'pipe'] })
      let logs = ''
      child.stdout.on('data', chunk => { logs += chunk })
      child.stderr.on('data', chunk => { logs += chunk })
      const code = await new Promise((resolve, reject) => { child.once('error', reject); child.once('close', resolve) })
      const receipt = JSON.parse(await readFile(join(output, 'receipt.json'), 'utf8'))
      assert.equal(code, regression ? 1 : 0, logs)
      assert.equal(receipt.pass, !regression, logs)
      assert.equal(receipt.rows.length, 1)
      const row = receipt.rows[0]
      assert.equal(row.pass, !regression)
      assert.equal(row.filterBenchmark.length, regression ? 1 : 4)
      for (const probe of row.filterBenchmark) {
        assert.ok(probe.filterP90Milliseconds > 0 && probe.referenceP90Milliseconds > 0)
        assert.equal(probe.threshold, 3)
        assert.ok(probe.busyControlRatio > 3)
        assert.ok(regression ? probe.pooledRatio > probe.threshold : probe.pooledRatio <= probe.threshold)
      }
      if (regression) {
        assert.match(row.error, /expected [\d.]+ to be less than or equal to 3/)
        assert.ok(row.events.some(event => event.name === 'playFunctionThrewException'))
        assert.equal(row.events.flatMap(event => event.payload.reporters ?? []).length, 0, 'Measurements survive without any Storybook reporter data')
      }
      console.log(JSON.stringify({ regression, pass: row.pass, probes: row.filterBenchmark }))
    }
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
