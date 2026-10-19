// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest'
import { registerInputPaintRegions, installInputPaint, type InputPaintSample } from './inputPaint.ts'

const cleanups: Array<() => void> = []
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.replaceChildren() })
const setup = () => {
  let time = 100
  const callbacks = new Set<() => void>()
  const tasks = new Set<() => void>()
  const samples: InputPaintSample[] = []
  const listeners = vi.spyOn(window, 'addEventListener')
  const stop = installInputPaint({
    emit: (sample) => samples.push(sample), now: () => time,
    paint: (callback) => { callbacks.add(callback); return () => { callbacks.delete(callback) } },
    task: (callback) => { tasks.add(callback); return () => { tasks.delete(callback) } },
  })
  cleanups.push(stop)
  const button = document.createElement('button')
  document.body.append(button)
  const input = (type = 'click', target: Element = button) => {
    const listener = listeners.mock.calls.find(([name]) => name === type)?.[1]
    if (typeof listener !== 'function') throw new Error('missing input capture')
    const event = new Proxy(new Event(type), { get: (event, key) => key === 'isTrusted' ? true : key === 'timeStamp' ? time - 5 : key === 'target' ? target : Reflect.get(event, key, event) })
    listener.call(window, event)
  }
  const mutate = async () => { button.textContent = String(time); await Promise.resolve() }
  const paint = () => {
    for (const callback of [...tasks]) callback()
    time += 16
    for (const callback of [...callbacks]) callback()
  }
  return { samples, button, input, mutate, paint, stop, advance: () => { time += 10 } }
}
it('records once after a DOM commit and paint, with only bounded contract attributes', async () => {
  const run = setup()
  run.input()
  await run.mutate()
  expect(run.samples).toHaveLength(0)
  run.paint(); run.paint(); run.stop()
  expect(run.samples).toHaveLength(1)
  expect(run.samples[0]).toEqual({ name: 'fractal.input.next_frame', startMs: 95, endMs: 116, attributes: {
    'span.label': 'input.next_frame', 'input.kind': 'mouse', 'input.action': 'click', surface: 'other', outcome: 'painted', accepted_to_frame_ms: 16, read_to_frame_ms: 21,
  } })
})
it('emits nothing for idle DOM changes or inputs with no DOM change', async () => {
  const run = setup()
  await run.mutate(); run.paint()
  run.input(); run.paint(); run.paint(); run.stop()
  expect(run.samples).toEqual([])
})
it('coalesces only when a later input commits before paint', async () => {
  const run = setup()
  run.input('keydown'); await run.mutate()
  run.advance(); run.input(); await run.mutate()
  run.paint()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['coalesced', 'painted'])
  expect(run.samples[0]?.attributes).not.toHaveProperty('read_to_frame_ms')
})
it('does not coalesce an earlier commit for a later no-op input', async () => {
  const run = setup()
  run.input(); await run.mutate()
  run.advance(); run.input('keydown'); run.paint()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})
it.each(['pagehide', 'unload', 'shutdown'])('ends committed input once as unacknowledged on %s', async (boundary) => {
  const run = setup()
  run.input(); await run.mutate()
  if (boundary === 'shutdown') run.stop()
  else window.dispatchEvent(new Event(boundary))
  run.paint(); run.stop()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['unacknowledged'])
})

it('ends committed input on hidden visibility and ignores subsequent input', async () => {
  const run = setup()
  run.input(); await run.mutate()
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
  document.dispatchEvent(new Event('visibilitychange'))
  run.input(); await run.mutate(); run.paint()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['unacknowledged'])
})

it('uses matched Event Timing presentation ends with a nonzero interaction id', async () => {
  let deliver: ((entries: Array<PerformanceEntry & { readonly interactionId: number }>) => void) | undefined
  vi.stubGlobal('PerformanceObserver', class {
    static supportedEntryTypes = ['event']
    constructor(callback: PerformanceObserverCallback) {
      deliver = (entries) => callback({
        getEntries: () => entries,
        getEntriesByName: (name) => entries.filter((entry) => entry.name === name),
        getEntriesByType: (type) => entries.filter((entry) => entry.entryType === type),
      }, this)
    }
    observe() {}
    disconnect() {}
    takeRecords() { return [] }
  })
  const run = setup()
  run.input()
  // Correlate the entry before commit; subsequent entries must retain this exact identity.
  const entry = { name: 'click', entryType: 'event', startTime: 95, duration: 10, interactionId: 42, toJSON: () => ({}) }
  deliver?.([entry])
  await run.mutate(); run.advance()
  run.input('keydown')
  deliver?.([{ ...entry, name: 'keydown', interactionId: 99 }])
  deliver?.([{ ...entry, startTime: 95.5 }, { ...entry, interactionId: 43 }])
  expect(run.samples).toHaveLength(0)
  deliver?.([entry])
  run.paint()
  expect(run.samples[0]?.endMs).toBe(105)
  expect(run.samples[0]?.attributes.read_to_frame_ms).toBe(10)
})

