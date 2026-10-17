// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import type * as ReactMarkdownModule from 'react-markdown'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { Markdown, MarkdownImagePolicy } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Markdown.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const parses = vi.hoisted(() => ({ count: 0 }))
vi.mock('react-markdown', async importOriginal => {
  const actual = await importOriginal<typeof ReactMarkdownModule>()
  return { ...actual, default: (props: React.ComponentProps<typeof actual.default>) => {
    parses.count += 1
    return React.createElement(actual.default, props)
  } }
})

let container: HTMLDivElement
let root: Root
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  parses.count = 0
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

it('keeps unchanged message parsing stable across unrelated frames while references remain reactive', async () => {
  const frame = (revision: number, text = 'Existing **message** doc/foo.md') => <div data-frame={revision}>
    <MarkdownImagePolicy value={{ renderInlineReference: path => path === 'doc/foo.md' ? <span data-reference>{`Reference ${revision}`}</span> : undefined }}>
      <Markdown text={text} />
    </MarkdownImagePolicy>
  </div>
  await act(async () => root.render(frame(0)))
  expect(parses.count).toBe(1)
  expect(container.querySelector('strong')?.textContent).toBe('message')
  await act(async () => root.render(frame(1)))
  expect(parses.count).toBe(1)
  expect(container.querySelector('[data-reference]')?.textContent).toBe('Reference 1')
  await act(async () => root.render(frame(2, 'Changed **message** doc/foo.md')))
  expect(parses.count).toBe(2)
  expect(container.textContent).toContain('Changed message')
})
