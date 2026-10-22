// @vitest-environment jsdom
import { createElement, useCallback, useLayoutEffect, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'
import { makeFrameIngest } from '../data/frameIngest.ts'
import { inputPaintRegionMounted, registerInputPaintRegions, installInputPaint, type InputPaintSample } from './inputPaint.ts'
import { PortalContainerRegistrationContext, RegisteredPortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/PortalContainer.tsx'
import { App as RealAppShell } from '../web/App.tsx'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, defineConsts: (constants: unknown) => constants, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))
vi.mock('virtual:build-identity', () => ({ buildIdentity: { machineVersion: 'test', dirty: false }, deploymentId: undefined }))
vi.mock('./meters.tsx', () => ({ acquireMeasurementEngine: () => () => {}, getDebug: () => 0 }))
vi.mock('../web/liveRuntime.ts', () => ({ live: {}, telemetry: { onCommit: () => {}, ux: undefined } }))
vi.mock('../data/react.tsx', () => ({ DataSourceProvider: ({ children }: { children: React.ReactNode }) => children }))
// Backend workspace content is irrelevant; the App host, status bar and changelog are real.
vi.mock('../web/LiveAgentWorkspace.tsx', () => ({ LiveAgentWorkspace: () => null }))

const cleanups: Array<() => void> = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0).reverse()) cleanup()
  vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.replaceChildren()
})
const OwnedComposer = () => {
  const [sent, setSent] = useState(0)
  const dock = useRef<HTMLDivElement>(null)
  const tail = useRef<HTMLElement | null>(null)
  const tailRef = useCallback((node: HTMLElement | null) => {
    tail.current = node
    if (node !== null && dock.current !== null) inputPaintRegionMounted(dock.current)
  }, [])
  useLayoutEffect(() => registerInputPaintRegions(dock.current!, () => [
    { target: dock.current!.querySelector('textarea')! },
    ...(tail.current === null ? [] : [{ target: tail.current }]),
  ]), [])
  return createElement('div', {},
    createElement('div', { id: 'history' }, sent === 0 ? null : createElement('section', { key: sent, ref: tailRef }, 'Sent ' + sent)),
    createElement('div', { ref: dock }, createElement('textarea', { defaultValue: 'Message' }), createElement('button', { id: 'send', onClick: () => setSent((value) => value + 1) }, 'Send')),
  )
}
const setup = ({ stopped = false, scheduleAfter = false, nested = false, timer = '', portal = false, captureNoop = false, appShell = false, scenario = '', treeExpanded = false } = {}) => {
  const frames = new Map<number, FrameRequestCallback>()
  const paints = new Set<() => void>()
  const samples: InputPaintSample[] = []
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
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
  const portalContainer = document.createElement('div')
  const reactRoots = new Set<Element>([container])
  const overlayHosts = new Set<Element>()
  const registerPortal = vi.fn((element: Element) => {
    reactRoots.add(element)
    overlayHosts.add(element)
    return () => { reactRoots.delete(element); overlayHosts.delete(element) }
  })
  const stream = document.createElement('output')
  document.body.append(container, portalContainer, stream)
  const ingest = makeFrameIngest<string, string>({ write: ({ value }) => { stream.textContent = value } })
  const App = () => {
    const [count, setCount] = useState(treeExpanded ? 1 : 0)
    if (scenario === 'send') return createElement(OwnedComposer)
    // SidebarTree's capture handler toggles the focused row and stops native propagation.
    // Children are siblings here, so only the row's aria-expanded can establish ownership.
    if (scenario === 'tree') return createElement('div', { role: 'treegrid', id: 'tree', onKeyDownCapture: (event: React.KeyboardEvent) => {
      if ((event.target as Element).getAttribute('role') !== 'row') return
      if (event.key === 'ArrowRight') setCount(1)
      else if (event.key === 'ArrowLeft') setCount(0)
      else return
      event.preventDefault()
      event.stopPropagation()
    } },
      createElement('div', { role: 'row', id: 'tree-row', tabIndex: 0, 'data-sidebar-tree-key': 'folder', 'aria-expanded': count !== 0 }, 'Folder'),
      count === 0 ? null : createElement('div', { role: 'row', 'data-sidebar-tree-key': 'child', id: 'tree-child' }, 'Child'),
    )
    if (scenario === 'menu') return createElement('div', {},
      createElement('button', { id: 'menu', onClick: () => setCount((value) => value + 1) }, 'Open'),
      createElement(RegisteredPortal, { container: portalContainer, children: count === 0 ? null : createElement('div', { role: 'menu' }, 'Opened') }),
    )
    if (appShell) return createElement('div', { onClick: (event: React.MouseEvent) => {
      if ((event.target as Element).closest('[data-changelog-popover]') !== null) {
        event.stopPropagation()
        setTimeout(() => flushSync(() => setCount((value) => value + 1)), 0)
      }
    } }, createElement(RealAppShell), createElement('output', { id: 'shell-timer' }, count))
    const content = createElement('div', {},
      createElement('button', { id: 'real', onClick: (event) => {
        if (stopped) event.stopPropagation()
        if (nested) document.querySelector<HTMLButtonElement>('#noop')!.click()
        setCount((previous) => previous + 1)
      } }, String(count)),
      createElement('button', { id: 'noop', [captureNoop ? 'onClickCapture' : 'onClick']: (event: React.MouseEvent) => {
        if (stopped) event.stopPropagation()
        if (scheduleAfter) ingest.accept({ key: 'stream', value: 'streamed' })
        if (timer !== '') setTimeout(() => {
          if (timer === 'dom') stream.textContent = 'timer'
          else flushSync(() => setCount((previous) => previous + 1))
        }, 0)
      } }, 'no-op'),
      createElement('button', { id: 'capture', onClickCapture: (event) => { event.stopPropagation(); setCount((previous) => previous + 1) } }, 'capture ' + count),
    )
    return portal ? createElement(RegisteredPortal, { container: portalContainer, children: content }) : content
  }
  const root = createRoot(container)
  flushSync(() => root.render(createElement(PortalContainerRegistrationContext.Provider, { value: registerPortal }, createElement(App))))
  for (const target of [container, portalContainer, ...document.querySelectorAll('[data-app-overlay-host]')]) {
    const rootAdd = target.addEventListener.bind(target)
    const rootRemove = target.removeEventListener.bind(target)
    vi.spyOn(target, 'addEventListener').mockImplementation((type, listener, options) => {
      if (listener !== null) rootAdd(type, adapt(listener), options)
    })
    vi.spyOn(target, 'removeEventListener').mockImplementation((type, listener, options) => {
      if (listener !== null) rootRemove(type, wrapped.get(listener) ?? listener, options)
    })
  }
  cleanups.push(() => root.unmount(), () => ingest.dispose())
  cleanups.push(installInputPaint({ reactRoots, overlayHosts, emit: (sample) => samples.push(sample), paint: (callback) => { paints.add(callback); return () => { paints.delete(callback) } } }))
  const click = (id: string) => {
    const event = new MouseEvent('click', { bubbles: true })
    trusted.add(event)
    document.querySelector(id)!.dispatchEvent(event)
  }
  const key = (id: string, value: string) => {
    const event = new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true })
    trusted.add(event)
    document.querySelector(id)!.dispatchEvent(event)
  }
  const frame = async () => {
    for (const [id, callback] of [...frames]) if (frames.delete(id)) callback(performance.now())
    await Promise.resolve()
    for (const callback of [...paints]) callback()
  }
  return { samples, click, key, frame, ingest, stream, reactRoots, portalContainer, registerPortal, unmount: () => flushSync(() => root.unmount()) }
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

