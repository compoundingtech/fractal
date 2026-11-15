/**
 * Objectives (docs/st3/objectives.md): an indicator that measures samples or counts events, the
 * targets that bound them, the window they are judged over and who a miss is delivered to. `objective` checks
 * the rules the native reader (`fractal_kernel::objective`) enforces, so a declaration fails where
 * it is written; `objectiveNode` lowers it to the `objective` KDL node.
 *
 * This module imports only `./kdl.ts`: `slo/objectives.kdl.genie.ts` renders the objectives
 * Fractal ships with it, and Genie bundles it without the package's dependencies.
 */
import { node, type Node, type Value } from './kdl.ts'
import type { AgentRef } from './subject-ref.ts'

/** A native duration: an integer with `ms`, `s`, `m`, `h` or `d` (`300ms`, `5m`, `7d`). */
export type Duration = `${number}${'ms' | 's' | 'm' | 'h' | 'd'}`

/** A claim that marks a moment of a sample; `at` names the field holding its time. */
export interface ClaimEvent {
  readonly kind: string
  /** A field with an RFC 3339 time or Unix milliseconds; without it, the claim's accepted time. */
  readonly at?: string
}

/** Work that starts and stops in claims, one sample per subject under `subjectPrefix`. */
export interface Interval {
  readonly subjectPrefix: string
  readonly start: ClaimEvent
  /** The members a sample must stop on: a list of strings in field `set` of the newest claim. */
  readonly expect?: ClaimEvent & { readonly set: string }
  /**
   * One member stopping, named by field `member` (required with `expect`). It may name the `start`
   * kind when both read their time from different `at` fields: one claim is then the whole sample.
   */
  readonly stop: ClaimEvent & { readonly member?: string }
  readonly cancel?: string
  readonly pause?: { readonly kind: string; readonly resume: string }
  /** Fields of the start or expect claim, or `member`, that split the rows. */
  readonly labels?: readonly string[]
  /** Leave out a sample whose start or expect claim has one of `values` in `field`. */
  readonly exclude?: readonly FieldValues[]
  /** Keep a sample only when its start claim has one of `values` in `field`; every `only` must hold. */
  readonly only?: readonly FieldValues[]
}

/** A claim field and the values an `exclude` leaves out or an `only` keeps. */
export interface FieldValues {
  readonly field: string
  readonly values: readonly string[]
}

/** Events in claims per hour, one per subject under `subjectPrefix` at its earliest `event` claim. */
export interface Count {
  readonly subjectPrefix: string
  /** `at` is required: a retry carries the same time, so it is deduplicated or before the window. */
  readonly event: ClaimEvent & { readonly at: string }
  /** Fields of the event claim that split the rows. */
  readonly labels?: readonly string[]
  /** Leave out an event whose claim has one of `values` in `field`. */
  readonly exclude?: readonly FieldValues[]
  /** Keep an event only when its claim has one of `values` in `field`; every `only` must hold. */
  readonly only?: readonly FieldValues[]
}

/** How a sample is measured; `kind` is the KDL indicator name. */
export type Indicator =
  | { readonly kind: 'latency'; readonly paths: readonly string[] }
  | { readonly kind: 'share'; readonly path: string }
  | { readonly kind: 'statement' }
  | { readonly kind: 'transaction' }
  | { readonly kind: 'writer-hold' }
  | { readonly kind: 'cpu'; readonly idleMinMinutes: number; readonly loadMaxCores: number }
  | { readonly kind: 'database' }
  | { readonly kind: 'divergence' }
  | ({ readonly kind: 'interval' } & Interval)
  | ({ readonly kind: 'count' } & Count)

type IndicatorOf<K extends Indicator['kind']> = Extract<Indicator, { readonly kind: K }>

/** One statistic and its bound; `scope: 'remote'` is a latency target for reads served remotely. */
export type Target = { readonly stat: string; readonly scope?: 'remote' } & (
  | { readonly within: Duration; readonly max?: never; readonly min?: never; readonly minPercent?: never }
  | { readonly max: number; readonly within?: never; readonly min?: never; readonly minPercent?: never }
  | { readonly min: number; readonly within?: never; readonly max?: never; readonly minPercent?: never }
  | { readonly minPercent: number; readonly within?: never; readonly max?: never; readonly min?: never }
)

