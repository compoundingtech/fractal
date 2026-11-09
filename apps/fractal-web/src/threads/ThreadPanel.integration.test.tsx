// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  props: () => ({}),
}))

import { ThreadPanel } from './ThreadPanel.tsx'

beforeEach(() => vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true))
afterEach(() => vi.unstubAllGlobals())

it('collapses the entire embedded conversation and keeps lifecycle state authoritative', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const change = vi.fn(async () => undefined)
  const render = (state: 'open' | 'closed') => act(async () => {
    root.render(<ThreadPanel thread={{ title: 'PR #42', title_revision: 'event/one', state }} messageCount={3} onChangeState={change}>
      <textarea aria-label="Thread message" />
    </ThreadPanel>)
  })
  const press = async (name: string) => {
    const button = Array.from(host.querySelectorAll('button')).find(button => button.textContent?.includes(name))
    expect(button).toBeDefined()
    await act(async () => button?.click())
  }
  try {
    await render('open')
    const header = host.querySelector('button')
    expect(header?.getAttribute('aria-expanded')).toBe('false')
    expect(header?.textContent).toContain('3 messages')
    expect(host.querySelector('textarea')).toBeNull()

    await press('PR #42')
    expect(header?.getAttribute('aria-expanded')).toBe('true')
    expect(host.querySelector('textarea')).not.toBeNull()
    await press('Close thread')
    expect(change).toHaveBeenCalledWith('closed')
    expect(host.textContent).toContain('Close thread')

    await render('closed')
    expect(host.textContent).toContain('Reopen thread')
    await press('Reopen thread')
    expect(change).toHaveBeenLastCalledWith('open')
    await press('PR #42')
    expect(header?.textContent).toContain('3 messages')
    expect(header?.textContent).toContain('Closed')
    expect(host.querySelector('textarea')).toBeNull()
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

it('keeps the title revision that the person began editing after a concurrent rename', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const rename = vi.fn(async () => 'The title changed. Review the latest title and try again.')
  const render = (title: string, revision: string) => act(async () => {
    root.render(<ThreadPanel thread={{ title, title_revision: revision, state: 'open' }} embedded={false} onRename={rename}>
      Thread contents
    </ThreadPanel>)
  })
  const press = async (label: string) => {
    const button = Array.from(host.querySelectorAll('button')).find(button => button.textContent === label)
    expect(button).toBeDefined()
    await act(async () => button?.click())
  }
  try {
    await render('PR #42', 'event/old')
    await press('Rename')
    await render('Worker check', 'event/new')
    expect(host.querySelector('h2')?.textContent).toBe('Worker check')
    expect(host.querySelector('input')?.value).toBe('PR #42')
    await press('Save')
    expect(rename).toHaveBeenCalledWith('PR #42', 'event/old')
    expect(host.querySelector('[role=alert]')?.textContent).toContain('The title changed')
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

it('keeps the original close reachable after the feed closes the thread and the view remounts', async () => {
  const { FractalClient } = await import('@fractal/client')
  const { RegistryContext, useAtomValue } = await import('@effect/atom-react')
  const AtomRegistry = await import('effect/reactivity/AtomRegistry')
  const Native = await import('@fractal/client/schema/read')
  const { gatewayThreads } = await import('./threadSource.ts')
  const fixture = (await import('../data/subjectReadPort.gateway.fixtures.json')).default
  const base = fixture.capabilities
  const writes: Array<{ id: string; parameters: { target_id: string } }> = []
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async (input, init) => {
    if (String(input).endsWith('/capabilities')) return Response.json({ ...base, value: { ...base.value, capabilities: [
      ...base.value.capabilities, ...['threads-v1', 'thread-summary-feed', 'thread.close', 'thread.reopen'].map(id => ({ id, version: 1, state: 'granted' })),
    ] } })
    const action = JSON.parse(String(init?.body)) as typeof writes[number]
    writes.push(action)
    if (writes.length === 1) throw new TypeError('The close committed but its reply was lost')
    return Response.json({ ...base, value: { kind: 'action-result', action_id: action.id, operation_id: 'operation/close', status: 'completed',
      affected_ids: [action.parameters.target_id], metadata_revision: 'claim/closed', snapshot_id: base.snapshot.id } })
  } })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const pendingChange = source.pendingChange
  const retryChange = source.retryChange
  if (!pendingChange || !retryChange) throw new Error('Expected retained lifecycle actions')
  const id = 'thread/review'
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const View = ({ state }: { state: 'open' | 'closed' }) => {
    const pending = useAtomValue(pendingChange(id))
    const thread = Native.decodeUnknownSync(Native.ThreadSummary)({ id, agent_id: 'agent/atlas', title: 'PR #42', title_revision: 'claim/title', revision: state === 'open' ? 'claim/open' : 'claim/closed', state })
    return <ThreadPanel thread={thread} embedded={false} pendingChange={pending} onRetryChange={() => retryChange(id)}
      onChangeState={state => source.changeState({ summary: thread, snapshotId: base.snapshot.id }, state)}>History</ThreadPanel>
  }
  const render = (state: 'open' | 'closed') => act(async () => root.render(<RegistryContext.Provider value={registry}><View state={state} /></RegistryContext.Provider>))
  const press = async (label: string) => {
    const button = [...host.querySelectorAll('button')].find(button => button.textContent === label)
    expect(button).toBeDefined()
    expect(button?.disabled).toBe(false)
    await act(async () => button?.click())
  }
  const stop = registry.mount(source.access)
  try {
    await vi.waitFor(() => expect(registry.get(source.access).actions.has('thread.close')).toBe(true))
    await render('open')
    await press('Close thread')
    await render('closed')
    expect([...host.querySelectorAll('button')].find(button => button.textContent === 'Reopen thread')?.disabled).toBe(true)
    await act(async () => root.render(<RegistryContext.Provider value={registry}>Another view</RegistryContext.Provider>))
    await render('closed')
    await press('Retry close')
    expect(writes).toHaveLength(2)
    expect(writes[1]).toEqual(writes[0])
    expect(host.textContent).not.toContain('Retry close')
    expect([...host.querySelectorAll('button')].find(button => button.textContent === 'Reopen thread')?.disabled).toBe(false)
  } finally { stop(); await act(async () => root.unmount()); registry.dispose(); host.remove() }
})
