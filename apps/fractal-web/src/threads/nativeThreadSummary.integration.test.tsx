// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import ts from 'typescript'
import { afterEach, expect, it, vi } from 'vitest'
import type { ThreadSummary } from '@fractal/client/schema/read'

// Exercise the production native paging, exact-summary hooks and JSX with real
// React mount/update/cleanup. Only native hosts and the timeline body are ports;
// this is a lifecycle regression, not a simulator or viewport-visibility proof.
function production(dependencies: Record<string, unknown>) {
  const source = readFileSync(resolve('../ios/screens/Threads.tsx'), 'utf8')
  const between = (start: string, end: string) => {
    const a = source.indexOf(start)
    const b = source.indexOf(end, a)
    expect(a).toBeGreaterThanOrEqual(0)
    expect(b).toBeGreaterThan(a)
    return source.slice(a, b)
  }
  const input = [
    between('function sameHead', '\ntype ThreadClient'),
    between('function ContextThreadList', '\nexport function ThreadScreen'),
    between('function threadSummaryLine', '\nfunction EmbeddedThreadHeader'),
    between('function useThreadSummary', '\nfunction threadTextIssue'),
  ].join('\n') + '\nexports.ContextThreadList = ContextThreadList;'
  const { outputText } = ts.transpileModule(input, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React,
  } })
  const exported: { ContextThreadList?: React.ComponentType<Record<string, unknown>> } = {}
  new Function('exports', 'React', ...Object.keys(dependencies), outputText)(exported, React, ...Object.values(dependencies))
  return exported.ContextThreadList!
}

type Window = { items: ThreadSummary[]; hasMore: boolean }
type Handlers = { onWindow: (window: Window) => void; onIssue: (issue: string, refused: boolean) => void }

const summary = (id: number): ThreadSummary => ({ id: `thread/${id}`, agent_id: 'agent/reviewer', title: `Review ${id}`,
  state: 'open', unread: true, message_count: 2, revision: 'fence/one', title_revision: 'title/one' })
const window = (items: ThreadSummary[], hasMore = false): Window => ({ items, hasMore })

