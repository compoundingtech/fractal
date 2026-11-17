import { FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import type { ThreadTimelinePage } from '@fractal/client/schema/read'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { Feed } from '../data/source.ts'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import type { ThreadObservation } from './threadSource.ts'
import { gatewayUnreadActions, type GatewayUnreadActionsOptions, type UnreadActionResult } from './unread-action-port.ts'

const registries: AtomRegistry.AtomRegistry[] = []
afterEach(() => {
  for (const registry of registries) registry.dispose()
  registries.length = 0
  vi.restoreAllMocks()
})

const threadId = 'thread/example'
const anchor = 'message/anchor'
const mark = Native.decodeUnknownSync(Native.ThreadUnreadMark)({ mark_id: 'signed-operation/mark', anchor_message_id: anchor })
const observation: ThreadObservation = {
  summary: Native.decodeUnknownSync(Native.ThreadSummary)({
    id: threadId, agent_id: 'agent/example', title: 'Example', title_revision: 'revision/title',
    revision: 'revision/thread', state: 'open', unread: false,
    unread_reminder: { marks: [mark], anchor_message_id: anchor },
  }),
  snapshotId: fixture.capabilities.snapshot.id,
}
const message = { kind: 'message', id: anchor, actor_id: 'person/example', accepted_at_unix_ms: 1, content: 'Real body', thread_quote_id: null } satisfies Native.ThreadTimelineEventEncoded
const page = (state: Native.ThreadTimelineAnchor['state'] = 'current', items: readonly Native.ThreadTimelineEventEncoded[] = state === 'current' ? [message] : []) => ({
  items, next_cursor: null, anchor: { message_id: anchor, state },
} satisfies Native.ThreadTimelinePageEncoded)

const harness = ({
  response = page(),
  sendResult = { _tag: 'Sent' },
}: { readonly response?: unknown; readonly sendResult?: UnreadActionResult } = {}) => {
  const registry = AtomRegistry.make()
  registries.push(registry)
  const access = Atom.make({ read: true, actor: 'person/example', actions: new Set(['thread.mark-unread', 'thread.clear-unread']) })
  const detail = Atom.make<Feed<ThreadObservation>>({ _tag: 'Observed', value: observation, freshness: 'live' })
  const sent: { thread: ThreadObservation; edit: Parameters<GatewayUnreadActionsOptions['send']>[1] }[] = []
  const hydrated: { threadId: string; page: ThreadTimelinePage }[] = []
  const requests: URL[] = []
  const client = new FractalClient({
    baseUrl: 'https://gateway.invalid',
    fetchImpl: async input => {
      requests.push(new URL(String(input)))
      if (requests.at(-1)?.pathname.endsWith('/capabilities')) return Response.json(fixture.capabilities)
      const value = await response
      return value instanceof Response ? value : Response.json({ ...fixture.capabilities, value })
    },
  })
  const port = gatewayUnreadActions({
    client, registry, access, detail: () => detail,
    send: async (thread, edit) => { sent.push({ thread, edit }); return sendResult },
    onFound: (id, found) => { hydrated.push({ threadId: id, page: found }) },
  })
  return { port, client, registry, access, detail, sent, hydrated, requests }
}

describe('gateway unread reminders', () => {
  it('serves canonical timeline fixtures through the actual client transport', async () => {
    const { client } = harness()
    const response = await client.threadTimeline(threadId, { around: anchor, limit: 40 })
    expect(Native.decodeUnknownSync(Native.ThreadTimelinePage)(response.value).anchor?.state).toBe('current')
  })
  it('reads only authoritative reminder marks, independently of natural unread', () => {
    const { port, registry, detail, requests } = harness()
    const reminder = port.reminder(threadId)
    expect(port.reminder(threadId)).toBe(reminder)
    expect(registry.get(reminder)).toEqual({ _tag: 'Known', reminder: { marks: [mark], anchor_message_id: anchor } })
    registry.set(detail, { _tag: 'Observed', value: { ...observation, summary: { ...observation.summary, unread: true, unread_reminder: Option.none() } }, freshness: 'live' })
    expect(registry.get(reminder)).toEqual({ _tag: 'Known', reminder: undefined })
    expect(requests).toEqual([])
  })

  it('delegates captured mark IDs and the live observation without minting IDs or doing network reads', async () => {
    const mint = vi.spyOn(crypto, 'randomUUID')
    const { port, sent, requests } = harness()
    await expect(port.mark({ threadId, anchor_message_id: anchor, observed_marks: [mark.mark_id] })).resolves.toEqual({ _tag: 'Sent' })
    await expect(port.clear({ threadId, observed_marks: [mark.mark_id] })).resolves.toEqual({ _tag: 'Sent' })
    expect(sent).toEqual([
      { thread: observation, edit: { kind: 'thread.mark-unread', anchor_message_id: anchor, observed_marks: [mark.mark_id] } },
      { thread: observation, edit: { kind: 'thread.clear-unread', observed_marks: [mark.mark_id] } },
    ])
    expect(requests).toEqual([])
    expect(mint).not.toHaveBeenCalled()
  })

  it('reacts to revoked grants and refuses edits without both grants', async () => {
    const { port, registry, access, sent } = harness()
    const reminder = port.reminder(threadId)
    expect(registry.get(reminder)._tag).toBe('Known')
    expect(port.canMark).toBe(true)
    registry.set(access, { read: true, actor: 'person/example', actions: new Set(['thread.mark-unread']) })
    expect(port.canMark).toBe(false)
    await expect(port.mark({ threadId, anchor_message_id: anchor, observed_marks: [] })).resolves.toMatchObject({ _tag: 'Unavailable' })
    registry.set(access, { read: false, actor: 'person/example', actions: new Set<string>() })
    expect(registry.get(reminder)._tag).toBe('Unavailable')
    await expect(port.clear({ threadId, observed_marks: [mark.mark_id] })).resolves.toMatchObject({ _tag: 'Unavailable' })
    expect(sent).toEqual([])
  })

  it('does not interpret a stale or unavailable read as an absent reminder or editable state', async () => {
    const { port, registry, detail, sent } = harness()
    const reminder = port.reminder(threadId)
    registry.set(detail, { _tag: 'Observed', value: observation, freshness: 'stale' })
    expect(registry.get(reminder)._tag).toBe('Unavailable')
    await expect(port.clear({ threadId, observed_marks: [mark.mark_id] })).resolves.toMatchObject({ _tag: 'Unavailable' })
    registry.set(detail, { _tag: 'Waiting' })
    expect(registry.get(reminder)._tag).toBe('Unavailable')
    await expect(port.mark({ threadId, anchor_message_id: anchor, observed_marks: [] })).resolves.toMatchObject({ _tag: 'Unavailable' })
    expect(sent).toEqual([])
  })

  it('keeps delivery refusals visible and does not optimistically replace reminder marks', async () => {
    const { port, registry } = harness({ sendResult: { _tag: 'Refused', detail: 'Fractal refused this edit.' } })
    await expect(port.clear({ threadId, observed_marks: [mark.mark_id] })).resolves.toEqual({ _tag: 'Refused', detail: 'Fractal refused this edit.' })
    expect(registry.get(port.reminder(threadId))).toEqual({ _tag: 'Known', reminder: { marks: [mark], anchor_message_id: anchor } })
  })

  it('seeks using the exact around API and hydrates only its real current message', async () => {
    const { port, requests, hydrated } = harness()
    await expect(port.seek({ threadId, anchor_message_id: anchor })).resolves.toEqual({ _tag: 'Found', messageId: anchor })
    expect(requests).toHaveLength(2)
    expect(requests[0]?.pathname).toBe('/v1/client/capabilities')
    expect(requests[1]?.pathname).toBe('/v1/client/threads/example/timeline')
    expect(requests[1]?.searchParams.get('around')).toBe(anchor)
    expect(requests[1]?.searchParams.has('cursor')).toBe(false)
    expect(hydrated).toHaveLength(1)
    expect(hydrated[0]?.threadId).toBe(threadId)
    expect(hydrated[0]?.page.items[0]).toMatchObject({ kind: 'message', id: anchor, content: 'Real body' })
  })

  it.each(['pending', 'unavailable'] as const)('never hydrates a %s anchor or activates a reminder from it', async state => {
    const { port, hydrated, registry } = harness({ response: page(state) })
    await expect(port.seek({ threadId, anchor_message_id: anchor })).resolves.toMatchObject({ _tag: state === 'pending' ? 'Pending' : 'Unavailable' })
    expect(hydrated).toEqual([])
    expect(registry.get(port.reminder(threadId))).toEqual({ _tag: 'Known', reminder: { marks: [mark], anchor_message_id: anchor } })
  })

  it('rejects missing bodies, mismatched anchors and malformed around reads without hydration', async () => {
    for (const response of [page('current', []), { ...page(), anchor: { message_id: 'message/other', state: 'current' } }, {}]) {
      const { port, hydrated } = harness({ response })
      await expect(port.seek({ threadId, anchor_message_id: anchor })).resolves.toMatchObject({ _tag: 'Unavailable' })
      expect(hydrated).toEqual([])
    }
  })

  it('renders unresolved observed-mark dependencies as unavailable, never inferred pending or reminder intent', async () => {
    const { port, registry, detail, hydrated } = harness({ response: Response.json({
      api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'seek/dependencies',
      code: 'thread-unavailable', message: 'Thread unavailable.', retryable: false,
    }, { status: 404 }) })
    registry.set(detail, { _tag: 'Observed', value: { ...observation, summary: { ...observation.summary, unread_reminder: Option.none() } }, freshness: 'live' })
    await expect(port.seek({ threadId, anchor_message_id: anchor })).resolves.toMatchObject({ _tag: 'Unavailable' })
    expect(hydrated).toEqual([])
    expect(registry.get(port.reminder(threadId))).toEqual({ _tag: 'Known', reminder: undefined })
  })

  it('does not hydrate an in-flight seek after read access is revoked', async () => {
    const response = Promise.withResolvers<unknown>()
    const { port, registry, access, hydrated } = harness({ response: response.promise })
    const seeking = port.seek({ threadId, anchor_message_id: anchor })
    registry.set(access, { read: false, actor: 'person/example', actions: new Set<string>() })
    response.resolve(page())
    await expect(seeking).resolves.toMatchObject({ _tag: 'Unavailable' })
    expect(hydrated).toEqual([])
  })

  it('does not let an older seek replace the latest requested timeline', async () => {
    const response = Promise.withResolvers<unknown>()
    const { port, hydrated } = harness({ response: response.promise })
    const first = port.seek({ threadId, anchor_message_id: anchor })
    const second = port.seek({ threadId, anchor_message_id: anchor })
    response.resolve(page())
    await expect(first).resolves.toMatchObject({ _tag: 'Unavailable' })
    await expect(second).resolves.toEqual({ _tag: 'Found', messageId: anchor })
    expect(hydrated).toHaveLength(1)
  })
})
