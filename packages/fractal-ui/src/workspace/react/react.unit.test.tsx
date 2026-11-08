// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { currentSpace, emptyWorkspace, panesOf, tabsOf, transition, type Command, type Layout, type WorkspaceConfig, type WorkspaceState } from '../mod.ts'
import type { WorkspaceStore } from '../store.ts'
import { actionFor, defaultBindings, matchesCombo, Pane, PaneAside, PaneBody, PaneContent, Tab, TabStrip, useEntityDrag, usePaneAside, useWorkspaceKeys, Workspace, WorkspaceProvider } from './mod.ts'

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
// jsdom has no CSS.escape, which React Aria's keyboard navigation uses.
if (typeof globalThis.CSS?.escape !== 'function') (globalThis as { CSS: Pick<typeof CSS, 'escape'> }).CSS = { escape: (value: string) => value.replace(/["\\]/g, '\\$&') }

const counter = (): WorkspaceConfig => {
  let next = 0
  return { newId: () => `id-${next++}` }
}

/** A synchronous in-memory store over the pure model, as the Effect store behaves for React. */
const memoryStore = (refs: readonly string[]): WorkspaceStore<unknown> & { readonly commands: Command<unknown>[] } => {
  const config = counter()
  let state: WorkspaceState<unknown> = refs.reduce((current, ref) => transition(current, { _tag: 'Open', ref }, config), emptyWorkspace<unknown>(config, { id: 'space' }))
  const listeners = new Set<() => void>()
  const commands: Command<unknown>[] = []
  return {
    commands,
    dispatch: command => {
      commands.push(command)
      const next = transition(state, command, config)
      if (next === state) return
      state = next
      for (const listener of listeners) listener()
    },
    getState: () => state,
    subscribe: listener => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    flush: () => {},
  }
}

const describeRef = (ref: string) => ({ kind: ref.split(':')[0] ?? 'thing', id: ref, title: ref.toUpperCase() })

const AsideToggle = () => {
  const aside = usePaneAside()
  return aside === undefined ? null : <button type="button" data-testid="toggle" onClick={aside.toggle}>{aside.isOpen ? 'close' : 'open'}</button>
}

const Keys = ({ onEdge, onPalette }: { readonly onEdge: (direction: string) => void; readonly onPalette: () => void }) => {
  useWorkspaceKeys({ onEdge, handlers: { palette: () => { onPalette() } } })
  return null
}

const Row = ({ entity }: { readonly entity: string }) => {
  const drag = useEntityDrag({ ref: entity })
  return <div data-testid={`row-${entity}`} {...drag}>{entity}</div>
}

const App = ({ store, aside = false, onEdge = () => {}, onPalette = () => {} }: {
  readonly store: WorkspaceStore<unknown>
  readonly aside?: boolean
  readonly onEdge?: (direction: string) => void
  readonly onPalette?: () => void
}) => <WorkspaceProvider store={store} describe={describeRef} renderDragChip={entity => <span data-testid="chip">{entity.title}</span>}>
  <Keys onEdge={onEdge} onPalette={onPalette} />
  <Row entity="note:x" />
  <Workspace renderPane={pane => <Pane key={pane.id} pane={pane} className={({ isFocused }) => isFocused ? 'focused' : 'pane'}>
    <TabStrip aria-label="Tabs" trailing={<AsideToggle />}>
      {tab => <Tab tab={tab}>{({ entity, isActive }) => `${entity.title}${isActive ? '*' : ''}`}</Tab>}
    </TabStrip>
    <PaneBody>
      <PaneContent renderDropOverlay={({ zone }) => <div data-testid="overlay">{zone._tag === 'Center' ? 'center' : zone.side}</div>}>
        {({ activeTab }) => <p>content of {activeTab?.ref ?? 'nothing'}</p>}
      </PaneContent>
      {aside ? <PaneAside threshold={600} defaultSize={240} minSize={160}>{({ activeTab }) => <p>details of {activeTab?.ref}</p>}</PaneAside> : null}
    </PaneBody>
  </Pane>} />
</WorkspaceProvider>

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  act(() => root.unmount())
  container.remove()
  vi.restoreAllMocks()
})

const render = (element: React.ReactElement) => act(() => root.render(element))
const tabs = () => Array.from(container.querySelectorAll<HTMLElement>('[role="tab"]'))
const key = (target: EventTarget, init: KeyboardEventInit) => act(() => {
  target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }))
  target.dispatchEvent(new KeyboardEvent('keyup', { bubbles: true, cancelable: true, ...init }))
})
const pointer = (target: EventTarget, type: string, init: { clientX?: number; clientY?: number; button?: number } = {}) => act(() => {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, ...init })
  Object.assign(event, { pointerId: 1, pointerType: 'mouse', width: 1, height: 1 })
  target.dispatchEvent(event)
})
const rect = (x: number, y: number, width: number, height: number): DOMRect =>
  ({ x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON: () => ({}) }) as DOMRect