/** The span a budget over claims is measured over, and the samples an interval needs to be met or missed. */
export interface Window {
  readonly span: Duration
  readonly minSamples?: number
}

/** An agent: a typed reference to an `agent.ts` module, or its `agent/ID` subject. */
export type AgentTarget = AgentRef | `agent/${string}`

/** A miss opens a fault for `owner`; `backup` is held, recorded and never the reviewer. */
export interface Fault {
  readonly kind: 'fault'
  readonly owner: AgentTarget
  readonly backup?: AgentTarget
  readonly condition: string
}

/** An objective's fields; `name` is absent in a subject tree, which derives it from the path. */
export interface ObjectiveInput {
  readonly name?: string
  readonly about: string
  readonly indicator: Indicator
  readonly window?: Window
  readonly targets: readonly Target[]
  readonly onBreach?: Fault
}

/** A checked objective: exactly the fields of its input, frozen. */
export type Objective = ObjectiveInput

const freeze = <T extends object>(value: T): T => Object.freeze(value)

/** The indicators an objective measures with. */
export const indicator = {
  /** Work that starts and stops in claims (a fleet declares this or `count`). */
  interval: (input: Interval): IndicatorOf<'interval'> => freeze({ kind: 'interval', ...input }),
  /** Events in claims per hour, one per subject (a fleet declares this or `interval`). */
  count: (input: Count): IndicatorOf<'count'> => freeze({ kind: 'count', ...input }),
  /** Daemon routes, stream first snapshots, publications or phone reports, by `METHOD /route`. */
  latency: ({ paths }: { readonly paths: readonly string[] }): IndicatorOf<'latency'> =>
    freeze({ kind: 'latency', paths }),
  /** A phone's live foreground time over its foreground time. */
  share: ({ path }: { readonly path: string }): IndicatorOf<'share'> => freeze({ kind: 'share', path }),
  /** One SQL statement. */
  statement: (): IndicatorOf<'statement'> => freeze({ kind: 'statement' }),
  /** Read and write transactions and writer holds. */
  transaction: (): IndicatorOf<'transaction'> => freeze({ kind: 'transaction' }),
  /** Each writer hold and long read, by holder. */
  writerHold: (): IndicatorOf<'writer-hold'> => freeze({ kind: 'writer-hold' }),
  /** The member's daemon and replication workers over closed UTC minutes. */
  cpu: ({
    idleMinMinutes,
    loadMaxCores,
  }: {
    readonly idleMinMinutes: number
    readonly loadMaxCores: number
  }): IndicatorOf<'cpu'> => freeze({ kind: 'cpu', idleMinMinutes, loadMaxCores }),
  /** The store's pages in use and their daily growth. */
  database: (): IndicatorOf<'database'> => freeze({ kind: 'database' }),
  /** Open projection divergence faults. */
  divergence: (): IndicatorOf<'divergence'> => freeze({ kind: 'divergence' }),
}

/** What a missed row delivers. */
export const response = {
  /** One `operational.failure` for `owner` per episode, with `condition`. */
  fault: (input: Omit<Fault, 'kind'>): Fault => freeze({ kind: 'fault', ...input }),
}

/** Lowercase words joined by `-`, as a name segment and a fault condition are. */
export const validWord = (word: string): boolean =>
  /^[a-z][a-z0-9-]*$/u.test(word) && word.endsWith('-') === false && word.includes('--') === false

/** One or more words joined by `/`: `cpu`, `ops/change-rollout`. */
export const validName = (name: string): boolean =>
  name.length <= 128 && name.split('/').every(validWord)

const units = { ms: 1, s: 1_000, m: 60_000, h: 3_600_000, d: 86_400_000 } as const

/** A native duration in milliseconds; bare integers are seconds, as the native reader reads them. */
const durationMs = ({ value, fail }: { readonly value: unknown; readonly fail: Fail }): number => {
  const match = typeof value === 'string' ? /^([0-9]+)(ms|s|m|h|d)?$/u.exec(value) : null
  if (match === null) return fail(`invalid duration \`${String(value)}\``)
  const ms = Number(match[1]) * units[(match[2] ?? 's') as keyof typeof units]
  if (Number.isSafeInteger(ms) === false) return fail('duration is too large')
  if (ms === 0) return fail('a required duration must be positive')
  return ms
}

type Fail = (message: string) => never
type Fields = Readonly<Record<string, unknown>>

