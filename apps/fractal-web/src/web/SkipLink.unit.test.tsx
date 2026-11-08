/** @vitest-environment jsdom */
import { createRoot, type Root } from 'react-dom/client'
import { act } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SkipLink } from './SkipLink.tsx'

vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  defineConsts: (constants: unknown) => constants,
  createTheme: () => ({}),
  props: () => ({}),
}))

/** The workspace's DOM shape: pane frames carry `data-focused`; tab surfaces sit beside them in tab-id order. */
const surface = (tab: string, pane: string, face: 'conversation' | 'conversation-loading' | 'terminal' | 'hidden') => {
  const shown = face !== 'hidden'
  const conversation = face === 'conversation' || face === 'conversation-loading'
  const lane = face === 'conversation-loading' ? '' : `<div data-testid="transcript-scroll" tabindex="0" data-lane="${tab}"></div>`
  return `<div data-tab-surface-id="${tab}" data-surface-pane="${pane}" aria-hidden="${!shown}"${shown ? '' : ' inert'}>
    <div${conversation ? ' data-wf-shown-pane=""' : ''}>${lane}<textarea data-composer="${tab}"></textarea></div>
  </div>`
}

let host: HTMLDivElement
let root: Root
beforeEach(() => {
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(() => {
  act(() => root.unmount())
  host.remove()
  document.body.replaceChildren()
})

const press = async (workspace: string) => {
  await act(async () => root.render(<SkipLink />))
  host.insertAdjacentHTML('beforeend', `<aside aria-label="Agents"><button>Collapse agents</button></aside>${workspace}`)
  const link = host.querySelector('a')!
  link.focus()
  await act(async () => link.click())
  return document.activeElement as HTMLElement | null
}

describe('Skip to conversation', () => {
  it('lands on the focused pane of a split, not the first pane in the DOM', async () => {
    const active = await press(`
      <div data-pane="left"></div><div data-pane="right" data-focused=""></div>
      ${surface('t1', 'left', 'conversation')}${surface('t2', 'right', 'conversation')}`)
    expect(active?.getAttribute('data-lane')).toBe('t2')
  })

  it('never lands on a retained hidden tab, which is inert', async () => {
    const active = await press(`
      <div data-pane="only" data-focused=""></div>
      ${surface('t1', 'only', 'hidden')}${surface('t2', 'only', 'terminal')}`)
    expect(active?.closest('[inert]')).toBeNull()
    expect(active?.getAttribute('data-lane')).toBeNull()
  })

  it("takes the focused pane's composer while its transcript loads, before another pane's transcript", async () => {
    const active = await press(`
      <div data-pane="left"></div><div data-pane="right" data-focused=""></div>
      ${surface('t1', 'left', 'conversation')}${surface('t2', 'right', 'conversation-loading')}`)
    expect(active?.getAttribute('data-composer')).toBe('t2')
  })

  it('falls back to another shown conversation when the focused pane shows its terminal', async () => {
    const active = await press(`
      <div data-pane="left"></div><div data-pane="right" data-focused=""></div>
      ${surface('t0', 'right', 'hidden')}${surface('t1', 'left', 'conversation')}${surface('t2', 'right', 'terminal')}`)
    expect(active?.getAttribute('data-lane')).toBe('t1')
  })

  it('lands at the start of the focused pane when no conversation is shown', async () => {
    const active = await press(`
      <div data-pane="left"><button>Left pane tab</button></div>
      <div data-pane="right" data-focused=""><div inert><button>Covered</button></div><button disabled>Unavailable</button><button>Open a tab</button><button>Inbox</button></div>
      ${surface('t1', 'right', 'hidden')}`)
    expect(active?.textContent).toBe('Open a tab')
  })
})
