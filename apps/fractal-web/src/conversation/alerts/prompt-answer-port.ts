import { ClientError, plainError, type FractalClient } from '@fractal/client'
import type * as Native from '@fractal/client/schema'
import { isResolutionRefusal, resolvedRefusal } from './alert-resolution.ts'

/** One answer to a native prompt: sent, or refused in words a person reads. */
export type PromptAnswerResult =
  | { readonly _tag: 'Sent' }
  | { readonly _tag: 'Refused'; readonly detail: string; readonly alreadyAnsweredBy?: string }

/** A native harness prompt's alert, as much of it as an answer names. */
export type PromptAlert = Pick<Native.Attention, 'id' | 'revision' | 'source_id'> & {
  readonly target_id: string
  readonly episode: string
}

/**
 * Answers native harness prompts (`prompt.respond`): `allow`/`deny`, `approve`, a question's
 * choices, or a screen prompt's own answer, as the alert offers them.
 */
export interface PromptAnswerPort {
  readonly respond: (request: {
    readonly alert: PromptAlert
    /** What is sent as the answer: the answer itself, or a question's choices as JSON. */
    readonly answer: string
    readonly key?: string
  }) => Promise<PromptAnswerResult>
  readonly forget?: (key: string) => void
}

/**
 * One fresh fence and submission per identity. Explicit same-key retries replay the
 * prepared request, including its original fence, rather than answering a newer prompt.
 */
export const gatewayPromptAnswers = (client: FractalClient): PromptAnswerPort => {
  const attempts = new Map<string, () => Promise<PromptAnswerResult>>()
  const respond: PromptAnswerPort['respond'] = async ({ alert, answer, key }) => {
    try {
      const retry = key === undefined ? undefined : attempts.get(key)
      if (retry !== undefined) return await retry()
      const observed = await client.capabilities()
      // Only the grant and the snapshot are read: the rest of the document is not this port's.
      const granted = ((observed.value as { capabilities?: ReadonlyArray<{ id?: unknown; state?: unknown }> }).capabilities ?? [])
        .some((capability) => capability.id === 'prompt.respond' && capability.state === 'granted')
      if (!granted) {
        return {
          _tag: 'Refused',
          detail: 'This device is not granted answering prompts. Answer it in the seat\'s terminal.',
        }
      }
      const snapshot = (observed.snapshot as { id?: unknown } | undefined)?.id
      if (typeof snapshot !== 'string') {
        return { _tag: 'Refused', detail: 'Fractal did not say what it last saw. Try again in a moment.' }
      }
      const id = key ?? `action/${crypto.randomUUID()}`
      const request = { id, idempotency_key: id,
        fence: { snapshot_id: snapshot, subject_revisions: { [alert.id]: alert.revision } },
        parameters: { target_id: alert.target_id, episode: alert.episode, answer } }
      const send = async (): Promise<PromptAnswerResult> => {
        const response = await client.promptRespond(request)
        const value = response.value
        const status = typeof value === 'object' && value !== null && 'status' in value ? value.status : undefined
        if (status === 'accepted' || status === 'completed') return { _tag: 'Sent' }
        return status === 'rejected'
          ? { _tag: 'Refused', detail: 'Fractal did not take this answer. Answer it in the seat\'s terminal.' }
          : { _tag: 'Refused', detail: 'Fractal did not say whether it took this answer; check the alert before answering again.' }
      }
      if (key !== undefined) attempts.set(key, send)
      return await send()
    } catch (error) {
      if (isResolutionRefusal(error)) {
        const conflict = await resolvedRefusal(client, { ...alert, attention_kind: 'harness-prompt' })
        if (conflict !== undefined) return conflict
      }
      return {
        _tag: 'Refused',
        detail:
          error instanceof ClientError
            ? `${plainError(error)}.`
            : 'The answer could not be confirmed; check the alert before answering again. It was not sent again.',
      }
    }
  }
  return { respond, forget: (key) => { attempts.delete(key) } }
}
