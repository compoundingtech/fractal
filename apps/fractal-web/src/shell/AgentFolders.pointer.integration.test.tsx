// @vitest-environment jsdom
// Sidebar rows on the workspace's pointer drag: filing among folders, and agents onto panes.
import * as React from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'
import type { AgentFoldersProps, SidebarNode } from '../folders/sidebarContract.ts'
import { SidebarAgentRow, sidebarRowAccessibleName } from '@fractal/ui/assistant-ui/shell'
import * as sidebarKit from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarTree.tsx'
import { sidebarRow } from '../web/sidebarRow.ts'
import { fleetFromAgents } from '../data/projections.ts'
import { Agent, decodeUnknownSync } from '@fractal/client/schema'
import { openAgent } from '@fractal/sdk/read'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: Record<string, unknown>) => styles, defineVars: (vars: unknown) => vars, createTheme: () => ({}), keyframes: () => '', props: () => ({}), when: { ancestor: () => ':focus-visible' }, defaultMarker: () => ({}) }))
import { AgentFolders } from './AgentFolders.tsx'
import { WorkspaceProvider } from '@fractal/ui/workspace/react'
import { createWorkspaceStore } from '../workspace/store.ts'
import { currentSpace } from '../workspace/model.ts'
import * as Workspace from '@fractal/ui/workspace'

const viewport = { width: 340, height: 800 }
class ViewportObserver implements ResizeObserver {
  constructor(private callback: ResizeObserverCallback) {}
  observe(target: Element): void {
    const size = { inlineSize: 340, blockSize: 800 }
    this.callback([{ target, contentRect: DOMRect.fromRect(viewport), borderBoxSize: [size], contentBoxSize: [size], devicePixelContentBoxSize: [size] }], this)
  }
  unobserve(): void {}
  disconnect(): void {}
}
globalThis.ResizeObserver = ViewportObserver
Element.prototype.getBoundingClientRect = function (this: Element) { return DOMRect.fromRect(this.tagName === 'SPAN' ? {} : viewport) }
Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
vi.stubGlobal('CSS', { escape: (value: string) => value.replaceAll('/', '\\/') })
const fixture: readonly SidebarNode[] = [
  { _tag: 'Folder', id: 'alpha', label: 'Alpha', collapsed: false, children: [
    { _tag: 'Folder', id: 'beta', label: 'Beta', collapsed: false, children: [
      { _tag: 'Folder', id: 'gamma', label: 'Gamma', collapsed: false, children: [] },
    ] },
    { _tag: 'Agent', id: 'agent/one', subject: 'agent/one', label: 'One' },
  ] },
  { _tag: 'Group', id: 'unfiled', label: 'Unfiled', collapsed: false, children: [
    { _tag: 'Agent', id: 'agent/two', subject: 'agent/two', label: 'Two' },
  ] },
]

const box = (top: number, height = 40): DOMRect => DOMRect.fromRect({ x: 0, y: top, width: 340, height })
const pointer = async (target: EventTarget, type: string, init: { clientX?: number; clientY?: number } = {}) => {
  await React.act(async () => {
    const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, ...init })
    Object.assign(event, { pointerId: 1, pointerType: 'mouse', width: 1, height: 1 })
    target.dispatchEvent(event)
  })
}

const mount = async (extra: Partial<AgentFoldersProps> = {}) => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  let ids = 0
  const store = createWorkspaceStore({ newId: () => `id-${ids++}` })
  store.dispatch({ _tag: 'Open', ref: 'agent:agent/zero' })
  const pane = currentSpace(store.getState()).local.focus
  const moves: unknown[] = []
  const refused: string[] = []
  const selected: string[] = []
  const props: AgentFoldersProps = {
    tree: fixture, canDrop: () => ({ ok: true }), onMove: move => { moves.push(move) }, onEditRefused: reason => { refused.push(reason) },
    onCreateFolder: () => {}, onRenameFolder: () => {}, onDeleteFolder: () => {}, onToggleCollapsed: () => {}, onSelect: id => { selected.push(id) },
    renderAgent: node => <span>{node.label}</span>,
    ...extra,
  }
  await React.act(async () => { root.render(<WorkspaceProvider store={store} describe={ref => ({ kind: 'agent', id: ref, title: ref })} renderDragChip={entity => <span data-testid="chip">{entity.title}</span>}>
    <AgentFolders {...props} />
    <div data-drop-content={pane} data-testid="pane" />
  </WorkspaceProvider>) })
  const row = (key: string): HTMLElement => host.querySelector<HTMLElement>(`[data-sidebar-tree-key="${key}"]`)!
  const drag = async (from: string, to: Element, y: number) => {
    document.elementsFromPoint = () => [to]
    await pointer(row(from), 'pointerdown', { clientX: 10, clientY: 5 })
    await pointer(window, 'pointermove', { clientX: 10, clientY: 60 })
    await pointer(window, 'pointermove', { clientX: 20, clientY: y })
  }
  const release = (y: number) => pointer(window, 'pointerup', { clientX: 20, clientY: y })
  const panes = () => Workspace.panesOf(currentSpace(store.getState()).layout).map(row => row.tabs.map(tab => tab.ref))
  return { host, row, drag, release, moves, refused, selected, panes, dispose: async () => { await React.act(async () => root.unmount()); host.remove(); store.dispose() } }
}

