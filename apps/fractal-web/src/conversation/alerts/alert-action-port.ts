import { ClientError, plainError, type FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema'
import { Option, Schema } from 'effect'
import { gatewayPromptAnswers } from './prompt-answer-port.ts'

/** One answer to an alert: sent, or refused in words a person reads. */
export type AlertActionResult =
  | { readonly _tag: 'Sent'; readonly controlRevision?: string | null; readonly undo?: () => Promise<AlertActionResult> }
  | { readonly _tag: 'Refused'; readonly detail: string }

/** The alert as the person read it: what an answer names and is fenced on. */
export type AlertAnswered = Pick<Native.Attention, 'id' | 'revision' | 'source_id' | 'episode'> &
  Partial<Pick<Native.Attention, 'attention_kind' | 'presentation_control'>>

/**
 * An answer to an alert, as the Fractal terminal UI and the phone send it:
 * `work.done` completes the waiting person step with a summary and, for a structured request,
 * the named answer; `review.*` decides a human gate, with the person's words as its reason;
 * `launch.*` approves the planner's current preview, sends it back with the person's words,
 * or cancels the launch.
 */
export type AlertAction =
  | {
      readonly type: 'work.done'
      readonly summary: string
      readonly answer?: { readonly id?: string; readonly text?: string }
    }
  | {
      readonly type: 'review.approve' | 'review.reject' | 'review.request-changes'
      readonly reason?: string
    }
  | { readonly type: 'launch.approve' | 'launch.cancel' }
  | { readonly type: 'launch.revise'; readonly feedback: string }
  | { readonly type: 'prompt.respond'; readonly target_id: string; readonly episode: string; readonly answer: string }
  | { readonly type: 'message.read' }
  | { readonly type: 'attention.dismiss' | 'attention.restore' }
  | { readonly type: 'attention.snooze'; readonly until: string | null }

export type AlertActionRequest = { readonly alert: AlertAnswered; readonly action: AlertAction; readonly key?: string }
export interface AlertActionPort {
  readonly act: (request: AlertActionRequest) => Promise<AlertActionResult>
  readonly readControl?: (alert: AlertAnswered) => Promise<Native.AttentionControl>
  readonly forget?: (key: string) => void
}

const refused = (detail: string): AlertActionResult => ({ _tag: 'Refused', detail })

const REVIEW = {
  'review.approve': (client: FractalClient) => client.reviewApprove.bind(client),
  'review.reject': (client: FractalClient) => client.reviewReject.bind(client),
  'review.request-changes': (client: FractalClient) => client.reviewRequestChanges.bind(client),
} as const

type Read = { readonly kind?: unknown; readonly id?: unknown; readonly revision?: unknown }
type AttentionRead = Read & {
  readonly state?: unknown
  readonly actions?: readonly unknown[]
  readonly source_id?: unknown
  readonly launch_id?: unknown
  readonly variant_id?: unknown
  readonly preview_token?: unknown
  readonly episode?: unknown
}
type VariantRead = Read & { readonly ordinal?: unknown; readonly preview_token?: unknown }

/** The launch an approval is about: named on the alert, else its planning session's own id. */
const launchOf = (item: AttentionRead): string =>
  typeof item.launch_id === 'string' ? item.launch_id : String(item.source_id).replace(/^planning-session\//, '')

/**
 * The variant the alert showed, by the id and preview token it names, or why there is none to
 * approve. The alert is read again at the same revision the person read, so these are what they
 * read: a newer variant, or the same one previewed again, is never approved in its place.
 */
const shownVariant = (items: readonly unknown[], item: AttentionRead): VariantRead | 'unnamed' | 'changed' => {
  // An open launch alert's episode is its preview's hash, which is the preview token.
  const token = typeof item.preview_token === 'string' ? item.preview_token : typeof item.episode === 'string' ? item.episode : undefined
  if (typeof item.variant_id !== 'string' || token === undefined) return 'unnamed'
  const variant = (items as readonly VariantRead[]).find((each) => each.kind === 'launch-variant' && each.id === item.variant_id)
  return variant !== undefined && variant.preview_token === token ? variant : 'changed'
}

/**
 * Reads the alert again, then submits once, fenced on the revision the person read. An alert Fractal
 * no longer holds open, or that no longer offers this answer, is not answered; neither is one
 * that changed since it was read. Explicit same-key retries replay the prepared request unchanged.
 */
export const gatewayAlertActions = (client: FractalClient): AlertActionPort => {
  const prompts = gatewayPromptAnswers(client)
  const attempts = new Map<string, () => Promise<AlertActionResult>>()
  const outcome = (response: { readonly value: unknown }): AlertActionResult =>
    (typeof response.value === 'object' && response.value !== null && 'status' in response.value && response.value.status === 'rejected')
      ? refused('Fractal did not take this answer.') : { _tag: 'Sent' }
  const act: AlertActionPort['act'] = async ({ alert, action, key }) => {
    const remember = (send: () => Promise<{ readonly value: unknown }>) => {
      if (key !== undefined) attempts.set(key, async () => outcome(await send()))
      return send()
    }
    try {
      if (action.type === 'prompt.respond') return await prompts.respond({
        alert: { ...alert, target_id: action.target_id, episode: action.episode }, answer: action.answer, key,
      })
      const retry = key === undefined ? undefined : attempts.get(key)
      if (retry !== undefined) return await retry()
      if (action.type === 'attention.dismiss' || action.type === 'attention.restore' || action.type === 'attention.snooze') {
        const control = alert.presentation_control
        if (control === undefined || alert.attention_kind === undefined) return refused('This alert lacks native control metadata. Refresh before trying again.')
        const observed = await client.capabilities()
        if (!observed.value.capabilities.some((capability) => capability.id === action.type && capability.state === 'granted')) {
          return refused('This device is not granted this native alert control.')
        }
        const id = key ?? `action/${crypto.randomUUID()}`
        const parameters = {
          target_id: alert.id, source_id: alert.source_id, attention_kind: alert.attention_kind,
          episode: alert.episode, source_revision: control.source_revision,
          expected_control_revision: Option.getOrNull(control.revision),
        }
        // Capture the complete wire request once; a retry never reads a newer control fence.
        const request = { id, idempotency_key: id, fence: { snapshot_id: observed.snapshot.id, subject_revisions: {} }, parameters }
        const send = action.type === 'attention.snooze'
          ? (() => { const prepared = { ...request, parameters: { ...parameters, until: action.until } }
            return () => client.attentionSnooze(prepared) })()
          : action.type === 'attention.dismiss' ? () => client.attentionDismiss(request) : () => client.attentionRestore(request)
        const deliver = async (): Promise<AlertActionResult> => {
          const response = await send()
          const result = outcome(response)
          return result._tag === 'Sent' && response.value.control_revision !== undefined
            ? { ...result, controlRevision: response.value.control_revision } : result
        }
        if (key !== undefined) attempts.set(key, deliver)
        return await deliver()
      }
      let current
      try {
        current = await client.attentionGet(alert.id)
      } catch (error) {
        if (error instanceof ClientError && error.response.code === 'not-found') {
          return refused('This was already answered or closed.')
        }
        throw error
      }
      const item = current.value as AttentionRead
      if (item.kind !== 'attention' || item.state !== 'open') return refused('This was already answered or closed.')
      // A launch is sent back to its planner while Fractal offers to approve it.
      const offers = action.type === 'launch.revise' ? 'launch.approve' : action.type
      if (!(item.actions ?? []).includes(offers)) return refused('Fractal no longer offers this answer here.')
      if (item.revision !== alert.revision) return refused('This changed since you read it. Read it again before answering.')
      const id = key ?? `action/${crypto.randomUUID()}`
      const seen = { [alert.id]: alert.revision }
      const response = await (async () => {
        switch (action.type) {
          case 'message.read':
            return remember(() => client.messageRead({ id, idempotency_key: id,
              fence: { snapshot_id: current.snapshot.id, subject_revisions: seen }, parameters: { target_id: alert.source_id } }))
          case 'work.done':
            return remember(() => client.workDone({
              id,
              idempotency_key: id,
              fence: { snapshot_id: current.snapshot.id, subject_revisions: seen },
              parameters: {
                target_id: alert.source_id,
                episode: alert.episode || alert.revision,
                summary: action.summary,
                ...(action.answer === undefined ? {} : { answer: action.answer }),
              },
            }))
          case 'review.approve':
          case 'review.reject':
          case 'review.request-changes':
            return remember(() => REVIEW[action.type](client)({
              id,
              idempotency_key: id,
              fence: { snapshot_id: current.snapshot.id, subject_revisions: seen },
              parameters: { target_id: alert.source_id, ...(action.reason ? { reason: action.reason } : {}) },
            }))
          case 'launch.approve':
          case 'launch.cancel':
          case 'launch.revise': {
            // A launch is fenced on the launch as Fractal holds it now, as the Fractal terminal UI fences it.
            const launchId = launchOf(item)
            const launch = await client.launchesGet(launchId)
            const read = launch.value as Read
            if (read.kind !== 'launch' || typeof read.revision !== 'string') return 'gone' as const
            const launchRevision = read.revision
            const fence = { snapshot_id: launch.snapshot.id, subject_revisions: { ...seen, [launchId]: read.revision } }
            if (action.type === 'launch.cancel') {
              return remember(() => client.launchCancel({ id, idempotency_key: id, fence, parameters: { target_id: launchId } }))
            }
            if (action.type === 'launch.revise') {
              return remember(() => client.launchRevise({
                id,
                idempotency_key: id,
                fence: { snapshot_id: launch.snapshot.id, subject_revisions: { [launchId]: launchRevision } },
                parameters: { launch_id: launchId, feedback: action.feedback },
              }))
            }
            const variants = await client.launchVariantsList(launchId, { limit: 50 })
            const variant = shownVariant((variants.value as { items?: readonly unknown[] }).items ?? [], item)
            if (variant === 'unnamed') return 'unnamed' as const
            if (variant === 'changed' || typeof variant.id !== 'string' || typeof variant.revision !== 'string') return 'changed' as const
            const variantId = variant.id
            const variantRevision = variant.revision
            return remember(() => client.launchApprove({
              id,
              idempotency_key: id,
              fence: {
                ...fence,
                subject_revisions: { ...fence.subject_revisions, [variantId]: variantRevision },
                preview_token: variant.preview_token as string,
              },
              parameters: { launch_id: launchId, variant_id: variant.id },
            } as Parameters<FractalClient['launchApprove']>[0]))
          }
        }
      })()
      if (response === 'gone') return refused('This launch is gone.')
      if (response === 'unnamed') return refused('This alert does not say which preview it shows, so it cannot be approved here. Review it in Fractal.')
      if (response === 'changed') return refused('The preview this alert shows has changed. Read it again before approving.')
      return outcome(response)
    } catch (error) {
      return refused(
        error instanceof ClientError
          ? `${plainError(error)}.`
          : 'The answer could not be confirmed; check the alert before answering again. It was not sent again.',
      )
    }
  }
  return {
    act, forget: (key) => { attempts.delete(key); prompts.forget?.(key) },
    readControl: async (alert) => {
      if (alert.presentation_control === undefined || alert.attention_kind === undefined)
        return Promise.reject(new TypeError('This alert lacks native control metadata.'))
      const response = await client.attentionControlsGet(alert.id, {
        source_id: alert.source_id, attention_kind: alert.attention_kind, episode: alert.episode,
        source_revision: alert.presentation_control.source_revision,
      })
      return Schema.decodeUnknownSync(Native.AttentionControl)(response.value)
    },
  }
}