it.each([false, true])('attributes an external portal stopped React commit (capture=%s)', async (capture) => {
  const run = setup({ portal: true, stopped: true })
  expect(run.registerPortal).toHaveBeenCalledWith(run.portalContainer)
  run.click(capture ? '#capture' : '#real')
  await Promise.resolve(); await Promise.resolve()
  expect(document.querySelector('#real')!.textContent).toBe('1')
  await run.frame()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
  run.unmount()
  expect(run.reactRoots.has(run.portalContainer)).toBe(false)
})

it.each([
  ['dom', false], ['dom', true], ['react', false], ['react', true],
] as const)('excludes external portal stopped no-op then %s timer (capture=%s)', async (timer, captureNoop) => {
  const run = setup({ portal: true, stopped: true, timer, captureNoop })
  run.click('#noop')
  await new Promise<void>((resolve) => setTimeout(resolve, 0))
  if (timer === 'dom') expect(run.stream.textContent).toBe('timer')
  else expect(document.querySelector('#real')!.textContent).toBe('1')
  await run.frame()
  expect(run.samples).toEqual([])
})

it('uses the registered app-shell host for the real status-bar changelog and excludes its stopped no-op timer', async () => {
  const run = setup({ appShell: true })
  // The diagnostics bar is a lazy chunk (App.tsx), so its status-bar changelog trigger mounts after the import settles.
  let changes: HTMLButtonElement | undefined
  await vi.waitFor(() => {
    changes = [...document.querySelectorAll('button')].find((button) => button.getAttribute('aria-label')?.startsWith('Changes'))
    expect(changes).toBeDefined()
  }, { timeout: 10_000 })
  // Opening is setup, not a measured trusted input.
  flushSync(() => changes!.click())
  await new Promise<void>((resolve) => setTimeout(resolve, 0))
  const popover = document.querySelector('[data-changelog-popover]')!
  expect(popover).not.toBeNull()
  const host = document.querySelector('[data-app-overlay-host]')!
  expect(host.contains(popover)).toBe(true)
  expect(run.reactRoots.has(host)).toBe(true)
  const heading = popover.querySelector('h1')!
  heading.id = 'changelog-noop'
  run.click('#changelog-noop')
  await new Promise<void>((resolve) => setTimeout(resolve, 0))
  expect(document.querySelector('#shell-timer')!.textContent).toBe('1')
  await run.frame()
  expect(run.samples).toEqual([])
})