// One mounted production list with a 40-row head and one older continuation
// row. `live` decides whether the paired feed serves this client at all.
function mount({ live, threadGet }: { live: boolean; threadGet?: (id: string) => Promise<{ value: ThreadSummary }> }) {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const head = Array.from({ length: 40 }, (_, i) => summary(i))
  const older = summary(40)
  const summaries = new Map([...head, older].map(item => [item.id, item]))
  const exact = new Map<string, Handlers>()
  const closed: string[] = []
  const feed = {
    belongsTo: () => live,
    followThreadSummaries: vi.fn((_selector: unknown, handlers: Handlers) => {
      handlers.onWindow(window(head, true))
      return { close: vi.fn() }
    }),
    followThreadSummary: vi.fn((id: string, handlers: Handlers) => {
      exact.set(id, handlers)
      handlers.onWindow(window([summaries.get(id)!]))
      return { close() { closed.push(id); exact.delete(id) } }
    }),
  }
  const client = {
    threadList: vi.fn(async (query: { cursor?: string }) => ({ value: query.cursor
      ? { items: [summaries.get(older.id)!], next_cursor: null } : { items: head, next_cursor: 'older-page' } })),
    threadGet: vi.fn(threadGet ?? (async (id: string) => ({ value: summaries.get(id)! }))),
  }
  const bodyMounted = vi.fn()
  function Body({ thread }: { thread: ThreadSummary }) {
    React.useEffect(() => { bodyMounted(thread.id) }, [])
    return <div data-body={thread.id} data-fence={thread.revision} data-title-fence={thread.title_revision}
      data-state={thread.state} data-unread={String(thread.unread)}>
      <input aria-label="Retained thread draft" defaultValue="Unsent draft" />
    </div>
  }
  const View = React.forwardRef<HTMLDivElement, { children?: React.ReactNode }>((props, ref) => <div ref={ref}>{props.children}</div>)
  const Text = ({ children }: { children?: React.ReactNode }) => <span>{children}</span>
  const Pressable = ({ children, onPress, accessibilityLabel, accessibilityState }: {
    children?: React.ReactNode; onPress: () => void; accessibilityLabel?: string; accessibilityState: { expanded: boolean }
  }) => <button onClick={onPress} aria-label={accessibilityLabel} aria-expanded={accessibilityState.expanded}>{children}</button>
  const Component = production({
    useRef: React.useRef, useState: React.useState, useCallback: React.useCallback, useMemo: React.useMemo, useEffect: React.useEffect,
    useFocusEffect: (callback: React.EffectCallback) => React.useEffect(callback, [callback]),
    useStore: () => ({ threadRefresh: 0, feed }), useWindowDimensions: () => ({ height: 800 }),
    useTextAppearance: () => ({ palette: {} }), THREAD_WINDOW_SIZE: 40, View, ScrollView: View, Pressable, T: Text,
    ThreadViewport: ({ children }: { children: React.ReactNode }) => children, ThreadBody: Body,
    theme: {}, errorText: String, ClientError: class extends Error {},
    Button: ({ label, onPress, disabled }: { label: string; onPress: () => void; disabled: boolean }) => <button disabled={disabled} onClick={onPress}>{label}</button>,
  })
  const press = async (name: string) => {
    const button = Array.from(host.querySelectorAll('button')).find(item => item.textContent?.includes(name))
    expect(button).toBeDefined()
    await act(async () => { button!.click() })
  }
  const header = (title: string) => host.querySelector(`button[aria-label^="${title},"]`)
  const render = () => act(async () => root.render(<Component client={client} context={{ agent_id: 'agent/reviewer' }} title="Threads" openThread={vi.fn()} />))
  const unmount = async () => {
    await act(async () => root.unmount())
    host.remove()
  }
  return { host, older, summaries, exact, closed, feed, client, bodyMounted, press, header, render, unmount }
}

afterEach(() => { vi.unstubAllGlobals() })

it('an expanded 41st thread follows exact summary changes without list refresh and releases its subscription', async () => {
  const view = mount({ live: true })
  const { host, older, summaries, exact, closed, feed, client, bodyMounted, press, header } = view
  const update = async (fields: Partial<ThreadSummary>) => {
    const current = { ...summaries.get(older.id)!, ...fields }
    summaries.set(older.id, current)
    await act(async () => { exact.get(older.id)!.onWindow(window([current])) })
  }
  try {
    await view.render()
    await press('Threads')
    expect(host.textContent).not.toContain('Review 40')
    await press('More threads')
    expect(host.textContent).toContain('Review 40')
    expect(feed.followThreadSummary).not.toHaveBeenCalled()
    await press('Review 40')
    expect(feed.followThreadSummary).toHaveBeenCalledExactlyOnceWith(older.id, expect.anything())
    const draft = host.querySelector<HTMLInputElement>('input')!
    draft.value = 'Keep this draft and focus'
    draft.focus()
    const listReads = client.threadList.mock.calls.length
    const exactReads = client.threadGet.mock.calls.length
    await update({ title: 'Remote renamed older thread', title_revision: 'title/two', revision: 'fence/two' })
    expect(header('Remote renamed older thread')).not.toBeNull()
    const body = host.querySelector('[data-body]')!
    expect(body.getAttribute('data-fence')).toBe('fence/two')
    expect(body.getAttribute('data-title-fence')).toBe('title/two')
    await update({ state: 'closed', revision: 'fence/three' })
    expect(body.getAttribute('data-state')).toBe('closed')
    expect(exact.has(older.id)).toBe(true)
    expect(feed.followThreadSummary).toHaveBeenCalledTimes(1)
    await update({ state: 'open', unread: false, revision: 'fence/four' })
    expect(body.getAttribute('data-state')).toBe('open')
    expect(body.getAttribute('data-unread')).toBe('false')
    expect(host.querySelector('input')).toBe(draft)
    expect(draft.value).toBe('Keep this draft and focus')
    expect(document.activeElement).toBe(draft)
    expect(bodyMounted).toHaveBeenCalledTimes(1)
    expect(client.threadList).toHaveBeenCalledTimes(listReads)
    expect(client.threadGet).toHaveBeenCalledTimes(exactReads)
    // Disclosure releases the exact socket, but retains its last authoritative
    // snapshot so an older page never regresses to its original stale title.
    await press('Remote renamed older thread')
    expect(closed).toEqual([older.id])
    expect(exact.size).toBe(0)
    expect(header('Remote renamed older thread')).not.toBeNull()
    expect(host.querySelector('[data-body]')).toBeNull()
    await press('Remote renamed older thread')
    expect(exact.has(older.id)).toBe(true)
    await press('Threads')
    expect(closed).toEqual([older.id, older.id])
    expect(host.querySelector('[data-body]')).toBeNull()
    await press('Threads')
    expect(exact.has(older.id)).toBe(true)
    expect(header('Remote renamed older thread')).not.toBeNull()
    // A refusal clears the previously authorized exact body; it never falls
    // back to older context metadata for lifecycle actions.
    await act(async () => exact.get(older.id)!.onIssue('Access revoked', true))
    expect(host.querySelector('[data-body]')).toBeNull()
    expect(host.textContent).toContain('Access revoked')
  } finally {
    await view.unmount()
    expect(exact.size).toBe(0)
    expect(closed).toEqual([older.id, older.id, older.id])
  }
})