const isRecord = (value: unknown): value is Fields =>
  typeof value === 'object' && value !== null && Array.isArray(value) === false

const isField = (value: unknown): value is string =>
  typeof value === 'string' && /^[A-Za-z0-9_-]+$/u.test(value)

const isAgentRef = (value: unknown): value is AgentRef =>
  isRecord(value) && value._tag === 'SubjectRef' && value.kind === 'agent'

/** Refuses fields other than `allowed`; an `undefined` field counts as absent. */
const only = ({
  value,
  allowed,
  what,
  fail,
}: {
  readonly value: Fields
  readonly allowed: readonly string[]
  readonly what: string
  readonly fail: Fail
}): void => {
  const unknown = Object.keys(value).find(
    (key) => value[key] !== undefined && allowed.includes(key) === false,
  )
  if (unknown !== undefined) fail(`${what} has an unknown field \`${unknown}\``)
}

const percentile = (stat: string): number | undefined =>
  /^p[1-9][0-9]?$/u.test(stat) ? Number(stat.slice(1)) : undefined

const event = ({
  value,
  name,
  extra,
  fail,
}: {
  readonly value: unknown
  readonly name: string
  readonly extra: readonly string[]
  readonly fail: Fail
}): Fields & ClaimEvent => {
  if (isRecord(value) === false) return fail(`\`${name}\` needs a claim \`kind\``)
  only({ value, allowed: ['kind', 'at', ...extra], what: `\`${name}\``, fail })
  if (typeof value.kind !== 'string' || value.kind === '') fail(`\`${name}\` names no claim kind`)
  if (value.at !== undefined && isField(value.at) === false) fail('`at` is not a field name')
  return value as Fields & ClaimEvent
}

const eventOf = (e: ClaimEvent): ClaimEvent => ({ kind: e.kind, ...(e.at === undefined ? {} : { at: e.at }) })

const checkPrefix = ({ value, fail }: { readonly value: Fields; readonly fail: Fail }): void => {
  const prefix = value.subjectPrefix
  if (
    typeof prefix !== 'string' ||
    prefix.endsWith('/') === false ||
    prefix.length < 2 ||
    prefix.startsWith('/')
  ) {
    fail(`subjectPrefix ${JSON.stringify(prefix)} must name subjects and end with \`/\``)
  }
}

const checkLabels = ({ value, fail }: { readonly value: Fields; readonly fail: Fail }): readonly unknown[] => {
  const labels = value.labels ?? []
  if (Array.isArray(labels) === false) return fail('`labels` is a list of fields')
  labels.forEach((label: unknown, index) => {
    if (isField(label) === false || labels.indexOf(label) !== index) {
      fail(`label ${JSON.stringify(label)} is not a field, or is repeated`)
    }
  })
  const fieldValues = (name: 'exclude' | 'only'): readonly Fields[] => {
    const list = value[name] ?? []
    if (Array.isArray(list) === false) return fail(`\`${name}\` is a list`)
    for (const entry of list as unknown[]) {
      if (
        isRecord(entry) === false ||
        isField(entry.field) === false ||
        Array.isArray(entry.values) === false ||
        entry.values.length === 0 ||
        entry.values.some((v: unknown) => typeof v !== 'string' || v === '')
      ) {
        fail(`\`${name}\` needs a field and at least one value`)
      }
      only({ value: entry as Fields, allowed: ['field', 'values'], what: `\`${name}\``, fail })
    }
    return list as Fields[]
  }
  fieldValues('exclude')
  fieldValues('only').forEach(({ field }, index, kept) => {
    if (kept.findIndex((other) => other.field === field) !== index) {
      fail(`\`only ${JSON.stringify(field)}\` is repeated: every \`only\` must hold, so each names its own field`)
    }
  })
  return labels
}

const freezeFieldValues = (list: readonly FieldValues[]): readonly FieldValues[] =>
  freeze(list.map(({ field, values }) => freeze({ field, values: freeze([...values]) })))

const labelsAndFilters = (value: Pick<Count, 'labels' | 'exclude' | 'only'>) => ({
  ...(value.labels === undefined ? {} : { labels: freeze([...value.labels]) }),
  ...(value.exclude === undefined ? {} : { exclude: freezeFieldValues(value.exclude) }),
  ...(value.only === undefined ? {} : { only: freezeFieldValues(value.only) }),
})

