import { basename, dirname, isAbsolute, join, relative, sep } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { Effect, FileSystem, Schema } from 'effect'

import { parse } from '@overeng/kdl'

import type { PinnedMissionRef } from './define.ts'
import {
  AgentSchema,
  emit,
  mission as missionNode,
  MissionSchema,
  missionWire,
  type MissionInput,
  SubjectId,
} from './mod.ts'
import { objective, validName, type Objective, type ObjectiveInput } from './objective.ts'
import { bundleProgram } from './program-bundler.ts'
import { ProgramEntry } from './program-engine.ts'
import { programInputNames, type InputSchema, type ProgramIntent } from './program.ts'
import type { SubjectKind, SubjectRef } from './subject-ref.ts'
export {
  defineAgent,
  defineMission,
  defineSchedule,
  defineObjective,
  pinMission,
  type AgentIntent,
  type MissionIntent,
  type PinnedMissionRef,
  type ScheduleIntent,
  type ObjectiveIntent,
} from './define.ts'
export type {
  SubjectKind,
  SubjectRef,
  SubjectDefinition,
  AgentRef,
  MissionRef,
} from './subject-ref.ts'

/** A subject tree that cannot be loaded; `path` is relative to the tree root where known. */
export class SubjectTreeError extends Schema.TaggedError<SubjectTreeError>()('SubjectTreeError', {
  path: Schema.String,
  message: Schema.String,
  cause: Schema.optional(Schema.Defect()),
}) {}

type Declaration<T> =
  | { readonly _tag: 'Kdl'; readonly text: string }
  | {
      readonly _tag: 'Module'
      readonly intent: T
      readonly exports: Readonly<Record<string, unknown>>
    }

/** A loaded declaration; `path` is POSIX and relative to the tree root. */
export type Subject =
  | {
      readonly kind: 'agent'
      readonly id: SubjectId
      readonly path: string
      readonly declaration: Declaration<typeof AgentSchema.Type>
    }
  | {
      readonly kind: 'mission' | 'schedule'
      readonly id: SubjectId
      readonly path: string
      readonly declaration: Declaration<typeof MissionSchema.Type>
    }
  | {
      readonly kind: 'objective'
      readonly id: SubjectId
      readonly path: string
      readonly declaration: Declaration<Objective>
    }

const declarationFile = /^(agent|mission|schedule|objective)\.(ts|kdl)$/u

/** The subject family of a kind: missions and schedules share `mission/<id>`. */
const family = (kind: SubjectKind) => (kind === 'schedule' ? 'mission' : kind)

const scheduleMismatch: Record<'mission' | 'schedule', string> = {
  mission: 'A mission with a schedule must be declared in schedule.ts or schedule.kdl',
  schedule: 'A schedule file must declare a mission with a schedule',
}

/** A fleet declares only interval or count objectives named `NAMESPACE/NAME`; Fractal ships the rest. */
const declaredObjective = (id: string) =>
  `A declared objective is an interval or count over claims named NAMESPACE/NAME; objective/${id} is not`

/** The indicators a fleet declares: over claims. */
const declarable = (kind: unknown): boolean => kind === 'interval' || kind === 'count'

const isRef = (value: unknown): value is SubjectRef<SubjectKind> =>
  typeof value === 'object' && value !== null && '_tag' in value && value._tag === 'SubjectRef'

const isPinnedRef = (value: unknown): value is PinnedMissionRef =>
  typeof value === 'object' &&
  value !== null &&
  '_tag' in value &&
  value._tag === 'PinnedMissionRef'

/** A declaration's subject ID is its directory relative to the tree root, POSIX-separated. */
const subjectId = ({
  root,
  file,
  path,
}: {
  readonly root: string
  readonly file: string
  readonly path: string
}) =>
  Schema.decodeEffect(SubjectId)(relative(root, dirname(file)).split(sep).join('/')).pipe(
    Effect.mapError(
      (cause) =>
        new SubjectTreeError({
          path,
          message: 'Subject directory is not a valid subject ID inside the tree',
          cause,
        }),
    ),
  )

