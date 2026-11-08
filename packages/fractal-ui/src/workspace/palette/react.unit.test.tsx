// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { OpenTarget, PaletteItem, PaletteMode, PaletteSection } from './core.ts'
import { memoryRecentStore, usePalette, useRecent, type PaletteSource } from './react.tsx'

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const sections: readonly PaletteSection[] = [
  { id: 'people', label: 'People', prefix: '@' },
  { id: 'said', label: 'Said' },
  { id: 'commands', label: 'Commands', prefix: '>' },
]
const item = (section: string, label: string): PaletteItem<string> => ({ id: `${section}/${label}`, section, label, action: label })
const people: PaletteSource<string> = { id: 'people', items: () => [item('people', 'Ada'), item('people', 'Atlas Builder')] }
const commands: PaletteSource<string> = { id: 'commands', items: () => [item('commands', 'Split right'), item('commands', 'Toggle theme')] }

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
})

const Palette = (props: {
  readonly sources: readonly PaletteSource<string>[]
  readonly onSelect: (item: PaletteItem<string>, target: OpenTarget) => void
  readonly onName?: (mode: Extract<PaletteMode, { _tag: 'Naming' }>, name: string) => void
  readonly mode?: PaletteMode
  readonly targets?: readonly OpenTarget[]
  readonly recent?: readonly string[]
  readonly initialSection?: string
  readonly initialSelected?: string
  readonly onState?: (state: ReturnType<typeof usePalette<string>>) => void
}) => {
  const palette = usePalette({ sections, ...props })
  props.onState?.(palette)
  return <div>
    <input {...palette.getInputProps()} />
    <div {...palette.getListProps()} data-loading={palette.loading ? '' : undefined}>
      {palette.groups.map(group => <div key={group.section.id} {...palette.getSectionProps(group.section)}>
        {group.items.map(({ item: row }) => <div key={row.id} {...palette.getItemProps(row)}>{row.label}</div>)}
      </div>)}
    </div>
  </div>
}

const input = () => container.querySelector('input')!
const labels = () => Array.from(container.querySelectorAll('[role="option"]')).map(node => node.textContent)
const selectedLabel = () => container.querySelector('[data-selected]')?.textContent
const type = (text: string) => act(() => {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!
  setter.call(input(), text)
  input().dispatchEvent(new Event('input', { bubbles: true }))
})
const key = (init: KeyboardEventInit) => act(() => { input().dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })) })

describe('usePalette', () => {
  it('lists sections for an empty query, ranks a query, and opens by modifier', () => {
    const onSelect = vi.fn()
    act(() => root.render(<Palette sources={[people, commands]} onSelect={onSelect} recent={['people/Atlas Builder']} />))
    expect(labels()).toEqual(['Atlas Builder', 'Ada', 'Split right', 'Toggle theme'])
    type('ab')
    expect(labels()).toEqual(['Atlas Builder'])
    key({ key: 'Enter', altKey: true })
    expect(onSelect).toHaveBeenLastCalledWith(expect.objectContaining({ label: 'Atlas Builder' }), 'right')
    expect(input().getAttribute('aria-activedescendant')).toBe(container.querySelector('[data-selected]')!.id)
  })

  it('scopes by prefix, jumps sections with Alt+digit, and moves the selection with arrows', () => {
    act(() => root.render(<Palette sources={[people, commands]} onSelect={() => {}} />))
    type('>t')
    expect(labels()).toEqual(['Toggle theme', 'Split right'])
    type('')
    key({ key: '™', code: 'Digit3', altKey: true })
    expect(labels()).toEqual(['Split right', 'Toggle theme'])
    key({ key: 'ArrowDown' })
    expect(selectedLabel()).toBe('Toggle theme')
    key({ key: 'ArrowDown' })
    expect(selectedLabel()).toBe('Split right')
  })

  it('opens scoped to a section with a row chosen, and moves the selection for the host', () => {
    let state: ReturnType<typeof usePalette<string>> | undefined
    act(() => root.render(<Palette sources={[people, commands]} onSelect={() => {}} initialSection="commands" initialSelected="commands/Toggle theme" onState={next => { state = next }} />))
    expect(labels()).toEqual(['Split right', 'Toggle theme'])
    expect(selectedLabel()).toBe('Toggle theme')
    act(() => state!.move(1))
    expect(selectedLabel()).toBe('Split right')
    // The jump is the palette's own, so Escape leaves it as it leaves Alt+digit.
    key({ key: 'Escape' })
    expect(labels()).toEqual(['Ada', 'Atlas Builder', 'Split right', 'Toggle theme'])
  })

  it('limits targets, falling back to here', () => {
    const onSelect = vi.fn()
    act(() => root.render(<Palette sources={[people]} onSelect={onSelect} targets={['here', 'tab']} />))
    key({ key: 'Enter', altKey: true, shiftKey: true })
    expect(onSelect).toHaveBeenLastCalledWith(expect.anything(), 'here')
  })

  it('asks an async source only from its minimum query, and shows it loading', async () => {
    let resolve: (items: readonly PaletteItem<string>[]) => void = () => {}
    const asked = vi.fn((query: string) => new Promise<readonly PaletteItem<string>[]>(done => { resolve = done; void query }))
    const said: PaletteSource<string> = { id: 'said', minQuery: 3, items: asked }
    act(() => root.render(<Palette sources={[people, said]} onSelect={() => {}} />))
    type('at')
    expect(asked).not.toHaveBeenCalled()
    type('atl')
    expect(asked).toHaveBeenCalledWith('atl', expect.any(AbortSignal))
    expect(container.querySelector('[role="listbox"]')!.hasAttribute('data-loading')).toBe(true)
    await act(async () => resolve([item('said', 'atlas said this')]))
    expect(labels()).toEqual(['Atlas Builder', 'atlas said this'])
  })

  it('takes a name in naming mode', () => {
    const onName = vi.fn()
    act(() => root.render(<Palette sources={[people]} onSelect={() => {}} onName={onName} mode={{ _tag: 'Naming', intent: 'rename', initial: 'Main' }} />))
    expect(input().value).toBe('Main')
    expect(labels()).toEqual([])
    type('Review')
    key({ key: 'Enter' })
    expect(onName).toHaveBeenCalledWith({ _tag: 'Naming', intent: 'rename', initial: 'Main' }, 'Review')
  })
})

describe('useRecent', () => {
  it('keeps recent ids newest first and bounded', () => {
    const store = memoryRecentStore(['a', 'b'])
    let push: (id: string) => void = () => {}
    const Probe = () => {
      const recent = useRecent({ store, limit: 2 })
      push = recent.push
      return <p>{recent.recent.join(',')}</p>
    }
    act(() => root.render(<Probe />))
    act(() => push('c'))
    expect(container.textContent).toBe('c,a')
  })
})