describe('sidebar rows on the pointer drag', () => {
  it('is never natively draggable, so the platform cannot cancel a pointer drag', async () => {
    const view = await mount()
    expect(view.host.querySelector('[draggable="true"]')).toBeNull()
    await view.dispose()
  })

  it('files a row into a folder through its middle, and beside a row through its edges, with a live chip', async () => {
    const view = await mount()
    const gamma = view.row('gamma')
    gamma.getBoundingClientRect = () => box(200)
    await view.drag('agent/two', gamma, 220)
    expect(document.querySelector('[data-testid="chip"]')?.textContent).toBe('agent:agent/two')
    expect(document.querySelector('[data-folder-drop]')?.getAttribute('data-folder-drop')).toBe('on')
    await view.release(220)
    expect(view.moves).toEqual([{ items: ['agent/two'], target: { _tag: 'Into', folder: 'gamma' } }])
    await view.drag('agent/two', gamma, 202)
    expect(document.querySelector('[data-folder-drop]')?.getAttribute('data-folder-drop')).toBe('before')
    await view.release(202)
    const one = view.row('agent/one')
    one.getBoundingClientRect = () => box(300)
    await view.drag('agent/two', one, 335)
    await view.release(335)
    expect(view.moves.slice(1)).toEqual([
      { items: ['agent/two'], target: { _tag: 'Before', sibling: 'gamma' } },
      { items: ['agent/two'], target: { _tag: 'After', sibling: 'agent/one' } },
    ])
    expect(view.selected, 'a drag is not also a press on the row').toEqual([])
    await view.dispose()
  })

  it('says why a refused move cannot land, and moves nothing', async () => {
    const view = await mount({ canDrop: () => ({ refused: 'Folders nest two deep at most.' }) })
    const gamma = view.row('gamma')
    gamma.getBoundingClientRect = () => box(200)
    await view.drag('alpha', gamma, 220)
    await view.release(220)
    expect(view.moves).toEqual([])
    expect(view.refused).toEqual(['Folders nest two deep at most.'])
    await view.dispose()
  })

  it('scrolls the tree while a row is held near its top or bottom edge', async () => {
    const view = await mount()
    const scroller = view.host.querySelector<HTMLElement>('[role="treegrid"]')!
    scroller.getBoundingClientRect = () => DOMRect.fromRect({ x: 0, y: 0, width: 340, height: 400 })
    let scrolled = 0
    Object.defineProperty(scroller, 'scrollTop', { configurable: true, get: () => scrolled, set: (value: number) => { scrolled = value } })
    await view.drag('agent/two', scroller, 396)
    await React.act(async () => { await new Promise(resolve => setTimeout(resolve, 120)) })
    expect(scrolled).toBeGreaterThan(0)
    await view.release(396)
    const after = scrolled
    await React.act(async () => { await new Promise(resolve => setTimeout(resolve, 60)) })
    expect(scrolled, 'it stops with the drag').toBe(after)
    await view.dispose()
  })

  it('opens a dragged agent in a pane, splitting at its edge, but never a folder', async () => {
    const view = await mount()
    const pane = view.host.querySelector<HTMLElement>('[data-testid="pane"]')!
    pane.getBoundingClientRect = () => DOMRect.fromRect({ x: 400, y: 0, width: 800, height: 600 })
    await view.drag('alpha', pane, 300)
    expect(document.querySelector('[data-testid="chip"]')?.textContent).toBe('Alpha')
    await view.release(300)
    expect(view.panes()).toEqual([['agent:agent/zero']])
    document.elementsFromPoint = () => [pane]
    await pointer(view.row('agent/two'), 'pointerdown', { clientX: 10, clientY: 5 })
    await pointer(window, 'pointermove', { clientX: 600, clientY: 300 })
    await pointer(window, 'pointermove', { clientX: 1190, clientY: 300 })
    await pointer(window, 'pointerup', { clientX: 1190, clientY: 300 })
    expect(view.panes()).toEqual([['agent:agent/zero'], ['agent:agent/two']])
    expect(view.selected).toEqual([])
    await view.dispose()
  })
})
