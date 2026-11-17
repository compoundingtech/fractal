// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

import { ThreadPanel } from './ThreadPanel.tsx'
import type { ThreadRefusal } from './threadSource.ts'

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
    expect(host.querySelector('[data-thread-panel]')?.getAttribute('role')).toBe('group')
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
  const rename = vi.fn(async () => ({ detail: 'The title changed. Review the latest title and try again.' }))
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
    // The field replaced the heading in place; the panel still names the current title.
    expect(host.querySelector('h2')).toBeNull()
    expect(host.querySelector('[data-thread-panel]')?.getAttribute('aria-label')).toBe('Worker check')
    expect(host.querySelector<HTMLInputElement>('[data-thread-rename-form] input')?.value).toBe('PR #42')
    await press('Save')
    expect(rename).toHaveBeenCalledWith('PR #42', 'event/old')
    expect(host.querySelector('[role=alert]')?.textContent).toContain('The title changed')
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

it.each([
  ['with Fractal\'s reason', { detail: "Couldn't close this thread.", explanation: 'The thread owner closed changes.' }],
  ['without a reason', { detail: "Couldn't close this thread." }],
] as const)('announces a refused change %s as one alert, the reason on its own line', async (_case, refusal) => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const change = vi.fn(async () => refusal)
  try {
    await act(async () => root.render(<ThreadPanel thread={{ title: 'PR #42', title_revision: 'event/one', state: 'open' }} embedded={false} onChangeState={change}>
      Thread contents
    </ThreadPanel>))
    const close = Array.from(host.querySelectorAll('button')).find(button => button.textContent === 'Close thread')
    await act(async () => close?.click())
    const alerts = host.querySelectorAll('[role=alert]')
    expect(alerts).toHaveLength(1)
    const lines = Array.from(alerts[0]?.children ?? [], line => line.textContent)
    expect(lines).toEqual('explanation' in refusal ? [refusal.detail, refusal.explanation] : [refusal.detail])
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

it.each(['close', 'rename'] as const)('keeps the original %s reachable after its summary changes and the view remounts', async kind => {
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
      ...base.value.capabilities, ...['threads-v1', 'thread-summary-feed', 'thread.close', 'thread.reopen', 'thread.rename'].map(id => ({ id, version: id === 'threads-v1' ? 3 : 1, state: 'granted' })),
    ] } })
    const action = JSON.parse(String(init?.body)) as typeof writes[number]
    writes.push(action)
    if (writes.length === 1) throw new TypeError('The close committed but its reply was lost')
    return Response.json({ ...base, value: { kind: 'action-result', action_id: action.id, operation_id: 'operation/close', status: 'completed',
      affected_ids: [action.parameters.target_id], metadata_claim_id: 'claim/closed', snapshot_id: base.snapshot.id } })
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
  const View = ({ updated }: { updated: boolean }) => {
    const state = updated && kind === 'close' ? 'closed' : 'open'
    const title = updated && kind === 'rename' ? 'New title' : 'PR #42'
    const pending = useAtomValue(pendingChange(id))
    const thread = Native.decodeUnknownSync(Native.ThreadSummary)({ id, agent_id: 'agent/atlas', title, title_revision: updated && kind === 'rename' ? 'claim/new-title' : 'claim/title', revision: updated ? 'claim/updated' : 'claim/open', state })
    return <ThreadPanel thread={thread} embedded={false} pendingChange={pending} onRetryChange={() => retryChange(id)}
      onRename={(title, revision) => source.rename({ summary: thread, snapshotId: base.snapshot.id }, title, revision)}
      onChangeState={state => source.changeState({ summary: thread, snapshotId: base.snapshot.id }, state)}>History</ThreadPanel>
  }
  const render = (updated: boolean) => act(async () => root.render(<RegistryContext.Provider value={registry}><View updated={updated} /></RegistryContext.Provider>))
  const press = async (label: string) => {
    const button = [...host.querySelectorAll('button')].find(button => button.textContent === label)
    expect(button).toBeDefined()
    expect(button?.disabled).toBe(false)
    await act(async () => button?.click())
  }
  const stop = registry.mount(source.access)
  try {
    await vi.waitFor(() => expect(registry.get(source.access).actions.has('thread.close')).toBe(true))
    await render(false)
    if (kind === 'close') await press('Close thread')
    else {
      await press('Rename')
      const input = host.querySelector('input')
      if (!input) throw new Error('Expected the rename input')
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(input, 'New title')
        input.dispatchEvent(new Event('input', { bubbles: true }))
      })
      await press('Save')
    }
    await render(true)
    if (kind === 'close') expect([...host.querySelectorAll('button')].find(button => button.textContent === 'Reopen thread')?.disabled).toBe(true)
    else {
      expect(host.querySelector('[data-thread-panel]')?.getAttribute('aria-label')).toBe('New title')
      expect([...host.querySelectorAll('button')].find(button => button.textContent === 'Save')?.disabled).toBe(true)
      await press('Cancel')
      expect(host.querySelector('input')).toBeNull()
      expect(host.querySelector('h2')?.textContent).toContain('New title')
    }
    await act(async () => root.render(<RegistryContext.Provider value={registry}>Another view</RegistryContext.Provider>))
    await render(true)
    await press(`Retry ${kind}`)
    expect(writes).toHaveLength(2)
    expect(writes[1]).toEqual(writes[0])
    expect(host.textContent).not.toContain(`Retry ${kind}`)
    const nextAction = kind === 'close' ? 'Reopen thread' : 'Rename'
    expect([...host.querySelectorAll('button')].find(button => button.textContent === nextAction)?.disabled).toBe(false)
  } finally { stop(); await act(async () => root.unmount()); registry.dispose(); host.remove() }
})

