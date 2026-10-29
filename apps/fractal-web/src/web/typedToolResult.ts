import { Schema } from 'effect'
import type { WorkLogCall } from '@fractal/ui/assistant-ui/work-log'
import { recordValue } from '../conversation/blocks.ts'
import { recordedWaitFor, type RecordedWait } from '../conversation/recordedWait.ts'

export interface RecordedMatch { readonly path?: string; readonly line?: number; readonly text?: string }
export interface RecordedSource { readonly title?: string; readonly url?: string; readonly snippet?: string }
export interface RecordedTask { readonly name?: string; readonly agent?: string; readonly state?: string; readonly ref?: string; readonly sessionId?: string; readonly requested: boolean; readonly report?: string }
interface CommonResult { readonly body?: string; readonly skipped: boolean }
export type TypedToolProjection =
  | (CommonResult & { readonly kind: 'search'; readonly query?: string; readonly path?: string; readonly matches: readonly RecordedMatch[]; readonly files: readonly string[] })
  | (CommonResult & { readonly kind: 'web'; readonly query?: string; readonly url?: string; readonly sources: readonly RecordedSource[] })
  | (CommonResult & { readonly kind: 'subagent'; readonly tasks: readonly RecordedTask[] })
  | (CommonResult & RecordedWait & { readonly kind: 'wait'; readonly running: boolean; readonly interrupted: boolean; readonly completed: boolean })
  | (CommonResult & { readonly kind: 'unknown' })

const RecordedJson = Schema.decodeUnknownOption(Schema.fromJsonString(Schema.Unknown))
const TextParts = Schema.Array(Schema.Union([Schema.Struct({ type: Schema.optional(Schema.String), text: Schema.String }), Schema.Unknown])).pipe(Schema.decodeUnknownOption)
const text = (value: unknown): string | undefined => typeof value === 'string' ? value : undefined
const strings = (value: unknown): readonly string[] => typeof value === 'string' ? [value] : Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : []
const records = (value: unknown): readonly Readonly<Record<string, unknown>>[] => Array.isArray(value) ? value.flatMap(entry => { const record = recordValue(entry); return record === undefined ? [] : [record] }) : []
/** Decode exact JSON objects and arrays. Malformed or non-JSON text remains prose. */
const decoded = (value: unknown): unknown => {
  if (typeof value !== 'string') return value
  const trimmed = value.trimStart()
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return value
  return RecordedJson(value).pipe(option => option._tag === 'Some' ? option.value : value)
}
const textParts = (value: unknown): string | undefined => TextParts(value).pipe(option => {
  if (option._tag !== 'Some') return undefined
  const parts = option.value.flatMap(entry => { const part = recordValue(entry); return part?.type === 'text' && typeof part.text === 'string' ? [part.text] : [] })
  return parts.length > 0 ? parts.join('\n\n') : undefined
})
/** Only text-bearing fields are prose. JSON envelopes never become the primary result. */
const bodyText = (value: unknown): string | undefined => {
  const decodedValue = decoded(value)
  if (typeof decodedValue === 'string') return decodedValue
  const directParts = textParts(decodedValue)
  if (directParts !== undefined) return directParts
  const record = recordValue(decodedValue)
  if (record === undefined) return undefined
  for (const key of ['text', 'body', 'output', 'report', 'summary']) if (record[key] !== undefined) { const prose = bodyText(record[key]); if (prose !== undefined) return prose }
  if (record.content !== undefined) return bodyText(record.content)
  if (record.result !== undefined) return bodyText(record.result)
  return undefined
}
const taskReport = (task: Readonly<Record<string, unknown>>): string | undefined => {
  for (const key of ['report', 'summary', 'text']) { const prose = bodyText(task[key]); if (prose !== undefined) return prose }
  const schema = recordValue(task.schema)
  return schema === undefined ? undefined : bodyText(schema.data)
}