const parseCount = ({ value, fail }: { readonly value: Fields; readonly fail: Fail }): IndicatorOf<'count'> => {
  only({ value, allowed: ['kind', 'subjectPrefix', 'event', 'labels', 'exclude', 'only'], what: 'indicator `count`', fail })
  if (value.subjectPrefix === undefined) fail('a count needs `subjectPrefix`')
  checkPrefix({ value, fail })
  if (value.event === undefined) fail('a count needs `event`')
  const counted = event({ value: value.event, name: 'event', extra: [], fail })
  if (counted.at === undefined) fail('`event` needs `at`: a retry carries the same time, so it never counts twice')
  checkLabels({ value, fail })
  // Every field was checked above.
  const count = value as unknown as IndicatorOf<'count'>
  return freeze({ kind: 'count', subjectPrefix: count.subjectPrefix, event: freeze({ kind: count.event.kind, at: count.event.at }), ...labelsAndFilters(count) })
}

const parseInterval = ({ value, fail }: { readonly value: Fields; readonly fail: Fail }): IndicatorOf<'interval'> => {
  only({
    value,
    allowed: ['kind', 'subjectPrefix', 'start', 'expect', 'stop', 'cancel', 'pause', 'labels', 'exclude', 'only'],
    what: 'indicator `interval`',
    fail,
  })
  if (value.subjectPrefix === undefined) fail('an interval needs `subjectPrefix`')
  checkPrefix({ value, fail })
  if (value.start === undefined) fail('an interval needs `start`')
  const start = event({ value: value.start, name: 'start', extra: [], fail })
  const expect =
    value.expect === undefined
      ? undefined
      : event({ value: value.expect, name: 'expect', extra: ['set'], fail })
  if (expect !== undefined && isField(expect.set) === false) fail('`expect` needs `set` naming a field')
  if (value.stop === undefined) fail('an interval needs `stop`')
  const stop = event({ value: value.stop, name: 'stop', extra: ['member'], fail })
  if (stop.member !== undefined && isField(stop.member) === false) fail('`member` is not a field name')
  if (value.cancel !== undefined && typeof value.cancel !== 'string') fail('`cancel` needs one string')
  const pause = value.pause
  if (pause !== undefined) {
    if (isRecord(pause) === false || typeof pause.kind !== 'string') return fail('`pause` needs one string')
    only({ value: pause, allowed: ['kind', 'resume'], what: '`pause`', fail })
    if (typeof pause.resume !== 'string' || pause.resume === '') fail('`pause` needs `resume`')
  }
  const labels = checkLabels({ value, fail })
  if (expect !== undefined && stop.member === undefined) fail('with `expect`, `stop` needs `member`')
  if (labels.includes('member') && stop.member === undefined) {
    fail('`label "member"` needs `stop.member`')
  }
  if (stop.kind === start.kind) {
    // One claim is the whole sample: its start and its stop are two of its fields.
    if (expect !== undefined || stop.member !== undefined) fail('a `stop` naming the `start` kind takes no `expect` or `member`')
    if (start.at === undefined || stop.at === undefined || start.at === stop.at) {
      fail('a `stop` naming the `start` kind needs both to name different `at` fields')
    }
  }
  // Every field was checked above.
  const interval = value as unknown as IndicatorOf<'interval'>
  const kinds = [
    interval.start.kind,
    // The expect claim may also start the sample (merge to live), and one claim may be the whole
    // sample (start and stop); every other kind has one role.
    ...(interval.expect === undefined || interval.expect.kind === interval.start.kind ? [] : [interval.expect.kind]),
    ...(interval.stop.kind === interval.start.kind ? [] : [interval.stop.kind]),
    ...(interval.cancel === undefined ? [] : [interval.cancel]),
    ...(interval.pause === undefined ? [] : [interval.pause.kind, interval.pause.resume]),
  ]
  const repeated = kinds.find((kind, index) => kinds.indexOf(kind) !== index)
  if (repeated !== undefined) fail(`claim kind ${JSON.stringify(repeated)} has two roles`)
  return freeze({
    kind: 'interval',
    subjectPrefix: interval.subjectPrefix,
    start: freeze(eventOf(interval.start)),
    ...(interval.expect === undefined
      ? {}
      : { expect: freeze({ ...eventOf(interval.expect), set: interval.expect.set }) }),
    stop: freeze({
      ...eventOf(interval.stop),
      ...(interval.stop.member === undefined ? {} : { member: interval.stop.member }),
    }),
    ...(interval.cancel === undefined ? {} : { cancel: interval.cancel }),
    ...(interval.pause === undefined
      ? {}
      : { pause: freeze({ kind: interval.pause.kind, resume: interval.pause.resume }) }),
    ...labelsAndFilters(interval),
  })
}

