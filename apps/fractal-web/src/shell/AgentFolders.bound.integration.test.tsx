// @vitest-environment jsdom
import * as React from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'
import type { AgentFoldersProps, SidebarNode } from '../folders/sidebarContract.ts'
import { expect as expectA11y, within } from 'storybook/test'
import { SidebarAgentRow } from '@smalltalk/fractal-ui/assistant-ui/shell'
import { sidebarRow } from '../web/sidebarRow.ts'
import { fleetFromAgents } from '../data/projections.ts'
import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: Record<string, unknown>) => styles, defineVars: (vars: unknown) => vars, createTheme: () => ({}), keyframes: () => '', props: () => ({}) }))
import { AgentFolders } from './AgentFolders.tsx'

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
const mount = async () => {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const toggles: { id: string; collapsed: boolean }[] = []
  const Harness = () => {
    const [collapsed, setCollapsed] = React.useState<ReadonlySet<string>>(new Set())
    const apply = (nodes: readonly SidebarNode[]): readonly SidebarNode[] => nodes.map(node => node._tag === 'Agent' ? node : { ...node, collapsed: collapsed.has(node.id), children: apply(node.children) })
    const props: AgentFoldersProps = {
      tree: apply(fixture), canDrop: () => ({ ok: true }), onMove: () => {}, onCreateFolder: () => {}, onRenameFolder: () => {}, onDeleteFolder: () => {},
      onToggleCollapsed: change => { toggles.push(change); setCollapsed(previous => { const next = new Set(previous); if (change.collapsed) next.add(change.id); else next.delete(change.id); return next }) },
      renderAgent: node => <span>{node.label}</span>,
    }
    return <AgentFolders {...props} />
  }
  await React.act(async () => { root.render(<Harness />) })
  const row = (label: string): HTMLElement => [...host.querySelectorAll<HTMLElement>('[role="row"]')].find(element => element.textContent?.trim() === label)!
  return { host, row, toggles, dispose: async () => { await React.act(async () => root.unmount()); host.remove() } }
}
describe('bound live folder tree', () => {
  it('derives levels and compact 8px offsets from RAC hierarchy, including folder members and Unfiled', async () => {
    const view = await mount()
    try {
      for (const [label, level, offset] of [['Alpha', 1, 0], ['Beta', 2, 8], ['Gamma', 3, 16], ['One', 2, 8], ['Two', 2, 8]] as const) {
        const row = view.row(label)
        expect(row, label).toBeDefined()
        expect(row.getAttribute('aria-level'), label).toBe(String(level))
        expect(Number.parseFloat(row.querySelector<HTMLElement>('[data-sidebar-content]')?.style.marginInlineStart ?? ''), label).toBe(offset)
      }
    } finally { await view.dispose() }
  })
  it('Right enters an expanded folder; Left moves to its parent, collapses, and Right expands', async () => {
    const view = await mount()
    const key = async (value: string) => { await React.act(async () => { document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true })) }) }
    try {
      await React.act(async () => view.row('Alpha').focus())
      await key('ArrowRight')
      expect(document.activeElement).toBe(view.row('Beta'))
      await key('ArrowLeft')
      expect(view.toggles).toContainEqual({ id: 'beta', collapsed: true })
      await key('ArrowLeft')
      expect(document.activeElement).toBe(view.row('Alpha'))
      await key('ArrowLeft')
      expect(view.toggles).toContainEqual({ id: 'alpha', collapsed: true })
      await key('ArrowRight')
      expect(view.toggles).toContainEqual({ id: 'alpha', collapsed: false })
    } finally { await view.dispose() }
  })
  describe.each(['root', 'folder', 'unfiled'] as const)('visible agent identity in %s', location => {
    it.each([undefined, 3])('names the tree row from rendered spans with unread=%s', async unread => {
      const host = document.createElement('div')
      document.body.append(host)
      const root = createRoot(host)
      const agent: SidebarNode = { _tag: 'Agent', id: 'agent/atlas', subject: 'agent/atlas', label: 'Atlas Builder' }
      const tree: readonly SidebarNode[] = location === 'root' ? [agent] : [{ _tag: location === 'folder' ? 'Folder' : 'Group', id: location, label: location === 'folder' ? 'Workshop' : 'Unfiled', collapsed: false, children: [agent] }]
      const projected = fleetFromAgents([decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/atlas', name: 'Atlas Builder', host_id: 'host/host-1', driver: 'codex', runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-04T12:00:00.000Z' })]).agents[0]!
      const item = { ...sidebarRow({ agent: projected, stale: false, now: 5000 }), unread }
      try {
        await React.act(async () => { root.render(<AgentFolders tree={tree} canDrop={() => ({ ok: true })} onMove={() => {}} onCreateFolder={() => {}} onRenameFolder={() => {}} onDeleteFolder={() => {}} onToggleCollapsed={() => {}} renderAgent={() => <SidebarAgentRow inTree item={item} now={5000} layout="SR2-B" extraSignals={['X-model']} />} />) })
        const row = within(host).getByRole('row', { name: 'Atlas Builder host-1 codex' })
        await expectA11y(row).toHaveAccessibleName('Atlas Builder host-1 codex')
        await expectA11y(row).toHaveAccessibleDescription(/idle/i)
        await expectA11y(row).not.toHaveAccessibleDescription(/unknown/i)
        if (unread === undefined) await expectA11y(row).not.toHaveAccessibleDescription(/unread/i)
        else await expectA11y(row).toHaveAccessibleDescription(/Unread: 3/)
        // RAC's old textValue-only name omits the visible host and harness.
        const labelledBy = row.getAttribute('aria-labelledby')!
        row.removeAttribute('aria-labelledby')
        expect(within(host).queryByRole('row', { name: 'Atlas Builder host-1 codex' })).toBeNull()
        row.setAttribute('aria-labelledby', labelledBy)
        await expectA11y(row).toHaveAccessibleName('Atlas Builder host-1 codex')
        const nextAgent = { ...agent, label: 'Orion Builder' }
        const nextTree = tree.map(node => node._tag === 'Agent' ? nextAgent : { ...node, children: [nextAgent] })
        const nextItem = { ...item, title: 'Orion Builder', status: 'working' as const, statusLabel: 'Working', unread: 4 }
        // Simulate RAC refreshing its own aria-* props on the reused row before the next commit.
        row.removeAttribute('aria-labelledby')
        row.removeAttribute('aria-describedby')
        await React.act(async () => { root.render(<AgentFolders tree={nextTree} canDrop={() => ({ ok: true })} onMove={() => {}} onCreateFolder={() => {}} onRenameFolder={() => {}} onDeleteFolder={() => {}} onToggleCollapsed={() => {}} renderAgent={() => <SidebarAgentRow inTree item={nextItem} now={5000} layout="SR2-B" extraSignals={['X-model']} />} />) })
        const updatedRow = within(host).getByRole('row', { name: 'Orion Builder host-1 codex' })
        expect(updatedRow).toBe(row)
        await expectA11y(updatedRow).toHaveAccessibleDescription(/Working/)
        await expectA11y(updatedRow).toHaveAccessibleDescription(/Unread: 4/)
      } finally {
        await React.act(async () => root.unmount())
        host.remove()
      }
    })
  })
})
