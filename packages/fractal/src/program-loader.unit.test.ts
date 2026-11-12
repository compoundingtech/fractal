import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { NodeFileSystem } from '@effect/platform-node'
import { describe, expect, it } from '@effect/vitest'
import { Effect } from 'effect'

import { emit, mission, type MissionInput } from './mod.ts'
import { bundleProgram } from './program-bundler.ts'
import { loadSubjectTree, programMission } from './tree.ts'

const reportTo = { id: 'factory' }
const sha256 = 'a'.repeat(64)
const programStep = { id: 'program', program: { bundle: '/bundles/land.mjs', sha256 } }
const render = (input: Partial<MissionInput<'factory/land'>>) =>
  emit([
    mission({
      id: 'factory/land',
      state: 'ready',
      reportTo,
      goal: 'Land.',
      steps: [programStep],
      ...input,
    }),
  ])

describe('program declarations as data', () => {
  it('render the program step, run authority in canonical order and concurrent runs', () => {
    expect(
      render({
        concurrentRuns: 2,
        authority: { write: ['resource/z', 'resource/a/*'], revise: ['factory/*'], publish: ['x'] },
        steps: [
          {
            ...programStep,
            timeout: '1h',
            program: { ...programStep.program, stalledAfter: '5m' },
          },
        ],
      }),
    ).toBe(
      [
        'version 2',
        'mission "factory/land" report-to="agent/factory" state="ready" {',
        '  concurrent-runs max=2',
        '  goal "Land."',
        '  authority {',
        '    publish "x"',
        '    revise "factory/*"',
        '    write "resource/a/*"',
        '    write "resource/z"',
        '  }',
        '  step "program" timeout="1h" {',
        `    program "/bundles/land.mjs" sha256="${sha256}" stalled-after="5m"`,
        '  }',
        '}',
        '',
      ].join('\n'),
    )
  })

  it('refuse a program step that does not alone decide its outcome', () => {
    for (const extra of [
      { assignedTo: { id: 'factory' } },
      { producesMission: 'factory/child' },
      { usesMission: { kind: 'revision', revision: `factory/x@${'0'.repeat(64)}` } },
    ] as const)
      expect(() => render({ steps: [{ ...programStep, ...extra }] })).toThrow(
        'a program step alone decides its outcome',
      )
    expect(() =>
      render({ steps: [{ id: 'work', agentless: true }], finally: [programStep] }),
    ).toThrow('a program step cannot be a finally step')
    expect(() =>
      render({ steps: [{ ...programStep, program: { bundle: 'land.mjs', sha256 } }] }),
    ).toThrow('a program bundle path must be absolute')
    expect(() =>
      render({
        steps: [{ ...programStep, program: { bundle: '/land.mjs', sha256: 'A'.repeat(64) } }],
      }),
    ).toThrow('a program sha256 is 64 lowercase hex digits')
  })

  it('refuse malformed authority and concurrent runs', () => {
    expect(() => render({ authority: {} })).toThrow('an authority block needs at least one rule')
    expect(() => render({ authority: { start: ['mission/factory/x'] } })).toThrow(
      'an authority mission ID is exact or ends in a /* namespace',
    )
    expect(() => render({ authority: { write: ['resource/*/x'] } })).toThrow(
      'an authority subject is exact or ends in a /* namespace',
    )
    expect(() => render({ authority: { publish: ['x', 'x'] } })).toThrow(
      'an authority rule is declared once',
    )
    expect(() => render({ concurrentRuns: 0 })).toThrow(
      'concurrent-runs max must be a positive integer',
    )
  })
})
const fixtures = fileURLToPath(new URL('../fixtures', import.meta.url))
const programs = join(fixtures, 'programs')
const golden = join(fixtures, 'land-program.kdl')