it('renames in place in the header: Escape cancels, Enter saves, and focus returns to Rename', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const rename = vi.fn(async (_title: string, _revision: string): Promise<ThreadRefusal | undefined> => undefined)
  const change = vi.fn(async () => undefined)
  const render = (title: string) => act(async () => {
    root.render(<ThreadPanel thread={{ title, title_revision: 'event/one', state: 'open' }} initiallyExpanded onRename={rename} onChangeState={change}>
      Thread contents
    </ThreadPanel>)
  })
  const press = async (label: string) => {
    const button = [...host.querySelectorAll('button')].find(button => button.textContent === label)
    expect(button).toBeDefined()
    await act(async () => button?.click())
  }
  const field = () => host.querySelector<HTMLInputElement>('[data-thread-rename-form] input')
  try {
    await render('PR #42')
    await press('Close thread')
    expect(host.querySelector('[data-thread-confirmed]')?.textContent).toBe('Closed “PR #42”.')
    await press('Rename')
    // Opening Rename clears the previous confirmation.
    expect(host.querySelector('[data-thread-confirmed]')).toBeNull()
    const form = host.querySelector('[data-thread-rename-form]')
    const renameButton = host.querySelector('[data-thread-rename]')
    if (!form || !renameButton) throw new Error('Expected the rename field and the Rename action')
    // The field takes the title's place in the header, ahead of the conversation and its actions.
    expect(host.querySelector('[aria-expanded]')).toBeNull()
    expect(form.compareDocumentPosition(renameButton) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(document.activeElement).toBe(field())
    expect(field()?.value).toBe('PR #42')

    await act(async () => field()?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })))
    expect(form.isConnected).toBe(false)
    expect(document.activeElement).toBe(host.querySelector('[data-thread-rename]'))
    expect(host.querySelector('button[aria-expanded]')?.textContent).toContain('PR #42')
    expect(rename).not.toHaveBeenCalled()

    await press('Rename')
    const input = field()
    if (!input) throw new Error('Expected the rename field')
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(input, 'Release checklist')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    // Enter in a single-field form is the form's implicit submission.
    await act(async () => input.form?.requestSubmit())
    expect(rename).toHaveBeenCalledWith('Release checklist', 'event/one')
    await render('Release checklist')
    expect(field()).toBeNull()
    expect(document.activeElement).toBe(host.querySelector('[data-thread-rename]'))
    expect(host.querySelector('[data-thread-confirmed]')?.textContent).toBe('Renamed to “Release checklist”.')
    expect(host.querySelector('button[aria-expanded]')?.textContent).toContain('Release checklist')
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

it('holds the confirmation while the panel has focus or a pointer, then lets it go', async () => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
  const host = document.createElement('div')
  const outside = document.createElement('button')
  document.body.append(host, outside)
  const root = createRoot(host)
  const change = vi.fn(async () => undefined)
  const confirmation = () => host.querySelector('[data-thread-confirmed]')
  const wait = (ms: number) => act(async () => { vi.advanceTimersByTime(ms) })
  try {
    await act(async () => {
      root.render(<ThreadPanel thread={{ title: 'PR #42', title_revision: 'event/one', state: 'open' }} embedded={false} onChangeState={change}>Thread contents</ThreadPanel>)
    })
    const panel = host.querySelector('[data-thread-panel]')
    if (!panel) throw new Error('Expected the panel')
    await act(async () => host.querySelector<HTMLButtonElement>('[data-thread-state]')?.click())
    expect(change).toHaveBeenCalledWith('closed')
    expect(confirmation()?.getAttribute('role')).toBe('status')
    expect(confirmation()?.textContent).toBe('Closed “PR #42”.')
    // Focus is back on the lifecycle button inside the panel, so the confirmation outlasts its usual time.
    expect(panel.contains(document.activeElement)).toBe(true)
    await wait(20_000)
    expect(confirmation()).not.toBeNull()

    // Focus leaves, but a pointer rests on the panel.
    await act(async () => outside.focus())
    await act(async () => panel.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, relatedTarget: outside })))
    await wait(20_000)
    expect(confirmation()).not.toBeNull()

    await act(async () => panel.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: outside })))
    await wait(10_000)
    expect(confirmation()).toBeNull()
  } finally {
    vi.useRealTimers()
    await act(async () => root.unmount())
    host.remove()
    outside.remove()
  }
})