const parseIndicator = ({ value, fail }: { readonly value: unknown; readonly fail: Fail }): Indicator => {
  if (isRecord(value) === false || typeof value.kind !== 'string') return fail('`indicator` needs a `kind`')
  const kind = value.kind
  const allow = (fields: readonly string[]) =>
    only({ value, allowed: ['kind', ...fields], what: `indicator \`${kind}\``, fail })
  switch (kind) {
    case 'latency': {
      allow(['paths'])
      const paths = value.paths
      if (Array.isArray(paths) === false || paths.length === 0) return fail('a latency indicator needs a `path`')
      if (paths.some((path: unknown) => typeof path !== 'string')) return fail('`path` needs one string')
      return freeze({ kind, paths: freeze([...(paths as string[])]) })
    }
    case 'share':
      allow(['path'])
      if (typeof value.path !== 'string') return fail('indicator `share` needs one `path`')
      return freeze({ kind, path: value.path })
    case 'statement':
    case 'transaction':
    case 'writer-hold':
    case 'database':
    case 'divergence':
      allow([])
      return freeze({ kind })
    case 'cpu': {
      allow(['idleMinMinutes', 'loadMaxCores'])
      const { idleMinMinutes, loadMaxCores } = value
      if (typeof idleMinMinutes !== 'number' || Number.isSafeInteger(idleMinMinutes) === false || idleMinMinutes <= 0) {
        return fail('idleMinMinutes is not a positive integer')
      }
      if (typeof loadMaxCores !== 'number' || Number.isFinite(loadMaxCores) === false) {
        return fail('loadMaxCores is not a number')
      }
      if (loadMaxCores <= 0) return fail('loadMaxCores is not positive')
      return freeze({ kind, idleMinMinutes, loadMaxCores })
    }
    case 'interval':
      return parseInterval({ value, fail })
    case 'count':
      return parseCount({ value, fail })
    default:
      return fail(`has an unknown indicator \`${kind}\``)
  }
}

/** A target with its bound in milliseconds or as a number. */
type CheckedTarget = { readonly target: Target; readonly ms?: number; readonly number?: number }

const parseTarget = ({ value, fail }: { readonly value: unknown; readonly fail: Fail }): CheckedTarget => {
  if (isRecord(value) === false || typeof value.stat !== 'string') return fail('a target needs a `stat`')
  only({ value, allowed: ['stat', 'within', 'max', 'min', 'minPercent', 'scope'], what: '`target`', fail })
  const stat = value.stat
  const bounds = (['within', 'max', 'min', 'minPercent'] as const).filter((name) => value[name] !== undefined)
  if (bounds.length !== 1) return fail(`target \`${stat}\` needs one bound: \`within\`, \`max\`, \`min\` or \`minPercent\``)
  if (value.scope !== undefined && value.scope !== 'remote') {
    return fail(`target \`${stat}\` has an unknown scope ${JSON.stringify(value.scope)}`)
  }
  const scope = value.scope === undefined ? {} : { scope: 'remote' as const }
  const bound = bounds[0]!
  const raw = value[bound]
  if (bound === 'within') {
    if (typeof raw !== 'string') return fail(`target \`${stat}\`'s \`within\` is not a duration`)
    const ms = durationMs({ value: raw, fail })
    return { target: freeze({ stat, within: raw as Duration, ...scope }), ms }
  }
  if (typeof raw !== 'number' || Number.isFinite(raw) === false) {
    return fail(`target \`${stat}\`'s \`${bound}\` is not a number`)
  }
  if (bound === 'minPercent') {
    if ((raw > 0 && raw <= 100) === false) return fail(`target \`${stat}\`'s \`minPercent\` is not between 0 and 100`)
    return { target: freeze({ stat, minPercent: raw, ...scope }), number: raw }
  }
  if (bound === 'min') return { target: freeze({ stat, min: raw, ...scope }), number: raw }
  return { target: freeze({ stat, max: raw, ...scope }), number: raw }
}