describe('tab strips', () => {
  it('renders a tab list whose tabs name their entity, and selects on click', () => {
    const store = memoryStore(['doc:a', 'doc:b'])
    render(<App store={store} />)
    expect(container.querySelector('[role="tablist"]')?.getAttribute('aria-label')).toBe('Tabs')
    expect(tabs().map(tab => tab.textContent)).toEqual(['DOC:A', 'DOC:B*'])
    expect(container.querySelector('[role="tabpanel"]')?.textContent).toBe('content of doc:b')
    pointer(tabs()[0]!, 'pointerdown')
    pointer(tabs()[0]!, 'pointerup')
    act(() => tabs()[0]!.click())
    expect(activeRef(store)).toBe('doc:a')
    expect(tabs()[0]!.hasAttribute('data-active')).toBe(true)
  })

  it('moves with the arrows, reorders with Alt+Shift+arrows and closes with Delete', () => {
    const store = memoryStore(['doc:a', 'doc:b', 'doc:c'])
    render(<App store={store} />)
    act(() => tabs()[2]!.focus())
    key(tabs()[2]!, { key: 'ArrowLeft' })
    expect(activeRef(store)).toBe('doc:b')
    key(document.activeElement!, { key: 'ArrowRight', altKey: true, shiftKey: true })
    expect(tabsOf(currentSpace(store.getState()).layout).map(tab => tab.ref)).toEqual(['doc:a', 'doc:c', 'doc:b'])
    key(tabs().find(tab => tab.hasAttribute('data-active'))!, { key: 'Delete' })
    expect(tabsOf(currentSpace(store.getState()).layout).map(tab => tab.ref)).toEqual(['doc:a', 'doc:c'])
  })
})

const activeRef = (store: WorkspaceStore<unknown>) => {
  const space = currentSpace(store.getState())
  const pane = panesOf(space.layout).find(row => row.id === space.local.focus)!
  return pane.tabs.find(tab => tab.id === pane.active)?.ref
}

describe('splits', () => {
  it('renders splits with a keyboard-resizable separator that resets on double-click', () => {
    const store = memoryStore(['doc:a', 'doc:b'])
    store.dispatch({ _tag: 'Split', side: 'right', tab: tabsOf(currentSpace(store.getState()).layout)[1]!.id })
    render(<App store={store} />)
    const separator = container.querySelector<HTMLElement>('[data-divider]')!
    expect(separator.getAttribute('aria-orientation')).toBe('vertical')
    expect(separator.getAttribute('aria-valuenow')).toBe('50')
    act(() => separator.focus())
    key(separator, { key: 'ArrowRight' })
    expect(separator.getAttribute('aria-valuenow')).toBe('52')
    key(separator, { key: 'ArrowLeft', shiftKey: true })
    expect(separator.getAttribute('aria-valuenow')).toBe('42')
    act(() => separator.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })))
    expect(separator.getAttribute('aria-valuenow')).toBe('50')
  })

  it('focuses a pane pressed within, and moves focus by direction, routing the left edge to the host', () => {
    const store = memoryStore(['doc:a', 'doc:b'])
    store.dispatch({ _tag: 'Split', side: 'right', tab: tabsOf(currentSpace(store.getState()).layout)[1]!.id })
    const onEdge = vi.fn()
    render(<App store={store} onEdge={onEdge} />)
    const [left, right] = Array.from(container.querySelectorAll<HTMLElement>('[data-pane]'))
    expect(right!.className).toBe('focused')
    pointer(left!.querySelector('p')!, 'pointerdown')
    expect(left!.className).toBe('focused')
    key(document.body, { key: 'ArrowLeft', code: 'ArrowLeft', altKey: true })
    expect(onEdge).toHaveBeenCalledWith('left')
    key(document.body, { key: 'ArrowRight', code: 'ArrowRight', altKey: true })
    expect(right!.className).toBe('focused')
  })

  it('zooms with Alt+O, showing only the focused pane', () => {
    const store = memoryStore(['doc:a', 'doc:b'])
    store.dispatch({ _tag: 'Split', side: 'right', tab: tabsOf(currentSpace(store.getState()).layout)[1]!.id })
    render(<App store={store} />)
    key(document.body, { key: 'ø', code: 'KeyO', altKey: true })
    expect(container.querySelectorAll('[data-pane-slot]:not([hidden])')).toHaveLength(1)
    expect(container.querySelectorAll('[data-divider]')).toHaveLength(0)
    expect(container.querySelector('[data-workspace]')!.hasAttribute('data-zoomed')).toBe(true)
  })
})

