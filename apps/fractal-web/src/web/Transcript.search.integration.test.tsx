// @vitest-environment jsdom
import { act } from 'react'
import * as React from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { EmbraceRuntimeProvider } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceRuntime.tsx'
import { Transcript, type TranscriptHistory, type TranscriptTurn } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'
import { TranscriptFind } from '../../../../packages/fractal-ui/src/assistant-ui/composition/TranscriptFind.tsx'
import { createTranscriptRevealStore } from '../../../../packages/fractal-ui/src/assistant-ui/composition/transcript-reveal.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const now = Date.parse('2026-11-07T12:00:00Z')
const at = new Date(now).toISOString()
const turn = (index: number, text = `Plain prompt ${index}`): TranscriptTurn => ({ id: `turn-${index}`, prompt: { _tag: 'Text', id: `prompt-${index}`, role: 'user', text, attachments: [], streaming: false, at }, items: [], work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false } })
const toolTurn: TranscriptTurn = {
  id: 'tool-turn',
  items: [{ _tag: 'ToolCall', id: 'tool/read', callId: 'call/read', name: 'read', input: { path: 'notes.md' }, status: 'success', callSeen: true, at, result: { content: 'one\ntwo\nthree\nfour\nfive\nneedle on line six', isError: false, at } }],
  work: { calls: [{ id: 'tool/read', kind: 'read', title: 'Read notes', status: 'success', startedAt: at, endedAt: at, detail: 'one\ntwo\nthree\nfour\nfive\nneedle on line six' }], durationMs: 1000, running: false, failed: false, interrupted: false },
}
const longTurns = [turn(0, 'needle in the oldest prompt'), ...Array.from({ length: 30 }, (_, index) => turn(index + 1)), turn(31, 'needle in the newest prompt')]

let root: Root
let container: HTMLDivElement
const settle = async (ms = 200) => { await act(async () => { await new Promise(resolve => setTimeout(resolve, ms)) }) }
const view = (turns: readonly TranscriptTurn[], props: { readonly history?: TranscriptHistory; readonly active?: boolean; readonly viewportKey?: string } = {}) => <>
  <input data-testid="composer" />
  <EmbraceRuntimeProvider options={{ messages: turns.flatMap(entry => entry.prompt === undefined ? entry.items : [entry.prompt, ...entry.items]), isRunning: false, onNew: async () => {} }}>
    <Transcript turns={turns} title="Search" sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} onOpenTool={() => {}} {...props} />
  </EmbraceRuntimeProvider>
</>
const render = async (element: React.ReactNode) => { await act(async () => root.render(element)); await settle(30) }
const press = async (init: KeyboardEventInit) => {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })
  await act(async () => { (document.activeElement ?? document.body).dispatchEvent(event) })
  return event
}
const find = () => container.querySelector<HTMLElement>('[data-testid="transcript-find"]')
const field = () => find()!.querySelector('input')!
const type = async (text: string) => {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(field(), text)
    field().dispatchEvent(new Event('input', { bubbles: true }))
  })
  await settle()
}
const composer = () => container.querySelector<HTMLInputElement>('[data-testid="composer"]')!
const count = () => container.querySelector('[data-testid="transcript-find-count"]')?.textContent
const status = () => container.querySelector('[data-testid="transcript-find-status"]')?.textContent

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => setTimeout(() => callback(0), 0) as unknown as number)
  vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id))
  vi.stubGlobal('CSS', { ...globalThis.CSS, escape: (value: string) => value.replace(/[^a-zA-Z0-9_-]/g, character => `\\${character}`) })
  // jsdom has no layout: every range reports an empty box.
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: () => null })
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  Reflect.deleteProperty(document, 'elementFromPoint')
  vi.unstubAllGlobals()
})

it('opens from the composer, passes a second press to browser Find, and Esc returns focus', async () => {
  await render(view(longTurns))
  composer().focus()
  expect((await press({ key: 'f', metaKey: true })).defaultPrevented).toBe(true)
  expect(document.activeElement).toBe(field())
  expect((await press({ key: 'f', metaKey: true })).defaultPrevented).toBe(false)
  await press({ key: 'Escape' })
  expect(find()).toBeNull()
  expect(document.activeElement).toBe(composer())
})

