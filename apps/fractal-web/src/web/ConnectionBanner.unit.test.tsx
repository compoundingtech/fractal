// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { ConnectionBanner } from './ConnectionBanner.tsx'
vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (values: unknown) => values, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))

it('keeps the same content node while connection chrome appears and disappears', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const render = (offline: boolean) => <><ConnectionBanner retained notice={offline ? { tone: 'offline', text: 'You are offline' } : undefined} /><article>Observed result</article></>
  try {
    await act(async () => root.render(render(false)))
    const content = host.querySelector('article')
    await act(async () => root.render(render(true)))
    expect(host.querySelector('article')).toBe(content)
    expect(host.querySelector('[role="status"]')?.textContent).toContain('Last observed content stays visible and may be out of date.')
    expect(host.querySelector('article')?.textContent).toBe('Observed result')
    await act(async () => root.render(render(false)))
    expect(host.querySelector('[data-connection-banner]')).toBeNull()
    expect(host.querySelector('article')).toBe(content)
  } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals() }
})