it('ignores unchanged attribute and text assignments', async () => {
  const run = setup()
  const button = document.querySelector('button')!
  button.setAttribute('title', 'same')
  button.textContent = 'same'
  await Promise.resolve()
  run.input()
  button.setAttribute('title', 'same')
  button.firstChild!.nodeValue = 'same'
  await Promise.resolve()
  run.paint()
  expect(run.samples).toEqual([])
})


it('observes only the input-owned control and ignores a large unrelated same-window update', async () => {
  const observed = vi.spyOn(MutationObserver.prototype, 'observe')
  const run = setup()
  const unrelated = document.createElement('section')
  document.body.append(unrelated)
  run.input()
  unrelated.replaceChildren(...Array.from({ length: 2000 }, () => {
    const row = document.createElement('div')
    row.textContent = 'Unrelated transcript update'
    return row
  }))
  document.documentElement.setAttribute('data-unrelated-theme', 'changed')
  await Promise.resolve()
  run.paint()
  expect(run.samples).toEqual([])
  expect(observed).toHaveBeenCalledTimes(1)
  expect(observed.mock.calls[0]?.[0]).toBe(document.querySelector('button'))
  expect(observed.mock.calls[0]?.[0]).not.toBe(document.documentElement)
  document.documentElement.removeAttribute('data-unrelated-theme')
})

it('drains queued Event Timing presentation before assigning page-hide outcomes', async () => {
  let queued: PerformanceEntryList = []
  vi.stubGlobal('PerformanceObserver', class {
    static supportedEntryTypes = ['event']
    observe() {}
    disconnect() {}
    takeRecords() { const entries = queued; queued = []; return entries }
  })
  const run = setup()
  run.input()
  await run.mutate()
  run.advance()
  queued = [{ name: 'click', entryType: 'event', startTime: 95, duration: 10, interactionId: 42, toJSON: () => ({}) } as PerformanceEntry]
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
  document.dispatchEvent(new Event('visibilitychange'))
  run.paint(); run.stop()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
  expect(run.samples[0]?.endMs).toBe(105)
})

it.each(['aria-controls', 'aria-owns'])('counts an %s target update without observing its surrounding surface', async (attribute) => {
  const run = setup()
  const controlled = document.createElement('div')
  controlled.id = attribute
  document.body.append(controlled)
  run.button.setAttribute(attribute, controlled.id)
  run.input()
  controlled.textContent = 'owned update'
  await Promise.resolve()
  run.paint()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})

it('counts a registered field/tail mutation only for inputs owned by its surface', async () => {
  const run = setup()
  const button = document.querySelector('button')!
  const tail = document.createElement('section')
  document.body.append(tail)
  cleanups.push(registerInputPaintRegions(button, () => [{ target: tail }]))
  run.input()
  tail.textContent = 'owned transcript tail'
  await Promise.resolve()
  run.paint()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})

it('uses a trusted field input event as its native editing commit, once', () => {
  const run = setup()
  const field = document.createElement('textarea')
  document.body.append(field)
  field.value = 'Typed text'
  run.input('input', field)
  run.paint(); run.paint()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
  expect(run.samples[0]?.attributes['input.kind']).toBe('input')
})

it('does not expand aria-controls ownership to a whole transcript', async () => {
  const observed = vi.spyOn(MutationObserver.prototype, 'observe')
  const run = setup()
  const history = document.createElement('section')
  history.id = 'history'
  history.dataset.testid = 'transcript-scroll'
  document.body.append(history)
  run.button.setAttribute('aria-controls', history.id)
  run.input()
  history.textContent = 'Unrelated history update'
  await Promise.resolve()
  run.paint()
  expect(run.samples).toEqual([])
  expect(observed.mock.calls.map(([target]) => target)).toEqual([run.button])
})