const resolveRef = Effect.fnUntraced(function* ({
  root,
  path,
  ref,
  expected,
}: {
  readonly root: string
  readonly path: string
  readonly ref: SubjectRef<SubjectKind>
  readonly expected: SubjectKind
}) {
  if (ref.kind !== expected) {
    return yield* new SubjectTreeError({
      path,
      message: `Reference must point at ${expected}.ts, got ${ref.kind}.ts`,
    })
  }
  const file = yield* Effect.try({
    try: () => fileURLToPath(ref.url),
    catch: (cause) =>
      new SubjectTreeError({ path, message: 'Reference has no module file URL', cause }),
  })
  if (basename(file) !== `${expected}.ts`) {
    return yield* new SubjectTreeError({
      path,
      message: `Reference must point at ${expected}.ts, got ${basename(file)}`,
    })
  }
  const fs = yield* FileSystem.FileSystem
  const canonical = yield* fs
    .realPath(file)
    .pipe(
      Effect.mapError(
        (cause) =>
          new SubjectTreeError({ path, message: 'Cannot resolve reference module', cause }),
      ),
    )
  if (basename(canonical) !== `${expected}.ts`) {
    return yield* new SubjectTreeError({
      path,
      message: `Reference must point at ${expected}.ts, got ${basename(canonical)}`,
    })
  }
  return yield* subjectId({ root, file: canonical, path })
})

const resolvePinnedRef = Effect.fnUntraced(function* ({
  root,
  path,
  ref,
}: {
  readonly root: string
  readonly path: string
  readonly ref: PinnedMissionRef
}) {
  if (/^[0-9a-fA-F]{64}$/u.test(ref.revision) === false) {
    return yield* new SubjectTreeError({
      path,
      message: 'Pinned mission revision must be a 64-hex native revision',
    })
  }
  if (isRef(ref.subject) === false) {
    return yield* new SubjectTreeError({
      path,
      message: 'Pinned mission needs a mission reference',
    })
  }
  const id = yield* resolveRef({ root, path, ref: ref.subject, expected: 'mission' })
  return `${id}@${ref.revision}`
})

type ReferenceFields = {
  readonly reportTo?: unknown
  readonly assignedTo?: unknown
  readonly target?: unknown
  readonly steps?: unknown
  readonly finally?: unknown
  readonly under?: unknown
  readonly gates?: unknown
  readonly until?: unknown
  readonly review?: unknown
}

