// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { WorkspaceProvider } from '@fractal/ui/workspace/react'
import { DraggableMention } from './DraggableMention.tsx'
import { createWorkspaceStore } from './store.ts'
import { currentSpace, tabsOf } from './model.ts'
import * as Workspace from '@fractal/ui/workspace'

let container: HTMLDivElement
let root: Root
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})
afterEach(() => {
  act(() => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

const pointer = (target: EventTarget, type: string, init: { clientX?: number; clientY?: number } = {}) => act(() => {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, ...init })
  Object.assign(event, { pointerId: 1, pointerType: 'mouse' })
  target.dispatchEvent(event)
})

it('drags a mentioned entity onto a pane edge, opening its tab there; the link\'s own drag gives way', () => {
  let ids = 0
  const store = createWorkspaceStore({ newId: () => `id-${ids++}` })
  store.dispatch({ _tag: 'Open', ref: 'agent:agent/a' })
  const pane = currentSpace(store.getState()).local.focus
  act(() => root.render(<WorkspaceProvider store={store} describe={ref => ({ kind: 'resource', id: ref, title: ref })}>
    <DraggableMention subject="doc/plan"><a href="doc/plan">doc/plan</a></DraggableMention>
    <div data-drop-content={pane} />
  </WorkspaceProvider>))
  const target = container.querySelector<HTMLElement>('[data-drop-content]')!
  target.getBoundingClientRect = () => DOMRect.fromRect({ x: 0, y: 0, width: 400, height: 300 })
  document.elementsFromPoint = () => [target]
  const link = container.querySelector('a')!
  const native = new Event('dragstart', { bubbles: true, cancelable: true })
  link.dispatchEvent(native)
  expect(native.defaultPrevented).toBe(true)
  pointer(link, 'pointerdown', { clientX: 10, clientY: 10 })
  pointer(window, 'pointermove', { clientX: 200, clientY: 150 })
  pointer(window, 'pointermove', { clientX: 390, clientY: 150 })
  pointer(window, 'pointerup', { clientX: 390, clientY: 150 })
  const panes = Workspace.panesOf(currentSpace(store.getState()).layout)
  expect(panes.map(row => row.tabs.map(tab => tab.ref))).toEqual([['agent:agent/a'], [tabsOf(store.getState()).at(-1)!.ref]])
  expect(tabsOf(store.getState()).at(-1)!.ref).toMatch(/doc\/plan/)
  store.dispose()
})