it('leaves the shortcut to the browser in a hidden pane and an open dialog', async () => {
  await render(view(longTurns, { active: false }))
  composer().focus()
  expect((await press({ key: 'f', ctrlKey: true })).defaultPrevented).toBe(false)
  expect(find()).toBeNull()
  await render(view(longTurns))
  composer().setAttribute('role', 'dialog')
  expect((await press({ key: 'f', ctrlKey: true })).defaultPrevented).toBe(false)
  expect(find()).toBeNull()
})

it('counts loaded rows the window has not mounted, and stepping mounts the row it lands on', async () => {
  await render(view(longTurns))
  expect(container.querySelector('[data-item-id="prompt-0"]')).toBeNull()
  composer().focus()
  await press({ key: 'f', ctrlKey: true })
  await type('NEEDLE')
  expect(status()).toContain('of 2')
  expect(count()).toMatch(/^[12]\/2$/)
  const first = count()
  await act(async () => { field().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })) })
  await settle(600)
  expect(count()).not.toBe(first)
  await act(async () => { field().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true, cancelable: true })) })
  expect(count()).toBe(first)
  await type('absent')
  expect(count()).toBe('No matches')
  expect(status()).toContain('No matches')
})

it('expands a collapsed tool row and renders the output a match sits in', async () => {
  await render(view([turn(0), toolTurn]))
  expect(container.querySelector('[data-testid="work-log"]')?.textContent).not.toContain('needle on line six')
  composer().focus()
  await press({ key: 'f', ctrlKey: true })
  await type('needle')
  await settle()
  expect(count()).toBe('1/1')
  expect(container.querySelector('[data-testid="tool-detail-preview"]')?.textContent).toContain('needle on line six')
  expect(container.querySelector('[data-testid="tool-preview-remaining"]')).toBeNull()
})

it('restores disclosures opened by search on close and when the pane becomes inactive', async () => {
  const turns = [turn(0), toolTurn]
  await render(view(turns))
  composer().focus()
  await press({ key: 'f', ctrlKey: true })
  await type('needle')
  await settle()
  expect(container.querySelector('[data-testid="tool-detail-preview"]')?.textContent).toContain('needle on line six')
  await press({ key: 'Escape' })
  await settle()
  expect(container.querySelector('[data-testid="work-log"]')?.textContent).not.toContain('needle on line six')
  composer().focus()
  await press({ key: 'f', ctrlKey: true })
  await settle()
  expect(container.querySelector('[data-testid="tool-detail-preview"]')?.textContent).toContain('needle on line six')
  await render(view(turns, { active: false }))
  await settle()
  expect(container.querySelector('[data-testid="work-log"]')?.textContent).not.toContain('needle on line six')
})

it('reveals a different query in the same entry and occurrence when it moves into collapsed detail', async () => {
  await render(view([turn(0), toolTurn]))
  composer().focus()
  await press({ key: 'f', ctrlKey: true })
  await type('Read notes')
  await settle()
  expect(count()).toBe('1/1')
  expect(container.querySelector('[data-testid="tool-raw-input"]')).toBeNull()
  expect(container.querySelector('[data-testid="tool-detail-preview"]')?.textContent).not.toContain('needle on line six')
  await type('needle')
  await settle()
  expect(count()).toBe('1/1')
  expect(container.querySelector('[data-testid="tool-detail-preview"]')?.textContent).toContain('needle on line six')
})

