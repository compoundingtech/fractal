// @vitest-environment jsdom
/**
 * Build identity and recent changes share one developer-bar entry: the real DevBar composes the
 * kit's build identity disclosure with the app's changelog, the trigger carries the unseen
 * indicator, intent only prefetches, opening shows identity and loads prose, the changes are
 * acknowledged only once the first entry is in view, Escape closes only the popover and returns
 * focus to the trigger, and an overflowed entry's unseen state is mirrored on the kit's +N control.
 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { installNativePopover } from '../testNativePopover.ts'
import { createIntersectionObserverModel } from '../testIntersectionObserver.ts'
import type * as ChangelogModule from '../changelog/build.ts'

const fixture = vi.hoisted(() => ({
  machineVersion: '0.1.0+test.123456789abc',
  entryId: '2026-10-14-shared-devbar',
  title: 'Shared developer bar',
  wait: undefined as Promise<void> | undefined,
  fail: false,
  loads: 0,
  loaded: undefined as typeof ChangelogModule | undefined,
}))

// Node tests stub only the CSS runtime; the composition and disclosure behaviour is under test.
vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
vi.mock('virtual:build-identity', () => ({
  buildIdentity: { displayVersion: '0.1.0', machineVersion: fixture.machineVersion, dirty: false, rev: '123456789abc' },
  deploymentId: undefined,
}))
// jsdom has no canvas; the meter strip and its collector lease are covered by the kit's own suites.
vi.mock('@overeng/meters/react', () => ({
  MetersProvider: ({ children }: { readonly children: unknown }) => children,
  MeterStrip: () => null,
  MeterTile: () => null,
}))
// One deterministic entry instead of the app registry, in both the unseen ids and the lazy prose.
vi.mock('../changelog/metadata.ts', () => ({ changelogDeployment: 'main', changelogEntryIds: [fixture.entryId] }))
vi.mock('../changelog/load.ts', () => ({
  loadedChangelog: () => fixture.loaded,
  loadChangelog: async () => {
    fixture.loads++
    await fixture.wait
    if (fixture.fail) throw new Error('Chunk unavailable')
    fixture.loaded = await import('../changelog/build.ts')
    return fixture.loaded
  },
}))
vi.mock('../changelog/build.ts', () => ({
  changelogDeployment: 'main',
  getChangelogManifest: () => ({
    schemaVersion: 1,
    product: 'fractal-web',
    build: { machineVersion: fixture.machineVersion },
    entries: [{ id: fixture.entryId, date: '2026-10-14', visibility: 'public', title: fixture.title, summary: 'Diagnostics use the shared kit.' }],
  }),
}))

let unmount: (() => Promise<void>) | undefined
let restorePopover: (() => void) | undefined
let intersections = createIntersectionObserverModel()

beforeEach(() => {
  fixture.wait = undefined
  fixture.fail = false
  fixture.loads = 0
  fixture.loaded = undefined
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  intersections = createIntersectionObserverModel()
  vi.stubGlobal('IntersectionObserver', intersections.IntersectionObserver)
  restorePopover = installNativePopover()
  window.localStorage.clear()
})

afterEach(async () => {
  await unmount?.()
  unmount = undefined
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  restorePopover?.()
  restorePopover = undefined
})

/** Fresh modules per test: the bar's preference registry and seen cursor are module singletons. */
const mount = async () => {
  vi.resetModules()
  const [React, { createRoot }, { DevBar }, { RegistryContext }, AtomRegistry] = await Promise.all([
    import('react'), import('react-dom/client'), import('./DevBar.tsx'), import('@effect/atom-react'), import('effect/reactivity/AtomRegistry'),
  ])
  const registry = AtomRegistry.make()
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  unmount = async () => {
    await React.act(async () => root.unmount())
    registry.dispose()
    host.remove()
  }
  await React.act(async () => root.render(
    React.createElement(RegistryContext.Provider, { value: registry }, React.createElement(DevBar, { defaultVisible: true })),
  ))
  return { act: React.act, host }
}

const buildTrigger = (host: HTMLElement) => {
  const trigger = host.querySelector<HTMLButtonElement>('[data-devbar-segment="build"] [data-devbar-segment-content] button[aria-haspopup="dialog"]')
  if (trigger === null) throw new Error('build trigger did not mount in its devbar segment')
  return trigger
}
/** The popover content mounts only while open. */
const buildPopover = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="Build and changes"] [data-devbar-build-changes]')
const unseenName = /^Build identity: 1234567 · n\/a, show Build and changes, unseen changes$/
const seenName = /^Build identity: 1234567 · n\/a, show Build and changes$/
const storedCursor = () => JSON.parse(window.localStorage.getItem('wf.ui@1') ?? '{}').entries?.['changelog.fractal-web.main']
const firstEntryTitle = () => [...(buildPopover()?.querySelectorAll('h3') ?? [])].find((heading) => heading.textContent === fixture.title)

