// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import type { WorkLogCall, WorkLogTurn } from '@fractal/ui/assistant-ui/work-log'
import { ViewportStore, ViewportStoreContext } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'
import { WorkLogV1 } from '../../../../packages/fractal-ui/src/assistant-ui/taste/WorkLogV1.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const at = '2026-11-07T12:00:00.000Z'
const call = (id: string): WorkLogCall => ({ id, kind: 'read', title: `Read ${id}`, status: 'success', startedAt: at, endedAt: at })
const live = (ids: readonly string[]): WorkLogTurn => ({ calls: ids.map(call), durationMs: undefined, running: true, failed: false, interrupted: false })

let root: Root
let container: HTMLDivElement
let store: ViewportStore
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  store = new ViewportStore()
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})
const render = (turn: WorkLogTurn) => act(async () => root.render(<ViewportStoreContext value={store}><WorkLogV1 turn={turn} summaryAnchorId="turn-1" hideLiveRow /></ViewportStoreContext>))
const header = () => container.querySelector<HTMLButtonElement>('[data-testid="work-log"] > button')!
const children = () => container.querySelector('[data-testid="work-log-children"]')
const toggle = () => act(async () => header().click())

it('collapses a running work log from its header and reopens it', async () => {
  await render(live(['a']))
  expect(header().textContent).toContain('Working')
  expect(header().disabled).toBe(false)
  expect(header().getAttribute('aria-expanded')).toBe('true')
  expect(children()).not.toBeNull()
  await toggle()
  expect(header().getAttribute('aria-expanded')).toBe('false')
  expect(children()).toBeNull()
  await toggle()
  expect(header().getAttribute('aria-expanded')).toBe('true')
  expect(children()).not.toBeNull()
})

it('toggles from the keyboard while running', async () => {
  await render(live(['a']))
  header().focus()
  expect(document.activeElement).toBe(header())
  await act(async () => {
    header().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }))
    document.activeElement!.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }))
  })
  expect(header().getAttribute('aria-expanded')).toBe('false')
  expect(document.activeElement).toBe(header())
})

it('stays collapsed through new tool updates and when the turn settles', async () => {
  await render(live(['a']))
  await toggle()
  await render(live(['a', 'b']))
  await render(live(['a', 'b', 'c']))
  expect(header().getAttribute('aria-expanded')).toBe('false')
  expect(children()).toBeNull()
  await render({ ...live(['a', 'b', 'c']), running: false, durationMs: 1000 })
  expect(header().getAttribute('aria-expanded')).toBe('false')
})

it('remembers the choice for that turn across a remount', async () => {
  await render(live(['a']))
  await toggle()
  await act(async () => root.unmount())
  root = createRoot(container)
  await render(live(['a', 'b']))
  expect(header().getAttribute('aria-expanded')).toBe('false')
})

it('still opens by default while running and folds by default once settled', async () => {
  await render(live(['a']))
  expect(header().getAttribute('aria-expanded')).toBe('true')
  await render({ ...live(['a']), running: false, durationMs: 1000 })
  expect(header().getAttribute('aria-expanded')).toBe('false')
})

it('keeps a failure visible when a failed run is collapsed', async () => {
  await render({ ...live(['a']), running: false, failed: true, failureNote: 'Run failed.' })
  await toggle()
  expect(header().getAttribute('aria-expanded')).toBe('false')
  expect(children()).toBeNull()
  expect(container.textContent).toContain('Run failed')
})