it('reports partial coverage and pages history on demand until complete, with Stop', async () => {
  let pages = 0
  const onLoadEarlier = () => { pages += 1 }
  const partial = longTurns.slice(10)
  const rerender = async (history: TranscriptHistory, turns = partial) => { await act(async () => root.render(view(turns, { history }))); await settle(30) }
  const action = (name: string) => [...find()!.querySelectorAll('button')].find(button => button.textContent === name)!
  const coverage = () => container.querySelector('[data-testid="transcript-find-coverage"]')?.textContent
  await render(view(partial, { history: { _tag: 'HasOlder', onLoadEarlier } }))
  composer().focus()
  await press({ key: 'f', ctrlKey: true })
  await type('needle')
  expect(count()).toMatch(/\/1$/)
  expect(coverage()).toContain('Loaded entries only')
  expect(status()).toContain('Loaded entries only')
  expect(find()!.dataset.coverage).toBe('partial')
  expect(pages).toBe(0)
  await act(async () => action('Search full history').click())
  await settle(30)
  expect(pages).toBe(1)
  await rerender({ _tag: 'HasOlder', onLoadEarlier, older: { _tag: 'Loading' } })
  expect(coverage()).toContain('Loading older history')
  // Stop ends the run: the page in flight lands, and no further one is asked for.
  await act(async () => action('Stop').click())
  await rerender({ _tag: 'HasOlder', onLoadEarlier }, longTurns.slice(5))
  expect(pages).toBe(1)
  await act(async () => action('Search full history').click())
  await settle(30)
  expect(pages).toBe(2)
  await rerender({ _tag: 'HasOlder', onLoadEarlier, older: { _tag: 'Failed' } }, longTurns.slice(5))
  expect(coverage()).toContain("Couldn't load older history")
  expect(pages).toBe(2)
  await act(async () => action('Retry').click())
  await settle(30)
  expect(pages).toBe(3)
  await rerender({ _tag: 'Complete' }, longTurns)
  expect(count()).toMatch(/\/2$/)
  expect(coverage()).toContain('Full history searched')
  expect(find()!.dataset.coverage).toBe('complete')
})

it('keeps each conversation its own search when one transcript serves several tabs', async () => {
  await render(view(longTurns, { viewportKey: 'tab-a' }))
  composer().focus()
  await press({ key: 'f', ctrlKey: true })
  await type('needle')
  await render(view(longTurns, { viewportKey: 'tab-b' }))
  expect(find()).toBeNull()
  await render(view(longTurns, { viewportKey: 'tab-a' }))
  expect(field().value).toBe('needle')
  // Coming back restores the search without stealing focus from wherever the reader is.
  expect(document.activeElement).not.toBe(field())
})

it('opens only the conversation nearest focus when two are on screen', async () => {
  const pane = (name: string) => <section data-pane={name}>
    <input data-testid={`composer-${name}`} />
    <EmbraceRuntimeProvider options={{ messages: longTurns.flatMap(entry => [entry.prompt!]), isRunning: false, onNew: async () => {} }}>
      <Transcript turns={longTurns} title={name} sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} viewportKey={name} />
    </EmbraceRuntimeProvider>
  </section>
  await render(<>{pane('left')}{pane('right')}</>)
  container.querySelector<HTMLInputElement>('[data-testid="composer-right"]')!.focus()
  expect((await press({ key: 'f', metaKey: true })).defaultPrevented).toBe(true)
  expect(container.querySelector('[data-pane="left"] [data-testid="transcript-find"]')).toBeNull()
  expect(container.querySelector('[data-pane="right"] [data-testid="transcript-find"]')).not.toBeNull()
})

it('loads the next page when a reader scrolls up to the boundary, and never on open, while a request is out, or after a failure', async () => {
  const load = vi.fn()
  const lane = () => container.querySelector<HTMLElement>('[data-testid="transcript-scroll"]')!
  const scrollTo = async (top: number) => { await act(async () => { lane().scrollTop = top; lane().dispatchEvent(new Event('scroll')) }) }
  await render(view([turn(0)], { history: { _tag: 'HasOlder', onLoadEarlier: load } }))
  // Opening, and the lane moving down as rows arrive, fetch nothing; the explicit button stays.
  // At the live tail nothing loads, however the lane moves there.
  const tail = vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(1000)
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(100)
  await scrollTo(900)
  expect(load).not.toHaveBeenCalled()
  expect(container.querySelector('[data-testid="history-boundary"] button')?.textContent).toBe('Load earlier messages')
  await scrollTo(0)
  expect(load).toHaveBeenCalledTimes(1)
  tail.mockReturnValue(1000)
  await render(view([turn(0)], { history: { _tag: 'HasOlder', onLoadEarlier: load, older: { _tag: 'Loading' } } }))
  await scrollTo(40); await scrollTo(0)
  await render(view([turn(0)], { history: { _tag: 'HasOlder', onLoadEarlier: load, older: { _tag: 'Failed' } } }))
  await scrollTo(40); await scrollTo(0)
  expect(load).toHaveBeenCalledTimes(1)
})