const parseAgent = ({ value, field, fail }: { readonly value: unknown; readonly field: string; readonly fail: Fail }): AgentTarget => {
  if (isAgentRef(value)) return value
  if (typeof value === 'string' && value.startsWith('agent/') && value.length > 'agent/'.length) {
    return value as AgentTarget
  }
  return fail(`\`onBreach\`'s \`${field}\` is not an agent: "agent/..." or an agent.ts reference`)
}

const sameAgent = (a: AgentTarget, b: AgentTarget): boolean =>
  typeof a === 'string' || typeof b === 'string' ? a === b : a.url === b.url

const parseBreach = ({ value, fail }: { readonly value: unknown; readonly fail: Fail }): Fault => {
  if (isRecord(value) === false || value.kind !== 'fault') return fail('`onBreach` is `fault`, the only response')
  only({ value, allowed: ['kind', 'owner', 'backup', 'condition'], what: '`onBreach`', fail })
  const owner = parseAgent({ value: value.owner, field: 'owner', fail })
  const backup = value.backup === undefined ? undefined : parseAgent({ value: value.backup, field: 'backup', fail })
  if (backup !== undefined && sameAgent(owner, backup)) {
    return fail("`onBreach`'s `backup` is an agent other than the owner")
  }
  if (typeof value.condition !== 'string' || validWord(value.condition) === false) {
    return fail('`onBreach` needs `condition`, lowercase words joined by `-`')
  }
  return freeze({ kind: 'fault', owner, ...(backup === undefined ? {} : { backup }), condition: value.condition })
}

/** Each stat an indicator reports, and the bound it takes. */
const statBound = ({ indicator, checked }: { readonly indicator: Indicator; readonly checked: CheckedTarget }): boolean => {
  const { target, ms, number } = checked
  const within = ms !== undefined
  const max = ({ integer, zero }: { readonly integer: boolean; readonly zero: boolean }) =>
    'max' in target &&
    number !== undefined &&
    (number > 0 || (zero && number === 0)) &&
    (integer === false || Number.isInteger(number))
  switch (indicator.kind) {
    case 'latency':
      return target.stat === 'p99' && within
    case 'share':
      return target.stat === 'live-share' && 'minPercent' in target
    case 'statement':
      return (target.stat === 'p99' || target.stat === 'max') && within
    case 'transaction':
    case 'writer-hold':
      return target.stat === 'max' && within
    case 'cpu':
      return (target.stat === 'cores' || target.stat === 'idle-cores') && max({ integer: false, zero: false })
    case 'database':
      return (target.stat === 'live-gb' || target.stat === 'growth-mb-per-day') && max({ integer: false, zero: false })
    case 'divergence':
      return target.stat === 'open' && max({ integer: true, zero: true })
    case 'count':
      return target.stat === 'per-hour' && ('min' in target ? number !== undefined && number > 0 : max({ integer: false, zero: true }))
    case 'interval':
      return percentile(target.stat) !== undefined && within
  }
}

const requiredStats: Readonly<Record<Indicator['kind'], readonly string[]>> = {
  latency: ['p99'],
  share: ['live-share'],
  statement: ['p99', 'max'],
  transaction: ['max'],
  'writer-hold': ['max'],
  cpu: ['cores', 'idle-cores'],
  database: ['live-gb', 'growth-mb-per-day'],
  divergence: ['open'],
  interval: [],
  count: ['per-hour'],
}