/** Lowers reference-bearing fields, including loop-until reviews, preserving scoped handles. */
const lowerReferences: (args: {
  readonly root: string
  readonly path: string
  readonly intent: unknown
}) => Effect.Effect<unknown, SubjectTreeError, FileSystem.FileSystem> = Effect.fnUntraced(
  function* ({ root, path, intent }) {
    if (typeof intent !== 'object' || intent === null) return intent
    const fields = intent as ReferenceFields
    let lowered: Record<PropertyKey, unknown> | undefined
    const set = ({ key, value }: { key: string; value: unknown }) => {
      lowered ??= { ...intent }
      lowered[key] = value
    }
    for (const key of ['reportTo', 'assignedTo', 'target'] as const) {
      if (key in intent && isRef(fields[key]) === true) {
        const id = yield* resolveRef({ root, path, ref: fields[key], expected: 'agent' })
        set({ key, value: { id } })
      }
    }
    for (const key of ['steps', 'finally', 'under', 'gates', 'until', 'review'] as const) {
      if (key in intent && Array.isArray(fields[key]) === true) {
        const values = fields[key]
        const next = yield* Effect.forEach(values as readonly unknown[], (value) =>
          lowerReferences({ root, path, intent: value }),
        )
        if (next.some((value, index) => value !== values[index]) === true) set({ key, value: next })
      }
    }
    if ('round' in intent) {
      const round = yield* lowerReferences({ root, path, intent: intent.round })
      if (round !== intent.round) set({ key: 'round', value: round })
    }
    if (
      '_tag' in intent &&
      intent._tag === 'Subject' &&
      'subject' in intent &&
      isRef(intent.subject) === true
    ) {
      const ref = intent.subject
      if (ref.kind !== 'agent' && ref.kind !== 'mission' && ref.kind !== 'schedule') {
        return yield* new SubjectTreeError({
          path,
          message: 'Review reference has an invalid kind',
        })
      }
      const id = yield* resolveRef({ root, path, ref, expected: ref.kind })
      set({ key: 'subject', value: `${ref.kind === 'agent' ? 'agent' : 'mission'}/${id}` })
    }
    if ('waitFor' in intent && isPinnedRef(intent.waitFor) === true) {
      if ('usesMission' in intent) {
        return yield* new SubjectTreeError({
          path,
          message: 'A step cannot specify both pinned waitFor and usesMission',
        })
      }
      const revision = yield* resolvePinnedRef({ root, path, ref: intent.waitFor })
      if (intent.waitFor.revision === '0'.repeat(64)) {
        return yield* new SubjectTreeError({
          path,
          message:
            'waitFor needs an explicit native revision; source receipt binding is schedule-only',
        })
      }
      set({ key: 'usesMission', value: { kind: 'revision', revision } })
      if (lowered !== undefined) delete lowered.waitFor
    }
    if ('schedule' in intent && typeof intent.schedule === 'object' && intent.schedule !== null) {
      const schedule = intent.schedule
      if ('work' in schedule && typeof schedule.work === 'object' && schedule.work !== null) {
        const work = schedule.work
        if (
          'mission' in work &&
          (isRef(work.mission) === true || isPinnedRef(work.mission) === true)
        ) {
          const mission = yield* isPinnedRef(work.mission) === true
            ? resolvePinnedRef({ root, path, ref: work.mission })
            : resolveRef({ root, path, ref: work.mission, expected: 'mission' })
          set({ key: 'schedule', value: { ...schedule, work: { ...work, mission } } })
        }
      }
    }
    return lowered ?? intent
  },
)

/** Where the loader writes program bundles, and the Bun that builds them. */
interface ProgramOptions {
  readonly bundleDir: string | undefined
  readonly bun: string
}

/**
 * The declaration of a program mission (docs/st3/program-engine.md §3): its native inputs, one
 * per field of `input`, and one `program` step pinning the module's bundle by sha256.
 */
const programDeclaration = Effect.fnUntraced(function* ({
  root,
  file,
  path,
  id,
  intent,
  programs,
}: {
  readonly root: string
  readonly file: string
  readonly path: string
  readonly id: SubjectId
  readonly intent: ProgramIntent<InputSchema, Schema.Codec<unknown>, undefined>
  readonly programs: ProgramOptions
}) {
  if (intent.id !== id) {
    return yield* new SubjectTreeError({
      path,
      message: `Declared id ${intent.id} must equal the subject directory ${id}`,
    })
  }
  const inputs = yield* Effect.try({
    try: () => programInputNames(intent.input),
    catch: (cause) => new SubjectTreeError({ path, message: 'Invalid program inputs', cause }),
  })
  if (programs.bundleDir === undefined) {
    return yield* new SubjectTreeError({
      path,
      message:
        'A program mission needs loadSubjectTree({ bundleDir }), the absolute directory its bundle is written to',
    })
  }
  const { bundle, sha256 } = yield* bundleProgram({
    module: file,
    cwd: root,
    bundleDir: programs.bundleDir,
    bun: programs.bun,
  }).pipe(
    Effect.mapError(
      (cause) => new SubjectTreeError({ path, message: 'Cannot bundle program', cause }),
    ),
  )
  const timeout = intent.timeout === undefined ? {} : { timeout: intent.timeout }
  const declaration = {
    id,
    state: 'ready',
    reportTo: intent.reportTo,
    goal: intent.goal,
    ...timeout,
    ...(intent.concurrentRuns === undefined ? {} : { concurrentRuns: intent.concurrentRuns }),
    ...(intent.authority === undefined ? {} : { authority: intent.authority }),
    inputs: inputs.map((name) => ({ name, kind: 'text' })),
    steps: [
      {
        id: 'program',
        ...timeout,
        program: {
          bundle,
          sha256,
          ...(intent.stalledAfter === undefined ? {} : { stalledAfter: intent.stalledAfter }),
        },
      },
    ],
  }
  return { declaration, bundle, sha256 }
})

