import { FractalClient } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it, vi } from 'vitest'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { gatewayThreads } from './threadSource.ts'

it('sends frozen observed-remove actions through bound delivery and hydrates only a current around page', async () => {
  const base = fixture.capabilities
  const thread = 'thread/reminder', anchor = 'message/old-anchor'
  const writes: Array<{ id: string; type: string; parameters: unknown; fence: unknown }> = []
  let marks = [{ mark_id: 'opaque/signed/mark', anchor_message_id: anchor }]
  let anchorState = 'current'
  const urls: URL[] = []
  const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
    const url = new URL(String(input)); urls.push(url)
    if (url.pathname.endsWith('/capabilities')) return Response.json({ ...base, value: {
      ...base.value, transport: 'local-web', capabilities: [...base.value.capabilities,
        { id: 'threads-v1', version: 3, state: 'granted' },
        { id: 'thread-summary-feed', version: 1, state: 'granted' },
        { id: 'thread.mark-unread', version: 0, state: 'granted' },
        { id: 'thread.clear-unread', version: 0, state: 'granted' },
      ],
    } })
    if (url.pathname.endsWith('/actions')) {
      const body = JSON.parse(String(init?.body)); writes.push(body)
      if (body.type === 'thread.clear-unread') marks = []
      return Response.json({ ...base, value: { kind: 'action-result', action_id: body.id,
        operation_id: 'operation/transport-is-not-a-mark-id', status: 'completed', affected_ids: [thread], snapshot_id: base.snapshot.id } })
    }
    if (url.pathname.endsWith('/timeline')) {
      const around = url.searchParams.get('around')
      return Response.json({ ...base, value: { items: around && anchorState === 'current' ? [{
        id: anchor, kind: 'message', actor_id: 'agent/example', content: 'Anchored content',
        accepted_at_unix_ms: 1, thread_quote_id: null,
      }] : [], next_cursor: null, ...(around === null ? {} : { anchor: { message_id: around, state: anchorState }, previous_cursor: 'cursor/newer' }) } })
    }
    return Response.json({ ...base, value: { id: thread, agent_id: 'agent/example', title: 'Reminder thread',
      title_revision: 'revision/title', revision: 'revision/thread', state: 'closed', unread: true,
      unread_reminder: marks.length === 0 ? null : { anchor_message_id: anchor, marks },
    } })
  } })
  const registry = AtomRegistry.make(), source = gatewayThreads(client, registry)
  const port = source.unreadActions!
  const read = port.reminder(thread), timeline = source.timeline(thread)
  const releases = [registry.mount(read), registry.mount(timeline)]
  try {
    await vi.waitFor(() => expect(registry.get(read)._tag).toBe('Known'))
    expect(port.canMark).toBe(true)
    expect(await port.mark({ threadId: thread, anchor_message_id: anchor, observed_marks: ['opaque/signed/mark'] })).toEqual({ _tag: 'Sent' })
    expect(writes[0]).toMatchObject({ type: 'thread.mark-unread', parameters: { target_id: thread, anchor_message_id: anchor, observed_marks: ['opaque/signed/mark'] },
      fence: { snapshot_id: base.snapshot.id, subject_revisions: { [thread]: 'revision/thread' } } })
    expect(await port.seek({ threadId: thread, anchor_message_id: anchor })).toEqual({ _tag: 'Found', messageId: anchor })
    await vi.waitFor(() => {
      const current = registry.get(timeline)
      expect(current._tag === 'Observed' ? current.value.items.map(item => item.id) : []).toEqual([anchor])
    })
    expect(urls.find(url => url.searchParams.has('around'))?.searchParams.get('around')).toBe(anchor)
    anchorState = 'pending'
    expect(await port.seek({ threadId: thread, anchor_message_id: 'message/not-arrived' })).toMatchObject({ _tag: 'Pending' })
    const stillCurrent = registry.get(timeline)
    expect(stillCurrent._tag === 'Observed' ? stillCurrent.value.items.map(item => item.id) : []).toEqual([anchor])
    expect(await port.clear({ threadId: thread, observed_marks: ['opaque/signed/mark'] })).toEqual({ _tag: 'Sent' })
    expect(writes[1]).toMatchObject({ type: 'thread.clear-unread', parameters: { target_id: thread, observed_marks: ['opaque/signed/mark'] } })
    expect(writes.map(write => write.type)).toEqual(['thread.mark-unread', 'thread.clear-unread'])
    await vi.waitFor(() => expect(registry.get(read)).toMatchObject({ _tag: 'Known', reminder: undefined }))
    const detail = registry.get(source.detail(thread))
    expect(detail._tag === 'Observed' && detail.value.summary.unread).toBe(true)
  } finally { releases.forEach(release => release()); registry.dispose() }
})