const checkTargets = ({
  indicator,
  targets,
  hasWindow,
  hasBreach,
  fail,
}: {
  readonly indicator: Indicator
  readonly targets: readonly CheckedTarget[]
  readonly hasWindow: boolean
  readonly hasBreach: boolean
  readonly fail: Fail
}): void => {
  const local = (stat: string) => targets.find(({ target }) => target.stat === stat && target.scope === undefined)
  targets.forEach(({ target }, index) => {
    if (targets.findIndex((other) => other.target.stat === target.stat && other.target.scope === target.scope) !== index) {
      fail(`repeats target \`${target.stat}\``)
    }
    if (target.scope === 'remote' && indicator.kind !== 'latency') fail('only a latency target has a remote scope')
  })
  const wrong = targets.find((checked) => statBound({ indicator, checked }) === false)
  if (wrong !== undefined) fail(`a \`${indicator.kind}\` indicator has no target \`${wrong.target.stat}\` with that bound`)
  const missing = requiredStats[indicator.kind].find((stat) => local(stat) === undefined)
  if (missing !== undefined) fail(`a \`${indicator.kind}\` indicator needs target \`${missing}\``)
  if (targets.length === 0) fail('has no `target`')
  const remote = targets.find(({ target }) => target.stat === 'p99' && target.scope === 'remote')
  if (remote !== undefined && remote.ms! < (local('p99')?.ms ?? 0)) fail('its remote latency is below its own')
  if (indicator.kind === 'statement' && local('max')!.ms! < local('p99')!.ms!) fail('its max is below its p99')
  if (indicator.kind === 'cpu' && local('idle-cores')!.number! > local('cores')!.number!) {
    fail("its idle CPU target is above the member's")
  }
  if (indicator.kind === 'cpu' && indicator.idleMinMinutes > 60) {
    fail('its idle minimum is not between 1 and 60 minutes')
  }
  const claims = indicator.kind === 'interval' || indicator.kind === 'count'
  if (claims && hasWindow === false) fail(`a \`${indicator.kind}\` indicator needs a \`window\``)
  if (claims === false && hasWindow) {
    fail('only an interval or a count takes a `window`; native telemetry keeps its own')
  }
  if (claims === false && hasBreach) {
    fail('only an interval or a count takes `onBreach`; native objectives respond as they are built')
  }
}

/**
 * Checks an objective by the native reader's rules and returns it frozen. A missing, unknown,
 * repeated or nonsensical field throws a `TypeError`, never a default. `name` may be absent only
 * where the subject tree derives it from the module's path.
 */
export const objective = <const I extends ObjectiveInput>(input: I): Objective & Pick<I, 'name'> => {
  if (isRecord(input) === false) throw new TypeError('an objective is an object of fields')
  const name: unknown = input.name
  if (name !== undefined && (typeof name !== 'string' || validName(name) === false)) {
    throw new TypeError('an objective has one name: lowercase words joined by `-`, segments by `/`')
  }
  const fail: Fail = (message) => {
    throw new TypeError(name === undefined ? `objective: ${message}` : `objective \`${name}\`: ${message}`)
  }
  only({ value: input, allowed: ['name', 'about', 'indicator', 'window', 'targets', 'onBreach'], what: 'an objective', fail })
  if (input.about === undefined) fail('has no `about`')
  if (typeof input.about !== 'string') fail('`about` needs one string')
  if (input.about.trim() === '') fail('`about` is empty')
  if (input.indicator === undefined) fail('has no `indicator`')
  const indicator = parseIndicator({ value: input.indicator, fail })
  const window: unknown = input.window
  if (window !== undefined) {
    if (isRecord(window) === false) fail('`window` needs a `span`')
    only({ value: window as Fields, allowed: ['span', 'minSamples'], what: '`window`', fail })
    durationMs({ value: input.window!.span, fail })
    const minSamples: unknown = input.window!.minSamples
    if (minSamples !== undefined && (typeof minSamples !== 'number' || Number.isSafeInteger(minSamples) === false || minSamples <= 0)) {
      fail('minSamples is not a positive integer')
    }
  }
  if (Array.isArray(input.targets) === false) fail('has no `target`')
  const targets = input.targets.map((value) => parseTarget({ value, fail }))
  const onBreach = input.onBreach === undefined ? undefined : parseBreach({ value: input.onBreach, fail })
  checkTargets({ indicator, targets, hasWindow: window !== undefined, hasBreach: onBreach !== undefined, fail })
  if (indicator.kind === 'count' && input.window?.minSamples !== undefined) {
    fail("a count's window takes no `minSamples`: an empty window is a real 0")
  }
  return freeze({
    ...(name === undefined ? {} : { name: name as string }),
    about: input.about,
    indicator,
    ...(input.window === undefined
      ? {}
      : {
          window: freeze({
            span: input.window.span,
            ...(input.window.minSamples === undefined ? {} : { minSamples: input.window.minSamples }),
          }),
        }),
    targets: freeze(targets.map(({ target }) => target)),
    ...(onBreach === undefined ? {} : { onBreach }),
  })
}

const agentSubject = ({ agent, name }: { readonly agent: AgentTarget; readonly name: string }): string => {
  if (typeof agent === 'string') return agent
  throw new TypeError(
    `objective \`${name}\`: an agent reference lowers through loadSubjectTree, which resolves its path`,
  )
}