it('leaves the shortcut to a focused terminal', async () => {
  await render(<>{view(longTurns)}<section data-testid="terminal-pane"><div tabIndex={0} aria-roledescription="terminal" data-testid="terminal-surface" /></section></>)
  container.querySelector<HTMLElement>('[data-testid="terminal-surface"]')!.focus()
  expect((await press({ key: 'f', metaKey: true })).defaultPrevented).toBe(false)
  expect(find()).toBeNull()
})

it('completes navigation when a virtualized match mounts after the former deadline', async () => {
  const revealRow = vi.fn()
  const reveal = createTranscriptRevealStore()
  const memory = new Map()
  const turns = [turn(0, 'needle in a distant row')]
  const rowIds = ['prompt-0']
  const fixture = (mounted: boolean) => <>
    <input data-testid="composer" />
    <section>
      <TranscriptFind scrollId="delayed-lane" turns={turns} rowIds={rowIds} revealRow={revealRow} history={{ _tag: 'Complete' }} active reveal={reveal} memory={memory} memoryKey="delayed" />
      <div id="delayed-lane">{mounted && <div data-item-id="prompt-0">needle in a distant row</div>}</div>
    </section>
  </>
  await render(fixture(false))
  composer().focus()
  await press({ key: 'f', ctrlKey: true })
  await type('needle')
  expect(revealRow).toHaveBeenCalledWith('prompt-0', true)
  expect(revealRow).not.toHaveBeenCalledWith('prompt-0')
  // A slow virtualizer can take arbitrarily longer than one second; only its mount is readiness.
  const clock = vi.spyOn(performance, 'now').mockReturnValue(performance.now() + 2000)
  try {
    await render(fixture(true))
    await settle()
    expect(revealRow).toHaveBeenCalledWith('prompt-0')
  } finally {
    clock.mockRestore()
  }
})

it('cancels pending virtualized navigation when the reader scrolls instead', async () => {
  const revealRow = vi.fn()
  const reveal = createTranscriptRevealStore()
  const memory = new Map()
  const turns = [turn(0, 'needle')]
  const rowIds = ['prompt-0']
  const fixture = (mounted: boolean) => <>
    <input data-testid="composer" />
    <section>
      <TranscriptFind scrollId="cancel-lane" turns={turns} rowIds={rowIds} revealRow={revealRow} history={{ _tag: 'Complete' }} active reveal={reveal} memory={memory} memoryKey="cancel" />
      <div id="cancel-lane">{mounted && <div data-item-id="prompt-0">needle</div>}</div>
    </section>
  </>
  await render(fixture(false))
  composer().focus()
  await press({ key: 'f', ctrlKey: true })
  await type('needle')
  expect(revealRow).toHaveBeenCalledWith('prompt-0', true)
  document.getElementById('cancel-lane')!.dispatchEvent(new WheelEvent('wheel', { bubbles: true }))
  await render(fixture(true))
  await settle()
  expect(revealRow).not.toHaveBeenCalledWith('prompt-0')
})

it('scrolls to a new query in the same mounted entry even when its occurrence and depth are unchanged', async () => {
  const revealRow = vi.fn()
  const reveal = createTranscriptRevealStore()
  const memory = new Map()
  const turns = [turn(0, 'first phrase followed by second phrase')]
  await render(<>
    <input data-testid="composer" />
    <section>
      <TranscriptFind scrollId="phrase-lane" turns={turns} rowIds={['prompt-0']} revealRow={revealRow} history={{ _tag: 'Complete' }} active reveal={reveal} memory={memory} memoryKey="phrases" />
      <div id="phrase-lane"><div data-item-id="prompt-0">first phrase followed by second phrase</div></div>
    </section>
  </>)
  const lane = document.getElementById('phrase-lane')!
  const bounds = vi.spyOn(lane, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 200, 100))
  const ranges = vi.spyOn(Range.prototype, 'getBoundingClientRect').mockImplementation(function (this: Range) {
    return new DOMRect(0, this.toString() === 'second phrase' ? 1000 : 10, 50, 10)
  })
  try {
    composer().focus()
    await press({ key: 'f', ctrlKey: true })
    await type('first phrase')
    expect(count()).toBe('1/1')
    expect(lane.scrollTop).toBe(0)
    await type('second phrase')
    expect(count()).toBe('1/1')
    expect(lane.scrollTop).toBeGreaterThan(900)
  } finally {
    ranges.mockRestore()
    bounds.mockRestore()
  }
})