/** Lowers a mission intent's references and decodes the native mission it declares. */
const decodeMission = Effect.fnUntraced(function* ({
  root,
  path,
  kind,
  intent,
}: {
  readonly root: string
  readonly path: string
  readonly kind: Exclude<SubjectKind, 'agent'>
  readonly intent: unknown
}) {
  const invalid = (cause: unknown) =>
    new SubjectTreeError({ path, message: `Invalid ${kind} declaration`, cause })
  const lowered = yield* lowerReferences({ root, path, intent })
  const wire = yield* Effect.try({
    // Trusted module authoring is validated by missionWire's strict schemas; the
    // boundary assertion preserves templates/handles until assembly, not a wire cast.
    try: () => missionWire(lowered as MissionInput<string>),
    catch: invalid,
  })
  return yield* Schema.decodeEffect(MissionSchema, { onExcessProperty: 'error' })(wire).pipe(
    Effect.mapError(invalid),
  )
})

const loadModule = Effect.fnUntraced(function* <C>({
  root,
  file,
  path,
  kind,
  id,
  context,
  programs,
}: {
  readonly root: string
  /** The module's realpath, which is what Node reports as its `import.meta.url`. */
  readonly file: string
  readonly path: string
  readonly kind: SubjectKind
  readonly id: SubjectId
  readonly context: C
  readonly programs: ProgramOptions
}) {
  const url = pathToFileURL(file).href
  const exports = yield* Effect.tryPromise({
    // oxlint-disable-next-line import/no-dynamic-require -- subject modules are trusted repository code discovered by path
    try: (): Promise<Readonly<Record<string, unknown>>> => import(url),
    catch: (cause) =>
      new SubjectTreeError({ path, message: 'Cannot import subject module', cause }),
  })
  if ('default' in exports === false) {
    return yield* new SubjectTreeError({ path, message: 'Subject module needs a default export' })
  }
  const value = exports.default
  const definition = isRef(value) === true ? value : undefined
  if (definition !== undefined && (definition.kind !== kind || definition.url !== url)) {
    return yield* new SubjectTreeError({
      path,
      message: `Module must export its own ${kind} definition, got ${definition.kind} from ${definition.url}`,
    })
  }
  const program = definition !== undefined && ProgramEntry in definition ? definition : undefined
  const source =
    definition === undefined ? value : 'intent' in definition ? definition.intent : undefined
  const intent =
    program === undefined
      ? yield* Effect.try({
          try: (): unknown => (typeof source === 'function' ? source(context) : source),
          catch: (cause) =>
            new SubjectTreeError({ path, message: 'Subject factory failed', cause }),
        })
      : (yield* programDeclaration({
          root,
          file,
          path,
          id,
          // A program definition's intent is the one `Fractal.mission(intent, program)` checked.
          intent: source as ProgramIntent<InputSchema, Schema.Codec<unknown>, undefined>,
          programs,
        })).declaration
  if (kind === 'objective') return yield* loadObjective({ root, path, id, intent, exports })
  // Definitions take their ID from the path; an explicit `id` in the intent must still agree.
  const identified =
    definition !== undefined && typeof intent === 'object' && intent !== null
      ? { id, ...intent }
      : intent
  if (
    typeof identified === 'object' &&
    identified !== null &&
    'id' in identified &&
    identified.id !== id
  ) {
    return yield* new SubjectTreeError({
      path,
      message: `Declared id ${String(identified.id)} must equal the subject directory ${id}`,
    })
  }
  if (kind === 'agent') {
    const lowered = yield* lowerReferences({ root, path, intent: identified })
    const agent = yield* Schema.decodeUnknownEffect(AgentSchema, { onExcessProperty: 'error' })(
      lowered,
    ).pipe(
      Effect.mapError(
        (cause) => new SubjectTreeError({ path, message: `Invalid ${kind} declaration`, cause }),
      ),
    )
    const subject: Subject = {
      kind,
      id,
      path,
      declaration: { _tag: 'Module', intent: agent, exports },
    }
    return subject
  }
  const mission = yield* decodeMission({ root, path, kind, intent: identified })
  if ((mission.schedule !== undefined) !== (kind === 'schedule')) {
    return yield* new SubjectTreeError({ path, message: scheduleMismatch[kind] })
  }
  const subject: Subject = {
    kind,
    id,
    path,
    declaration: { _tag: 'Module', intent: mission, exports },
  }
  return subject
})

