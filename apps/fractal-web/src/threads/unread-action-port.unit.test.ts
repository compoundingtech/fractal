import { FractalClient } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { gatewayUnreadActions, type ThreadUnreadReminder } from './unread-action-port.ts'

afterEach(() => vi.restoreAllMocks())

const harness = () => {
  let requests = 0
  let credentials = 0
  const client = new FractalClient({
    baseUrl: 'https://gateway.invalid',
    credential: async () => {
      credentials += 1
      return 'unused'
    },
    fetchImpl: async () => {
      requests += 1
      throw new Error('Unread reminders must not call an unsupported gateway API.')
    },
  })
  return { client, requests: () => requests, credentials: () => credentials }
}

const unsupported = {
  _tag: 'Unavailable',
  detail: 'This connection does not support unread reminders.',
}

const reminder: ThreadUnreadReminder = {
  marks: [{ mark_id: 'signed-operation/opaque', anchor_message_id: 'message/anchor' }],
  anchor_message_id: 'message/anchor',
}

describe('gatewayUnreadActions', () => {
  it('keeps the per-thread read unavailable rather than claiming an empty reminder', () => {
    const gateway = harness()
    const port = gatewayUnreadActions(gateway.client)
    const registry = AtomRegistry.make()
    try {
      expect(port.canMark).toBe(false)
      expect(port.reminder('thread/one')).toBe(port.reminder('thread/one'))
      expect(registry.get(port.reminder('thread/one'))).toEqual(unsupported)
      expect(registry.get(port.reminder('thread/two'))).toEqual(unsupported)
      expect(gateway.requests()).toBe(0)
      expect(gateway.credentials()).toBe(0)
    } finally {
      registry.dispose()
    }
  })

  it('returns unavailable for mark, clear and seek without network, credentials or minted IDs', async () => {
    const randomUUID = vi.spyOn(crypto, 'randomUUID')
    const gateway = harness()
    const port = gatewayUnreadActions(gateway.client)
    const observed_marks = Object.freeze(reminder.marks.map(mark => mark.mark_id))
    const mark = Object.freeze({ threadId: 'thread/one', anchor_message_id: reminder.anchor_message_id, observed_marks })
    const clear = Object.freeze({ threadId: 'thread/one', observed_marks })
    const seek = Object.freeze({ threadId: 'thread/one', anchor_message_id: reminder.anchor_message_id })

    expect(await port.mark(mark)).toEqual(unsupported)
    expect(await port.clear(clear)).toEqual(unsupported)
    expect(await port.seek(seek)).toEqual(unsupported)
    expect(await port.mark({ ...mark, observed_marks: [] })).toEqual(unsupported)
    expect(await port.clear({ ...clear, observed_marks: [] })).toEqual(unsupported)
    expect(observed_marks).toEqual(['signed-operation/opaque'])
    expect(gateway.requests()).toBe(0)
    expect(gateway.credentials()).toBe(0)
    expect(randomUUID).not.toHaveBeenCalled()
  })
})
