import { ClientError, type FractalClient } from '@fractal/client'
import { describe, expect, it, vi } from 'vitest'
import { gatewayAlertActions, type AlertAnswered } from './alert-action-port.ts'
import { readAlertResolution } from './alert-resolution.ts'
import { gatewayPromptAnswers } from './prompt-answer-port.ts'

const alert: AlertAnswered = { id: 'attention/original', revision: 'claim/episode', source_id: 'step-run/example',
  episode: 'claim/episode', attention_kind: 'person-step' }
const refused = (code: string) => new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0',
  request_id: 'request/test', code, message: 'The source is no longer waiting.', retryable: false, details: {} }, 409)
const proof = () => ({ value: { claim_id: 'claim/accepted', resolved_by: 'agent/delegate',
  person_id: 'person/receiver', resolved_at: '2026-11-01T12:00:00.000Z' } })
const answer = { type: 'work.done' as const, summary: 'My local answer' }

describe('original attention resolution transport', () => {
  it('drops a never-submitted local answer only with an exact accepted actor proof', async () => {
    const attentionResolutionsGet = vi.fn(async () => proof())
    const workDone = vi.fn()
    const port = gatewayAlertActions({ attentionGet: async () => { throw refused('not-found') },
      attentionResolutionsGet, workDone } as unknown as FractalClient)
    expect(await port.act({ alert, action: answer })).toEqual({ _tag: 'Refused',
      detail: 'Already answered by agent/delegate.', alreadyAnsweredBy: 'agent/delegate' })
    expect(attentionResolutionsGet).toHaveBeenCalledWith(alert.id, {
      source_id: alert.source_id, attention_kind: alert.attention_kind, episode: alert.episode,
    })
    expect(workDone).not.toHaveBeenCalled()
  })

  it('keeps missing or unavailable attribution a refusal without inventing an actor', async () => {
    for (const failure of [refused('not-found'), new Error('offline')]) {
      const port = gatewayAlertActions({ attentionGet: async () => { throw refused('not-found') },
        attentionResolutionsGet: async () => { throw failure } } as unknown as FractalClient)
      expect(await port.act({ alert, action: answer })).toEqual({ _tag: 'Refused', detail: 'This was already answered or closed.' })
    }
  })

  it('does not reclassify uncertain network delivery from a possibly own receipt', async () => {
    const attentionResolutionsGet = vi.fn(async () => proof())
    const port = gatewayAlertActions({ attentionGet: async () => ({ value: { kind: 'attention', state: 'open',
      revision: alert.revision, actions: ['work.done'] }, snapshot: { id: 'snapshot/test' } }),
      workDone: async () => { throw new Error('timeout') }, attentionResolutionsGet } as unknown as FractalClient)
    expect(await port.act({ alert, action: answer })).toMatchObject({ _tag: 'Refused' })
    expect(attentionResolutionsGet).not.toHaveBeenCalled()
  })

  it('attributes native prompt stale refusal without sending the local answer again', async () => {
    const promptRespond = vi.fn(async () => { throw refused('stale-fence') })
    const port = gatewayPromptAnswers({ capabilities: async () => ({ value: { capabilities: [{ id: 'prompt.respond', state: 'granted' }] },
      snapshot: { id: 'snapshot/test' } }), promptRespond, attentionResolutionsGet: async () => proof() } as unknown as FractalClient)
    expect(await port.respond({ alert: { ...alert, target_id: 'agent/seat' }, answer: 'allow' }))
      .toMatchObject({ _tag: 'Refused', alreadyAnsweredBy: 'agent/delegate' })
    expect(promptRespond).toHaveBeenCalledTimes(1)
  })

  it('requires an authoritative message read state, not disappearance or another state', async () => {
    for (const state of ['sent', 'delivered', 'closed', 'read']) {
      const client = { messagesGet: async () => ({ value: { kind: 'message', state } }) } as unknown as FractalClient
      expect(await readAlertResolution(client, { ...alert, attention_kind: 'unread-message' }))
        .toEqual(state === 'read' ? { _tag: 'Read' } : undefined)
    }
  })
})
