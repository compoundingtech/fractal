import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const gateName = 'Validate public sidebar fixtures and scrubber self-tests'
const workflow = readFileSync(new URL('../.github/workflows/fleet.yml', import.meta.url), 'utf8')
const jobs = new Map([...workflow.matchAll(/^  ([\w-]+):\n([\s\S]*?)(?=^  [\w-]+:|$(?![\s\S]))/gm)]
  .map((match) => [match[1], match[2]]))

const assertGate = (job) => {
  assert.ok(job, 'job exists')
  const steps = job.split(/^      - /m)
  const gate = steps.find((step) => step.startsWith(`name: ${gateName}\n`))
  assert.ok(gate, 'fixture gate exists')
  assert.match(gate, /bash -euc/)
  assert.match(gate, /node --test scripts\/check-sidebar-fixtures\.test\.mjs; node scripts\/check-sidebar-fixtures\.mjs/)
  assert.doesNotMatch(gate, /continue-on-error/)
  assert.doesNotMatch(job.split('    steps:')[0], /continue-on-error/)
  return job.indexOf(gateName)
}

test('required freshness runs the public-data gate before client consumers', () => {
  const freshness = jobs.get('genie-freshness')
  assert.ok(assertGate(freshness) < freshness.indexOf('scripts/ci-genie-freshness'))
  for (const name of ['typescript-client', 'fractal-web-execution']) {
    const job = jobs.get(name)
    assert.ok(job, name)
    assert.match(job.split('    if:')[0], /needs:[\s\S]*\bgenie-freshness\b/)
    assert.match(job, /needs\.genie-freshness\.result == 'success'/)
  }
  const local = readFileSync(new URL('./ci-genie-freshness', import.meta.url), 'utf8')
  assert.match(local, /^node --test scripts\/check-sidebar-fixtures\.test\.mjs$/m)
  assert.match(local, /^node scripts\/check-sidebar-fixtures\.mjs$/m)
})

test('all four required Rust partitions scrub fixtures before replay', () => {
  for (const name of ['linux-tests', 'linux-tests-shard-2', 'linux-tests-shard-3', 'linux-tests-shard-4']) {
    const job = jobs.get(name)
    const gate = assertGate(job)
    assert.ok(gate < job.indexOf('scripts/ci-linux tests'), name)
    const step = job.split(/^      - /m).find((value) => value.startsWith(`name: ${gateName}\n`))
    assert.doesNotMatch(step, /^        if:/m, 'running partitions cannot skip the gate')
  }
  assert.match(jobs.get('linux-gate'), /needs:[\s\S]*- linux-tests[\s\S]*- linux-tests-shard-4/)
})
