// @vitest-environment jsdom
import * as React from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'
import { within } from 'storybook/test'
import type * as RowModule from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/RowOverlays.tsx'
import type { SidebarAgentRow as Row } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const row: Row = { ref: 'agent/builder', id: 'builder', title: 'Builder', status: 'working', statusLabel: 'Working', host: 'local', freshness: 'live', needsMe: true, unread: 2, usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, children: [] }
let root: Root
const container = document.createElement('div')
let release: () => void = () => {}

afterEach(async () => {
  release()
  if (root !== undefined) await React.act(async () => root.unmount())
  container.remove()
  vi.doUnmock('../../../../packages/fractal-ui/src/assistant-ui/sidebar/RowOverlays.tsx')
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

it.each(['resolved', 'rejected'] as const)('keeps the roster and action trigger usable when optional controls are %s', async outcome => {
  vi.resetModules()
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  if (globalThis.CSS?.escape === undefined) vi.stubGlobal('CSS', { escape: (value: string) => value.replace(/[^\w-]/g, character => `\\${character}`) })
  const viewport = { width: 340, height: 800 }
  vi.stubGlobal('ResizeObserver', class {
    constructor(private callback: ResizeObserverCallback) {}
    observe(target: Element) { const size = { inlineSize: 340, blockSize: 800 }; this.callback([{ target, contentRect: DOMRect.fromRect(viewport), borderBoxSize: [size], contentBoxSize: [size], devicePixelContentBoxSize: [size] }], this) }
    unobserve() {}
    disconnect() {}
  })
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) { return DOMRect.fromRect(this.tagName === 'SPAN' ? {} : viewport) })
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  const requested = vi.fn()
  let reject!: (error: Error) => void
  const gate = new Promise<void>((resolve, rejectImport) => { release = resolve; reject = rejectImport })
  vi.doMock('../../../../packages/fractal-ui/src/assistant-ui/sidebar/RowOverlays.tsx', async importOriginal => {
    requested()
    await gate
    return importOriginal<typeof RowModule>()
  })
  // Static imports cannot exercise the deliberately held module-loading boundary.
  const { MilestoneAgentRow } = await import('../../../../packages/fractal-ui/src/assistant-ui/sidebar/MilestoneAgentRow.tsx')
  const { SidebarTree } = await import('../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarTree.tsx')
  const selected = vi.fn()
  document.body.append(container)
  root = createRoot(container)
  await React.act(async () => root.render(<SidebarTree aria-label="Agents" items={[{ id: row.ref, textValue: row.title, depth: 0, children: [] }]} expandedKeys={new Set()} onExpandedChange={() => {}} onAction={selected} renderRow={() => <MilestoneAgentRow row={row} now={0} />} />))
  const canvas = within(container)
  const painted = container.querySelector('[data-milestone-row]')
  const treeRow = canvas.getByRole('row')
  expect(requested).not.toHaveBeenCalled()
  expect(canvas.getByLabelText('Needs your attention')).toBeTruthy()
  await React.act(async () => {
    treeRow.focus()
    treeRow.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    treeRow.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', bubbles: true }))
  })
  expect(selected).toHaveBeenCalledWith(row.ref)
  const trigger = canvas.getByRole('button', { name: 'Actions for Builder' })
  await React.act(async () => { trigger.focus(); trigger.click() })
  expect(requested).toHaveBeenCalledOnce()
  expect(container.querySelector('[data-milestone-row]')).toBe(painted)
  expect(canvas.getByRole('button', { name: 'Actions for Builder' })).toBe(trigger)
  expect(document.activeElement).toBe(trigger)
  expect(trigger.getAttribute('aria-expanded')).toBe('true')
  if (outcome === 'rejected') {
    await React.act(async () => reject(new Error('Chunk unavailable')))
    const failure = await within(document.body).findByRole('alert')
    expect(failure.textContent).toBe('These agent actions could not load.')
    expect(container.querySelector('[data-milestone-row]')).toBe(painted)
    expect(container.querySelector('[data-agent-menu]')).toBe(trigger)
    await React.act(async () => within(document.body).getByRole('button', { name: 'Close' }).click())
    await React.act(async () => {
      treeRow.focus()
      treeRow.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
      treeRow.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', bubbles: true }))
    })
    expect(document.activeElement).toBe(treeRow)
    expect(selected).toHaveBeenCalledTimes(2)
    await React.act(async () => canvas.getByRole('button', { name: 'Reported details for Builder' }).click())
    expect((await within(document.body).findByRole('alert')).textContent).toBe('These agent details could not load.')
    await React.act(async () => within(document.body).getByRole('button', { name: 'Close' }).click())
    const { AgentHoverCard } = await import('../../../../packages/fractal-ui/src/assistant-ui/sidebar/DeferredRowActions.tsx')
    await React.act(async () => root.render(<><span data-testid="workspace">Workspace</span><AgentHoverCard row={row} now={0} /></>))
    expect(canvas.getByTestId('workspace').textContent).toBe('Workspace')
    expect((await canvas.findByRole('alert')).textContent).toBe('This agent hover card could not load.')
    // Vitest caches failed module evaluation too; replace that transport failure, not the mounted loader.
    vi.resetModules()
    vi.doMock('../../../../packages/fractal-ui/src/assistant-ui/sidebar/RowOverlays.tsx', async importOriginal => importOriginal<typeof RowModule>())
    await React.act(async () => root.render(<span data-testid="workspace">Workspace</span>))
    await React.act(async () => root.render(<><span data-testid="workspace">Workspace</span><AgentHoverCard row={row} now={0} /></>))
    expect(await canvas.findByTestId('agent-hover-card')).toBeTruthy()
    expect(canvas.queryByRole('alert')).toBeNull()
    await React.act(async () => root.render(<SidebarTree aria-label="Agents" items={[{ id: row.ref, textValue: row.title, depth: 0, children: [] }]} expandedKeys={new Set()} onExpandedChange={() => {}} onAction={selected} renderRow={() => <MilestoneAgentRow row={row} now={0} />} />))
    await React.act(async () => canvas.getByRole('button', { name: 'Actions for Builder' }).click())
    expect(await within(document.body).findByRole('menu', { name: 'Actions for Builder' })).toBeTruthy()
    await React.act(async () => within(document.body).getByRole('menuitem', { name: 'Reported agent details' }).click())
    expect(await within(document.body).findByRole('dialog', { name: 'Reported details for Builder' })).toBeTruthy()
    expect(within(document.body).queryByRole('alert')).toBeNull()
    return
  }
  await React.act(async () => { release(); await import('../../../../packages/fractal-ui/src/assistant-ui/sidebar/RowOverlays.tsx') })
  expect(await within(document.body).findByRole('menu', { name: 'Actions for Builder' })).toBeTruthy()
  expect(container.querySelector('[data-milestone-row]')).toBe(painted)
  expect(container.querySelector('[data-agent-menu]')).toBe(trigger)
})