describe('pane identity', () => {
  it('never remounts a pane\'s content when the layout splits, a neighbour closes, or it zooms', () => {
    const mounts = new Map<string, number>()
    const Probe = ({ id }: { readonly id: string }) => {
      React.useState(() => mounts.set(id, (mounts.get(id) ?? 0) + 1))
      return <p>{id}</p>
    }
    const store = memoryStore(['doc:a', 'doc:b'])
    const first = panesOf(currentSpace(store.getState()).layout)[0]!.id
    render(<WorkspaceProvider store={store} describe={describeRef}>
      <Workspace renderPane={pane => <Pane key={pane.id} pane={pane}><PaneContent><Probe id={pane.id} /></PaneContent></Pane>} />
    </WorkspaceProvider>)
    act(() => store.dispatch({ _tag: 'Split', side: 'right', tab: tabsOf(currentSpace(store.getState()).layout)[1]!.id }))
    const second = panesOf(currentSpace(store.getState()).layout)[1]!.id
    act(() => store.dispatch({ _tag: 'Split', side: 'bottom', pane: first, ref: 'note:x' }))
    act(() => store.dispatch({ _tag: 'FocusPane', pane: first }))
    act(() => store.dispatch({ _tag: 'ToggleZoom' }))
    expect(container.querySelectorAll('[data-pane-slot]:not([hidden])')).toHaveLength(1)
    act(() => store.dispatch({ _tag: 'ToggleZoom' }))
    act(() => store.dispatch({ _tag: 'Close', tab: tabsOf(currentSpace(store.getState()).layout).find(tab => tab.ref === 'doc:b')!.id }))
    expect(panesOf(currentSpace(store.getState()).layout).map(pane => pane.id)).not.toContain(second)
    expect(mounts.get(first)).toBe(1)
    expect(mounts.get(second)).toBe(1)
  })
})

describe('pointer drag and drop', () => {
  it('drags a row onto a pane edge with a live chip and overlay, and splits there', () => {
    const store = memoryStore(['doc:a'])
    render(<App store={store} />)
    const content = container.querySelector<HTMLElement>('[data-drop-content]')!
    vi.spyOn(content, 'getBoundingClientRect').mockReturnValue(rect(0, 0, 400, 300))
    document.elementsFromPoint = () => [content]
    const row = container.querySelector('[data-testid="row-note:x"]')!
    pointer(row, 'pointerdown', { clientX: 200, clientY: 150 })
    pointer(window, 'pointermove', { clientX: 202, clientY: 150 })
    expect(document.querySelector('[data-testid="chip"]'), 'a press is not yet a drag').toBeNull()
    pointer(window, 'pointermove', { clientX: 390, clientY: 150 })
    expect(document.querySelector('[data-testid="chip"]')?.textContent).toBe('NOTE:X')
    expect(document.documentElement.style.userSelect, 'no text selection while dragging').toBe('none')
    expect(container.querySelector('[data-testid="overlay"]')?.textContent).toBe('right')
    expect(content.getAttribute('data-drop-zone')).toBe('right')
    pointer(window, 'pointerup', { clientX: 390, clientY: 150 })
    expect(document.querySelector('[data-testid="chip"]')).toBeNull()
    expect(document.documentElement.style.userSelect).toBe('')
    expect(panesOf(currentSpace(store.getState()).layout).map(pane => pane.tabs.map(tab => tab.ref))).toEqual([['doc:a'], ['note:x']])
  })

  it('moves a tab into another pane\'s strip, at the slot under the pointer', () => {
    const store = memoryStore(['doc:a', 'doc:b', 'doc:c'])
    store.dispatch({ _tag: 'Split', side: 'right', tab: tabsOf(currentSpace(store.getState()).layout)[2]!.id })
    render(<App store={store} />)
    const strips = Array.from(container.querySelectorAll<HTMLElement>('[data-drop-strip]'))
    const target = strips[1]!
    vi.spyOn(target.querySelector<HTMLElement>('[data-workspace-tab]')!, 'getBoundingClientRect').mockReturnValue(rect(500, 0, 80, 24))
    document.elementsFromPoint = () => [target]
    const source = tabs()[0]!
    pointer(source, 'pointerdown', { clientX: 10, clientY: 10 })
    pointer(window, 'pointermove', { clientX: 300, clientY: 10 })
    pointer(window, 'pointermove', { clientX: 510, clientY: 10 })
    expect(tabs()[2]!.hasAttribute('data-drop-before')).toBe(true)
    pointer(window, 'pointerup', { clientX: 510, clientY: 10 })
    expect(panesOf(currentSpace(store.getState()).layout).map(pane => pane.tabs.map(tab => tab.ref))).toEqual([['doc:b'], ['doc:a', 'doc:c']])
  })

  it('cancels on Escape and drops nothing where nothing would change', () => {
    const store = memoryStore(['doc:a'])
    render(<App store={store} />)
    const content = container.querySelector<HTMLElement>('[data-drop-content]')!
    vi.spyOn(content, 'getBoundingClientRect').mockReturnValue(rect(0, 0, 400, 300))
    document.elementsFromPoint = () => [content]
    pointer(tabs()[0]!, 'pointerdown', { clientX: 10, clientY: 10 })
    pointer(window, 'pointermove', { clientX: 390, clientY: 150 })
    expect(container.querySelector('[data-testid="overlay"]'), 'its own single-tab pane\'s edge is no drop').toBeNull()
    const row = container.querySelector('[data-testid="row-note:x"]')!
    pointer(window, 'pointerup')
    pointer(row, 'pointerdown', { clientX: 10, clientY: 10 })
    pointer(window, 'pointermove', { clientX: 200, clientY: 150 })
    expect(container.querySelector('[data-testid="overlay"]')?.textContent).toBe('center')
    key(window, { key: 'Escape' })
    pointer(window, 'pointerup', { clientX: 200, clientY: 150 })
    expect(tabsOf(currentSpace(store.getState()).layout).map(tab => tab.ref)).toEqual(['doc:a'])
  })
})

