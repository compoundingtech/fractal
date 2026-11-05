// @vitest-environment jsdom
import { setTimeout as delay } from 'node:timers/promises'
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { HighlightedSource, Markdown } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const grammar = vi.hoisted(() => {
  let release!: () => void
  let loaded!: () => void
  const ready = new Promise<void>(resolve => { release = resolve })
  const finished = new Promise<void>(resolve => { loaded = resolve })
  return { ready, release, finished, loaded, requested: false }
})
vi.mock('@pierre/highlights', async importOriginal => {
  grammar.requested = true
  await grammar.ready
  const actual = await importOriginal()
  grammar.loaded()
  return actual
})

it('keeps readable prose, code, focus and wrap state while the real highlighter loads', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const paints: FrameRequestCallback[] = []
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { paints.push(callback); return 1 })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () => root.render(<Markdown text={'## Ready now\n\nPlain **prose**.\n\n```unknown\nliteral <script>\n```'} />))
    expect(grammar.requested).toBe(false)
    expect(container.querySelector('h2')?.textContent).toBe('Ready now')
    expect(container.querySelector('strong')?.textContent).toBe('prose')
    expect(container.querySelector('code')?.textContent).toBe('literal <script>\n')
    // Flush every scheduled paint/timer: an unsupported label must never even
    // subscribe to the loader, rather than merely waiting until after paint.
    await act(async () => {
      for (const paint of paints.splice(0)) paint(0)
      await delay(30)
    })
    expect(paints).toHaveLength(0)
    expect(grammar.requested).toBe(false)
    await act(async () => root.render(<HighlightedSource code="literal" language="  NOT-A-LANGUAGE  " />))
    expect(paints).toHaveLength(0)
    expect(grammar.requested).toBe(false)
    await act(async () => root.render(<Markdown text={'## Ready now\n\n```typescript\nconst answer = 42\n```'} />))
    await act(async () => { for (const paint of paints.splice(0)) paint(0); await vi.waitFor(() => expect(grammar.requested).toBe(true)) })
    expect(grammar.requested).toBe(true)
    const pre = container.querySelector('pre')!
    const code = pre.querySelector('code')!
    const wrap = container.querySelector<HTMLButtonElement>('[aria-label="Wrap code"]')!
    expect(pre.getAttribute('role')).toBe('group')
    expect(pre.getAttribute('aria-label')).toBe('Code, typescript')
    expect(code.textContent).toBe('const answer = 42\n')
    expect(code.querySelector('[data-syntax-token]')).toBeNull()
    await act(async () => { wrap.focus(); wrap.click() })
    await act(async () => { grammar.release(); await grammar.finished })
    expect(container.querySelector('pre')).toBe(pre)
    expect(pre.querySelector('code')).toBe(code)
    expect(code.querySelector('[data-syntax-token="keyword"]')?.textContent).toBe('const')
    expect(code.textContent).toBe('const answer = 42\n')
    expect(wrap.getAttribute('aria-pressed')).toBe('true')
    expect(document.activeElement).toBe(wrap)
    await act(async () => root.render(<HighlightedSource code="const answer = 42" language="  Ts  " />))
    expect(container.querySelector('[data-syntax-token="keyword"]')?.textContent).toBe('const')
    expect(container.textContent).toBe('const answer = 42')
  } finally {
    await act(async () => root.unmount())
    container.remove()
    vi.unstubAllGlobals()
  }
})