/**
 * Bundles one program mission (`Fractal.mission(intent, program)` from `fractal/effect`) and
 * renders its native declaration exactly as `loadSubjectTree` loads that subject, for a loader
 * that walks its own tree. `file` is a `mission.ts` below the tree `root` and `definition` its
 * default export; the subject ID is the file's directory below `root`. The bundle is built from
 * `root` and written to `bundleDir` as `<sha256>.mjs`.
 */
export const programMission = Effect.fn('fractal.programMission')(function* ({
  root,
  file,
  definition,
  bundleDir,
  bun = 'bun',
}: {
  readonly root: string
  readonly file: string
  readonly definition: unknown
  readonly bundleDir: string
  readonly bun?: string
}) {
  if (isAbsolute(bundleDir) === false) {
    return yield* new SubjectTreeError({
      path: bundleDir,
      message: 'bundleDir must be an absolute directory',
    })
  }
  const fs = yield* FileSystem.FileSystem
  const real = (target: string, message: string) =>
    fs
      .realPath(target)
      .pipe(Effect.mapError((cause) => new SubjectTreeError({ path: target, message, cause })))
  const tree = yield* real(root, 'Cannot read subject tree')
  const module = yield* real(file, 'Cannot resolve declaration')
  const path = relative(tree, module).split(sep).join('/')
  if (basename(module) !== 'mission.ts') {
    return yield* new SubjectTreeError({ path, message: 'A program mission is a mission.ts' })
  }
  if (
    typeof definition !== 'object' ||
    definition === null ||
    ProgramEntry in definition === false ||
    'intent' in definition === false ||
    'url' in definition === false ||
    typeof definition.url !== 'string'
  ) {
    return yield* new SubjectTreeError({
      path,
      message: 'The default export is not a program mission',
    })
  }
  const defined = yield* real(fileURLToPath(definition.url), 'Cannot resolve declaration')
  if (defined !== module) {
    return yield* new SubjectTreeError({
      path,
      message: `Module must export its own mission definition, got one from ${definition.url}`,
    })
  }
  const id = yield* subjectId({ root: tree, file: module, path })
  const { declaration, bundle, sha256 } = yield* programDeclaration({
    root: tree,
    file: module,
    path,
    id,
    intent: definition.intent as ProgramIntent<InputSchema, Schema.Codec<unknown>, undefined>,
    programs: { bundleDir, bun },
  })
  const intent = yield* decodeMission({ root: tree, path, kind: 'mission', intent: declaration })
  return { kdl: emit([missionNode(intent as MissionInput<string>)]), bundle, sha256 }
})

