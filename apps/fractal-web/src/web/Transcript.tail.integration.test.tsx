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
import { observeUserMessageOverflow } from '../../../../packages/fractal-ui/src/assistant-ui/composition/user-message-overflow.ts'

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
let viewportHeight: number
let viewportWidth: number

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
  viewportHeight = 50
  viewportWidth = 600
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
    return this.tagName === 'SPAN' ? spanHeight : viewportHeight
  })
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => viewportWidth)
  vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(function (this: HTMLElement) {
    return this.tagName === 'SPAN' ? spanScrollHeight : renderedRows(this).length * 100
  })
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const lane = this.closest<HTMLElement>('[data-testid="transcript-scroll"]')
    if (lane !== null && this !== lane) {
      const row = this.closest<HTMLElement>('[data-test-tail-row], [data-testid="transcript-turn"]')
      if (row !== null) return rect(renderedRows(lane).indexOf(row) * 100 - lane.scrollTop, 100)
    }
    return rect(0, viewportHeight)
  })
  vi.stubGlobal('CSS', { ...globalThis.CSS, escape: (value: string) => value.replace(/[^a-zA-Z0-9_-]/g, character => `\\${character}`) })
  Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: () => {
    const lane = container.querySelector<HTMLElement>('[data-testid="transcript-scroll"]')
    // The harness uses uniform 100px rows; avoid re-querying all DOM rows once per hit-test candidate.
    return lane === null ? null : renderedRows(lane)[Math.floor(lane.scrollTop / 100)] ?? null
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
const flushFrames = async (count = 6, afterFrame?: (frame: number) => void) => {
  for (let frame = 0; frame < count && frames.size > 0; frame++) {
    await act(async () => {
      const pending = [...frames.values()]
      frames.clear()
      pending.forEach(callback => callback(frame * 16))
    })
    afterFrame?.(frame)
  }
}
const flushHistory = (count: number, afterFrame?: (frame: number) => void) =>
  flushFrames(Math.ceil(count / 4) + 6, afterFrame)
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
const transcript = (entries: readonly TranscriptTurn[] = turns, active = true) => <EmbraceRuntimeProvider options={{ messages: entries.flatMap(turn => turn.prompt === undefined ? turn.items : [turn.prompt, ...turn.items]), isRunning: false, onNew: async () => {} }}>
  <Transcript turns={entries} active={active} title="Tail history" sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} viewportKey="tail-test" />
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
  it.each([3, 6].flatMap(count => [440, 1440].map(width => ({ count, width }))))('keeps all $count turns mounted at $width px even when the short history overflows', async ({ count, width }) => {
    viewportWidth = width
    const entries = turns.slice(0, count)
    await render(transcript(entries))
    expect(lane().scrollHeight).toBeGreaterThan(2 * lane().clientHeight)
    expect(renderedRows(lane()).map(row => row.dataset.itemId)).toEqual(entries.map(turn => turn.id))
    expect(container.querySelector('[data-testid="transcript-history-earlier"]')).toBeNull()
    expect(container.querySelector('[data-testid="transcript-history-all"]')).toBeNull()
  })

  it('pins the latest rows before the first animation frame, including after a conversation switch', async () => {
    await act(async () => { root.render(viewport('A')) })
    expect(lane().scrollTop).toBe(lane().scrollHeight - lane().clientHeight)
    await flushFrames()
    await scrollUp(25)
    await act(async () => { root.render(viewport('B')) })
    expect(lane().scrollTop).toBe(lane().scrollHeight - lane().clientHeight)
  })

  it('fills the resized viewport when its width and height change during the initial mount', async () => {
    await act(async () => { root.render(viewport('A')) })
    viewportWidth = 300
    viewportHeight = 150
    await act(async () => {
      for (const observer of observers) {
        observer.callback([...observer.elements].map(target => ({ target, contentRect: target.getBoundingClientRect(), borderBoxSize: [], contentBoxSize: [], devicePixelContentBoxSize: [] })), {} as ResizeObserver)
      }
    })
    await flushFrames()
    expect(lane().clientWidth).toBe(300)
    expect(lane().scrollHeight).toBeGreaterThanOrEqual(2 * lane().clientHeight)
    expect(lane().scrollTop).toBe(lane().scrollHeight - lane().clientHeight)
    expect(renderedRows(lane()).length).toBeLessThan(rows.length)
  })

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

  it('offers accessible full-history actions instead of incomplete hidden browser-find proxies', async () => {
    await render(transcript())
    expect(container.querySelectorAll('[data-testid="transcript-turn"]').length).toBeLessThan(turns.length)
    expect(container.querySelector('[hidden="until-found"]')).toBeNull()
    const notice = [...container.querySelectorAll('[role="status"]')].find(node => node.textContent?.includes('Browser Find'))
    expect(notice?.textContent).toMatch(/Browser Find misses \d+ earlier turns until you choose Show all messages/)
    const controls = container.querySelector<HTMLElement>('[data-testid="transcript-history-controls"]')!
    expect(controls.getAttribute('role')).toBe('group')
    expect(controls.getAttribute('aria-label')).toBe('Conversation history range')
    expect(controls.closest('[data-testid="transcript-scroll"]')).toBeNull()
    expect(controls.contains(notice!)).toBe(true)
    const summary = controls.querySelector<HTMLElement>('[data-testid="transcript-history-summary"]')!
    expect(summary.textContent).toContain('Find: shown only')
    expect(summary.hidden).toBe(false)
    expect(summary.closest('[hidden]')).toBeNull()
    expect(summary.getAttribute('aria-hidden')).toBe('true')
    expect(lane().scrollTop).toBe(lane().scrollHeight - lane().clientHeight)
    const earlier = container.querySelector<HTMLButtonElement>('button[aria-label="Show earlier messages"]')!
    const all = container.querySelector<HTMLButtonElement>('button[aria-label="Show all messages"]')!
    expect(earlier).toBeDefined()
    expect(all).toBeDefined()
    expect(controls.contains(earlier)).toBe(true)
    expect(controls.contains(all)).toBe(true)
    const count = renderedRows(lane()).length
    await act(async () => {
      earlier.focus()
      earlier.click()
    })
    await flushFrames()
    expect(renderedRows(lane())).toHaveLength(count + 4)
    expect(document.activeElement).toBe(earlier)
    await act(async () => { all.click() })
    await flushHistory(turns.length)
    expect(container.querySelectorAll('[data-testid="transcript-turn"]')).toHaveLength(turns.length)
    expect(container.querySelector('[data-testid="transcript-turn"][data-item-id="turn-0"]')?.textContent).toContain('Historical prompt 0')
  })

  it('keeps the focused scroll lane and visible anchor while prepending through the keyboard', async () => {
    await render(viewport('A'))
    const anchor = renderedRows(lane())[0]!
    await act(async () => {
      lane().tabIndex = 0
      lane().focus()
      lane().dispatchEvent(new KeyboardEvent('keydown', { key: 'PageUp', bubbles: true }))
      lane().scrollTop = 25
      lane().dispatchEvent(new Event('scroll'))
    })
    await flushFrames()
    expect(document.activeElement).toBe(lane())
    expect(anchor.getBoundingClientRect().top).toBe(-25)
  })

  it('progressively makes full history available without cancelling the native Find shortcut', async () => {
    await render(transcript())
    expect(container.querySelectorAll('[data-testid="transcript-turn"]').length).toBeLessThan(turns.length)
    const find = new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true, cancelable: true })
    await act(async () => { document.dispatchEvent(find) })
    await flushHistory(turns.length)
    expect(find.defaultPrevented).toBe(false)
    expect(container.querySelectorAll('[data-testid="transcript-turn"]')).toHaveLength(turns.length)
    expect(container.querySelector('[data-testid="transcript-turn"][data-item-id="turn-0"]')?.textContent).toContain('Historical prompt 0')
  })

  it.each(['Show all messages', 'Ctrl-F', 'Cmd-F'])('bounds %s preparation across 520 turns and preserves focus and the reading anchor in every chunk', async action => {
    const entries: readonly TranscriptTurn[] = Array.from({ length: 520 }, (_, index) => ({
      ...turns[0]!,
      id: `long-turn-${index}`,
      prompt: { ...turns[0]!.prompt!, id: `long-prompt-${index}`, text: `Long history prompt ${index}` },
    }))
    await render(transcript(entries))
    await scrollUp(25)
    const hitTest = vi.spyOn(document, 'elementFromPoint')
    const anchor = renderedRows(lane()).find(row => row.getBoundingClientRect().bottom > 0)!
    const top = anchor.getBoundingClientRect().top
    const button = container.querySelector<HTMLButtonElement>('button[aria-label="Show all messages"]')!
    const focused = action === 'Show all messages' ? button : lane()
    let previousCount = renderedRows(lane()).length
    await act(async () => {
      focused.tabIndex = 0
      focused.focus()
      if (action === 'Show all messages') button.click()
      else document.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', ctrlKey: action === 'Ctrl-F', metaKey: action === 'Cmd-F', bubbles: true, cancelable: true }))
    })
    expect(renderedRows(lane()).length - previousCount).toBeLessThanOrEqual(4)
    expect(renderedRows(lane()).length).toBeLessThan(entries.length)
    const controls = container.querySelector<HTMLElement>('[data-testid="transcript-history-controls"]')!
    expect(controls.closest('[data-testid="transcript-scroll"]')).toBeNull()
    expect(controls.getAttribute('aria-busy')).toBe('true')
    expect(controls.textContent).toContain('Browser Find may miss earlier turns until this finishes.')
    expect(controls.querySelector('[data-testid="transcript-history-summary"]')?.textContent).toContain('Find: preparing')
    previousCount = renderedRows(lane()).length
    let chunks = 0
    await flushHistory(entries.length, () => {
      const count = renderedRows(lane()).length
      expect(count - previousCount).toBeGreaterThanOrEqual(0)
      expect(count - previousCount).toBeLessThanOrEqual(4)
      if (count > previousCount) chunks += 1
      expect(anchor.isConnected).toBe(true)
      expect(anchor.getBoundingClientRect().top).toBe(top)
      expect(hitTest).not.toHaveBeenCalled()
      expect(document.activeElement).toBe(count === entries.length && action === 'Show all messages' ? lane() : focused)
      if (count < entries.length) {
        expect(controls.getAttribute('aria-busy')).toBe('true')
        expect(controls.querySelector('[data-testid="transcript-history-all"]')).toBe(button)
        expect(controls.textContent).toContain('Browser Find may miss earlier turns until this finishes.')
        expect(controls.querySelector('[data-testid="transcript-history-summary"]')?.textContent).toContain('Find: preparing')
      }
      previousCount = count
    })
    expect(chunks).toBeGreaterThan(100)
    expect(hitTest).not.toHaveBeenCalled()
    expect(renderedRows(lane())).toHaveLength(entries.length)
    expect(container.querySelector('[data-item-id="long-prompt-0"]')?.textContent).toContain('Long history prompt 0')
    expect(controls.getAttribute('aria-busy')).toBe('false')
    expect(controls.textContent).toContain('Browser Find searches all loaded messages.')
    expect(controls.querySelector('[data-testid="transcript-history-summary"]')?.textContent).toContain('Find: all loaded')
  }, 30_000)

  it('recaptures a new reader anchor after scrolling during preparation and reuses it for the remaining chunks', async () => {
    await render(transcript())
    await scrollUp(25)
    const originalAnchor = renderedRows(lane()).find(row => row.getBoundingClientRect().bottom > 0)!
    await act(async () => { container.querySelector<HTMLButtonElement>('button[aria-label="Show all messages"]')!.click() })
    await flushFrames(2)
    const hitTest = vi.spyOn(document, 'elementFromPoint')
    await act(async () => {
      lane().dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }))
      lane().scrollTop -= 100
      lane().dispatchEvent(new Event('scroll'))
    })
    const anchor = renderedRows(lane()).find(row => row.getBoundingClientRect().bottom > 0)!
    const top = anchor.getBoundingClientRect().top
    expect(anchor).not.toBe(originalAnchor)
    await flushFrames(2, () => {
      expect(anchor.isConnected).toBe(true)
      expect(anchor.getBoundingClientRect().top).toBe(top)
    })
    expect(hitTest).toHaveBeenCalled()
    hitTest.mockClear()
    await flushHistory(turns.length, () => {
      expect(anchor.isConnected).toBe(true)
      expect(anchor.getBoundingClientRect().top).toBe(top)
      expect(hitTest).not.toHaveBeenCalled()
    })
    expect(renderedRows(lane())).toHaveLength(turns.length)
    expect(anchor.getBoundingClientRect().top).toBe(top)
  })

  it('exposes progressive preparation to viewport children until the last chunk commits', async () => {
    await render(<EmbraceScrollViewport items={rows} stateKey="progress" initialTailCount={2} data-testid="transcript-scroll">
      {(startIndex, revealRow, materializing) => <>
        <button data-prepare-history onClick={() => revealRow(rows[0]!.id, true)}>Prepare history</button>
        <span data-materializing={String(materializing)} />
        {rows.slice(startIndex).map(row => <div key={row.id} data-test-tail-row data-item-id={row.id}>{row.id}</div>)}
      </>}
    </EmbraceScrollViewport>)
    let count = renderedRows(lane()).length
    await act(async () => { container.querySelector<HTMLButtonElement>('[data-prepare-history]')!.click() })
    expect(container.querySelector('[data-materializing="true"]')).not.toBeNull()
    expect(renderedRows(lane()).length - count).toBeLessThanOrEqual(4)
    count = renderedRows(lane()).length
    await flushHistory(rows.length, () => {
      const nextCount = renderedRows(lane()).length
      expect(nextCount - count).toBeLessThanOrEqual(4)
      if (nextCount < rows.length) expect(container.querySelector('[data-materializing="true"]')).not.toBeNull()
      count = nextCount
    })
    expect(renderedRows(lane())).toHaveLength(rows.length)
    expect(container.querySelector('[data-materializing="false"]')).not.toBeNull()
  })

  it('preserves a detached reading anchor when Ctrl-F materializes the full transcript', async () => {
    await render(transcript())
    await scrollUp(25)
    const anchor = renderedRows(lane()).find(row => row.getBoundingClientRect().bottom > 0)!
    const top = anchor.getBoundingClientRect().top
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true, cancelable: true }))
    })
    await flushHistory(turns.length)
    expect(renderedRows(lane())).toHaveLength(turns.length)
    expect(anchor.getBoundingClientRect().top).toBe(top)
  })

  it('pauses pending full-history preparation while inactive and retains streaming updates and the reading anchor on resume', async () => {
    const streamingTurn = (text: string): TranscriptTurn => ({
      ...turns.at(-1)!,
      items: [{ _tag: 'Text', id: 'paused-streaming-answer', role: 'assistant', text, attachments: [], streaming: true, at: new Date(now).toISOString() }],
    })
    let entries: readonly TranscriptTurn[] = [...turns.slice(0, -1), streamingTurn('Before pausing')]
    await render(transcript(entries))
    await scrollUp(25)
    const anchor = renderedRows(lane()).find(row => row.getBoundingClientRect().bottom > 0)!
    const top = anchor.getBoundingClientRect().top
    await act(async () => { container.querySelector<HTMLButtonElement>('button[aria-label="Show all messages"]')!.click() })
    await flushFrames(2)
    expect(renderedRows(lane()).length).toBeLessThan(entries.length)
    expect(container.querySelector('[data-testid="transcript-history-controls"]')?.getAttribute('aria-busy')).toBe('true')
    await act(async () => root.render(transcript(entries, false)))
    const pausedRows = renderedRows(lane())
    await flushFrames(12)
    expect(renderedRows(lane())).toEqual(pausedRows)
    const appended: TranscriptTurn = { ...turns[0]!, id: 'paused-new-turn', prompt: { ...turns[0]!.prompt!, id: 'paused-new-prompt', text: 'Arrived while inactive' } }
    entries = [
      { ...turns[0]!, prompt: { ...turns[0]!.prompt!, text: 'Old prompt updated while inactive' } },
      ...turns.slice(1, -1),
      streamingTurn('An **inactive update**'),
      appended,
    ]
    await act(async () => root.render(transcript(entries, false)))
    await flushFrames(12)
    expect(renderedRows(lane()).map(row => row.dataset.itemId)).toEqual([...pausedRows.map(row => row.dataset.itemId), appended.id])
    expect(container.querySelector('[data-item-id="prompt-0"]')).toBeNull()
    expect(anchor.isConnected).toBe(true)
    expect(anchor.getBoundingClientRect().top).toBe(top)
    await act(async () => root.render(transcript(entries, true)))
    await flushFrames(2)
    expect(renderedRows(lane()).length).toBeLessThan(entries.length)
    expect(container.querySelector('[data-item-id="paused-streaming-answer"] strong')?.textContent).toBe('inactive update')
    expect(container.querySelector('[data-item-id="paused-new-prompt"]')?.textContent).toContain('Arrived while inactive')
    entries = [...entries.slice(0, -2), streamingTurn('An **active preparation update**'), appended]
    await act(async () => root.render(transcript(entries)))
    await flushHistory(entries.length, () => {
      expect(anchor.isConnected).toBe(true)
      expect(container.querySelector(`[data-testid="transcript-turn"][data-item-id="${anchor.dataset.itemId}"]`)).toBe(anchor)
      expect(anchor.getBoundingClientRect().top).toBe(top)
    })
    expect(renderedRows(lane())).toHaveLength(entries.length)
    expect(container.querySelector('[data-item-id="prompt-0"]')?.textContent).toContain('Old prompt updated while inactive')
    expect(container.querySelector('[data-item-id="paused-streaming-answer"] strong')?.textContent).toBe('active preparation update')
    expect(container.querySelector('[data-item-id="paused-streaming-answer"]')?.textContent).not.toContain('Before pausing')
    expect(container.querySelector('[data-item-id="paused-new-prompt"]')?.textContent).toContain('Arrived while inactive')
    expect(container.querySelector('[data-testid="transcript-history-controls"]')?.getAttribute('aria-busy')).toBe('false')
  })

  it('keeps following new rows after native Find is opened and cancelled without navigating', async () => {
    await render(transcript())
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', metaKey: true, bubbles: true, cancelable: true }))
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    })
    await flushHistory(turns.length)
    expect(lane().scrollTop).toBe(lane().scrollHeight - lane().clientHeight)
    await render(transcript([...turns, { ...turns[0]!, id: 'turn-new', prompt: { ...turns[0]!.prompt!, id: 'prompt-new', text: 'New streaming prompt' } }]))
    expect(lane().scrollTop).toBe(lane().scrollHeight - lane().clientHeight)
    expect(renderedRows(lane()).at(-1)?.dataset.itemId).toBe('turn-new')
  })

  it('re-parses streaming Markdown updates on the same message id', async () => {
    const streamed: TranscriptTurn = {
      ...turns.at(-1)!,
      items: [{ _tag: 'Text', id: 'streaming-answer', role: 'assistant', text: 'An **unfinished', attachments: [], streaming: true, at: new Date(now).toISOString() }],
    }
    await render(transcript([streamed]))
    expect(container.querySelector('[data-testid="agent-message"]')?.textContent).toContain('unfinished')
    await render(transcript([{ ...streamed, items: [{ ...streamed.items[0]!, _tag: 'Text', id: 'streaming-answer', role: 'assistant', text: 'An **updated answer**', attachments: [], streaming: true, at: new Date(now).toISOString() }] }]))
    expect(container.querySelector('[data-testid="agent-message"] strong')?.textContent).toBe('updated answer')
    expect(container.querySelector('[data-testid="agent-message"]')?.textContent).not.toContain('unfinished')
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
      expect(container.querySelector('[data-testid="user-message"][data-item-id="prompt-0"]')).toBeNull()
      expect(scrollIntoView).not.toHaveBeenCalled()
      let count = renderedRows(lane()).length
      await flushHistory(turns.length, () => {
        const nextCount = renderedRows(lane()).length
        expect(nextCount - count).toBeLessThanOrEqual(4)
        if (container.querySelector('[data-item-id="prompt-0"]') === null) expect(scrollIntoView).not.toHaveBeenCalled()
        count = nextCount
      })
      const target = container.querySelector('[data-testid="user-message"][data-item-id="prompt-0"]')
      expect(target).not.toBeNull()
      expect(scrollIntoView.mock.contexts).toContain(target)
    } finally {
      Reflect.deleteProperty(HTMLElement.prototype, 'scrollIntoView')
    }
  })

  it.each([
    ['removed', '', true],
    ['invalid', '#%E0%A4%A', true],
    ['nonmessage', '#conversation-settings', true],
    ['removed without a hashchange event', '', false],
  ])('cancels an old-message hash that is %s during progressive preparation', async (_description, replacement, dispatchHashChange) => {
    const scrollIntoView = vi.mocked(HTMLElement.prototype.scrollIntoView)
    await render(transcript())
    await act(async () => {
      window.history.replaceState(null, '', '#prompt-0')
      window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    await flushFrames(2)
    expect(container.querySelector('[data-item-id="prompt-0"]')).toBeNull()
    expect(scrollIntoView).not.toHaveBeenCalled()
    await act(async () => {
      window.history.replaceState(null, '', `${window.location.pathname}${replacement}`)
      if (dispatchHashChange) window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    await flushFrames(2)
    expect(scrollIntoView).not.toHaveBeenCalled()
    await act(async () => { container.querySelector<HTMLButtonElement>('button[aria-label="Show all messages"]')!.click() })
    await flushHistory(turns.length, () => {
      expect(window.location.hash).toBe(replacement)
      expect(scrollIntoView).not.toHaveBeenCalled()
    })
    expect(container.querySelector('[data-testid="user-message"][data-item-id="prompt-0"]')).not.toBeNull()
    expect(scrollIntoView).not.toHaveBeenCalled()
  })

  it('only scrolls to the replacement message hash and never the obsolete target when it later materializes', async () => {
    const scrollIntoView = vi.mocked(HTMLElement.prototype.scrollIntoView)
    await render(transcript())
    await act(async () => {
      window.history.replaceState(null, '', '#prompt-0')
      window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    await flushFrames(2)
    expect(container.querySelector('[data-item-id="prompt-0"]')).toBeNull()
    expect(container.querySelector('[data-item-id="prompt-4"]')).toBeNull()
    expect(scrollIntoView).not.toHaveBeenCalled()
    await act(async () => {
      window.history.replaceState(null, '', '#prompt-4')
      window.dispatchEvent(new HashChangeEvent('hashchange'))
    })
    await flushHistory(turns.length, () => {
      expect(window.location.hash).toBe('#prompt-4')
      const target = container.querySelector('[data-testid="user-message"][data-item-id="prompt-4"]')
      if (target === null) expect(scrollIntoView).not.toHaveBeenCalled()
      else expect(scrollIntoView.mock.contexts.every(context => context === target)).toBe(true)
    })
    const replacement = container.querySelector('[data-testid="user-message"][data-item-id="prompt-4"]')
    expect(replacement).not.toBeNull()
    expect(scrollIntoView.mock.contexts).toEqual([replacement])
    await act(async () => { container.querySelector<HTMLButtonElement>('button[aria-label="Show all messages"]')!.click() })
    await flushHistory(turns.length)
    expect(container.querySelector('[data-testid="user-message"][data-item-id="prompt-0"]')).not.toBeNull()
    expect(window.location.hash).toBe('#prompt-4')
    expect(scrollIntoView.mock.contexts).toEqual([replacement])
  })
})

describe('batched message measurements', () => {
  it('reads and publishes at most sixteen prompt measurements per animation frame', async () => {
    const events: string[] = []
    const published = vi.fn()
    const cleanups = Array.from({ length: 40 }, (_, index) => {
      const element = document.createElement('span')
      container.appendChild(element)
      Object.defineProperty(element, 'scrollHeight', { configurable: true, get: () => {
        events.push(`read-${index}`)
        return 180
      } })
      return observeUserMessageOverflow(element, overflows => {
        events.push(`publish-${index}`)
        published(overflows)
      })
    })
    try {
      expect(events).toHaveLength(0)
      for (const expectedTotal of [16, 32, 40]) {
        events.length = 0
        await act(async () => {
          const pending = [...frames.values()]
          frames.clear()
          pending.forEach(callback => callback(0))
        })
        expect(published).toHaveBeenCalledTimes(expectedTotal)
        const firstPublish = events.findIndex(event => event.startsWith('publish-'))
        expect(firstPublish).toBeGreaterThan(0)
        expect(firstPublish).toBeLessThanOrEqual(16)
        expect(events.slice(0, firstPublish).every(event => event.startsWith('read-'))).toBe(true)
        expect(events.slice(firstPublish).every(event => event.startsWith('publish-'))).toBe(true)
      }
      expect(published.mock.calls.every(([overflows]) => overflows === true)).toBe(true)
      expect(frames.size).toBe(0)
    } finally {
      cleanups.forEach(cleanup => cleanup())
    }
  })

  it('cancels queued measurement continuations on unmount and does not publish them after a switch', async () => {
    const oldPublish = vi.fn()
    const oldCleanups = Array.from({ length: 40 }, () => observeUserMessageOverflow(document.createElement('span'), oldPublish))
    let newCleanup: (() => void) | undefined
    try {
      await act(async () => {
        const pending = [...frames.values()]
        frames.clear()
        pending.forEach(callback => callback(0))
      })
      expect(oldPublish).toHaveBeenCalledTimes(16)
      expect(frames.size).toBe(1)
      oldCleanups.forEach(cleanup => cleanup())
      expect(frames.size).toBe(0)
      const newPublish = vi.fn()
      newCleanup = observeUserMessageOverflow(document.createElement('span'), newPublish)
      await flushFrames()
      expect(oldPublish).toHaveBeenCalledTimes(16)
      expect(newPublish).toHaveBeenCalledTimes(1)
    } finally {
      oldCleanups.forEach(cleanup => cleanup())
      newCleanup?.()
    }
  })

  it('unregisters queued overflow work for retained inactive panes and refreshes measurements on reactivation', async () => {
    await render(transcript())
    await act(async () => {
      container.querySelector<HTMLButtonElement>('button[aria-label="Show all messages"]')!.click()
    })
    await flushHistory(turns.length)
    const messages = [...container.querySelectorAll<HTMLElement>('[data-testid="user-message"]')]
    const texts = messages.map(message => message.querySelector('span')!)
    const readHeight = vi.fn(() => 180)
    texts.forEach(text => Object.defineProperty(text, 'scrollHeight', { configurable: true, get: readHeight }))
    await act(async () => {
      for (const observer of observers) {
        const targets = [...observer.elements].filter(element => texts.includes(element as HTMLSpanElement))
        if (targets.length > 0) observer.callback(targets.map(target => ({ target, contentRect: target.getBoundingClientRect(), borderBoxSize: [], contentBoxSize: [], devicePixelContentBoxSize: [] })), {} as ResizeObserver)
      }
    })
    expect(readHeight).not.toHaveBeenCalled()
    await act(async () => root.render(transcript(turns, false)))
    expect(messages.every(message => message.isConnected)).toBe(true)
    expect([...observers].some(observer => texts.some(text => observer.elements.has(text)))).toBe(false)
    await flushFrames(12)
    expect(readHeight).not.toHaveBeenCalled()
    expect(messages.every(message => !message.textContent?.includes('Show full message'))).toBe(true)
    await act(async () => root.render(transcript(turns, true)))
    await flushFrames(Math.ceil(texts.length / 16) + 6)
    expect(readHeight).toHaveBeenCalledTimes(texts.length)
    expect(messages.every(message => message.textContent?.includes('Show full message'))).toBe(true)
    expect(messages.every(message => message.isConnected)).toBe(true)
  })

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
