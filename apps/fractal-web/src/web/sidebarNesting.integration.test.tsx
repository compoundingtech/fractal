// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it, vi } from 'vitest'
import { DataSourceProvider } from '../data/react.tsx'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { nestingFolderId, nestingFolderSource, nestingSource } from './sidebarNesting.fixture.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
it('mounts authoritative nested folders in the live route and opens the real agent row', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('CSS', { escape: (value: string) => value.replace(/[^\w-]/g, char => `\\${char}`) })
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => undefined)
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  localStorage.clear()
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const registry = AtomRegistry.make()
  try {
    await act(async () => root.render(<DataSourceProvider source={nestingSource} registry={registry}><LiveAgentWorkspace folderSource={nestingFolderSource} /></DataSourceProvider>))
    const tree = host.querySelector('[aria-label="Agents and folders"]')!
    expect(tree).not.toBeNull()
    const row = tree.querySelector(`[data-key="folder:${nestingFolderId('a')}"]`)!
    const expand = async (id: string) => {
      const button = tree.querySelector<HTMLButtonElement>(`[data-key="${id}"] button[slot="chevron"]`)!
      expect(button).not.toBeNull()
      await act(async () => button.click())
    }
    expect(row.getAttribute('aria-level')).toBe('1')
    await expand(`folder:${nestingFolderId('a')}`)
    expect(tree.querySelector(`[data-key="folder:${nestingFolderId('b')}"]`)!.getAttribute('aria-level')).toBe('2')
    await expand(`folder:${nestingFolderId('b')}`)
    const agent = tree.querySelector<HTMLElement>('[data-key="agent/test/one"]')!
    expect(agent.getAttribute('aria-level')).toBe('3')
    expect(agent.querySelector('[data-testid="taste-agent-row"]')).not.toBeNull()
    expect(tree.textContent).not.toContain('dormant')
    expect(tree.textContent).not.toContain('machine/retained')
    await act(async () => agent.dispatchEvent(new MouseEvent('click', { bubbles: true })))
    expect(window.location.pathname).toBe('/w/agent/test/one')
    const input = host.querySelector<HTMLInputElement>('input[aria-label="Search agents"]')!
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 'one')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    expect(host.querySelector('[data-key="agent/test/one"]')!.getAttribute('aria-level')).toBe('3')
    expect(host.querySelector('[data-key="agent/test/unplaced"]')).toBeNull()
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 'machine:missing')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    expect(host.querySelector('[aria-label="Agents and folders"] [data-key]')).toBeNull()
  } finally {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
    vi.unstubAllGlobals()
    window.history.replaceState(null, '', '/')
  }
})
