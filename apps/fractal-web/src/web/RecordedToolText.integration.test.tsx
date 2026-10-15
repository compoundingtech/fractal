/** @vitest-environment jsdom */
import * as React from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { within } from 'storybook/test'
import { RecordedToolText } from './RecordedToolText.tsx'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (values: unknown) => values, createTheme: () => ({}), keyframes: () => '', props: () => ({}) }))

it('formats parsed JSON before previewing, reveals the complete formatted result, and preserves unparseable source', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const text = JSON.stringify({ path: 'src/catalog.ts', content: 'A catalog row with several readable fields.\n'.repeat(300), complete: true })
  const formatted = JSON.stringify(JSON.parse(text), null, 2)
  try {
    await React.act(async () => root.render(<RecordedToolText text={text} />))
    expect(container.querySelector('[data-tool-output]')?.textContent).toBe(formatted.slice(0, 4000))
    expect(container.querySelector('[data-recorded-text]')?.getAttribute('data-recorded-lines')).toBe('1')
    expect(container.querySelector('[data-recorded-text]')?.getAttribute('data-formatted-lines')).toBe('5')
    await React.act(async () => within(container).getByRole('button', { name: 'Show all 5 lines' }).click())
    expect(container.querySelector('[data-tool-output]')?.textContent).toBe(formatted)
    await React.act(async () => within(container).getByRole('button', { name: 'Show 12-line preview' }).click())
    expect(container.querySelector('[data-tool-output]')?.textContent).toBe(formatted.slice(0, 4000))
    const source = 'export const selectedRow = "row-a";\n'
    await React.act(async () => root.render(<RecordedToolText text={source} language="typescript" />))
    expect(container.querySelector('[data-tool-output]')?.textContent).toBe(source)
  } finally {
    await React.act(async () => root.unmount())
    container.remove()
    vi.unstubAllGlobals()
  }
})