it('opens build identity and recent changes from one devbar trigger, acknowledges them once the first entry is in view, and closes on Escape with focus restored', async () => {
  const { act, host } = await mount()
  const trigger = buildTrigger(host)
  expect(trigger.getAttribute('aria-label')).toMatch(unseenName)
  expect(trigger.querySelector('[data-devbar-unseen]')).not.toBeNull()
  expect(host.querySelectorAll('button[aria-haspopup="dialog"][aria-label^="Build identity"]')).toHaveLength(1)

  // A kit panel stays open underneath; Escape must close only the build popover.
  const counters = [...host.querySelectorAll('button')].find((button) => button.textContent === 'Counters')
  await act(async () => counters?.click())
  expect(host.querySelector('section[aria-label="Counters"]')).not.toBeNull()

  trigger.focus()
  await act(async () => trigger.click())
  await vi.waitFor(async () => {
    await act(async () => {})
    expect(buildPopover()?.textContent).toContain(fixture.title)
  })
  expect(trigger.getAttribute('aria-expanded')).toBe('true')
  expect(buildPopover()?.querySelector('[data-build-summary]')?.textContent).toContain('1234567')
  expect(buildPopover()?.textContent).toContain('123456789abc')
  expect(buildPopover()?.textContent).toContain(fixture.machineVersion)
  expect(buildPopover()?.textContent).toContain("What's new")
  // Opening alone proves nothing was read: the first entry is observed but not yet in view.
  const title = firstEntryTitle()!
  expect(intersections.observed()).toEqual([title])
  expect(trigger.getAttribute('aria-label')).toMatch(unseenName)
  expect(trigger.querySelector('[data-devbar-unseen]')).not.toBeNull()
  expect(storedCursor()).toBeUndefined()

  await act(async () => intersections.reveal(title))
  expect(trigger.getAttribute('aria-label')).toMatch(seenName)
  expect(trigger.querySelector('[data-devbar-unseen]')).toBeNull()
  expect(storedCursor()?.value).toEqual({ visited: true, entryIds: [fixture.entryId] })
  expect(intersections.observed()).toEqual([])
  // The open list keeps the grouping it was read with.
  expect(firstEntryTitle()).toBe(title)

  const content = buildPopover()!
  await act(async () => { content.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })) })
  await vi.waitFor(async () => {
    await act(async () => {})
    expect(buildPopover()).toBeNull()
    expect(document.activeElement).toBe(buildTrigger(host))
  })
  expect(buildTrigger(host).getAttribute('aria-expanded')).toBe('false')
  expect(host.querySelector('section[aria-label="Counters"]')).not.toBeNull()
}, 30_000)

it('prefetches on focus without opening or acknowledging changes', async () => {
  const { act, host } = await mount()
  expect(fixture.loads).toBe(0)
  await act(async () => buildTrigger(host).focus())
  expect(fixture.loads).toBe(1)
  expect(buildPopover()).toBeNull()
  expect(buildTrigger(host).getAttribute('aria-label')).toMatch(unseenName)
  expect(JSON.parse(window.localStorage.getItem('wf.ui@1') ?? '{}').entries?.['changelog.fractal-web.main']).toBeUndefined()
  await act(async () => buildTrigger(host).click())
  expect(buildPopover()?.textContent).toContain(fixture.title)
  expect(buildPopover()?.querySelector('[role="status"]')).toBeNull()
}, 30_000)

it('opens idle-prefetched code without an intermediate loading state', async () => {
  await (await import('../changelog/load.ts')).loadChangelog()
  const { act, host } = await mount()
  await act(async () => buildTrigger(host).click())
  expect(buildPopover()?.textContent).toContain(fixture.title)
  expect(buildPopover()?.querySelector('[role="status"]')).toBeNull()
  expect(fixture.loads).toBe(1)
}, 30_000)

it('opens immediately on a cold load, contains a failed chunk with retry, and acknowledges only once an entry is in view', async () => {
  const { promise, resolve: release } = Promise.withResolvers<void>()
  fixture.wait = promise
  fixture.fail = true
  const { act, host } = await mount()
  await act(async () => buildTrigger(host).click())
  expect(buildPopover()?.textContent).toContain('Loading recent changes')
  expect(intersections.observed()).toEqual([])
  await act(async () => release())
  await vi.waitFor(async () => {
    await act(async () => {})
    expect(buildPopover()?.textContent).toContain('workspace is unaffected')
  })
  expect(intersections.observed()).toEqual([])
  expect(buildTrigger(host).getAttribute('aria-label')).toMatch(unseenName)
  fixture.fail = false
  const retry = [...buildPopover()!.querySelectorAll('button')].find(button => button.textContent === 'Retry')
  await act(async () => retry?.click())
  await vi.waitFor(async () => {
    await act(async () => {})
    expect(buildPopover()?.textContent).toContain(fixture.title)
  })
  expect(buildTrigger(host).getAttribute('aria-label')).toMatch(unseenName)
  expect(storedCursor()).toBeUndefined()
  await act(async () => intersections.reveal(firstEntryTitle()!))
  expect(buildTrigger(host).getAttribute('aria-label')).toMatch(seenName)
  expect(storedCursor()?.value).toEqual({ visited: true, entryIds: [fixture.entryId] })
}, 30_000)