/** Project native fields without parsing names, summaries or output prose into facts. */
export const projectTypedToolResult = (call: WorkLogCall): TypedToolProjection => {
  const decodedOutput = decoded(call.output)
  const output = recordValue(decodedOutput)
  const envelope = recordValue(output?.result) ?? (Array.isArray(decodedOutput) ? undefined : recordValue(decodedOutput))
  const result = recordValue(output?.result)
  const metadata = call.metadata ?? recordValue(result?.metadata) ?? recordValue(output?.metadata) ?? recordValue(envelope?.metadata)
  const view = call.view ?? recordValue(result?.view) ?? recordValue(output?.view) ?? recordValue(envelope?.view)
  const input = recordValue(decoded(call.input))
  const details = recordValue(metadata?.details) ?? recordValue(output?.details) ?? recordValue(result?.details) ?? recordValue(envelope?.details)
  const sources = [view, metadata, details, result, output, envelope, input]
  const field = (...keys: readonly string[]): unknown => {
    for (const source of sources) for (const key of keys) if (source?.[key] !== undefined) return source[key]
    return undefined
  }
  const body = bodyText(call.output) ?? bodyText(result) ?? bodyText(envelope) ?? bodyText(call.detail)
  const common = { ...(body === undefined ? {} : { body }), skipped: call.skipped === true }
  switch (call.kind) {
    case 'search': return { ...common, kind: 'search', query: text(field('query', 'pattern')), path: text(field('path', 'file_path', 'scope')), files: strings(field('files', 'paths')), matches: records(field('matches')).map(match => ({ path: text(match.path) ?? text(match.file), line: typeof match.line === 'number' && Number.isFinite(match.line) ? match.line : undefined, text: text(match.text) ?? text(match.content) })) }
    case 'web': return { ...common, kind: 'web', query: text(field('query', 'search_query')), url: text(field('url')), sources: records(field('sources', 'results')).map(source => ({ title: text(source.title), url: text(source.url), snippet: text(source.snippet) ?? text(source.description) })) }
    case 'subagent': {
      // Invocation tasks describe requests; prefer observed result/progress records.
      const observed = [view, metadata, details, result, output].flatMap(source => {
        for (const key of ['results', 'progress', 'agents', 'tasks']) {
          const entries = records(source?.[key])
          if (entries.length > 0) return entries
        }
        return []
      })
      const taskRecords = observed.length > 0 ? observed : records(input?.tasks)
      const tasks = taskRecords.length > 0 ? taskRecords : sources.filter((source): source is Readonly<Record<string, unknown>> => source !== undefined && ['name', 'agent', 'agent_id', 'agentRef', 'ref'].some(key => source[key] !== undefined))
      return { ...common, kind: 'subagent', tasks: tasks.map(task => ({ name: text(task.name) ?? text(task.id) ?? text(task.agent), ...(text(task.agent) === undefined ? {} : { agent: text(task.agent) }), state: text(task.state) ?? text(task.status), ref: text(task.ref) ?? text(task.agentRef) ?? text(task.agent_id) ?? text(task.id), sessionId: text(recordValue(task.conversation)?.session_id), requested: observed.length === 0, ...(taskReport(task) === undefined ? {} : { report: taskReport(task) }) })) }
    }
    case 'wait': return { ...common, kind: 'wait', ...recordedWaitFor({ input, view, metadata, arrived: result?.arrived ?? output?.arrived }), running: call.status === 'running' || field('running') === true, interrupted: call.status === 'interrupted' || field('interrupted') === true, completed: call.status === 'success' && !common.skipped }
    default: return { ...common, kind: 'unknown' }
  }
}

export const recordedToolEvidence = (call: WorkLogCall): string | undefined => {
  if ([call.input, call.output, call.view, call.metadata, call.detail].every(value => value === undefined)) return undefined
  try { return JSON.stringify({ input: call.input, output: call.output, view: call.view, metadata: call.metadata, detail: call.detail }, null, 2) } catch { return call.detail }
}

/** Native destinations only; a recorded unsafe URL remains visible but not executable. */
export const recordedWebHref = (url: string | undefined): string | undefined => {
  if (url === undefined) return undefined
  try { const parsed = new URL(url); return parsed.protocol === 'https:' || parsed.protocol === 'http:' ? url : undefined } catch { return undefined }
}
