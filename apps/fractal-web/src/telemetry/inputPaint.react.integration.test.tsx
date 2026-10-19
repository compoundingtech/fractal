// @vitest-environment jsdom
import { createElement, useState } from 'react'
import { flushSync } from 'react-dom'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'
import { makeFrameIngest } from '../data/frameIngest.ts'
import { installInputPaint, type InputPaintSample } from './inputPaint.ts'

const cleanups: Array<() => void> = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0).reverse()) cleanup()
  vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.replaceChildren()
})
const setup = ({ stopped = false, scheduleAfter = false, nested = false, timer = '' } = {}) => {
  const frames = new Map<number, FrameRequestCallback>()
  const paints = new Set<() => void>()
  const samples: InputPaintSample[] = []
  let nextFrame = 0
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { const id = ++nextFrame; frames.set(id, callback); return id })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => { frames.delete(id) })
  // jsdom cannot manufacture trusted input. Only this flag is adapted; event dispatch,
  // propagation, React createRoot and its microtask scheduling are real, not hand-flushed.
  const trusted = new WeakSet<Event>()
  const proxies = new WeakMap<Event, Event>()
  const wrapped = new Map<EventListenerOrEventListenerObject, EventListener>()
  const add = window.addEventListener.bind(window)
  const remove = window.removeEventListener.bind(window)
  const adapt = (listener: EventListenerOrEventListenerObject): EventListener => {
    let wrapper = wrapped.get(listener)
    if (wrapper === undefined) {
      wrapper = (event) => {
        let proxy = proxies.get(event)
        if (proxy === undefined) {
          proxy = new Proxy(event, { get: (value, key) => key === 'isTrusted' ? trusted.has(event) : Reflect.get(value, key, value) })
          proxies.set(event, proxy)
        }
        if (typeof listener === 'function') listener.call(window, proxy)
        else listener.handleEvent(proxy)
      }
      wrapped.set(listener, wrapper)
    }
    return wrapper
  }
  vi.spyOn(window, 'addEventListener').mockImplementation((type, listener, options) => {
    if (listener !== null) add(type, adapt(listener), options)
  })
  vi.spyOn(window, 'removeEventListener').mockImplementation((type, listener, options) => {
    if (listener !== null) remove(type, wrapped.get(listener) ?? listener, options)
  })
  const container = document.createElement('div')
  const stream = document.createElement('output')
  document.body.append(container, stream)
  const ingest = makeFrameIngest<string, string>({ write: ({ value }) => { stream.textContent = value } })
  const App = () => {
    const [count, setCount] = useState(0)
    return createElement('div', {},
      createElement('button', { id: 'real', onClick: (event) => {
        if (stopped) event.stopPropagation()
        if (nested) document.querySelector<HTMLButtonElement>('#noop')!.click()
        setCount((previous) => previous + 1)
      } }, String(count)),
      createElement('button', { id: 'noop', onClick: (event) => {
        if (stopped) event.stopPropagation()
        if (scheduleAfter) ingest.accept({ key: 'stream', value: 'streamed' })
        if (timer !== '') setTimeout(() => {
          if (timer === 'dom') stream.textContent = 'timer'
          else flushSync(() => setCount((previous) => previous + 1))
        }, 0)
      } }, 'no-op'),
      createElement('button', { id: 'capture', onClickCapture: (event) => { event.stopPropagation(); setCount((previous) => previous + 1) } }, 'capture'),
    )
  }
  const root = createRoot(container)
  flushSync(() => root.render(createElement(App)))
  const rootAdd = container.addEventListener.bind(container)
  const rootRemove = container.removeEventListener.bind(container)
  vi.spyOn(container, 'addEventListener').mockImplementation((type, listener, options) => {
    if (listener !== null) rootAdd(type, adapt(listener), options)
  })
  vi.spyOn(container, 'removeEventListener').mockImplementation((type, listener, options) => {
    if (listener !== null) rootRemove(type, wrapped.get(listener) ?? listener, options)
  })
  cleanups.push(() => root.unmount(), () => ingest.dispose())
  cleanups.push(installInputPaint({ reactRoots: [container], emit: (sample) => samples.push(sample), paint: (callback) => { paints.add(callback); return () => { paints.delete(callback) } } }))
  const click = (id: string) => {
    const event = new MouseEvent('click', { bubbles: true })
    trusted.add(event)
    document.querySelector(id)!.dispatchEvent(event)
  }
  const frame = async () => {
    for (const [id, callback] of [...frames]) if (frames.delete(id)) callback(performance.now())
    await Promise.resolve()
    for (const callback of [...paints]) callback()
  }
  return { samples, click, frame, ingest, stream }
}
it('attributes React createRoot discrete commits flushed in a microtask', async () => {
  const run = setup()
  run.click('#real')
  expect(document.querySelector('#real')!.textContent).toBe('0')
  await Promise.resolve(); await Promise.resolve()
  expect(document.querySelector('#real')!.textContent).toBe('1')
  await run.frame()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})
it.each([false, true])('attributes stopped propagation microtask commits (stopped=%s)', async (stopped) => {
  const run = setup({ stopped })
  run.click('#real')
  await Promise.resolve(); await Promise.resolve()
  await run.frame()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})
it.each([false, true])('excludes ingest rAF registered before capture even with stopped propagation=%s', async (stopped) => {
  const run = setup({ stopped })
  run.click('#real')
  await Promise.resolve(); await Promise.resolve()
  run.ingest.accept({ key: 'stream', value: 'streamed' })
  run.click('#noop')
  await Promise.resolve(); await Promise.resolve()
  await run.frame()
  expect(run.stream.textContent).toBe('streamed')
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})
it.each([false, true])('excludes ingest rAF registered in the no-op handler (stopped=%s)', async (stopped) => {
  const run = setup({ stopped, scheduleAfter: true })
  run.click('#real')
  await Promise.resolve(); await Promise.resolve()
  run.click('#noop')
  await Promise.resolve(); await Promise.resolve()
  await run.frame()
  expect(run.stream.textContent).toBe('streamed')
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})
it('does not let an older dispatch microtask close a newer overlapping capture', async () => {
  const run = setup()
  run.click('#noop'); run.click('#real')
  await Promise.resolve(); await Promise.resolve()
  await run.frame()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})
it('ignores nested untrusted dispatch without losing the trusted outer React commit', async () => {
  const run = setup({ nested: true })
  run.click('#real')
  await Promise.resolve(); await Promise.resolve()
  await run.frame()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})

it.each(['dom', 'react'])('excludes an actual %s timer commit before rAF after a stopped no-op click', async (timer) => {
  const run = setup({ stopped: true, timer })
  run.click('#noop')
  await new Promise<void>((resolve) => setTimeout(resolve, 0))
  if (timer === 'dom') expect(run.stream.textContent).toBe('timer')
  else expect(document.querySelector('#real')!.textContent).toBe('1')
  await run.frame()
  expect(run.samples).toEqual([])
})
it('attributes a React capture-phase handler that stops native propagation', async () => {
  const run = setup()
  run.click('#capture')
  await Promise.resolve(); await Promise.resolve()
  expect(document.querySelector('#real')!.textContent).toBe('1')
  await run.frame()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})
