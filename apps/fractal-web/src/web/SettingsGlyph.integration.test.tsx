// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { ChromeControl } from '../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it('draws the settings control as a cog: one closed toothed outline around a hole, never separate rays', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () => root.render(<ChromeControl icon="gear" label="Settings" />))
    const glyph = host.querySelector('button[aria-label="Settings"] svg')!
    const outlines = [...glyph.querySelectorAll('path')].map(path => path.getAttribute('d') ?? '')
    expect(outlines).toHaveLength(1)
    // One subpath, closed: a sun's rays are many open strokes.
    expect(outlines[0]!.match(/[Mm]/g)).toHaveLength(1)
    expect(outlines[0]!.trim()).toMatch(/z$/i)
    // Teeth: an outline alternating between two radii needs many points; a plain ring or a few rays has far fewer.
    expect(outlines[0]!.match(/-?\d*\.?\d+/g)!.length).toBeGreaterThanOrEqual(24)
    expect(glyph.querySelectorAll('circle')).toHaveLength(1)
  } finally {
    await act(async () => root.unmount())
    host.remove()
    vi.unstubAllGlobals()
  }
})
