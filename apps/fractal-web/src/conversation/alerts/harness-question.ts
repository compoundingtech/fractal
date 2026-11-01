import type { ConversationCall } from './AgentAlerts.tsx'

export interface HarnessOption {
  readonly position: number
  readonly label: string
  readonly description?: string
  readonly preview?: string
}

type QuestionContent = {
  /** Native identity, retained for the metadata join; never use a display label as an answer key. */
  readonly id: string
  readonly title: string
  readonly options: readonly HarnessOption[]
  readonly recommended?: number
}

export type HarnessQuestion = QuestionContent & ({ readonly _tag: 'SingleSelect' } | { readonly _tag: 'MultiSelect' })
export interface HarnessAsk {
  readonly _tag: 'OmpAsk'
  readonly callId: string
  readonly title: string
  readonly questions: readonly HarnessQuestion[]
}

const record = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value)

/** OMP strips both placeholders from optional fields before executing the native ask. */
const nativeOptional = (value: unknown): unknown => value == null || value === 'null' ? undefined : value

const projectQuestion = (value: unknown): HarnessQuestion | undefined => {
  if (!record(value) || typeof value.id !== 'string' || value.id.length === 0 || typeof value.question !== 'string'
    || !Array.isArray(value.options) || value.options.length > 64) return undefined
  const multi = nativeOptional(value.multi)
  const recommended = nativeOptional(value.recommended)
  if (multi !== undefined && typeof multi !== 'boolean') return undefined
  const options: HarnessOption[] = []
  for (const [position, option] of value.options.entries()) {
    if (!record(option) || typeof option.label !== 'string') return undefined
    const description = nativeOptional(option.description)
    const preview = nativeOptional(option.preview)
    if ((description !== undefined && typeof description !== 'string')
      || (preview !== undefined && typeof preview !== 'string')) return undefined
    options.push({ position, label: option.label,
      ...(typeof description === 'string' ? { description } : {}),
      ...(typeof preview === 'string' ? { preview } : {}) })
  }
  if (recommended !== undefined && (typeof recommended !== 'number' || !Number.isInteger(recommended)
    || recommended < 0 || recommended >= options.length)) return undefined
  return {
    _tag: multi === true ? 'MultiSelect' : 'SingleSelect', id: value.id, title: value.question, options,
    ...(typeof recommended === 'number' ? { recommended } : {}),
  }
}

/** Full native arguments only: the semantic timeline view clips previews and is not a safe join source. */
export const projectHarnessAsks = (items: readonly ConversationCall[]): readonly HarnessAsk[] => {
  const calls = new Map<string, ConversationCall[]>()
  for (const item of items) {
    if (item._tag !== 'ToolCall' || item.callId === undefined) continue
    const group = calls.get(item.callId)
    if (group === undefined) calls.set(item.callId, [item])
    else group.push(item)
  }
  const asks: HarnessAsk[] = []
  for (const [callId, group] of calls) {
    const call = group[0]!
    // An ambiguous invocation, result, interrupted run, or clipped preview cannot be answered here.
    if (group.length !== 1 || call.name !== 'ask' || call.callSeen !== true || call.result !== undefined
      || call.status !== 'running' || !record(call.input) || !Array.isArray(call.input.questions)
      || call.input.questions.length === 0 || call.input.questions.length > 16) continue
    const questions: HarnessQuestion[] = []
    const ids = new Set<string>()
    for (const raw of call.input.questions) {
      const question = projectQuestion(raw)
      if (question === undefined || ids.has(question.id)) break
      ids.add(question.id)
      questions.push(question)
    }
    if (questions.length !== call.input.questions.length) continue
    asks.push({ _tag: 'OmpAsk', callId, title: 'The agent has a question', questions })
  }
  return asks
}
