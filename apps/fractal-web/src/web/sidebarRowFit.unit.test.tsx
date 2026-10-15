// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SidebarRowSignals, attachSidebarLine1Fit } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarRowSignals.tsx'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, props: () => ({}) }))

// jsdom has no layout. Elements report the widths their data attributes name, and every layout read
// or style write is logged, so a test can count the reads that would force a layout in a browser.
let log: ('read' | 'write')[]
let frames: FrameRequestCallback[]
const restore: (() => void)[] = []
const patch = <T extends object>(target: T, key: string, descriptor: (original: PropertyDescriptor) => PropertyDescriptor) => {
  const original = Object.getOwnPropertyDescriptor(target, key)
  Object.defineProperty(target, key, { ...descriptor(original ?? { value: undefined }), configurable: true })
  restore.push(() => original === undefined ? Reflect.deleteProperty(target, key) : Object.defineProperty(target, key, original))
}
// SidebarRowSignals renders its own span and a wrapper per field, so those take their widths from
// the row's data-signals-cw and from the field's content.
const width = (element: Element, name: string) => {
  const source = name === 'cw' && element.getAttribute('data-row-column') === 'subtitle' ? element.closest<HTMLElement>('[data-signals-cw]')?.dataset.signalsCw
    : element.hasAttribute('data-row-priority') ? (element.firstElementChild as HTMLElement | null)?.dataset[name]
    : (element as HTMLElement).dataset?.[name]
  return Number(source ?? 0)
}
const runFrames = () => { while (frames.length > 0) frames.splice(0).forEach(frame => frame(0)) }
/** Reads that follow a write each force a layout in a browser. */
const forcedLayouts = () => log.filter((entry, index) => entry === 'read' && log[index - 1] === 'write').length

let root: Root
let host: HTMLDivElement

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  frames = []
  log = []
  vi.stubGlobal('requestAnimationFrame', (frame: FrameRequestCallback) => frames.push(frame))
  vi.stubGlobal('cancelAnimationFrame', () => undefined)
  patch(Element.prototype, 'getBoundingClientRect', () => ({ value(this: Element) { log.push('read'); return new DOMRect(0, 0, width(this, 'w'), 10) } }))
  patch(Element.prototype, 'clientWidth', () => ({ get(this: Element) { log.push('read'); return width(this, 'cw') } }))
  patch(Element.prototype, 'scrollWidth', () => ({ get(this: Element) { log.push('read'); return width(this, 'w') } }))
  patch(Range.prototype, 'getBoundingClientRect', () => ({ value() { log.push('read'); return new DOMRect() } }))
  const getComputedStyle = window.getComputedStyle.bind(window)
  // jsdom builds a computed declaration through the same setters; those are not writes to the page.
  let computing = false
  vi.stubGlobal('getComputedStyle', (element: Element) => { log.push('read'); computing = true; try { return getComputedStyle(element) } finally { computing = false } })
  for (const key of ['display', 'position', 'visibility', 'maxWidth', 'width', 'flexShrink'].filter(key => Object.getOwnPropertyDescriptor(CSSStyleDeclaration.prototype, key)?.set !== undefined))
    patch(CSSStyleDeclaration.prototype, key, original => ({ get: original.get, set(this: CSSStyleDeclaration, value: string) { if (!computing) log.push('write'); original.set!.call(this, value) } }))
  patch(CSSStyleDeclaration.prototype, 'setProperty', original => ({ value(this: CSSStyleDeclaration, ...args: [string, string]) { if (!computing) log.push('write'); return original.value.apply(this, args) } }))
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(() => {
  act(() => root.unmount())
  // The fit batches are module state; a browser always runs the frame they asked for.
  runFrames()
  host.remove()
  restore.splice(0).reverse().forEach(undo => undo())
  vi.unstubAllGlobals()
})

const Row = ({ titleWidth = 150, signalsWidth = 200, fieldWidths = [30, 30] }: { readonly titleWidth?: number; readonly signalsWidth?: number; readonly fieldWidths?: readonly number[] }) => {
  const anchor = React.useCallback((node: HTMLDivElement | null) => attachSidebarLine1Fit(node), [])
  return <div ref={anchor} data-testid="taste-agent-row" data-cw="200" data-signals-cw={signalsWidth}>
    <span data-row-column="title-text" data-cw={titleWidth} />
    <span data-row-column="time" data-cw="40"><span data-w="24">4m<span data-line1-drop="tokens">12k</span><span data-line1-drop="scope">st</span></span></span>
    <SidebarRowSignals>{fieldWidths.map((field, index) => <span key={index} data-w={field}>field {index}</span>)}</SidebarRowSignals>
  </div>
}
const sidebar = (rows: number, props: React.ComponentProps<typeof Row> = {}) =>
  <div data-testid="agent-sidebar">{Array.from({ length: rows }, (_, index) => <Row key={index} {...props} />)}</div>

describe('sidebar row fitting', () => {
  it('measures nothing while React commits the rows', () => {
    act(() => root.render(sidebar(40)))
    expect(log.filter(entry => entry === 'read')).toEqual([])
  })

  it('fits every row in one frame with a bounded number of forced layouts', () => {
    act(() => root.render(sidebar(40)))
    // Each row's first fit is queued; one frame callback per kind of fit runs them all.
    frames.splice(0).forEach(frame => frame(0))
    const rows = [...host.querySelectorAll<HTMLElement>('[data-testid="taste-agent-row"]')]
    expect(rows.every(row => row.dataset.rowTimeNatural === '24')).toBe(true)
    expect(host.querySelectorAll('[data-row-dropped]')).toHaveLength(80)
    // Measuring each row in turn after its own writes would force at least one layout per row.
    expect(forcedLayouts()).toBeLessThanOrEqual(6)
  })

  it('keeps the fitting rules: wide fields drop, and line 1 drops counts before scope', () => {
    act(() => root.render(sidebar(2, { titleWidth: 100, signalsWidth: 100, fieldWidths: [30, 160, 20] })))
    runFrames()
    const fields = [...host.querySelector('[data-row-column="subtitle"]')!.children] as HTMLElement[]
    // Later fields are placed first; the 160px field cannot fit in what is left and drops, and the first still fits after it.
    expect(fields.map(field => field.dataset.rowDropped)).toEqual(['false', 'true', 'false'])
    // A title under sixty percent of the row drops every line-1 extra.
    const drops = [...host.querySelectorAll<HTMLElement>('[data-line1-drop]')].slice(0, 2)
    expect(drops.map(drop => drop.style.display)).toEqual(['none', 'none'])
  })

  it('leaves line 1 whole when the title has room', () => {
    act(() => root.render(sidebar(3, { titleWidth: 150 })))
    runFrames()
    expect([...host.querySelectorAll<HTMLElement>('[data-line1-drop]')].map(drop => drop.style.display)).toEqual(Array(6).fill(''))
  })
})
