// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EmbraceRuntimeProvider } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceRuntime.tsx'
import { EmbraceScrollViewport, ViewportStore, ViewportStoreContext } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'
import { Transcript } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'
import type { TranscriptTurn } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'

// Only CSS and browser layout are supplied by the harness; all runtime, mounting,
// observer scheduling and scroll-controller behavior uses the real components.
vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const now = Date.parse('2026-10-17T12:30:00Z')
const rows = Array.from({ length: 80 }, (_, index) => ({ id: `turn-${index}`, version: String(index) }))
const turns: readonly TranscriptTurn[] = rows.map(({ id }, index) => ({
  id,
  prompt: { _tag: 'Text', id: `prompt-${index}`, role: 'user', text: `Historical prompt ${index}`, attachments: [], streaming: false, at: new Date(now).toISOString() },
  items: [],
  work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false },
}))

let root: Root
let container: HTMLDivElement
let nextFrame: number
let frames: Map<number, FrameRequestCallback>
let observers: Set<{ callback: ResizeObserverCallback; elements: Set<Element> }>
let spanHeight: number
let spanScrollHeight: number
let originalScrollIntoView: PropertyDescriptor | undefined

const renderedRows = (lane: Element) => [...lane.querySelectorAll<HTMLElement>('[data-test-tail-row], [data-testid="transcript-turn"]')]
const rect = (top: number, height: number) => ({ top, bottom: top + height, left: 0, right: 600, width: 600, height, x: 0, y: top, toJSON() {} })

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  originalScrollIntoView = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollIntoView')
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { configurable: true, value: vi.fn() })
  frames = new Map()
  nextFrame = 0
  observers = new Set()
  spanHeight = 100
  spanScrollHeight = 100
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    frames.set(++nextFrame, callback)
    return nextFrame
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id))
  vi.stubGlobal('ResizeObserver', class {
    readonly entry: { callback: ResizeObserverCallback; elements: Set<Element> }
    constructor(callback: ResizeObserverCallback) {
      this.entry = { callback, elements: new Set() }
      observers.add(this.entry)
    }
    observe(element: Element) { this.entry.elements.add(element) }
    unobserve(element: Element) { this.entry.elements.delete(element) }
    disconnect() { observers.delete(this.entry) }
  })
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(function (this: HTMLElement) {
    return this.tagName === 'SPAN' ? spanHeight : 50
  })
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600)
  vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(function (this: HTMLElement) {
    return this.tagName === 'SPAN' ? spanScrollHeight : renderedRows(this).length * 100
  })
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const lane = this.closest<HTMLElement>('[data-testid="transcript-scroll"]')
    if (lane !== null && this !== lane) {
      const row = this.closest<HTMLElement>('[data-test-tail-row], [data-testid="transcript-turn"]')
      if (row !== null) return rect(renderedRows(lane).indexOf(row) * 100 - lane.scrollTop, 100)
    }
    return rect(0, 50)
  })
  vi.stubGlobal('CSS', { ...globalThis.CSS, escape: (value: string) => value.replace(/[^a-zA-Z0-9_-]/g, character => `\\${character}`) })
  Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: () => {
    const lane = container.querySelector<HTMLElement>('[data-testid="transcript-scroll"]')
    return lane === null ? null : renderedRows(lane).find(row => row.getBoundingClientRect().bottom > 0) ?? null
  } })
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  Reflect.deleteProperty(document, 'elementFromPoint')
  if (originalScrollIntoView === undefined) Reflect.deleteProperty(HTMLElement.prototype, 'scrollIntoView')
  else Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', originalScrollIntoView)
  window.history.replaceState(null, '', window.location.pathname)
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

// Each drain runs one browser frame and then React's resulting commits. Settling
// frames are deterministic, with no wall-clock sleeps or fake component logic.
const flushFrames = async () => {
  for (let frame = 0; frame < 6 && frames.size > 0; frame++) {
    await act(async () => {
      const pending = [...frames.values()]
      frames.clear()
      pending.forEach(callback => callback(frame * 16))
    })
  }
}
const render = async (element: React.ReactNode) => {
  await act(async () => root.render(element))
  await flushFrames()
}
const lane = () => container.querySelector<HTMLElement>('[data-testid="transcript-scroll"]')!
const viewport = (key: string, store?: ViewportStore) => <ViewportStoreContext value={store}>
  <EmbraceScrollViewport items={rows} stateKey={key} initialTailCount={2} data-testid="transcript-scroll">
    {startIndex => rows.slice(startIndex).map(row => <div key={row.id} data-test-tail-row data-item-id={row.id}>{row.id}</div>)}
  </EmbraceScrollViewport>
</ViewportStoreContext>
const transcript = (entries: readonly TranscriptTurn[] = turns) => <EmbraceRuntimeProvider options={{ messages: entries.flatMap(turn => turn.prompt === undefined ? turn.items : [turn.prompt, ...turn.items]), isRunning: false, onNew: async () => {} }}>
  <Transcript turns={entries} title="Tail history" sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} viewportKey="tail-test" />
</EmbraceRuntimeProvider>
const scrollUp = async (top: number) => {
  await act(async () => {
    lane().dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }))
    lane().scrollTop = top
    lane().dispatchEvent(new Event('scroll'))
  })
  await flushFrames()
}