/** Renders the landing program with its bundle directory shown as `/bundles`. */
const renderLanding = (bundleDir: string) =>
  Effect.gen(function* () {
    const subjects = yield* loadSubjectTree({ root: programs, context: undefined, bundleDir })
    const landing = subjects.find((subject) => subject.id === 'factory/land-program')
    if (landing?.declaration._tag !== 'Module')
      return yield* Effect.die('the landing program did not load')
    const kdl = emit([mission(landing.declaration.intent as MissionInput<string>)])
    const sha256 = /sha256="([0-9a-f]{64})"/u.exec(kdl)?.[1]
    if (sha256 === undefined) return yield* Effect.die(`no program step in\n${kdl}`)
    return { kdl, sha256, shown: kdl.replaceAll(bundleDir, '/bundles') }
  }).pipe(Effect.provide(NodeFileSystem.layer))

const refusal = (root: string, bundleDir: string | undefined) =>
  loadSubjectTree({
    root,
    context: undefined,
    ...(bundleDir === undefined ? {} : { bundleDir }),
  }).pipe(Effect.flip, Effect.provide(NodeFileSystem.layer))

const scratch = () => mkdtempSync(join(tmpdir(), 'fractal-bundles-'))

describe('program missions', () => {
  it.effect(
    'render to a pinned bundle and the KDL that runs it',
    () =>
      Effect.gen(function* () {
        const bundleDir = scratch()
        const { kdl, sha256, shown } = yield* renderLanding(bundleDir)
        // `FRACTAL_UPDATE_GOLDENS=1` rewrites the golden after an intended change to the program
        // or to code its bundle inlines.
        if (process.env.FRACTAL_UPDATE_GOLDENS === '1') writeFileSync(golden, shown)
        expect(shown).toBe(readFileSync(golden, 'utf8'))
        const bundle = readFileSync(join(bundleDir, `${sha256}.mjs`))
        expect(createHash('sha256').update(bundle).digest('hex')).toBe(sha256)
        expect(kdl).toContain(`program "${join(bundleDir, `${sha256}.mjs`)}"`)
      }),
    120_000,
  )

  it.effect(
    'bundle deterministically',
    () =>
      Effect.gen(function* () {
        const first = scratch()
        const second = scratch()
        const a = yield* renderLanding(first)
        const b = yield* renderLanding(second)
        expect(b.sha256).toBe(a.sha256)
        expect(readFileSync(join(second, `${b.sha256}.mjs`))).toEqual(
          readFileSync(join(first, `${a.sha256}.mjs`)),
        )
      }),
    120_000,
  )

  it.effect(
    'bundle the same bytes from any build directory, as for a tree checked out elsewhere',
    () =>
      Effect.gen(function* () {
        // Dependencies outside the tree (a loader's linked node_modules) sit at another relative
        // path from every checkout; the bytes must not record that path.
        const module = join(programs, 'factory/land-program/mission.ts')
        const near = yield* bundleProgram({ module, cwd: programs, bundleDir: scratch(), bun: 'bun' })
        const deeper = yield* bundleProgram({
          module,
          cwd: join(programs, 'factory/land-program'),
          bundleDir: scratch(),
          bun: 'bun',
        })
        expect(deeper.sha256).toBe(near.sha256)
      }).pipe(Effect.provide(NodeFileSystem.layer)),
    120_000,
  )

  it.effect(
    'render the same through programMission for a loader that walks its own tree',
    () =>
      Effect.gen(function* () {
        const bundleDir = scratch()
        const { kdl, sha256 } = yield* renderLanding(bundleDir)
        // Reached through a symlink, the tree still names its subjects by their real directories.
        const root = join(scratch(), 'tree')
        symlinkSync(programs, root)
        const file = join(root, 'factory/land-program/mission.ts')
        const real = pathToFileURL(join(programs, 'factory/land-program/mission.ts')).href
        const module = yield* Effect.promise(
          (): Promise<{ readonly default: unknown }> => import(real),
        )
        const rendered = yield* programMission({ root, file, bundleDir, definition: module.default })
        expect(rendered).toEqual({ kdl, bundle: join(bundleDir, `${sha256}.mjs`), sha256 })
        const refused = yield* programMission({ root, file, bundleDir, definition: {} }).pipe(
          Effect.flip,
        )
        expect(refused.message).toBe('The default export is not a program mission')
        const elsewhere = join(fixtures, 'program-refusals/id-mismatch/factory/land/mission.ts')
        const foreign = yield* programMission({
          root,
          file: elsewhere,
          bundleDir,
          definition: module.default,
        }).pipe(Effect.flip)
        expect(foreign.message).toContain('Module must export its own mission definition')
      }).pipe(Effect.provide(NodeFileSystem.layer)),
    120_000,
  )

  it.effect('need an absolute bundle directory', () =>
    Effect.gen(function* () {
      const missing = yield* refusal(programs, undefined)
      expect(missing.path).toBe('factory/land-program/mission.ts')
      expect(missing.message).toContain('needs loadSubjectTree({ bundleDir })')
      const relative = yield* refusal(programs, 'bundles')
      expect(relative.message).toBe('bundleDir must be an absolute directory')
    }),
  )

  it.effect('declare their own directory as their id', () =>
    Effect.gen(function* () {
      const error = yield* refusal(join(fixtures, 'program-refusals/id-mismatch'), scratch())
      expect(error.path).toBe('factory/land/mission.ts')
      expect(error.message).toBe(
        'Declared id factory/elsewhere must equal the subject directory factory/land',
      )
    }),
  )

  it.effect('have inputs that encode to strings', () =>
    Effect.gen(function* () {
      const error = yield* refusal(join(fixtures, 'program-refusals/number-input'), scratch())
      expect(error.path).toBe('factory/land/mission.ts')
      expect(String(error.cause)).toContain(
        'Program input attempts must be required and encode to a string',
      )
    }),
  )
})

