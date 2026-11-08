import { describe, expect, it } from 'vitest'
import { connectionView, paneBandShown } from './connection.ts'

const online = { _tag: 'Online' } as const
const offline = { _tag: 'Offline' } as const

describe('connectionView', () => {
  it('says nothing without a gateway observation', () => {
    expect(connectionView({ observation: undefined, network: offline, now: 0, canReconnect: true })).toEqual({ state: 'live', reconnect: false })
  })

  it('is live while connected or still connecting', () => {
    expect(connectionView({ observation: { status: { _tag: 'Live', since: 0 }, observedAt: 0 }, network: online, now: 0, canReconnect: true }).state).toBe('live')
    expect(connectionView({ observation: { status: { _tag: 'Connecting', attempt: 1, since: 0 }, observedAt: 0 }, network: online, now: 0, canReconnect: true }).state).toBe('live')
  })

  it('counts down to the next try while reconnecting, and offers Reconnect', () => {
    const view = connectionView({ observation: { status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 2, nextAt: 7000, issue: 'socket closed' } }, observedAt: 0 }, network: online, now: 3200, canReconnect: true })
    expect(view).toEqual({ state: 'reconnecting', text: 'Next try in 4s · attempt 2', reconnect: true })
  })

  it('offers Reconnect only when the source can', () => {
    const view = connectionView({ observation: { status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 2, nextAt: 7000, issue: 'socket closed' } }, observedAt: 0 }, network: online, now: 3200, canReconnect: false })
    expect(view.reconnect).toBe(false)
  })

  it('puts an offline network over a live observation', () => {
    expect(connectionView({ observation: { status: { _tag: 'Live', since: 0 }, observedAt: 0 }, network: offline, now: 0, canReconnect: true })).toMatchObject({ state: 'offline', reconnect: true })
  })

  it('names a failure by its safe reason and offers no Reconnect, which would repeat it', () => {
    const view = connectionView({ observation: { status: { _tag: 'Failed', cause: { _tag: 'Server', code: 'forbidden', message: 'denied' } }, observedAt: 0 }, network: online, now: 0, canReconnect: true })
    expect(view).toEqual({ state: 'failed', text: 'No access to connection · ask an administrator', reconnect: false })
  })
})

describe('paneBandShown', () => {
  it('reaches into the focused pane only when the connection is gone, not while it recovers', () => {
    expect((['live', 'reconnecting', 'stale', 'offline', 'failed'] as const).filter(paneBandShown)).toEqual(['offline', 'failed'])
  })
})
