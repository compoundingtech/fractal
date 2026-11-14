/**
 * Builds the bundle a program mission's `program` step pins (docs/st3/program-engine.md §4): one
 * self-contained ES module with Effect and the `fractal/effect` runtime inside, named by its
 * sha256. Node-only; it runs Bun.
 */
import { spawn } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { join } from 'node:path'

import { Data, Effect, FileSystem } from 'effect'

/** The program bundle of `module` could not be built or is not a program. */
export class ProgramBundleError extends Data.TaggedError('ProgramBundleError')<{
  readonly message: string
}> {}

const run = ({ bun, args, cwd }: { bun: string; args: readonly string[]; cwd: string }) =>
  Effect.callback<{ readonly status: number | null; readonly output: string }, ProgramBundleError>(
    (resume) => {
      const child = spawn(bun, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] })
      let output = ''
      child.stdout.setEncoding('utf8').on('data', (chunk: string) => (output += chunk))
      child.stderr.setEncoding('utf8').on('data', (chunk: string) => (output += chunk))
      child.once('error', (cause) =>
        resume(Effect.fail(new ProgramBundleError({ message: `${bun}: ${cause.message}` }))),
      )
      child.once('close', (status) => resume(Effect.succeed({ status, output })))
      return Effect.sync(() => child.kill('SIGKILL'))
    },
  )

/** Loads the bundle as the program host does and checks its default export is a program. */
const entryCheck = (bundle: string) =>
  `const m = await import(${JSON.stringify(bundle)}); ` +
  `if (typeof m.default?.[Symbol.for('fractal.program.v1')] !== 'function') ` +
  `{ console.error('its default export does not carry Symbol.for("fractal.program.v1")'); process.exit(1) }`

/**
 * Bundles `module` with `bun build --target=bun` and writes it to `<bundleDir>/<sha256>.mjs`.
 * Unminified, Bun heads each inlined module with a comment naming its path relative to `cwd`; a
 * dependency outside the tree (a loader's linked node_modules) then names a different path from
 * every checkout. `--minify-whitespace` drops those comments, so the bytes, and the hash, are the
 * same wherever the tree and its dependencies are checked out. Only a bundle that is a program
 * reaches `bundleDir`, and it appears there whole: it is renamed into place, and a bundle already
 * present under its hash is never rewritten.
 */
export const bundleProgram = Effect.fnUntraced(function* ({
  module,
  cwd,
  bundleDir,
  bun,
}: {
  readonly module: string
  readonly cwd: string
  readonly bundleDir: string
  readonly bun: string
}) {
  const fs = yield* FileSystem.FileSystem
  const failed = (message: string) => (cause: unknown) =>
    new ProgramBundleError({ message: `${message}: ${String(cause)}` })
  const scratch = yield* fs
    .makeTempDirectoryScoped({ prefix: 'fractal-program-' })
    .pipe(Effect.mapError(failed('cannot create a build directory')))
  const built = join(scratch, 'bundle.mjs')
  const build = yield* run({
    bun,
    cwd,
    args: [
      'build',
      module,
      '--target=bun',
      '--format=esm',
      '--minify-whitespace',
      `--outfile=${built}`,
    ],
  })
  if (build.status !== 0)
    return yield* new ProgramBundleError({ message: `bun build failed:\n${build.output.trim()}` })
  const check = yield* run({ bun, cwd, args: ['--eval', entryCheck(built)] })
  if (check.status !== 0)
    return yield* new ProgramBundleError({
      message: `${module} is not a program: ${check.output.trim()}`,
    })
  const bytes = yield* fs.readFile(built).pipe(Effect.mapError(failed('cannot read the bundle')))
  const sha256 = createHash('sha256').update(bytes).digest('hex')
  const bundle = join(bundleDir, `${sha256}.mjs`)
  const present = yield* fs.exists(bundle).pipe(Effect.mapError(failed(`cannot read ${bundle}`)))
  if (!present) {
    const staged = join(bundleDir, `.${sha256}.${randomUUID()}.tmp`)
    yield* fs.makeDirectory(bundleDir, { recursive: true }).pipe(
      Effect.andThen(fs.writeFile(staged, bytes)),
      Effect.andThen(fs.rename(staged, bundle)),
      Effect.mapError(failed(`cannot write ${bundle}`)),
    )
  }
  return { bundle, sha256 }
}, Effect.scoped)
