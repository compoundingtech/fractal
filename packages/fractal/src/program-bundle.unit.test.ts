import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { describe, expect, it } from '@effect/vitest'
import { Effect } from 'effect'

import { ProgramEntry, type HostContextV1, type ProgramExit } from './program-engine.ts'
import { fakeHost, journalOf } from './program-host.fixture.ts'

const landing = fileURLToPath(
  new URL('../fixtures/programs/factory/land-program/mission.ts', import.meta.url),
)

/** What `steps/factory.py <verb>` writes for a routine change, by verb: result file and value. */
const routine: Readonly<Record<string, readonly [string, unknown]>> = {
  classify: ['class', { change: 'abc', author: 'dev', classes: ['routine'], reasons: [] }],
  check: ['check', { mode: 'declaration', errors: [] }],
  merge: ['merged', { commit: 'c1', parent: 'p0', attempts: 1 }],
  publish: ['publish', { host: 'local', head: 'c1', changed: ['mission/factory/x'] }],
  verify: ['verify', { head: 'c1', stale: [] }],
}

describe('a program bundle', () => {
  it.effect(
    'built with bun --target=bun runs the landing program against a host',
    () =>
      Effect.gen(function* () {
        // Programs are Bun bundles; `nix develop .#web` puts bun on PATH.
        const version = spawnSync('bun', ['--version'], { encoding: 'utf8' })
        expect(version.status, 'bun must be on PATH: run the tests in `nix develop .#web`').toBe(0)
        const cwd = mkdtempSync(join(tmpdir(), 'fractal-program-'))
        const bundle = join(cwd, 'land.mjs')
        const build = spawnSync(
          'bun',
          ['build', landing, '--target=bun', '--format=esm', `--outfile=${bundle}`],
          { encoding: 'utf8' },
        )
        expect(build.status, build.stderr).toBe(0)
        const module: {
          readonly default: {
            readonly [ProgramEntry]: (host: HostContextV1) => Promise<ProgramExit>
          }
        } = yield* Effect.promise(() => import(pathToFileURL(bundle).href))

        const verbs: string[] = []
        const host = fakeHost({
          payload: { change: 'abc', reviewer: 'scripted' },
          cwd,
          spawn: async (argv) => {
            const verb = argv[2]!
            verbs.push(verb)
            const [result, value] = routine[verb]!
            const dir = join(cwd, 'rundata', 'mission-run_run-1')
            mkdirSync(dir, { recursive: true })
            writeFileSync(join(dir, `${result}.json`), JSON.stringify(value))
            return { code: 0, signal: null, stdout: '', stderr: '' }
          },
        })
        const exit = { _tag: 'Success', value: { class: 'routine', merged: 'c1', head: 'c1' } }
        expect(yield* Effect.promise(() => module.default[ProgramEntry](host.context))).toEqual(
          exit,
        )
        expect(verbs).toEqual(['classify', 'check', 'merge', 'publish', 'verify'])
        expect(host.recorded.map((entry) => entry.key)).toEqual([
          'classify',
          'check',
          'merge',
          'publish-local',
          'verify-local',
        ])

        const replay = fakeHost({
          payload: { change: 'abc', reviewer: 'scripted' },
          cwd,
          journal: journalOf(host.recorded),
          spawn: () => Promise.reject(new Error('a replayed step ran its command')),
        })
        expect(yield* Effect.promise(() => module.default[ProgramEntry](replay.context))).toEqual(
          exit,
        )
      }),
    60_000,
  )
})
