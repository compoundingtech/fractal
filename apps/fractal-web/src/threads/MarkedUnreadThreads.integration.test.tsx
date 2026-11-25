// @vitest-environment jsdom
import * as Native from '@fractal/client/schema/read'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { RegistryContext } from '@effect/atom-react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type { ThreadListQuery } from '@fractal/client'
import type { Feed } from '../data/source.ts'
import { MarkedUnreadThreads, type MarkedUnreadSource } from './MarkedUnreadThreads.tsx'
import type { UnreadSeekResult } from './unread-action-port.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const anchor = 'message/return'
const row = (id: string, state: 'open' | 'closed', unread: boolean) => ({ id, agent_id: `agent/${id}`, title: id, title_revision: 'revision/title', revision: 'revision/thread', state, unread,
  unread_reminder: { anchor_message_id: anchor, marks: [{ mark_id: `mark/${id}`, anchor_message_id: anchor }] } })
const page = (items: readonly object[], next: string | null = null): Feed<Native.ThreadSummaryPage> => ({
  _tag: 'Observed', freshness: 'live', value: Native.decodeUnknownSync(Native.ThreadSummaryPage)({ items, next_cursor: next }),
})
const mount = async (read: (query: ThreadListQuery) => Feed<Native.ThreadSummaryPage>, result: Promise<UnreadSeekResult> = Promise.resolve({ _tag: 'Found', messageId: anchor })) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const registry = AtomRegistry.make()
  const queries: ThreadListQuery[] = []
  const seek = vi.fn(() => result), mark = vi.fn(), clear = vi.fn(), open = vi.fn(), reload = vi.fn()
  const pages = Atom.family((cursor: string | undefined) => {
    const query: ThreadListQuery = { marked_unread: true, ...(cursor === undefined ? {} : { cursor }) }
    return Atom.make(read(query))
  })
  const source: MarkedUnreadSource = {
    list: query => { queries.push(query); return pages(query.cursor) }, reload,
    unreadActions: { canMark: true, reminder: () => Atom.make({ _tag: 'Known', reminder: undefined }), seek, mark, clear },
  }
  const host = document.createElement('div'); document.body.append(host)
  const root = createRoot(host)
  await act(async () => root.render(<RegistryContext.Provider value={registry}><MarkedUnreadThreads source={source} onOpenThread={open} /></RegistryContext.Provider>))
  return { host, queries, seek, mark, clear, open, reload,
    observe: async (cursor: string | undefined, feed: Feed<Native.ThreadSummaryPage>) => act(async () => registry.set(pages(cursor), feed)),
    dispose: async () => { await act(async () => root.unmount()); registry.dispose(); host.remove(); vi.unstubAllGlobals() },
  }
}

it('shows owner-wide closed and open rows, with private reminders separate from natural unread', async () => {
  const pending = Promise.withResolvers<UnreadSeekResult>()
  const view = await mount(query => query.cursor === undefined ? page([], 'next') : page([row('thread/closed', 'closed', false), row('thread/open', 'open', true)]), pending.promise)
  try {
    expect(view.queries).toEqual([{ marked_unread: true }, { marked_unread: true, cursor: 'next' }])
    const closed = view.host.querySelector('[data-marked-unread-thread="thread/closed"]')!
    const natural = view.host.querySelector('[data-marked-unread-thread="thread/open"]')!
    expect(closed.textContent).toContain('Closed')
    expect(closed.textContent).toContain('Private reminder')
    expect(closed.textContent).not.toContain('Unread, ')
    expect(natural.textContent).toContain('Unread, ')
    await act(async () => closed.querySelector('button')!.click())
    expect(view.seek).toHaveBeenCalledWith({ threadId: 'thread/closed', anchor_message_id: anchor })
    expect(view.open).not.toHaveBeenCalled()
    expect(closed.textContent).toContain('Loading reminder message')
    await act(async () => pending.resolve({ _tag: 'Found', messageId: anchor }))
    expect(view.open).toHaveBeenCalledWith('thread/closed')
    expect(view.mark).not.toHaveBeenCalled(); expect(view.clear).not.toHaveBeenCalled()
  } finally { await view.dispose() }
})

it.each(['Pending', 'Unavailable'] as const)('shows the %s anchor explanation without navigation or mutations', async tag => {
  const view = await mount(() => page([row('thread/a', 'closed', false)]), Promise.resolve({ _tag: tag, detail: 'The reminder message is not available yet.' }))
  try {
    await act(async () => view.host.querySelector('button')!.click())
    expect(view.host.textContent).toContain('The reminder message is not available yet.')
    expect(view.host.querySelector('[role="alert"]')).toBeNull()
    expect(view.host.querySelector('[role="status"]')?.textContent).toContain('The reminder message is not available yet.')
    expect(view.open).not.toHaveBeenCalled(); expect(view.mark).not.toHaveBeenCalled(); expect(view.clear).not.toHaveBeenCalled()
  } finally { await view.dispose() }
})

it.each(['RepeatedCursor', 'PageBoundExceeded', 'FetchFailed'] as const)('renders %s as an error with Retry, never empty', async tag => {
  const view = await mount(query => tag === 'FetchFailed' ? { _tag: 'Unavailable', reason: 'failed', detail: 'Offline' }
    : page([], tag === 'RepeatedCursor' ? 'loop' : `${query.cursor ?? ''}x`))
  try {
    expect(view.host.querySelector(`[data-marked-unread-error="${tag}"]`)).not.toBeNull()
    expect(view.host.textContent).not.toContain('No marked-unread threads')
    const retry = [...view.host.querySelectorAll('button')].find(button => button.textContent?.includes('Retry'))!
    await act(async () => retry.click())
    expect(view.reload).toHaveBeenCalledOnce()
  } finally { await view.dispose() }
})

it('shows empty only after traversing the final empty page', async () => {
  const view = await mount(query => page([], query.cursor === undefined ? 'terminal' : null))
  try {
    expect(view.queries).toHaveLength(2)
    expect(view.host.textContent).toContain('No marked-unread threads')
  } finally { await view.dispose() }
})

it('removes a reactively cleared summary without fabricating a private marker, and continues later cursors', async () => {
  const first = row('thread/cleared', 'closed', false)
  const view = await mount(query => query.cursor === undefined ? page([first], 'later') : page([row('thread/remaining', 'open', true)]))
  try {
    expect(view.host.querySelector('[data-marked-unread-thread="thread/cleared"]')).not.toBeNull()
    await view.observe(undefined, page([{ ...first, unread_reminder: null }], 'later'))
    expect(view.host.querySelector('[data-marked-unread-thread="thread/cleared"]')).toBeNull()
    expect(view.host.querySelector('[data-marked-unread-thread="thread/remaining"]')).not.toBeNull()
    expect(view.host.querySelectorAll('[data-private-unread-reminder]')).toHaveLength(1)
    expect(view.host.textContent).not.toContain('No marked-unread threads')
    expect(view.mark).not.toHaveBeenCalled(); expect(view.clear).not.toHaveBeenCalled()
  } finally { await view.dispose() }
})