describe('program bundles', () => {
  const build = (source: string, bun = 'bun') =>
    Effect.gen(function* () {
      const dir = scratch()
      writeFileSync(join(dir, 'mission.ts'), source)
      return yield* bundleProgram({
        module: join(dir, 'mission.ts'),
        cwd: dir,
        bundleDir: dir,
        bun,
      })
    }).pipe(Effect.flip, Effect.provide(NodeFileSystem.layer))

  it.effect('refuse a module whose default export is not a program', () =>
    Effect.gen(function* () {
      const error = yield* build('export default { name: "not a program" }\n')
      expect(error.message).toContain('is not a program')
      expect(error.message).toContain('Symbol.for("fractal.program.v1")')
    }),
  )

  it.effect('refuse a module bun cannot build', () =>
    Effect.gen(function* () {
      const error = yield* build("export { default } from 'no-such-package'\n")
      expect(error.message).toContain('bun build failed')
      expect(error.message).toContain('no-such-package')
    }),
  )

  it.effect('refuse a missing bun', () =>
    Effect.gen(function* () {
      const error = yield* build('export default {}\n', '/nonexistent/bun')
      expect(error.message).toContain('/nonexistent/bun')
    }),
  )
})

// Native conformance is opt-in, as elsewhere in this package: `ST_BIN` is the `fractal` binary.
const stBin = process.env.ST_BIN
const testWithSt = stBin !== undefined && stBin !== '' ? it.effect : it.effect.skip

describe('program declarations', () => {
  testWithSt(
    'validate natively, including the bundle hash',
    () =>
      Effect.gen(function* () {
        const bundleDir = scratch()
        const { kdl, sha256 } = yield* renderLanding(bundleDir)
        const file = join(bundleDir, 'land-program.kdl')
        writeFileSync(file, kdl)
        const valid = spawnSync(stBin!, ['validate', file], { encoding: 'utf8' })
        expect(valid.status, `${valid.stderr}\n${valid.stdout}`).toBe(0)
        // The native parser reads the bundle and refuses one whose bytes are not the pinned ones.
        writeFileSync(join(bundleDir, `${sha256}.mjs`), '// changed after rendering\n')
        const stale = spawnSync(stBin!, ['validate', file], { encoding: 'utf8' })
        expect(stale.status).not.toBe(0)
        expect(stale.stderr).toContain(`the mission declares ${sha256}`)
      }),
    120_000,
  )
})
