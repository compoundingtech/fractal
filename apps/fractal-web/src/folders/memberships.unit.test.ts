import { expect, it } from '@effect/vitest'
import { ClientError } from '@fractal/client'
import type { OrderedMembershipFrame, Snapshot } from '@fractal/client'
import { MembershipPaging, executeMembershipRead, membershipFrame, membershipHttpFailure, membershipMetadata, type MembershipEffect, type MembershipFrontier, type MembershipPage, type MembershipScope, type MembershipToken, type MembershipWirePage } from './memberships.ts'

const scope: MembershipScope = { owner: 'person/test', container: 'arrangement/test', endpoint: 'endpoint/test', host: 'host/test-1', projection: 'client-projection.v0' }
const frontier = (index = 1, count = 1): MembershipFrontier => ({ epoch: 'epoch/test', changed_index: index, live_count: count })
const snapshot: Snapshot = { id: 'snapshot/test', host_id: scope.host, store_index: 1, projection_version: 'client-projection.v0', created_at: '2026-11-01T00:00:00Z' }
const page = (index = 1, count = 1, ids = ['a'], next?: string): MembershipPage => ({ frontier: frontier(index, count), snapshot, members: ids.map(member => ({ id: member, member, container: scope.container, revision: 'claim/test', position: { bucket: null, key: 'a0' } })), has_more: next !== undefined, ...(next === undefined ? {} : { next }) })
const tokenOf = (effects: MembershipEffect[]): MembershipToken => {
  const effect = effects[0]
  if (effect?._tag !== 'SubscribeAndRead') throw new TypeError('Expected subscription before first read')
  return effect.token
}
const refresh = (state: MembershipPaging, generation = 1): MembershipToken => tokenOf(state.reduce({ _tag: 'Refresh', scope, generation }))
const ready = (): { state: MembershipPaging; token: MembershipToken } => {
  const state = new MembershipPaging(); const token = refresh(state)
  expect(state.reduce({ _tag: 'Page', token, page: page() })).toEqual([{ _tag: 'PersistBase' }])
  return { state, token }
}
it('publishes only complete partitions under unrelated snapshot advancement', () => {
  for (let count = 0; count < 12; count++) for (let width = 1; width <= 12; width++) {
    const state = new MembershipPaging(); const token = refresh(state)
    state.reduce({ _tag: 'Metadata', token, frontier: frontier(1, count) })
    for (let start = 0; start < Math.max(1, count); start += width) {
      const ids = Array.from({ length: Math.min(width, count - start) }, (_, i) => `member-${start + i}`)
      const next = start + width < count ? `cursor-${start}` : undefined
      const input = { ...page(1, count, ids, next), snapshot: { ...snapshot, id: `snapshot/${start}`, store_index: 100 + start } }
      const effects = state.reduce({ _tag: 'Page', token, page: input })
      if (next) { expect(state.complete).toBeUndefined(); expect(effects).toEqual([{ _tag: 'Read', token, cursor: next }]) }
      else expect(effects).toEqual([{ _tag: 'PersistBase' }])
    }
    expect(state.complete?.members).toHaveLength(count); expect(state.stale).toBe(false)
    state.reduce({ _tag: 'Metadata', token, frontier: frontier(0, 0) }); expect(state.status).toBe('Ready')
  }
})
it('refuses malformed pages without erasing the last complete base', () => {
  const malformed: ((p: MembershipPage) => MembershipPage)[] = [
    p => ({ ...p, next: undefined }), p => ({ ...p, has_more: false }), p => ({ ...p, next: '' }),
    p => ({ ...p, members: p.members.map(row => ({ ...row, container: 'other' })) }), p => ({ ...p, members: [...p.members, ...p.members] }),
    p => ({ ...p, frontier: frontier(2, 0) }), p => ({ ...p, has_more: false, next: undefined }), p => ({ ...p, members: [] }),
    p => ({ ...p, frontier: { ...p.frontier, live_count: 4097 } }), p => ({ ...p, frontier: { ...p.frontier, changed_index: -1 } }),
  ]
  for (const mutate of malformed) {
    const { state } = ready(); const token = refresh(state)
    expect(state.reduce({ _tag: 'Page', token, page: mutate(page(2, 2, ['b'], 'cursor')) })).toEqual([])
    expect(state.status).toBe('Refused'); expect(state.complete?.members[0]?.member).toBe('a')
  }
  for (const last of [page(2, 2, ['c'], 'cursor'), page(2, 2, ['b']), page(2, 3, ['c'])]) {
    const { state } = ready(); const token = refresh(state)
    state.reduce({ _tag: 'Page', token, page: page(2, 2, ['b'], 'cursor') }); state.reduce({ _tag: 'Page', token, page: last })
    expect(state.status).toBe('Refused'); expect(state.complete?.members[0]?.member).toBe('a')
  }
})
it('bounds restarts at three attempts and ignores late responses', () => {
  for (const cause of ['reset', 'metadata', 'page', 'host', 'epoch']) {
    const { state } = ready(); const old = refresh(state); let token = old
    for (let attempt = 1; attempt <= 3; attempt++) {
      const first = page(attempt, 2, ['b'], 'cursor'); state.reduce({ _tag: 'Page', token, page: first })
      const input = page(attempt + 1, 2, ['c'])
      const effects = cause === 'reset' ? state.reduce(membershipHttpFailure(token, 409, 'membership-frontier-reset'))
        : cause === 'metadata' ? state.reduce({ _tag: 'Metadata', token, frontier: input.frontier })
          : state.reduce({ _tag: 'Page', token, page: cause === 'host' ? { ...input, snapshot: { ...snapshot, host_id: 'host/test-2' } } : cause === 'epoch' ? { ...input, frontier: { ...input.frontier, epoch: 'epoch/other' } } : input })
      expect(state.reduce({ _tag: 'Page', token, page: page(1, 0, []) })).toEqual([])
      if (attempt < 3) token = tokenOf(effects)
      else expect(state.status).toBe('Exhausted')
    }
    expect(state.complete?.members[0]?.member).toBe('a'); refresh(state); expect(state.status).toBe('Paging')
  }
})
it('joins metadata and pages by epoch/index/count, never by host snapshot index', () => {
  for (const paging of [false, true]) {
    const { state, token: initial } = ready(); const token = paging ? refresh(state) : initial
    if (paging) state.reduce({ _tag: 'Page', token, page: page(1, 2, ['b'], 'cursor') })
    state.reduce({ _tag: 'Metadata', token, frontier: frontier(1, 3) }); expect(state.status).toBe('Refused'); expect(state.stale).toBe(true)
  }
  for (const input of [page(4), { ...page(5), frontier: { ...frontier(5), epoch: 'epoch/other' } }, { ...page(5), snapshot: { ...snapshot, host_id: 'host/test-2' } }]) {
    const state = new MembershipPaging(); const token = refresh(state); state.reduce({ _tag: 'Metadata', token, frontier: frontier(5) })
    tokenOf(state.reduce({ _tag: 'Page', token, page: input })); expect(state.complete).toBeUndefined()
  }
  const state = new MembershipPaging(); const token = refresh(state)
  state.reduce({ _tag: 'Metadata', token, frontier: frontier(1, 2) }); state.reduce({ _tag: 'Page', token, page: page(3, 2, ['a'], 'cursor') })
  state.reduce({ _tag: 'Metadata', token, frontier: frontier(2, 1) }); state.reduce({ _tag: 'Page', token, page: page(3, 2, ['b']) })
  expect(state.status).toBe('Ready'); expect(state.complete?.snapshots).toHaveLength(2)
  const next = tokenOf(state.reduce({ _tag: 'Metadata', token, frontier: frontier(4, 2) })); expect(next.attempt).toBeGreaterThan(token.attempt)
})
it('restores stale, isolates every scope field and generation, and retains data on disconnect/refusal', () => {
  const { state, token: old } = ready(); const restored = new MembershipPaging(state.complete); expect(restored.stale).toBe(true)
  const token = refresh(state, 2)
  const fields: (keyof MembershipScope)[] = ['owner', 'container', 'endpoint', 'host', 'projection']
  for (const wrong of [{ ...token, generation: 1 }, { ...token, attempt: old.attempt }, ...fields.map(key => ({ ...token, scope: { ...scope, [key]: 'other' } }))]) {
    for (const event of [{ _tag: 'Page', token: wrong, page: page() }, { _tag: 'Refused', token: wrong }, { _tag: 'Expired', token: wrong }, { _tag: 'Metadata', token: wrong, frontier: frontier(99) }] satisfies Parameters<MembershipPaging['reduce']>[0][]) expect(state.reduce(event)).toEqual([])
    expect(state.status).toBe('Paging')
  }
  state.reduce({ _tag: 'TransportLost', token }); expect(state.status).toBe('Disconnected'); expect(state.complete).toBeDefined()
  const reconnect = refresh(state, 3); state.reduce({ _tag: 'Refused', token: reconnect }); expect(state.status).toBe('Refused')
  for (const key of fields) { const next = new MembershipPaging(state.complete); next.reduce({ _tag: 'Refresh', scope: { ...scope, [key]: 'other' }, generation: 4 }); expect(next.complete).toBeUndefined() }
})
it('adapts real page/frame shapes, subscribes before reading, and fails closed without D metadata', async () => {
  const state = new MembershipPaging(); const token = refresh(state); const calls: string[] = []
  const value: MembershipWirePage = { collection: 'ordered-memberships', kind: 'page', filters: { person: scope.owner, subject: scope.container }, items: [...page().members], page: { limit: 100, has_more: false }, membership: { ...frontier(), container: scope.container } }
  const port = { subscribe: async () => { calls.push('subscribe') }, read: async () => { calls.push('read'); return { value, snapshot } } }
  const event = await executeMembershipRead({ _tag: 'SubscribeAndRead', token }, port)
  expect(calls).toEqual(['subscribe', 'read']); expect(state.reduce(event)).toEqual([{ _tag: 'PersistBase' }])
  const frame: OrderedMembershipFrame = { id: 'subscription/test', collection: 'ordered-memberships', kind: 'changes', has_more: false, order: [], removes: [], upserts: [], snapshot, membership: { ...frontier(2), container: scope.container } }
  expect(membershipFrame(token, frame)._tag).toBe('Metadata')
  expect(membershipFrame(token, { ...frame, snapshot: { ...snapshot, host_id: 'host/test-2' } })._tag).toBe('Expired')
  expect(membershipMetadata(token, { container: scope.container, changed_index: 1, live_count: 1 })._tag).toBe('Refused')
  for (const missing of [{ ...value, membership: undefined }, { ...value, filters: {} }]) expect((await executeMembershipRead({ _tag: 'Read', token, cursor: 'cursor' }, { ...port, read: async () => ({ value: missing, snapshot }) }))._tag).toBe('Refused')
  const denied = new ClientError({ api_version: 'fractal.client.v0', error_version: 'fractal.client.error.v0', code: 'forbidden', message: 'Denied', request_id: 'request/test', retryable: false, details: {} }, 403)
  const expired = new ClientError({ ...denied.response, code: 'page-cursor-expired' }, 410)
  for (const [error, tag] of [[denied, 'Refused'], [expired, 'Expired'], [new TypeError('Transport lost'), 'TransportLost']]) expect((await executeMembershipRead({ _tag: 'Read', token, cursor: 'cursor' }, { ...port, read: async () => { throw error } }))._tag).toBe(tag)
  expect(membershipHttpFailure(token, 409, 'membership-frontier-reset')._tag).toBe('Expired'); expect(membershipHttpFailure(token, 403, 'membership-frontier-reset')._tag).toBe('Refused')
})
it('refuses equal-index page conflicts and permits an incomparable epoch after resubscription', () => {
  const state = new MembershipPaging(); let token = refresh(state)
  state.reduce({ _tag: 'Metadata', token, frontier: frontier(5, 2) })
  state.reduce({ _tag: 'Page', token, page: page(5, 1) }); expect(state.status).toBe('Refused')
  expect(state.reduce({ _tag: 'Expired', token })).toEqual([])
  token = refresh(state)
  state.reduce({ _tag: 'Page', token, page: page(2, 2, ['a'], 'cursor') })
  const changed = { ...page(1, 2, ['😀', '\ue000']), frontier: { ...frontier(1, 2), epoch: 'epoch/new' } }
  token = tokenOf(state.reduce({ _tag: 'Page', token, page: changed }))
  expect(state.reduce({ _tag: 'Page', token, page: changed })).toEqual([{ _tag: 'PersistBase' }])
  expect(state.complete?.members.map(row => row.member)).toEqual(['\ue000', '😀'])
  state.reduce({ _tag: 'Disconnect' }); expect(state.complete).toBeDefined(); expect(state.stale).toBe(true)
})
it('reports HTTP 404 as Unsupported for needs-newer-daemon, preserving stale data and token isolation', async () => {
  const { state, token: old } = ready(); const token = refresh(state, 2)
  expect(state.reduce(membershipHttpFailure(old, 404, 'not-found'))).toEqual([]); expect(state.status).toBe('Paging')
  const unavailable = new ClientError({ api_version: 'fractal.client.v0', error_version: 'fractal.client.error.v0', code: 'not-found', message: 'Unsupported endpoint', request_id: 'request/test', retryable: false, details: {} }, 404)
  const event = await executeMembershipRead({ _tag: 'SubscribeAndRead', token }, { subscribe: async () => {}, read: async () => { throw unavailable } })
  expect(event).toEqual({ _tag: 'Unsupported', token }); expect(state.reduce(event)).toEqual([])
  expect(state.status).toBe('Unsupported'); expect(state.stale).toBe(true); expect(state.complete?.members[0]?.member).toBe('a')
  expect(state.reduce({ _tag: 'Page', token, page: page() })).toEqual([]); expect(state.status).toBe('Unsupported')
  refresh(state, 3); expect(state.status).toBe('Paging')
})
it('restarts the whole staged attempt for server 409 collection changes and 410 cursor expiry', () => {
  for (const error of [{ status: 409, code: 'membership-collection-changed' }, { status: 410, code: 'page-cursor-expired' }]) {
    const { state } = ready(); const old = refresh(state)
    state.reduce({ _tag: 'Page', token: old, page: page(2, 2, ['b'], 'cursor') })
    const token = tokenOf(state.reduce(membershipHttpFailure(old, error.status, error.code)))
    expect(token.attempt).toBeGreaterThan(old.attempt); expect(state.complete?.members[0]?.member).toBe('a')
    expect(state.reduce({ _tag: 'Page', token: old, page: page(2, 2, ['c']) })).toEqual([])
    expect(state.reduce({ _tag: 'Page', token, page: page(3, 0, []) })).toEqual([{ _tag: 'PersistBase' }]); expect(state.complete?.members).toEqual([])
  }
  expect(membershipHttpFailure(ready().token, 409, 'page-cursor-expired')._tag).toBe('Refused')
  expect(membershipHttpFailure(ready().token, 410, 'membership-collection-changed')._tag).toBe('Refused')
})