const intervalChildren = (interval: Interval): Node[] => {
  const at = (e: ClaimEvent): Record<string, Value> => (e.at === undefined ? {} : { at: e.at })
  return [
    node({ name: 'subject-prefix', args: [interval.subjectPrefix] }),
    node({ name: 'start', args: [interval.start.kind], props: at(interval.start) }),
    ...(interval.expect === undefined
      ? []
      : [node({ name: 'expect', args: [interval.expect.kind], props: { ...at(interval.expect), set: interval.expect.set } })]),
    node({
      name: 'stop',
      args: [interval.stop.kind],
      props: { ...at(interval.stop), ...(interval.stop.member === undefined ? {} : { member: interval.stop.member }) },
    }),
    ...(interval.cancel === undefined ? [] : [node({ name: 'cancel', args: [interval.cancel] })]),
    ...(interval.pause === undefined
      ? []
      : [node({ name: 'pause', args: [interval.pause.kind], props: { resume: interval.pause.resume } })]),
    ...(interval.labels ?? []).map((label) => node({ name: 'label', args: [label] })),
    ...(interval.exclude ?? []).map(({ field, values }) => node({ name: 'exclude', args: [field, ...values] })),
    ...(interval.only ?? []).map(({ field, values }) => node({ name: 'only', args: [field, ...values] })),
  ]
}

const indicatorChildren = (indicator: Indicator): Node[] => {
  switch (indicator.kind) {
    case 'latency':
      return indicator.paths.map((path) => node({ name: 'path', args: [path] }))
    case 'share':
      return [node({ name: 'path', args: [indicator.path] })]
    case 'cpu':
      return [
        node({ name: 'idle-min-minutes', args: [indicator.idleMinMinutes] }),
        node({ name: 'load-max-cores', args: [indicator.loadMaxCores] }),
      ]
    case 'interval':
      return intervalChildren(indicator)
    case 'count':
      return [
        node({ name: 'subject-prefix', args: [indicator.subjectPrefix] }),
        node({ name: 'event', args: [indicator.event.kind], props: { at: indicator.event.at } }),
        ...(indicator.labels ?? []).map((label) => node({ name: 'label', args: [label] })),
        ...(indicator.exclude ?? []).map(({ field, values }) => node({ name: 'exclude', args: [field, ...values] })),
        ...(indicator.only ?? []).map(({ field, values }) => node({ name: 'only', args: [field, ...values] })),
      ]
    case 'statement':
    case 'transaction':
    case 'writer-hold':
    case 'database':
    case 'divergence':
      return []
  }
}

const targetProps = (target: Target): Record<string, Value> => ({
  ...(target.within === undefined ? {} : { within: target.within }),
  ...(target.max === undefined ? {} : { max: target.max }),
  ...(target.min === undefined ? {} : { min: target.min }),
  ...(target.minPercent === undefined ? {} : { 'min-percent': target.minPercent }),
  ...(target.scope === undefined ? {} : { scope: target.scope }),
})

/** Checks a named objective and lowers it to its `objective` KDL node. */
export const objectiveNode = (input: ObjectiveInput & { readonly name: string }): Node => {
  const o = objective(input)
  const name = o.name
  if (name === undefined) throw new TypeError('objective: lowering needs a `name`')
  const kids = indicatorChildren(o.indicator)
  return node({
    name: 'objective',
    args: [name],
    children: [
      node({ name: 'about', args: [o.about] }),
      node({ name: 'indicator', args: [o.indicator.kind], ...(kids.length === 0 ? {} : { children: kids }) }),
      ...(o.window === undefined
        ? []
        : [
            node({
              name: 'window',
              args: [o.window.span],
              props: o.window.minSamples === undefined ? {} : { 'min-samples': o.window.minSamples },
            }),
          ]),
      ...o.targets.map((target) => node({ name: 'target', args: [target.stat], props: targetProps(target) })),
      ...(o.onBreach === undefined
        ? []
        : [
            node({
              name: 'on-breach',
              args: ['fault'],
              props: {
                owner: agentSubject({ agent: o.onBreach.owner, name }),
                ...(o.onBreach.backup === undefined ? {} : { backup: agentSubject({ agent: o.onBreach.backup, name }) }),
                condition: o.onBreach.condition,
              },
            }),
          ]),
    ],
  })
}
