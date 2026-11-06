import { describe, expect, it, vi } from 'vitest'
import type { FractalClient } from '@fractal/client'
import { DateTime, Effect, Option, SubscriptionRef } from 'effect'
import type * as Native from '@fractal/client/schema'
import { gatewayAlertActions, type AlertAction } from './alert-action-port.ts'
import { optimisticAlertActions } from './optimistic-alert-actions.ts'

const alert = {
  kind: 'attention', id: 'attention/native', revision: 'delivery/r1', source_id: 'step-run/example/build',
  episode: 'claim/example', attention_kind: 'person-step', state: 'open', title: 'Review build', detail: '',
  priority: 'normal', requested_at: DateTime.makeUnsafe('2026-11-06T10:00:00Z'), person_id: 'person/example',
  conversation_ids: [], actions: ['work.done'], alert: true,
  presentation_control: { source_revision: 'semantic/r1', revision: Option.none(), dismissed: false, snoozed_until: Option.none() },
} as Native.Attention
const capabilities = vi.fn(async () => ({ snapshot: { id: 'snapshot/now' }, value: {
  capabilities: ['attention.dismiss', 'attention.restore', 'attention.snooze'].map((id) => ({ id, state: 'granted' })),
} }))

describe('native attention controls', () => {
  it.each<AlertAction>([{ type: 'attention.dismiss' }, { type: 'attention.restore' },
    { type: 'attention.snooze', until: '2026-11-06T11:00:00Z' }, { type: 'attention.snooze', until: null }])(
    'sends generated native parameters for $type without answering work', async (action) => {
      const send = vi.fn(async (_request: Parameters<FractalClient['attentionSnooze']>[0]) => ({ value: { status: 'completed', control_revision: 'control/r1' } }))
      const workDone = vi.fn()
      const port = gatewayAlertActions({ capabilities, attentionDismiss: send, attentionRestore: send, attentionSnooze: send, workDone } as unknown as FractalClient)
      expect(await port.act({ alert, action, key: 'action/native' })).toEqual({ _tag: 'Sent', controlRevision: 'control/r1' })
      expect(send.mock.calls[0]?.[0]).toEqual({ id: 'action/native', idempotency_key: 'action/native',
        fence: { snapshot_id: 'snapshot/now', subject_revisions: {} }, parameters: {
          target_id: alert.id, source_id: alert.source_id, attention_kind: alert.attention_kind, episode: alert.episode,
          source_revision: 'semantic/r1', expected_control_revision: null,
          ...(action.type === 'attention.snooze' ? { until: action.until } : {}),
        } })
      expect(workDone).not.toHaveBeenCalled()
    },
  )

  it('replays the identical prepared request after uncertain delivery', async () => {
    const send = vi.fn(async (_request: unknown) => {
      if (send.mock.calls.length === 1) throw new TypeError('network')
      return { value: { status: 'completed', control_revision: 'control/r1' } }
    })
    const port = gatewayAlertActions({ capabilities, attentionDismiss: send } as unknown as FractalClient)
    const request = { alert, action: { type: 'attention.dismiss' } as const, key: 'action/replay' }
    expect((await port.act(request))._tag).toBe('Refused')
    expect((await port.act({ ...request, alert: { ...alert, revision: 'delivery/new' } }))._tag).toBe('Sent')
    expect(send.mock.calls[1]?.[0]).toBe(send.mock.calls[0]?.[0])
  })

  it('refuses missing metadata and restores small refused actions with a visible note', async () => {
    const send = vi.fn()
    const port = gatewayAlertActions({ capabilities, attentionDismiss: send } as unknown as FractalClient)
    expect((await port.act({ alert: { ...alert, presentation_control: undefined }, action: { type: 'attention.dismiss' } }))._tag).toBe('Refused')
    expect(send).not.toHaveBeenCalled()
    const inbox = optimisticAlertActions({ act: async () => ({ _tag: 'Refused', detail: 'Not permitted' }) })
    await Effect.runPromise(inbox.accept([alert]))
    await inbox.port.act({ alert, action: { type: 'attention.snooze', until: '2026-11-06T11:00:00Z' } })
    await Effect.runPromise(inbox.accept([]))
    const state = Effect.runSync(SubscriptionRef.get(inbox.state))
    expect(state.pending.size).toBe(0)
    expect(inbox.project([alert], state.pending, state.notices)[0]?.optimisticNote).toBe('Not permitted')
  })

  it('retains ACK overlays on omission, missing control metadata and unrelated revisions', async () => {
    const inbox = optimisticAlertActions({ act: async () => ({ _tag: 'Sent', controlRevision: 'control/r1' }) })
    await Effect.runPromise(inbox.accept([alert]))
    await inbox.port.act({ alert, action: { type: 'attention.dismiss' } })
    for (const frame of [[], [{ ...alert, presentation_control: undefined }], [{ ...alert, revision: 'delivery/new' }],
      [{ ...alert, presentation_control: { ...alert.presentation_control!, revision: Option.some('control/r1') } }]]) {
      await Effect.runPromise(inbox.accept(frame))
      expect(Effect.runSync(SubscriptionRef.get(inbox.state)).pending.size).toBe(1)
    }
    await Effect.runPromise(inbox.accept([{ ...alert, presentation_control: {
      ...alert.presentation_control!, revision: Option.some('control/r1'), dismissed: true,
    } }]))
    expect(Effect.runSync(SubscriptionRef.get(inbox.state)).pending.size).toBe(0)
  })

  it('submits Undo through the shared layer with a fresh key and exact receipt CAS', async () => {
    const dismiss = vi.fn(async (_request: Parameters<FractalClient['attentionDismiss']>[0]) => ({ value: { status: 'completed', control_revision: 'control/r1' } }))
    const restore = vi.fn(async (_request: Parameters<FractalClient['attentionRestore']>[0]) => ({ value: { status: 'completed', control_revision: 'control/r2' } }))
    const inbox = optimisticAlertActions(gatewayAlertActions({ capabilities, attentionDismiss: dismiss, attentionRestore: restore } as unknown as FractalClient))
    await Effect.runPromise(inbox.accept([alert]))
    const result = await inbox.port.act({ alert, action: { type: 'attention.dismiss' } })
    expect(result._tag).toBe('Sent')
    if (result._tag !== 'Sent') return
    expect(result.undo).toBeTypeOf('function')
    await result.undo!()
    const request = restore.mock.calls[0]![0]
    expect(request.id).not.toBe(dismiss.mock.calls[0]![0].id)
    expect(request.parameters.expected_control_revision).toBe('control/r1')
    const state = Effect.runSync(SubscriptionRef.get(inbox.state))
    expect(state.pending.size).toBe(2)
    expect(inbox.project([alert], state.pending)).toHaveLength(1)
    await Effect.runPromise(inbox.accept([{ ...alert, presentation_control: {
      ...alert.presentation_control!, revision: Option.some('control/r2'), dismissed: false,
    } }]))
    expect(Effect.runSync(SubscriptionRef.get(inbox.state)).pending.size).toBe(0)
  })

  it('does not expose Undo for a synthetic in-flight ACK', async () => {
    const deferred = Promise.withResolvers<{ readonly _tag: 'Sent'; readonly controlRevision: string }>()
    const inbox = optimisticAlertActions({ act: () => deferred.promise })
    const sending = inbox.port.act({ alert, action: { type: 'attention.dismiss' } })
    await Promise.resolve()
    expect(await inbox.port.act({ alert, action: { type: 'attention.dismiss' } })).toEqual({ _tag: 'Sent' })
    deferred.resolve({ _tag: 'Sent', controlRevision: 'control/r1' })
    const result = await sending
    expect(result._tag === 'Sent' && result.undo).toBeTypeOf('function')
  })
})