it('caches the exact REST summary of an expanded older thread when no live feed serves the client', async () => {
  const view = mount({ live: false })
  const { host, older, summaries, feed, client, press, header } = view
  try {
    await view.render()
    await press('Threads')
    await press('More threads')
    expect(header('Review 40')).not.toBeNull()
    // The server renamed the thread after the continuation page was read.
    summaries.set(older.id, { ...older, title: 'Renamed over REST', title_revision: 'title/rest', revision: 'fence/rest' })
    await press('Review 40')
    expect(client.threadGet).toHaveBeenCalledExactlyOnceWith(older.id, expect.anything())
    expect(feed.followThreadSummary).not.toHaveBeenCalled()
    const body = host.querySelector('[data-body]')!
    expect(body.getAttribute('data-fence')).toBe('fence/rest')
    expect(body.getAttribute('data-title-fence')).toBe('title/rest')
    await press('Renamed over REST')
    expect(host.querySelector('[data-body]')).toBeNull()
    expect(header('Renamed over REST')).not.toBeNull()
    expect(header('Review 40')).toBeNull()
  } finally {
    await view.unmount()
  }
})

it('a collapsed thread ignores its in-flight REST read and any frame for its released follow', async () => {
  const pending: Array<(value: { value: ThreadSummary }) => void> = []
  const view = mount({ live: true, threadGet: () => {
    const { promise, resolve } = Promise.withResolvers<{ value: ThreadSummary }>()
    pending.push(resolve)
    return promise
  } })
  const { host, older, exact, closed, press, header } = view
  try {
    await view.render()
    await press('Threads')
    await press('More threads')
    await press('Review 40')
    const released = exact.get(older.id)!
    expect(pending.length).toBeGreaterThan(0)
    await press('Review 40')
    expect(closed).toEqual([older.id])
    expect(host.querySelector('[data-body]')).toBeNull()
    // Late answers for the cancelled reader must not reach the context cache.
    await act(async () => {
      for (const resolve of pending) resolve({ value: { ...older, title: 'Late REST title', revision: 'fence/late' } })
      released.onWindow(window([{ ...older, title: 'Late live title', revision: 'fence/late' }]))
      released.onIssue('Late refusal', true)
    })
    expect(header('Review 40')).not.toBeNull()
    expect(host.textContent).not.toContain('Late REST title')
    expect(host.textContent).not.toContain('Late live title')
    expect(host.textContent).not.toContain('Late refusal')
  } finally {
    await view.unmount()
  }
})