it('mirrors the unseen indicator on the overflow control when the build segment is hidden', async () => {
  // jsdom reports no row width, so any measured content overflows every segment.
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(DOMRect.fromRect({ width: 48, height: 20 }))
  const { host } = await mount()
  const overflow = host.querySelector<HTMLButtonElement>('button[aria-haspopup="dialog"][aria-label*="more status items"]')
  expect(overflow?.getAttribute('aria-label')).toMatch(/: .*build, including unseen changes$/)
  expect(overflow?.getAttribute('aria-label')).not.toMatch(/\b(version|changelog)\b/)
  expect(overflow?.querySelector('[data-devbar-unseen]')).not.toBeNull()
}, 30_000)

it('publishes the measured bar height to the workspace, follows resizes, and clears it when hidden or unmounted', async () => {
  const observers: { readonly notify: () => void; readonly observed: Set<Element>; disconnected: boolean }[] = []
  vi.stubGlobal('ResizeObserver', class {
    readonly entry
    constructor(notify: () => void) {
      this.entry = { notify, observed: new Set<Element>(), disconnected: false }
      observers.push(this.entry)
    }
    observe(element: Element) { this.entry.observed.add(element) }
    unobserve(element: Element) { this.entry.observed.delete(element) }
    disconnect() { this.entry.disconnected = true }
  })
  let rowHeight = 44
  const isRow = (element: Element) => element.matches('[role="toolbar"][aria-label="Developer bar"]')
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    return DOMRect.fromRect({ width: 48, height: isRow(this) ? rowHeight : 500 })
  })
  const space = () => document.body.style.getPropertyValue('--wf-devbar-space')
  const rowObservers = () => observers.filter(({ observed, disconnected }) => !disconnected && [...observed].some(isRow) && observed.size === 1)

  const { act, host } = await mount()
  // Same module instance as the mounted bar: mount resets the graph once, before importing it.
  const { toggleDevBar: toggleBar } = await import('./devBarPreference.ts')
  expect(space()).toBe('44px')

  rowHeight = 52
  expect(rowObservers()).toHaveLength(1)
  await act(async () => rowObservers()[0]!.notify())
  expect(space()).toBe('52px')

  await act(async () => toggleBar())
  expect(host.querySelector('[data-testid="wf-devbar"]')).toBeNull()
  expect(space()).toBe('')
  expect(rowObservers()).toHaveLength(0)

  rowHeight = 60
  await act(async () => toggleBar())
  expect(space()).toBe('60px')
  expect(rowObservers()).toHaveLength(1)

  await unmount?.()
  unmount = undefined
  expect(space()).toBe('')
  expect(rowObservers()).toHaveLength(0)
}, 30_000)

it('updates request details from the instrumentation without a timer, and unsubscribes on unmount', async () => {
  vi.resetModules()
  const transport = await import('./transport.ts')
  const { instrumentFetch, subscribeRequestLog } = transport
  let subscribers = 0
  vi.spyOn(transport, 'subscribeRequestLog').mockImplementation((notify) => {
    subscribers++
    const unsubscribe = subscribeRequestLog(notify)
    return () => {
      subscribers--
      unsubscribe()
    }
  })
  const [React, { createRoot }, { RequestDetails }] = await Promise.all([
    import('react'), import('react-dom/client'), import('./transportMeters.tsx'),
  ])
  const interval = vi.spyOn(globalThis, 'setInterval')
  const fetch = instrumentFetch({ fetchImpl: async () => new Response(undefined, { status: 404 }), origin: 'https://example.test' })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await React.act(async () => root.render(React.createElement(RequestDetails)))
  expect(subscribers).toBe(1)
  expect(interval).not.toHaveBeenCalled()
  expect(container.textContent).toContain('Latest errors\nnone')

  await React.act(async () => { await fetch('https://example.test/v1/client/usage/quota') })
  expect(container.textContent).toContain('404')
  expect(container.textContent).toContain('GET /v1/client/usage/quota')
  expect(interval).not.toHaveBeenCalled()

  await React.act(async () => root.unmount())
  container.remove()
  expect(subscribers).toBe(0)
}, 30_000)
