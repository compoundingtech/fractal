import type { ToolCallItem } from './model.ts'
import { recordValue } from './blocks.ts'
import { skippedTool, toolKindFor } from './toolClassification.ts'
import { recordedStSendFor } from './recordedStSend.ts'

export type SemanticTool =
  | { readonly type: 'skipped'; readonly reason: 'interrupted-before-execution' }
  | { readonly type: 'message'; readonly recipients: readonly string[]; readonly broadcast: boolean; readonly body: string; readonly subject?: string; readonly receipts: readonly { readonly to: string; readonly outcome: string }[] }
  | { readonly type: 'wait'; readonly awaited: readonly string[]; readonly arrived: readonly string[]; readonly interrupted: boolean; readonly running: boolean; readonly waited?: number }
const strings = (value: unknown): readonly string[] => typeof value === 'string' ? [value] : Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []
const jobNames = (value: unknown): readonly string[] => Array.isArray(value) ? value.flatMap(value => {
  if (typeof value === 'string') return [value]
  const job = recordValue(value)
  const id = job?.jobId ?? job?.id ?? job?.name
  return typeof id === 'string' ? [id] : []
}) : []

/** Shared recorded-message model for the transcript card and its inspector. */
export const outgoingMessageFor = ({ input: rawInput, view, metadata }: { readonly input?: unknown; readonly view?: Readonly<Record<string, unknown>>; readonly metadata?: Readonly<Record<string, unknown>> }): Extract<SemanticTool, { type: 'message' }> => {
  const input = recordValue(rawInput), message = recordValue(metadata?.message)
  const send = recordedStSendFor({ input: rawInput, view })
  const path = typeof view?.path === 'string' ? view.path : typeof input?.path === 'string' ? input.path : undefined
  const recipients = strings(message?.to ?? view?.to ?? input?.to ?? input?.recipient)
  const to = recipients.length > 0 ? recipients : path?.startsWith('agent://') ? [path.slice('agent://'.length)] : send !== undefined ? [send.to] : []
  const body = message?.body ?? view?.body ?? input?.content ?? input?.message ?? input?.body ?? send?.body
  const subject = message?.subject ?? view?.subject ?? input?.subject ?? send?.subject
  const receipts = Array.isArray(message?.receipts) ? message.receipts.flatMap(value => {
    const receipt = recordValue(value)
    return typeof receipt?.to === 'string' && typeof receipt.outcome === 'string' ? [{ to: receipt.to, outcome: receipt.outcome }] : []
  }) : []
  return { type: 'message', recipients: to, broadcast: path === 'agent://all' || to.length > 1, body: typeof body === 'string' ? body : '', ...(typeof subject === 'string' ? { subject } : {}), receipts }
}
export const semanticToolFor = (item: ToolCallItem): SemanticTool | undefined => {
  if (item.skipped || skippedTool(item)) return { type: 'skipped', reason: 'interrupted-before-execution' }
  const kind = toolKindFor(item), input = recordValue(item.input), metadata = item.result?.metadata, view = item.view ?? item.result?.view
  if (kind === 'message') return outgoingMessageFor({ input: item.input, metadata, view })
  if (kind === 'wait') {
    const resultView = item.result?.view
    const arrived = jobNames(resultView?.arrived ?? metadata?.arrived ?? metadata?.jobs)
    const from = strings(metadata?.from)
    const waited = metadata?.waited
    return { type: 'wait', awaited: jobNames(view?.awaited ?? input?.jobs ?? input?.handles ?? input?.ids), arrived: [...arrived, ...from], interrupted: item.status === 'interrupted' || metadata?.interrupted === true || resultView?.interrupted === true, running: item.status === 'running', ...(typeof waited === 'number' && Number.isFinite(waited) ? { waited } : {}) }
  }
  return undefined
}
