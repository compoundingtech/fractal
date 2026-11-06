import { ClientError, type FractalClient } from '@fractal/client'
import type * as Native from '@fractal/client/schema'
import { DateTime, Effect, SubscriptionRef } from 'effect'
import { describe, expect, it, vi } from 'vitest'
import { gatewayAlertActions } from './alert-action-port.ts'
import { optimisticAlertActions } from './optimistic-alert-actions.ts'

const alert = {
  kind: 'attention', id: 'attention/original', revision: 'delivery/r1', source_id: 'step-run/example',
  episode: 'claim/original', attention_kind: 'person-step', state: 'open', title: 'Answer', detail: '',
  priority: 'normal', requested_at: DateTime.makeUnsafe('2026-11-06T10:00:00Z'), person_id: 'person/reader',
  conversation_ids: [], actions: ['work.done'], alert: true,
} as Native.Attention
const action = { type: 'work.done', summary: 'My answer' } as const
const actorProof = { value: { claim_id: 'claim/accepted', resolved_by: 'agent/delegate',
  person_id: 'person/reader', resolved_at: '2026-11-06T10:01:00.000Z' } }
const attentionGet = async () => ({ value: alert, snapshot: { id: 'snapshot/test' } })

describe('source answer reconciliation', () => {
  it('retains accepted answers through bounded omission and unrelated revision changes', async () => {
    const inbox = optimisticAlertActions({ act: async () => ({ _tag: 'Sent' }) })
    await Effect.runPromise(inbox.accept([alert]))
    await inbox.port.act({ alert, action })
    for (const frame of [[], [{ ...alert, revision: 'delivery/r2' }]]) {
      await Effect.runPromise(inbox.accept(frame))
      const state = Effect.runSync(SubscriptionRef.get(inbox.state))
      expect([...state.pending.values()][0]?.phase).toBe('accepted')
      expect(inbox.project([alert], state.pending)[0]?.optimisticAnswer).toMatchObject({ phase: 'accepted', action })
    }
  })

  it('settles from a real original-source actor point proof after ACK', async () => {
    const attentionResolutionsGet = vi.fn(async () => actorProof)
    const forget = vi.fn()
    const transport = gatewayAlertActions({ attentionGet, workDone: async () => ({ value: { status: 'completed' } }),
      attentionResolutionsGet } as unknown as FractalClient)
    const inbox = optimisticAlertActions({ ...transport, forget })
    await Effect.runPromise(inbox.accept([alert]))
    expect(await inbox.port.act({ alert, action })).toEqual({ _tag: 'Sent' })
    expect(attentionResolutionsGet).toHaveBeenCalledExactlyOnceWith(alert.id, {
      source_id: alert.source_id, attention_kind: alert.attention_kind, episode: alert.episode,
    })
    const state = Effect.runSync(SubscriptionRef.get(inbox.state))
    expect(state.pending.size).toBe(0)
    expect(state.notices).toEqual([])
    expect(forget).toHaveBeenCalledTimes(1)
  })

  it('keeps failed point reads accepted and refreshes proofs without resending', async () => {
    const act = vi.fn(async () => ({ _tag: 'Sent' } as const))
    const readResolution = vi.fn(async () => {
      if (readResolution.mock.calls.length === 1) throw new TypeError('offline')
      return { _tag: 'Resolved', by: 'agent/delegate' } as const
    })
    const inbox = optimisticAlertActions({ act, readResolution })
    await inbox.port.act({ alert, action })
    await Effect.runPromise(inbox.accept([]))
    expect([...Effect.runSync(SubscriptionRef.get(inbox.state)).pending.values()][0]?.phase).toBe('accepted')
    await Effect.runPromise(inbox.refreshProofs)
    expect(readResolution).toHaveBeenCalledTimes(2)
    expect(act).toHaveBeenCalledTimes(1)
    expect(Effect.runSync(SubscriptionRef.get(inbox.state)).pending.size).toBe(0)
    await Effect.runPromise(inbox.refreshProofs)
    expect(readResolution).toHaveBeenCalledTimes(2)
  })

  it('drops a refused answer and projects the actual actor notice from AlreadyAnswered', async () => {
    const workDone = vi.fn()
    const inbox = optimisticAlertActions(gatewayAlertActions({ attentionGet: async () => {
      throw new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0',
        request_id: 'request/test', code: 'not-found', message: 'Closed', retryable: false, details: {} }, 404)
    }, attentionResolutionsGet: async () => actorProof, workDone } as unknown as FractalClient))
    await Effect.runPromise(inbox.accept([alert]))
    expect(await inbox.port.act({ alert, action })).toMatchObject({ _tag: 'Refused', alreadyAnsweredBy: 'agent/delegate' })
    const state = Effect.runSync(SubscriptionRef.get(inbox.state))
    expect(state.pending.size).toBe(0)
    expect(state.notices).toHaveLength(1)
    expect(inbox.project([alert], state.pending, state.notices)[0]).toMatchObject({
      optimisticNote: 'Already answered by agent/delegate.', optimisticResolvedBy: 'agent/delegate',
    })
    expect(workDone).not.toHaveBeenCalled()
  })

  it('does not settle an in-flight answer from actor proof before its own ACK', async () => {
    const ack = Promise.withResolvers<{ readonly _tag: 'Sent' }>()
    const started = Promise.withResolvers<void>()
    const inbox = optimisticAlertActions({ act: () => { started.resolve(); return ack.promise } })
    await Effect.runPromise(inbox.accept([alert]))
    const sending = inbox.port.act({ alert, action })
    await started.promise
    await Effect.runPromise(inbox.acceptResolution(alert, { _tag: 'Resolved', by: 'agent/delegate' }))
    expect([...Effect.runSync(SubscriptionRef.get(inbox.state)).pending.values()][0]?.phase).toBe('sending')
    ack.resolve({ _tag: 'Sent' })
    await sending
    await Effect.runPromise(inbox.accept([]))
    expect(Effect.runSync(SubscriptionRef.get(inbox.state)).pending.size).toBe(0)
  })

  it('settles message.read only after the source message proves Read', async () => {
    let state = 'delivered'
    const messagesGet = vi.fn(async () => ({ value: { kind: 'message', state } }))
    const message = { ...alert, attention_kind: 'unread-message', source_id: 'message/report', actions: ['message.read'] } as Native.Attention
    const inbox = optimisticAlertActions(gatewayAlertActions({ attentionGet: async () => ({ value: message, snapshot: { id: 'snapshot/test' } }),
      messageRead: async () => ({ value: { status: 'completed' } }), messagesGet } as unknown as FractalClient))
    await inbox.port.act({ alert: message, action: { type: 'message.read' } })
    await Effect.runPromise(inbox.accept([]))
    expect([...Effect.runSync(SubscriptionRef.get(inbox.state)).pending.values()][0]?.phase).toBe('accepted')
    state = 'read'
    await Effect.runPromise(inbox.refreshProofs)
    expect(messagesGet).toHaveBeenCalledWith(message.source_id)
    expect(Effect.runSync(SubscriptionRef.get(inbox.state)).pending.size).toBe(0)
  })
})