describe('the aside', () => {
  it('opens by default in a wide pane, and an explicit toggle holds', () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(rect(0, 0, 900, 600))
    const store = memoryStore(['doc:a'])
    render(<App store={store} aside />)
    const aside = container.querySelector<HTMLElement>('[data-pane-aside]')!
    expect(aside.textContent).toBe('details of doc:a')
    expect(aside.style.flex).toBe('0 0 240px')
    act(() => container.querySelector<HTMLElement>('[data-testid="toggle"]')!.click())
    expect(container.querySelector('[data-pane-aside]')).toBeNull()
    const pane = panesOf(currentSpace(store.getState()).layout)[0]!
    expect(currentSpace(store.getState()).local.panes[pane.id]?.aside?.explicit).toEqual({ open: false, band: 'wide' })
    const resizer = () => container.querySelector<HTMLElement>('[data-aside-resizer]')
    act(() => container.querySelector<HTMLElement>('[data-testid="toggle"]')!.click())
    act(() => resizer()!.focus())
    key(resizer()!, { key: 'ArrowLeft' })
    expect(container.querySelector<HTMLElement>('[data-pane-aside]')!.style.flex).toBe('0 0 256px')
  })

  it('stays closed by default in a narrow pane', () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(rect(0, 0, 400, 600))
    render(<App store={memoryStore(['doc:a'])} aside />)
    expect(container.querySelector('[data-pane-aside]')).toBeNull()
    expect(container.querySelector('[data-testid="toggle"]')?.textContent).toBe('open')
  })
})

describe('keys', () => {
  const event = (init: Partial<{ key: string; code: string; altKey: boolean; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }>) =>
    ({ key: '', code: '', altKey: false, shiftKey: false, metaKey: false, ctrlKey: false, ...init })

  it('matches Mod per platform and Alt chords by physical key', () => {
    expect(matchesCombo(event({ key: 'd', code: 'KeyD', metaKey: true }), 'Mod+D', true)).toBe(true)
    expect(matchesCombo(event({ key: 'd', code: 'KeyD', ctrlKey: true }), 'Mod+D', false)).toBe(true)
    expect(matchesCombo(event({ key: 'd', code: 'KeyD', ctrlKey: true }), 'Mod+D', true)).toBe(false)
    expect(matchesCombo(event({ key: '∑', code: 'KeyW', altKey: true }), 'Alt+W', true)).toBe(true)
    expect(actionFor(event({ key: 'D', code: 'KeyD', metaKey: true, shiftKey: true }), defaultBindings(true), true)).toBe('splitDown')
    expect(actionFor(event({ key: 't', code: 'KeyT', ctrlKey: true }), defaultBindings(true), true)).toBe('palette')
    expect(actionFor(event({ key: 't', code: 'KeyT', ctrlKey: true }), defaultBindings(false), false)).toBeUndefined()
  })

  it('splits with Mod+D and hands host actions to the host', () => {
    const store = memoryStore(['doc:a'])
    const onPalette = vi.fn()
    render(<App store={store} onPalette={onPalette} />)
    const mac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)
    key(document.body, { key: 'd', code: 'KeyD', metaKey: mac, ctrlKey: !mac })
    expect(panesOf(currentSpace(store.getState()).layout)).toHaveLength(2)
    expect((currentSpace(store.getState()).layout as Extract<Layout, { axis: string }>).axis).toBe('row')
    key(document.body, { key: 'k', code: 'KeyK', metaKey: mac, ctrlKey: !mac })
    expect(onPalette).toHaveBeenCalledOnce()
  })
})
