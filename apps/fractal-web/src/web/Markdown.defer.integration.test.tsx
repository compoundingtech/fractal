// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { Markdown } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const grammar = vi.hoisted(() => {
  const { promise: ready, resolve: release } = Promise.withResolvers<void>()
  const { promise: finished, resolve: loaded } = Promise.withResolvers<void>()
  return { ready, release, finished, loaded, requested: false }
})
vi.mock('@pierre/highlights', async importOriginal => {
  grammar.requested = true
  await grammar.ready
  const actual = await importOriginal()
  grammar.loaded()
  return actual
})

it('defers labelled code coloring until after first paint and leaves unlabelled and unknown code readable', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const paints: FrameRequestCallback[] = []
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { paints.push(callback); return paints.length })
  const paint = () => { for (const callback of paints.splice(0)) callback(0) }
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () => root.render(<Markdown text={'```\nunlabelled <script>\n```'} />))
    await act(async () => { paint(); await vi.dynamicImportSettled() })
    expect(grammar.requested).toBe(false)
    expect(container.querySelector('code')?.textContent).toBe('unlabelled <script>\n')
    await act(async () => root.render(<Markdown text={'```\nunlabelled update\n```'} />))
    await act(async () => { paint(); await vi.dynamicImportSettled() })
    expect(grammar.requested).toBe(false)
    expect(container.querySelector('code')?.textContent).toBe('unlabelled update\n')
    await act(async () => root.render(<Markdown text={'## Ready now\n\nPlain **prose**.\n\n```unknown\nliteral <script>\n```'} />))
    expect(grammar.requested).toBe(false)
    expect(container.querySelector('h2')?.textContent).toBe('Ready now')
    expect(container.querySelector('strong')?.textContent).toBe('prose')
    expect(container.querySelector('code')?.textContent).toBe('literal <script>\n')
    await act(async () => root.render(<Markdown text={'## Ready now\n\n```typescript\nconst answer = 42\n```\n\n```unknown\nliteral <script>\n```'} />))
    expect(grammar.requested).toBe(false)
    await act(async () => { paint(); await vi.waitFor(() => expect(grammar.requested).toBe(true)) })
    expect(grammar.requested).toBe(true)
    const pre = container.querySelector('pre')!
    const code = pre.querySelector('code')!
    const wrap = container.querySelector<HTMLButtonElement>('[aria-label="Wrap code"]')!
    expect(pre.getAttribute('role')).toBe('group')
    expect(pre.getAttribute('aria-label')).toBe('Code, typescript')
    expect(code.textContent).toBe('const answer = 42\n')
    expect(code.querySelector('[data-syntax-token]')).toBeNull()
    const unknown = container.querySelectorAll('code')[1]!
    expect(unknown.textContent).toBe('literal <script>\n')
    expect(unknown.querySelector('[data-syntax-token]')).toBeNull()
    await act(async () => { wrap.focus(); wrap.click() })
    await act(async () => { grammar.release(); await grammar.finished })
    expect(container.querySelector('pre')).toBe(pre)
    expect(pre.querySelector('code')).toBe(code)
    expect(code.querySelector('[data-syntax-token="keyword"]')?.textContent).toBe('const')
    expect(code.textContent).toBe('const answer = 42\n')
    expect(wrap.getAttribute('aria-pressed')).toBe('true')
    expect(document.activeElement).toBe(wrap)
    expect(container.querySelectorAll('code')[1]).toBe(unknown)
    expect(unknown.textContent).toBe('literal <script>\n')
    expect(unknown.querySelector('[data-syntax-token]')).toBeNull()
  } finally {
    await act(async () => root.unmount())
    container.remove()
    vi.unstubAllGlobals()
  }
})