it('lines rows up in a shared list with a trailing age column and never shows a bare count', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () => {
      root.render(<div>
        <ThreadPanel thread={{ title: 'PR #42', state: 'open' }} inList unread messageCount={3} age={<time dateTime="2026-11-14T10:00:00Z">4m ago</time>}>First</ThreadPanel>
        <ThreadPanel thread={{ title: 'Worker check', state: 'closed' }} inList messageCount={1}>Second</ThreadPanel>
      </div>)
    })
    const [first, second] = [...host.querySelectorAll('[data-thread-panel] button[aria-expanded]')]
    if (!first || !second) throw new Error('Expected two row headers')
    // Age is the header's final column; a row without one leaves its metadata at the end.
    expect(first.lastElementChild?.matches('[data-thread-age]')).toBe(true)
    expect(first.lastElementChild?.textContent).toBe('4m ago')
    expect(second.querySelector('[data-thread-age]')).toBeNull()
    expect(second.textContent).toContain('· Closed')
    // The narrow label carries its unit, so a bare right-aligned number stays free for an unread count.
    const shown = [...first.querySelectorAll('[aria-hidden="true"]')].map(node => node.textContent)
    expect(shown).toContain('3 msgs')
    expect(shown).not.toContain('3')
    expect(first.textContent).toContain('3 messages')
    // Names live in visually hidden text, not aria-label on generic spans.
    expect(first.textContent).toContain('Unread, ')
    expect(host.querySelectorAll('span[aria-label]')).toHaveLength(0)
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

it('gives the dedicated view a heading with its origin, and lets an embedded row open elsewhere', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const activate = vi.fn()
  const expansion = vi.fn()
  const origin = <a href="#review">With Atlas · about “Review the release checklist”</a>
  try {
    await act(async () => {
      root.render(<ThreadPanel thread={{ title: 'PR #42', state: 'open' }} embedded={false} origin={origin}>Thread contents</ThreadPanel>)
    })
    // A read thread's heading starts with its title: no empty unread slot indents it.
    expect(host.querySelector('h2')?.firstElementChild?.textContent).toBe('PR #42')
    expect(host.querySelector('[data-thread-origin] a')?.textContent).toBe('With Atlas · about “Review the release checklist”')

    await act(async () => {
      root.render(<ThreadPanel thread={{ title: 'PR #42', state: 'open' }} origin={origin} onActivate={activate} onExpansionChange={expansion}>
        <textarea aria-label="Thread message" />
      </ThreadPanel>)
    })
    expect(host.querySelector('[data-thread-origin]')).toBeNull()
    const header = host.querySelector<HTMLButtonElement>('[data-thread-panel] button')
    expect(header?.hasAttribute('aria-expanded')).toBe(false)
    await act(async () => header?.click())
    expect(activate).toHaveBeenCalledTimes(1)
    expect(expansion).not.toHaveBeenCalled()
    expect(host.querySelector('textarea')).toBeNull()
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

it('keeps keyboard focus on the disclosure as the unread action appears and disappears', async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const markRead = async () => undefined
  const render = (unread: boolean) => act(async () => {
    root.render(<ThreadPanel thread={{ title: 'PR #42', state: 'open' }} messageCount={3} unread={unread}
      onMarkRead={unread ? markRead : undefined} canMarkRead>
      <textarea aria-label="Thread message" />
    </ThreadPanel>)
  })
  try {
    await render(true)
    const header = host.querySelector<HTMLButtonElement>('button[aria-expanded]')!
    await act(async () => header.focus())
    expect(document.activeElement).toBe(header)
    await act(async () => header.click())
    expect(host.querySelector('button[aria-expanded]')).toBe(header)
    expect(header.getAttribute('aria-expanded')).toBe('true')
    expect(document.activeElement).toBe(header)
    expect(host.querySelector('[aria-label="Mark visible replies read"]')).not.toBeNull()

    // A replicated read update also removes the action without moving the focused disclosure.
    await render(false)
    expect(host.querySelector('[aria-label="Mark visible replies read"]')).toBeNull()
    expect(host.querySelector('button[aria-expanded]')).toBe(header)
    expect(document.activeElement).toBe(header)
    await act(async () => header.click())
    expect(header.getAttribute('aria-expanded')).toBe('false')
    expect(document.activeElement).toBe(header)
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})
