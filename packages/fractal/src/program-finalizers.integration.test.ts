import { spawn, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath } from 'node:url'

import { expect, it } from '@effect/vitest'

const fixture = fileURLToPath(new URL('../fixtures/programs/cancel-finalizers/mission.ts', import.meta.url))
const host = fileURLToPath(new URL('../../../crates/fractal-reconcile/src/reconcile/program_host.mjs', import.meta.url))

for (const hang of ['no', 'yes']) {
  it(`real Effect ensuring step and command ${hang === 'yes' ? 'time out explicitly' : 'are journaled after cancellation'}`, async () => {
    const root = fileURLToPath(new URL('../../../tmp/', import.meta.url))
    mkdirSync(root, { recursive: true })
    const dir = mkdtempSync(join(root, 'program-finalizers-'))
    const module = join(dir, 'program.mjs')
    const build = spawnSync('bun', ['build', fixture, '--target=bun', '--format=esm', `--outfile=${module}`], { encoding: 'utf8' })
    expect(build.status, build.stderr).toBe(0)
    const groups = join(dir, 'groups')
    mkdirSync(groups)
    const child = spawn('bun', [host], {
      env: {
        ...process.env,
        FRACTAL_PROGRAM_GROUPS: groups,
        FRACTAL_PROGRAM_BOOT_ID: 'finalizers-test',
        FRACTAL_PROGRAM_WORKER_DEADLINE_MS: '30000',
        FRACTAL_PROGRAM_FINALIZER_GRACE_MS: '30000',
      },
      stdio: ['pipe', 'pipe', 'pipe'],
    })
    const ids = { step: 'step-run/test/program', attempt: 1 }
    const send = (message: object) => child.stdin.write(`${JSON.stringify({ ...ids, ...message })}\n`)
    const journal: { key: string; status: string; value?: unknown }[] = []
    let outcome: string | undefined
    let stderr = ''
    child.stderr.on('data', (chunk) => { stderr += String(chunk) })
    try {
      const { promise, resolve, reject } = Promise.withResolvers<void>()
      const timeout = AbortSignal.timeout(20_000)
      timeout.addEventListener('abort', () => reject(new Error(`worker acknowledgement timed out; ${stderr}`)), { once: true })
      child.on('error', reject)
      child.on('exit', (code) => reject(new Error(`host exited before worker acknowledgement: ${code}; ${stderr}`)))
      const lines = createInterface({ input: child.stdout })
      lines.on('line', (line) => {
        try {
          const message = JSON.parse(line)
          if (message.t === 'journal') {
            journal.push({ key: message.key, status: message.status, value: message.value })
            if (message.status !== 'started') send({ t: 'ack', seq: message.seq })
            if (message.key === 'work' && message.status === 'started') send({ t: 'cancel', reason: 'test cancellation' })
          } else if (message.t === 'finalizers') {
            outcome = message.outcome
          } else if (message.t === 'cancelled') {
            resolve()
          }
        } catch (error) {
          reject(error)
        }
      })
      send({
        t: 'start', run: 'mission-run/test', cwd: dir, env: {}, payload: { hang }, journal: [],
        finalizerGraceMs: hang === 'yes' ? 1000 : 10000,
        module, sha256: createHash('sha256').update(readFileSync(module)).digest('hex'),
      })
      await promise
      expect(journal).toContainEqual({ key: 'drain', status: 'started', value: undefined })
      expect(outcome).toBe(hang === 'yes' ? 'timed-out' : 'completed')
      expect(journal.filter((entry) => entry.key === 'drain' && entry.status === 'completed')).toEqual(
        hang === 'yes' ? [] : [{ key: 'drain', status: 'completed', value: 'drained' }],
      )
    } finally {
      child.stdin.end()
      child.kill('SIGKILL')
      rmSync(dir, { recursive: true, force: true })
    }
  }, 30_000)
}