/**
 * Checks an objective module's intent: its `name`, if given, is the path ID; `onBreach` agent
 * references resolve to `agent/<id>`; and it is an interval or count over claims named
 * `NAMESPACE/NAME`, the only objectives a fleet declares (the native telemetry ones ship with Fractal).
 */
const loadObjective = Effect.fnUntraced(function* ({
  root,
  path,
  id,
  intent,
  exports,
}: {
  readonly root: string
  readonly path: string
  readonly id: SubjectId
  readonly intent: unknown
  readonly exports: Readonly<Record<string, unknown>>
}) {
  if (typeof intent !== 'object' || intent === null) {
    return yield* new SubjectTreeError({ path, message: 'An objective module exports an objective' })
  }
  if ('name' in intent && intent.name !== undefined && intent.name !== id) {
    return yield* new SubjectTreeError({
      path,
      message: `Declared name ${String(intent.name)} must equal the subject directory ${id}`,
    })
  }
  const agent = (value: unknown) =>
    isRef(value) === true
      ? resolveRef({ root, path, ref: value, expected: 'agent' }).pipe(Effect.map((agentId) => `agent/${agentId}`))
      : Effect.succeed(value)
  const breach = 'onBreach' in intent ? intent.onBreach : undefined
  const onBreach =
    typeof breach === 'object' && breach !== null
      ? {
          ...breach,
          ...('owner' in breach ? { owner: yield* agent(breach.owner) } : {}),
          ...('backup' in breach && breach.backup !== undefined ? { backup: yield* agent(breach.backup) } : {}),
        }
      : breach
  const named: unknown = { ...intent, ...(onBreach === undefined ? {} : { onBreach }), name: id }
  const value = yield* Effect.try({
    // The intent is untrusted module data; `objective` checks every field, unknown ones included.
    try: () => objective(named as ObjectiveInput),
    catch: (cause) => new SubjectTreeError({ path, message: 'Invalid objective declaration', cause }),
  })
  if (declarable(value.indicator.kind) === false || id.includes('/') === false) {
    return yield* new SubjectTreeError({ path, message: declaredObjective(id) })
  }
  const subject: Subject = {
    kind: 'objective',
    id,
    path,
    declaration: { _tag: 'Module', intent: value, exports },
  }
  return subject
})

const loadKdl = Effect.fnUntraced(function* ({
  fs,
  file,
  path,
  kind,
  id,
}: {
  readonly fs: FileSystem.FileSystem
  readonly file: string
  readonly path: string
  readonly kind: SubjectKind
  readonly id: SubjectId
}) {
  const text = yield* fs
    .readFileString(file)
    .pipe(
      Effect.mapError(
        (cause) => new SubjectTreeError({ path, message: 'Cannot read KDL declaration', cause }),
      ),
    )
  const nodes = yield* Effect.try({
    try: () => parse(text).nodes,
    catch: (cause) => new SubjectTreeError({ path, message: 'Invalid KDL declaration', cause }),
  })
  const name = family(kind)
  const [version, declaration] = nodes
  if (
    nodes.length !== 2 ||
    version?.getName() !== 'version' ||
    version.getArgument(0) !== 2 ||
    declaration?.getName() !== name ||
    declaration.getArgument(0) !== id
  ) {
    return yield* new SubjectTreeError({
      path,
      message: `KDL declaration must be \`version 2\` followed by exactly one \`${name} "${id}"\` node`,
    })
  }
  const scheduled = declaration.children?.findNodeByName('schedule') !== undefined
  if ((kind === 'mission' || kind === 'schedule') && scheduled !== (kind === 'schedule')) {
    return yield* new SubjectTreeError({ path, message: scheduleMismatch[kind] })
  }
  if (
    kind === 'objective' &&
    (declarable(declaration.children?.findNodeByName('indicator')?.getArgument(0)) === false ||
      id.includes('/') === false ||
      validName(id) === false)
  ) {
    return yield* new SubjectTreeError({ path, message: declaredObjective(id) })
  }
  const subject: Subject = { kind, id, path, declaration: { _tag: 'Kdl', text } }
  return subject
})

