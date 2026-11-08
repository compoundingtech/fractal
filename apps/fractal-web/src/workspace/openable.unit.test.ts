import { describe, expect, it } from 'vitest'
import { panesOf } from '@fractal/ui/workspace'
import { activeTab, currentSpace, emptyWorkspace, tabsOf, transition, workspaceConfig } from './model.ts'
import { openableDragType, openableProps, openCommand, readOpenableDrag, type OpenGesture } from './openable.ts'
import { paneKey, parsePane, subjectRef } from './subject.ts'

const counter = () => {
  let next = 0
  return workspaceConfig(() => `id-${next++}`)
}
const focus = (state: ReturnType<typeof emptyWorkspace>) => currentSpace(state).local.focus

const recorder = () => {
  const calls: (readonly [string, OpenGesture | undefined])[] = []
  return { calls, open: (ref: string, gesture?: OpenGesture) => { calls.push([ref, gesture]) } }
}

const key = (over: Partial<{ key: string; altKey: boolean; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }>) => {
  let prevented = false
  return { event: { key: 'Enter', altKey: false, shiftKey: false, metaKey: false, ctrlKey: false, preventDefault: () => { prevented = true }, ...over }, prevented: () => prevented }
}

describe('openCommand', () => {
  it('opens and focuses by default, behind on a background gesture', () => {
    expect(openCommand({ ref: 'agent/a', focus: 'main' })).toEqual({ _tag: 'Open', ref: 'agent:agent/a' })
    expect(openCommand({ ref: 'agent/a', gesture: { background: true }, focus: 'main' })).toEqual({ _tag: 'Open', ref: 'agent:agent/a', activate: false })
  })

  it('splits the focused pane for the keyboard, landing where a drag on that edge lands', () => {
    const context = counter()
    const base = transition(transition(emptyWorkspace(context), openCommand({ ref: 'agent/b', focus: 'x' }), context), openCommand({ ref: 'doc/x', focus: 'x' }), context)
    const pane = focus(base)
    const byKey = transition(base, openCommand({ ref: 'agent/a', gesture: { split: 'right' }, focus: pane }), counter())
    const byDrag = transition(base, openCommand({ ref: 'agent/a', gesture: { at: { _tag: 'Edge', pane, side: 'right' } }, focus: pane }), counter())
    const shape = (state: typeof base) => panesOf(currentSpace(state).layout).map(row => row.tabs.map(tab => tab.ref))
    expect(shape(byKey)).toEqual(shape(byDrag))
    expect(shape(byKey)).toEqual([['agent:agent/b', 'document:doc/x'], ['agent:agent/a']])
  })

  it('makes a split through the reducer, and moves a tab that is already open', () => {
    const context = counter()
    let state = transition(emptyWorkspace(context), openCommand({ ref: 'agent/a', focus: 'main' }), context)
    state = transition(state, openCommand({ ref: 'agent/b', focus: focus(state) }), context)
    state = transition(state, openCommand({ ref: 'agent/a', gesture: { split: 'bottom' }, focus: focus(state) }), context)
    expect(panesOf(currentSpace(state).layout).map(pane => pane.tabs.map(tab => tab.ref))).toEqual([['agent:agent/b'], ['agent:agent/a']])
    expect(activeTab(state)?.ref).toBe('agent:agent/a')
  })

  it('lets a dragged tab re-enter as the same Open, moving rather than duplicating', () => {
    const context = counter()
    let state = transition(emptyWorkspace(context), openCommand({ ref: 'agent/a', focus: 'main' }), context)
    state = transition(state, openCommand({ ref: 'mail/1', focus: focus(state) }), context)
    const dragged = tabsOf(state)[1]!
    const ref = subjectRef(parsePane(dragged.ref))
    state = transition(state, openCommand({ ref, gesture: { at: { _tag: 'Strip', pane: focus(state), index: 0 } }, focus: focus(state) }), context)
    expect(tabsOf(state).map(tab => tab.id)).toEqual([dragged.id, tabsOf(state)[1]!.id])
    expect(tabsOf(state)).toHaveLength(2)
    expect(paneKey(parsePane(dragged.ref))).toBe('resource:mail/1')
  })
})

describe('openableProps', () => {
  it('gives every surface click, Cmd-click and Ctrl-click together', () => {
    const { calls, open } = recorder()
    const props = openableProps('doc/x', open)
    props.onClick({ metaKey: false, ctrlKey: false })
    props.onClick({ metaKey: true, ctrlKey: false })
    props.onClick({ metaKey: false, ctrlKey: true })
    expect(calls).toEqual([['doc/x', {}], ['doc/x', { background: true }], ['doc/x', { background: true }]])
  })

  it('has a keyboard way to every gesture', () => {
    const { calls, open } = recorder()
    const props = openableProps('doc/x', open)
    const presses = [key({}), key({ metaKey: true }), key({ altKey: true }), key({ altKey: true, shiftKey: true })]
    for (const press of presses) {
      props.onKeyDown(press.event)
      expect(press.prevented()).toBe(true)
    }
    expect(calls).toEqual([['doc/x', {}], ['doc/x', { background: true }], ['doc/x', { split: 'right' }], ['doc/x', { split: 'bottom' }]])
  })

  it('leaves other keys alone, and plain Enter to a control that already activates on it', () => {
    const { calls, open } = recorder()
    const other = key({ key: 'a' })
    openableProps('doc/x', open).onKeyDown(other.event)
    const plain = key({})
    const props = openableProps('doc/x', open, { activatesOnEnter: true })
    props.onKeyDown(plain.event)
    expect(other.prevented()).toBe(false)
    expect(plain.prevented()).toBe(false)
    expect(calls).toEqual([])
    props.onKeyDown(key({ altKey: true }).event)
    expect(calls).toEqual([['doc/x', { split: 'right' }]])
  })

  it('carries the subject as the shared entity descriptor', () => {
    const data = new Map<string, string>()
    const transfer = { effectAllowed: 'none' as DataTransfer['effectAllowed'], setData: (type: string, value: string) => { data.set(type, value) } }
    const props = openableProps('agent/a', () => {})
    props.onDragStart({ dataTransfer: transfer })
    expect(props.draggable).toBe(true)
    // The shared entity descriptor: an agent, by its ref.
    expect(JSON.parse(data.get(openableDragType)!)).toEqual({ kind: 'agent', id: 'agent/a', title: 'agent/a' })
    expect(readOpenableDrag({ getData: type => data.get(type) ?? '' })).toBe('agent/a')
    expect(readOpenableDrag({ getData: () => '' })).toBeUndefined()
  })
})
