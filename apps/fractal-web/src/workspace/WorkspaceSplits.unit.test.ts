// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { PaneSlots, leadPane, surfacePlacement } from './WorkspaceSplits.tsx'
import { emptyWorkspace, workspaceConfig } from './model.ts'
import * as Workspace from '@fractal/ui/workspace'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const boxed = (rect: { x: number; y: number; width: number; height: number }) => {
  const element = document.createElement('div')
  element.getBoundingClientRect = () => DOMRect.fromRect(rect)
  return element
}

describe('pane slots', () => {
  it('writes each pane\'s slot rect on the host, relative to it, and keeps a hidden pane\'s last one', () => {
    const slots = new PaneSlots()
    const host = boxed({ x: 100, y: 40, width: 800, height: 600 })
    slots.setHost(host)
    const left = boxed({ x: 100, y: 80, width: 400, height: 560 })
    slots.slot('left')(left)
    expect(host.style.getPropertyValue('--wf-pane-left-x')).toBe('0px')
    expect(host.style.getPropertyValue('--wf-pane-left-y')).toBe('40px')
    expect(host.style.getPropertyValue('--wf-pane-left-w')).toBe('400px')
    left.getBoundingClientRect = () => DOMRect.fromRect({ x: 0, y: 0, width: 0, height: 0 })
    slots.measure()
    expect(host.style.getPropertyValue('--wf-pane-left-w')).toBe('400px')
    expect(surfacePlacement('left')).toEqual({ left: 'var(--wf-pane-left-x, 0px)', top: 'var(--wf-pane-left-y, 0px)', width: 'var(--wf-pane-left-w, 100%)', height: 'var(--wf-pane-left-h, 100%)' })
  })

  it('keeps a stable ref per pane and tells header listeners when a pane\'s header slot comes and goes', () => {
    const slots = new PaneSlots()
    expect(slots.header('a')).toBe(slots.header('a'))
    const heard = vi.fn()
    slots.subscribeHeaders(heard)
    const element = document.createElement('div')
    slots.header('a')(element)
    expect(slots.getHeaders().get('a')).toBe(element)
    slots.header('a')(null)
    expect(slots.getHeaders().has('a')).toBe(false)
    expect(heard).toHaveBeenCalledTimes(2)
  })
})

describe('the lead pane', () => {
  it('is the top-left pane, the zoomed one, or the focused one when collapsed', () => {
    let ids = 0
    const config = workspaceConfig(() => `id-${ids++}`)
    let state = emptyWorkspace(config)
    state = Workspace.transition(state, { _tag: 'Open', ref: 'agent:agent/a' }, config)
    const first = Workspace.currentSpace(state).local.focus
    state = Workspace.transition(state, { _tag: 'Split', side: 'left' }, config)
    const second = Workspace.currentSpace(state).local.focus
    expect(leadPane(state, false)).toBe(second)
    state = Workspace.transition(state, { _tag: 'FocusPane', pane: first }, config)
    expect(leadPane(state, true)).toBe(first)
    state = Workspace.transition(state, { _tag: 'ToggleZoom' }, config)
    expect(leadPane(state, false)).toBe(first)
  })
})