/**
 * Loads `<root>/<id>/{agent,mission,schedule,objective}.{ts,kdl}` declarations in path order.
 *
 * The ID is the POSIX directory path of the file's realpath below `root`'s realpath, so symlinked
 * aliases load once and links leaving the tree are rejected. Seats use the `agent/<id>` namespace; missions
 * and schedules share `mission/<id>`, so one directory declares at most one of them; objectives use
 * `objective/<id>`. KDL files must hold `version 2` and one node whose kind and ID match the path. A
 * TS module's default export is a `define*` definition for its own path and kind, a plain intent, or
 * a factory; factories receive `context`. Typed references are lowered, then intents are decoded
 * with `AgentSchema` or `MissionSchema`, or checked by `Fractal.objective`. A declared objective is an
 * interval or count over claims. Subject modules are trusted code and are imported.
 *
 * A program mission (`Fractal.mission(intent, program)` from `fractal/effect`) is bundled with
 * `bun build --target=bun` from the tree root into `bundleDir`, and loads as the mission that runs
 * it: one text input per field of `intent.input` and one `program` step pinning the bundle's sha256.
 */
export const loadSubjectTree = Effect.fn('fractal.loadSubjectTree')(function* <C>({
  root,
  context,
  bundleDir,
  bun = 'bun',
}: {
  readonly root: string
  readonly context: C
  /** Absolute directory program bundles are written to as `<sha256>.mjs`. */
  readonly bundleDir?: string
  /** The Bun that builds program bundles; default `bun` on PATH. */
  readonly bun?: string
}) {
  if (bundleDir !== undefined && isAbsolute(bundleDir) === false) {
    return yield* new SubjectTreeError({
      path: bundleDir,
      message: 'bundleDir must be an absolute directory',
    })
  }
  const fs = yield* FileSystem.FileSystem
  const tree = yield* fs.realPath(root).pipe(
    Effect.flatMap((real) =>
      fs
        .readDirectory(real, { recursive: true })
        .pipe(Effect.map((entries) => ({ real, entries }))),
    ),
    Effect.mapError(
      (cause) => new SubjectTreeError({ path: root, message: 'Cannot read subject tree', cause }),
    ),
  )
  const programs: ProgramOptions = { bundleDir, bun }
  const seen = new Set<string>()
  const loaded = new Set<string>()
  const subjects: Subject[] = []
  for (const entry of tree.entries.toSorted()) {
    const match = declarationFile.exec(basename(entry))
    if (match === null) continue
    const kind = match[1] as SubjectKind
    const discovered = entry.split(sep).join('/')
    // Directory symlinks are followed while listing; IDs and refs both use the canonical file.
    const file = yield* fs.realPath(join(tree.real, entry)).pipe(
      Effect.mapError(
        (cause) =>
          new SubjectTreeError({
            path: discovered,
            message: 'Cannot resolve declaration',
            cause,
          }),
      ),
    )
    if (loaded.has(file) === true) continue
    loaded.add(file)
    const id = yield* subjectId({ root: tree.real, file, path: discovered })
    const path = relative(tree.real, file).split(sep).join('/')
    const key = `${family(kind)}/${id}`
    if (seen.has(key) === true) {
      return yield* new SubjectTreeError({ path, message: `Duplicate subject ${key}` })
    }
    seen.add(key)
    subjects.push(
      match[2] === 'kdl'
        ? yield* loadKdl({ fs, file, path, kind, id })
        : yield* loadModule({ root: tree.real, file, path, kind, id, context, programs }),
    )
  }
  return subjects.toSorted((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))
})
