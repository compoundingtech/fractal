// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest'
import { installInputPaint, type InputPaintSample } from './inputPaint.ts'

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
  const input = (type = 'click') => {
    const listener = listeners.mock.calls.find(([name]) => name === type)?.[1]
    if (typeof listener !== 'function') throw new Error('missing input capture')
    const event = new Proxy(new Event(type), { get: (event, key) => key === 'isTrusted' ? true : key === 'timeStamp' ? time - 5 : key === 'target' ? button : Reflect.get(event, key, event) })
    listener.call(window, event)
  }
  const mutate = async () => { button.textContent = String(time); await Promise.resolve() }
  const paint = () => {
    for (const callback of [...tasks]) callback()
    time += 16
    for (const callback of [...callbacks]) callback()
  }
  return { samples, input, mutate, paint, stop, advance: () => { time += 10 } }
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

