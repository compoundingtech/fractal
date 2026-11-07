// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'
import { SidebarToolbar } from './Toolbar.tsx'
import { matchesRow, type SidebarRow } from './filter.ts'
import type { ShowRow } from './show.ts'
import { defaultFilters, type SidebarFilters } from './state.ts'
import { dailyDriver } from '../fixtures/layouts.ts'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))

afterEach(() => vi.unstubAllGlobals())

it('writes Filter menu choices into the query and settings, ANDs them with typed facets, counts them, and resets while preserving bare text and sort', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  // React Aria uses CSS.escape for generated collection keys; jsdom does not supply it.
  vi.stubGlobal('CSS', { escape: (value: string) => Array.from(value, char => /[a-zA-Z_-]/.test(char) ? char : `\\${char.codePointAt(0)?.toString(16)} `).join('') })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  let filters: SidebarFilters = { ...defaultFilters, query: 'compiler machine:host-a HOST:host', sort: 'name' }
  const showRows: readonly ShowRow[] = [{ host: 'host-a', asking: false, others: true }, { host: 'host-b', asking: true, others: true }]
  const update = (value: SidebarFilters) => { filters = value; render() }
  const render = () => root.render(<SidebarToolbar filters={filters} update={update} hosts={['host-a', 'host-b']} showRows={showRows} />)
  const row: SidebarRow = { workspace: { id: 'agent/compiler', title: 'Compiler', host: 'host-a', layout: dailyDriver, unread: [], notifications: [] }, agent: undefined, status: 'idle', needsMe: false, asking: false }
  const item = (name: string) => document.body.querySelector<HTMLElement>(`[role="menuitemcheckbox"][aria-label="${name}"]`)
  try {
    await act(async () => render())
    const search = host.querySelector<HTMLInputElement>('input[aria-label="Search agents"]')!
    expect(search.getAttribute('aria-description')).toBe('Free text matches agent names and IDs. Use is:asking for agents waiting for an answer, repo:, state:, or tool: to search other facts, and machine:build-1 or host:build-1,build-2 to filter host prefixes, ignoring case. Comma-separated values match any host; separate tokens must all match.')
    const trigger = () => [...host.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent?.startsWith('Filter'))!
    expect(host.querySelector('button[aria-label="Show finished agents"]')).toBeNull()
    expect(search.getAttribute('placeholder')).toBe('Find an agent')
    expect(trigger().textContent).toContain(', 2 active')
    await act(async () => trigger().click())
    // Typed facets check their entries; an unobserved typed machine stays listed so it can be cleared.
    expect(item('host-a, 1')?.getAttribute('aria-checked')).toBe('true')
    expect(item('host, 2')?.getAttribute('aria-checked')).toBe('true')
    expect(item('host-b, 1')?.getAttribute('aria-checked')).toBe('false')
    expect(item('Waiting for answer, 0')?.getAttribute('aria-checked')).toBe('false')

    await act(async () => item('host-b, 1')!.click())
    expect(filters.query).toBe('machine:host-a,host,host-b compiler')
    expect(matchesRow({ row, filters })).toBe(true)
    expect(item('Waiting for answer, 1')).not.toBeNull()

    await act(async () => item('Waiting for answer, 1')!.click())
    expect(filters.query).toBe('is:asking machine:host-a,host,host-b compiler')
    expect(matchesRow({ row, filters })).toBe(false)
    expect(matchesRow({ row: { ...row, asking: true }, filters })).toBe(true)
    expect(trigger().textContent).toContain(', 4 active')

    await act(async () => item('Waiting for answer, 1')!.click())
    expect(filters.query).toBe('machine:host-a,host,host-b compiler')

    // Statuses and finished agents live in the same menu, and stay checked as they toggle.
    await act(async () => item('Working, 0')!.click())
    expect(filters.statuses).toEqual(['working'])
    await act(async () => item('Show ended, 0')!.click())
    expect(filters.hideEnded).toBe(true)
    expect(trigger().textContent).toContain(', 5 active')
    expect(item('Show ended, 0')?.getAttribute('aria-checked')).toBe('false')

    // Rare statuses fold behind More until asked for.
    const menuItem = (name: string) => [...document.body.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(node => node.textContent === name)
    expect(item('Offline, 0')).toBeNull()
    await act(async () => menuItem('More')!.click())
    expect(item('Offline, 0')).not.toBeNull()
    expect(menuItem('Fewer')).not.toBeUndefined()

    await act(async () => menuItem('Reset filters')!.click())
    expect(filters).toEqual({ ...defaultFilters, query: 'compiler', sort: 'name' })
    expect(trigger().textContent).toBe('Filter')
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})