it('records Send whose only visible commit appends an explicitly owned transcript tail', async () => {
  const run = setup({ scenario: 'send' })
  run.click('#send')
  await Promise.resolve(); await Promise.resolve()
  expect(document.querySelector('#send')!.textContent).toBe('Send')
  expect(document.querySelector('#history')!.textContent).toBe('Sent 1')
  await run.frame()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
})

it('records a menu trigger whose only commit opens a registered portal, using childList only', async () => {
  const observed = vi.spyOn(MutationObserver.prototype, 'observe')
  const run = setup({ scenario: 'menu' })
  run.click('#menu')
  await Promise.resolve(); await Promise.resolve()
  await run.frame()
  expect(document.querySelector('#menu')!.textContent).toBe('Open')
  expect(run.portalContainer.textContent).toBe('Opened')
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
  expect(observed.mock.calls.find(([target]) => target === run.portalContainer)?.[1]).toEqual({ childList: true })
})

it.each([
  { key: 'ArrowRight', expanded: false, after: 'true' },
  { key: 'ArrowLeft', expanded: true, after: 'false' },
])('records $key tree expansion via row attributes while observing only the interactive row', async ({ key, expanded, after }) => {
  const observed = vi.spyOn(MutationObserver.prototype, 'observe')
  const run = setup({ scenario: 'tree', treeExpanded: expanded })
  const row = document.querySelector('#tree-row')!
  run.key('#tree-row', key)
  await Promise.resolve(); await Promise.resolve()
  expect(row.getAttribute('aria-expanded')).toBe(after)
  expect(row.textContent).toBe('Folder')
  expect(document.querySelector('#tree-child') !== null).toBe(after === 'true')
  expect(row.querySelector('#tree-child')).toBeNull()
  await run.frame()
  expect(run.samples.map((sample) => sample.attributes.outcome)).toEqual(['painted'])
  expect(run.samples[0]?.attributes).toMatchObject({ 'input.kind': 'key', surface: 'agents' })
  expect(observed.mock.calls.map(([target]) => target)).toEqual([row])
  expect(observed.mock.calls[0]?.[1]).toMatchObject({ subtree: true, attributes: true, attributeOldValue: true })
})
