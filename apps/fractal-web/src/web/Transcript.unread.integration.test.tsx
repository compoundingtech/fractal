// @vitest-environment jsdom
import { act } from 'react'
import * as React from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { EmbraceRuntimeProvider } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceRuntime.tsx'
import { Transcript, type TranscriptTurn } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const now = Date.parse('2026-11-07T12:00:00Z')
const at = new Date(now).toISOString()
const work = { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false }
const turns: readonly TranscriptTurn[] = [
  { id: 'turn-0', prompt: { _tag: 'Text', id: 'prompt-0', role: 'user', text: 'First ask', attachments: [], streaming: false, at }, items: [
    { _tag: 'Text', id: 'answer-0', role: 'assistant', text: 'Old answer', attachments: [], streaming: false, at },
  ], work },
  { id: 'turn-1', prompt: { _tag: 'Text', id: 'prompt-1', role: 'user', text: 'Second ask', attachments: [], streaming: false, at }, items: [
    { _tag: 'Text', id: 'answer-1a', role: 'assistant', text: 'Seen part', attachments: [], streaming: false, at },
    { _tag: 'Text', id: 'answer-1b', role: 'assistant', text: 'New part', attachments: [], streaming: false, at },
  ], work },
]

/** Records observed elements; a test reports which of them are in view. */
class FakeIntersectionObserver {
  static all = new Set<FakeIntersectionObserver>()
  readonly elements = new Set<Element>()
  constructor(readonly callback: IntersectionObserverCallback) { FakeIntersectionObserver.all.add(this) }
  observe(element: Element) { this.elements.add(element) }
  unobserve(element: Element) { this.elements.delete(element) }
  disconnect() { this.elements.clear(); FakeIntersectionObserver.all.delete(this) }
  takeRecords() { return [] }
}
const showTurns = async (ids: readonly string[]) => {
  await act(async () => {
    for (const observer of FakeIntersectionObserver.all) {
      const entries = [...observer.elements].map(target => ({ target, isIntersecting: ids.includes(target.getAttribute('data-item-id') ?? ''), intersectionRatio: 1 }) as unknown as IntersectionObserverEntry)
      observer.callback(entries, observer as unknown as IntersectionObserver)
    }
  })
}

let root: Root
let container: HTMLDivElement
const settle = async (ms = 50) => { await act(async () => { await new Promise(resolve => setTimeout(resolve, ms)) }) }
const render = async (props: Partial<React.ComponentProps<typeof Transcript>>) => {
  await act(async () => root.render(<EmbraceRuntimeProvider options={{ messages: turns.flatMap(entry => entry.prompt === undefined ? entry.items : [entry.prompt, ...entry.items]), isRunning: false, onNew: async () => {} }}>
    <Transcript turns={turns} title="Unread" sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} {...props} />
  </EmbraceRuntimeProvider>))
  await settle()
}
const divider = () => container.querySelector<HTMLElement>('[data-testid="transcript-unread"]')

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => setTimeout(() => callback(0), 0) as unknown as number)
  vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id))
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  FakeIntersectionObserver.all.clear()
  vi.unstubAllGlobals()
})

it('draws "New since you last looked" right before the first unread item, inside its turn', async () => {
  await render({ unreadFrom: 'answer-1b' })
  const line = divider()
  expect(line?.textContent).toBe('New since you last looked')
  expect(line?.getAttribute('role')).toBe('separator')
  expect(line?.hasAttribute('tabindex')).toBe(false)
  expect(line?.closest('[data-testid="transcript-turn"]')?.getAttribute('data-item-id')).toBe('turn-1')
  const text = container.textContent ?? ''
  expect(text.indexOf('Seen part')).toBeLessThan(text.indexOf('New since you last looked'))
  expect(text.indexOf('New since you last looked')).toBeLessThan(text.indexOf('New part'))
})

it('draws no divider when nothing is unread', async () => {
  await render({})
  expect(divider()).toBeNull()
})

it('reports the items of turns in view as seen, only while active and visible', async () => {
  const onSeen = vi.fn()
  await render({ onSeen })
  await showTurns(['turn-1'])
  expect(onSeen).toHaveBeenLastCalledWith(['prompt-1', 'answer-1a', 'answer-1b'])
  onSeen.mockClear()
  const hidden = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
  await showTurns(['turn-0'])
  expect(onSeen).not.toHaveBeenCalled()
  hidden.mockRestore()
  await render({ onSeen, active: false })
  await showTurns(['turn-0'])
  expect(onSeen).not.toHaveBeenCalled()
})
