import type { TimelineBlock } from '@fractal/client/schema/read'
import { recordValue } from './blocks.ts'
import { inspectPreview } from './inspect.ts'
import type { ConversationItem } from './model.ts'

type RecordContext = { readonly id: string; readonly at: string; readonly harness: string }
type Fields = Readonly<Record<string, unknown>>
type Facts = { readonly title: string; readonly text: string; readonly inspect: unknown }

const shortString = (value: unknown): string | undefined =>
  typeof value === 'string' && value.trim() !== '' && value.length <= 80 && !/[\r\n]/.test(value) ? value.trim() : undefined
const finite = (value: unknown): number | undefined => typeof value === 'number' && Number.isFinite(value) ? value : undefined
/** Only named facts reach a row, so a future field stays out until someone decides it is safe to show. */
const pick = (record: Fields, keys: readonly string[]): Fields =>
  Object.fromEntries(keys.flatMap(key => record[key] === undefined ? [] : [[key, record[key]]]))

/** Token counts are renamed because the shared preview redacts any `*token*` key. */
const modelUsage = (record: Fields): Facts => {
  const usage = recordValue(record.usage) ?? {}
  const total = finite(usage.totalTokens)
  const model = shortString(record.model)
  return {
    title: 'Model usage',
    text: [model, total === undefined ? undefined : `${total.toLocaleString('en-US')} tokens`].filter(field => field !== undefined).join(' · ') || 'Usage reported',
    inspect: {
      ...pick(record, ['model', 'provider', 'purpose', 'stopReason', 'timestamp']),
      usage: { ...pick(usage, ['input', 'output', 'cacheRead', 'cacheWrite']), total, costUsd: finite(recordValue(usage.cost)?.total) },
    },
  }
}

const SESSION_INIT_FACTS = ['agent', 'modelRole', 'resolvedModel', 'readOnly', 'spawns', 'timestamp']
/** Allow-listed: the record carries the system prompt, the task and the tool list, and none may reach a row. */
const sessionInit = (record: Fields): Facts => ({
  title: 'Session started',
  text: [shortString(record.agent), shortString(record.resolvedModel)].filter(field => field !== undefined).join(' · ') || 'Session details recorded',
  inspect: { ...pick(record, SESSION_INIT_FACTS), notShown: Object.keys(record).filter(key => ['systemPrompt', 'task', 'tools'].includes(key)) },
})

const promptDigest = (record: Fields): Facts => {
  const digest = shortString(record.data)
  return { title: 'System prompt digest', text: digest?.slice(0, 16) ?? 'Digest recorded', inspect: { ...(digest === undefined ? {} : { digest }), ...pick(record, ['timestamp']) } }
}

/** A readable rule name: the last path segment without its extension; anything longer or multi-line is rule text, not a name. */
const ruleName = (rule: unknown): string | undefined => {
  const name = shortString(typeof rule === 'string' ? rule : recordValue(rule)?.name)
  return name?.split(/[\\/]/).filter(Boolean).at(-1)?.replace(/\.(?:md|mdc|txt)$/i, '') || undefined
}
/** The user chose to keep this one visible: one quiet row naming the rule, never its text. */
const ruleApplied = (record: Fields): string => {
  const rules = Array.isArray(record.injectedRules) ? record.injectedRules : []
  const names = [...new Set(rules.flatMap(rule => ruleName(rule) ?? []))]
  if (names.length === 0) return 'Rule applied'
  const shown = names.slice(0, 3).join(', ') + (names.length > 3 ? ` +${names.length - 3}` : '')
  return `${names.length === 1 ? 'Rule' : 'Rules'} applied: ${shown}`
}

const lookup = <T>(table: Readonly<Record<string, T>>, key: string): T | undefined =>
  Object.hasOwn(table, key) ? table[key] : undefined
const statusRecords: Readonly<Record<string, (record: Fields) => Facts>> = { model_usage: modelUsage, session_init: sessionInit }
const customRecords: Readonly<Record<string, (record: Fields) => Facts>> = { 'system-prompt-digest': promptDigest }

/**
 * The harness's own bookkeeping, recognized only by the typed shapes Fractal serves: an `omp_event`
 * view on a `status` block, or the customType of an `unknown` custom block. A visible custom message
 * sharing a name keeps its shape, and any other record keeps its F2 marker.
 *
 * Items carry only the bounded, redacted preview built here, like F2 markers: the native record
 * never crosses into reading-flow items. Hidden kinds come back under the system-events view.
 */
export const systemRecordItem = (block: TimelineBlock, ctx: RecordContext): ConversationItem | undefined => {
  const raw = recordValue(recordValue(block.payload)?.raw)
  const view = block.kind === 'status' && block.view?.type === 'omp_event' ? block.view : undefined
  // A view's details become marked text when large; the raw payload stays complete.
  const record = raw ?? recordValue(view?.details) ?? {}
  if (view?.kind === 'ttsr_injection') return { _tag: 'Notice', id: ctx.id, at: ctx.at, kind: 'event', text: ruleApplied(record) }
  const custom = block.kind === 'unknown' && block.source_type === 'custom' && typeof raw?.customType === 'string' ? raw.customType : undefined
  const kind = view?.kind ?? custom
  const describe = view !== undefined ? lookup(statusRecords, view.kind) : custom !== undefined ? lookup(customRecords, custom) : undefined
  if (kind === undefined || describe === undefined) return undefined
  const facts = describe(record)
  return {
    _tag: 'Event', id: ctx.id, at: ctx.at, kind: 'system-record', title: facts.title, text: facts.text,
    sender: { kind: 'harness', label: ctx.harness }, data: {},
    notice: { _tag: 'SystemRecord', record: kind, payload: inspectPreview(facts.inspect).text, raw: '' },
  }
}

/**
 * Whether an entry's shared body is this record rather than prose beside it. Fractal writes a record's
 * bounded copy as `[omp <kind>]`, and text blocks carry no prose of their own to separate from it.
 */
export const bodyRepeatsRecord = (block: TimelineBlock, text: string): boolean => {
  const kind = block.view?.type === 'omp_event' ? block.view.kind : undefined
  const prompt = recordValue(recordValue(block.payload)?.raw)?.systemPrompt
  return (kind !== undefined && text.startsWith(`[omp ${kind}]`)) || (typeof prompt === 'string' && prompt !== '' && text.includes(prompt))
}

/** Hidden from the reading lane by default; `Show all system events` brings the row back with its details. */
export const isHiddenSystemRecord = (item: ConversationItem): boolean =>
  item._tag === 'Event' && item.notice?._tag === 'SystemRecord'
