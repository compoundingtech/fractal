import { ClientError, type FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema'
import { Schema } from 'effect'
import type { AlertActionResult, AlertAnswered } from './alert-action-port.ts'

export type AlertResolution = { readonly _tag: 'Resolved'; readonly by: string } | { readonly _tag: 'Read' }

/** A bounded attention omission is not a proof. Read the original source locator instead. */
export const readAlertResolution = async (client: FractalClient, alert: AlertAnswered): Promise<AlertResolution | undefined> => {
  try {
    if (alert.attention_kind === 'unread-message') {
      const response = await client.messagesGet(alert.source_id)
      const message = response.value as { readonly kind?: unknown; readonly state?: unknown }
      return message.kind === 'message' && message.state === 'read' ? { _tag: 'Read' } : undefined
    }
    if (alert.attention_kind === undefined || !alert.episode) return undefined
    const response = await client.attentionResolutionsGet(alert.id, {
      source_id: alert.source_id, attention_kind: alert.attention_kind, episode: alert.episode,
    })
    const proof = Schema.decodeUnknownSync(Native.AttentionResolution)(response.value)
    return { _tag: 'Resolved', by: proof.resolved_by }
  } catch (error) {
    if (error instanceof ClientError && error.response.code === 'not-found') return undefined
    throw error
  }
}

/** Only a typed definitive source refusal can turn a local answer into a proven conflict. */
export const isResolutionRefusal = (error: unknown): boolean => error instanceof ClientError &&
  ['not-found', 'stale-fence', 'stale-launch-preview', 'already-resolved'].includes(error.response.code)

export const resolvedRefusal = async (client: FractalClient, alert: AlertAnswered): Promise<AlertActionResult | undefined> => {
  const proof = await readAlertResolution(client, alert).catch(() => undefined)
  return proof?._tag === 'Resolved'
    ? { _tag: 'Refused', detail: `Already answered by ${proof.by}.`, alreadyAnsweredBy: proof.by }
    : undefined
}