describe('tail-first transcript mounting', () => {
  it('mounts the latest rows first and prepends older rows without moving the reading anchor', async () => {
    await render(viewport('A'))
    const initial = renderedRows(lane())
    expect(initial.length).toBeLessThan(rows.length)
    expect(initial.at(-1)?.dataset.itemId).toBe('turn-79')
    expect(lane().scrollTop).toBe(lane().scrollHeight - lane().clientHeight)
    const anchor = initial[0]!
    await scrollUp(25)
    expect(renderedRows(lane()).length).toBeGreaterThan(initial.length)
    expect(renderedRows(lane()).at(-1)?.dataset.itemId).toBe('turn-79')
    expect(anchor.isConnected).toBe(true)
    expect(anchor.getBoundingClientRect().top).toBe(-25)
  })

  it('starts a switched conversation at its tail rather than carrying over a detached scroll', async () => {
    await render(viewport('A'))
    await scrollUp(25)
    await render(viewport('B'))
    expect(renderedRows(lane()).at(-1)?.dataset.itemId).toBe('turn-79')
    expect(lane().scrollTop).toBe(lane().scrollHeight - lane().clientHeight)
    expect(renderedRows(lane()).length).toBeLessThan(rows.length)
  })

  it('restores the expanded row window together with its saved reading coordinate', async () => {
    const store = new ViewportStore()
    await render(viewport('A', store))
    await scrollUp(25)
    const first = renderedRows(lane())[0]!.dataset.itemId
    const top = lane().scrollTop
    await render(viewport('B', store))
    await render(viewport('A', store))
    expect(renderedRows(lane())[0]?.dataset.itemId).toBe(first)
    expect(lane().scrollTop).toBe(top)
  })

  it('materializes an old turn for native browser find without mounting every turn initially', async () => {
    await render(transcript())
    expect(container.querySelectorAll('[data-testid="transcript-turn"]').length).toBeLessThan(turns.length)
    expect(container.querySelector('[data-testid="transcript-turn"][data-item-id="turn-0"]')).toBeNull()
    const search = container.querySelector<HTMLElement>('[data-transcript-search-id="turn-0"]')!
    expect(search).not.toBeNull()
    expect(search.getAttribute('hidden')).toBe('until-found')
    expect(search.textContent).toContain('Historical prompt 0')
    await act(async () => { search.dispatchEvent(new Event('beforematch', { bubbles: true })) })
    await flushFrames()
    expect(container.querySelector('[data-testid="transcript-turn"][data-item-id="turn-0"]')?.textContent).toContain('Historical prompt 0')
  })

  it('makes the full history available to native Find without cancelling the browser shortcut', async () => {
    await render(transcript())
    expect(container.querySelectorAll('[data-testid="transcript-turn"]').length).toBeLessThan(turns.length)
    const find = new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true, cancelable: true })
    await act(async () => { document.dispatchEvent(find) })
    await flushFrames()
    expect(find.defaultPrevented).toBe(false)
    expect(container.querySelectorAll('[data-testid="transcript-turn"]')).toHaveLength(turns.length)
    expect(container.querySelector('[data-testid="transcript-turn"][data-item-id="turn-0"]')?.textContent).toContain('Historical prompt 0')
  })

  it('reveals an unmounted old message before jumping to its hash target', async () => {
    const scrollIntoView = vi.fn()
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { configurable: true, value: scrollIntoView })
    try {
      await render(transcript())
      expect(container.querySelector('[data-testid="user-message"][data-item-id="prompt-0"]')).toBeNull()
      await act(async () => {
        window.history.replaceState(null, '', '#prompt-0')
        window.dispatchEvent(new HashChangeEvent('hashchange'))
      })
      await flushFrames()
      const target = container.querySelector('[data-testid="user-message"][data-item-id="prompt-0"]')
      expect(target).not.toBeNull()
      expect(scrollIntoView.mock.contexts).toContain(target)
    } finally {
      Reflect.deleteProperty(HTMLElement.prototype, 'scrollIntoView')
    }
  })
})

describe('batched message measurements', () => {
  it('updates the overflow disclosure after ResizeObserver reports a narrower and then wider message', async () => {
    await render(transcript(turns.slice(-1)))
    const message = container.querySelector<HTMLElement>('[data-testid="user-message"]')!
    const text = message.querySelector('span')!
    expect(message.textContent).not.toContain('Show full message')
    const resizeText = async () => {
      await act(async () => {
        for (const observer of observers) if (observer.elements.has(text)) {
          observer.callback([{ target: text, contentRect: text.getBoundingClientRect(), borderBoxSize: [], contentBoxSize: [], devicePixelContentBoxSize: [] }], {} as ResizeObserver)
        }
      })
      await flushFrames()
    }
    spanScrollHeight = 180
    await resizeText()
    expect(message.textContent).toContain('Show full message')
    expect(message.querySelector('button')?.getAttribute('aria-expanded')).toBe('false')
    spanScrollHeight = 100
    await resizeText()
    expect(message.textContent).not.toContain('Show full message')
  })

  it('keeps localized completion times and machine instants when formatting multiple answer rows', async () => {
    const entries: readonly TranscriptTurn[] = [0, 1].map(index => ({
      ...turns[index]!,
      items: [{ _tag: 'Text', id: `answer-${index}`, role: 'assistant', text: `Answer ${index}`, attachments: [], streaming: false, at: new Date(now + index * 60_000).toISOString() }],
    }))
    await render(transcript(entries))
    const times = [...container.querySelectorAll<HTMLTimeElement>('[data-testid="message-time"]')]
    expect(times).toHaveLength(2)
    times.forEach((time, index) => {
      const completed = new Date(now + index * 60_000)
      expect(time.dateTime).toBe(completed.toISOString())
      expect(time.textContent).toBe(new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' }).format(completed))
    })
  })
})
