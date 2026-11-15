// @vitest-environment jsdom
import type { AutoResolvedAsk } from '@fractal/client'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const data = vi.hoisted(() => ({
  attention: { _tag: 'Observed', value: [] as unknown[], freshness: 'live' } as unknown,
  port: undefined as unknown,
  actions: 'granted' as string,
}))
vi.mock('../data/react.tsx', () => ({
  useAttention: () => data.attention,
  useAttentionDecode: () => ({ decodeStale: false, undecodable: [] }),
  useDataSource: () => ({ alertActions: data.port }),
  useGrants: () => ({ actions: data.actions }),
  useNow: () => now,
}))

import type { AlertActionResult, AutoResolvedRead } from '../conversation/alerts/alert-action-port.ts'
import { observed } from '../data/source.ts'
import { AlertsList, AlertsTab } from './AlertsTab.tsx'
import { AutoResolvedSection } from './AutoResolvedAsks.tsx'

const now = Date.parse('2026-11-01T12:00:00Z')
const COPY = 'Your answer replaces the default. Work already done on the default is not undone; example/atlas is told.'

/** An ask a default answered at its deadline, still open to override. */
const ask = (n: string, title: string): AutoResolvedAsk => ({
  resolution_id: n.repeat(64), title, requester_id: 'agent/example/atlas', resolved_at: '2026-11-01T10:00:00Z',
  target_id: `step-run/release/${n}`, episode: `claim/${n}`, default_answer: { id: 'hold', label: 'Hold' },
  request: {
    version: 1, type: 'decision', question: title, why_person: 'You own the release.',
    answers: [
      { id: 'merge', label: 'Merge', consequence: 'It joins the merge queue.' },
      { id: 'hold', label: 'Hold', consequence: 'The builder waits.' },
      { id: 'drop', label: 'Drop', consequence: 'The change is closed.' },
    ],
  },
  action_parameters: { 'work.override': { target_id: `step-run/release/${n}`, episode: `claim/${n}` } },
})
const first = ask('a', 'Merge the parser change?')
const second = ask('b', 'Merge the lexer change?')
const seen = (asks: readonly AutoResolvedAsk[]): AutoResolvedRead => ({ _tag: 'Observed', asks, complete: true })

let host: HTMLDivElement
let root: Root
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  vi.unstubAllGlobals()
})

const show = (read: AutoResolvedRead | undefined, onOverride?: (ask: AutoResolvedAsk, answer: string) => Promise<AlertActionResult>) =>
  act(async () => root.render(<AutoResolvedSection read={read} now={now} {...(onOverride === undefined ? {} : { onOverride })} />))
const row = (each: AutoResolvedAsk) => host.querySelector<HTMLElement>(`[data-auto-resolved-id="${each.resolution_id}"]`)
const buttons = (each: AutoResolvedAsk) => [...row(each)!.querySelectorAll<HTMLButtonElement>('button')]
const press = (each: AutoResolvedAsk, label: string) => act(async () => buttons(each).find((button) => button.textContent === label)!.click())

describe('Auto-resolved section', () => {
  it('shows what the default was, who asked, when, and the other answers with what overriding does', async () => {
    await show(seen([first]), async () => ({ _tag: 'Sent' }))
    expect(host.querySelector('h3')?.textContent).toBe('Auto-resolved')
    expect(row(first)!.textContent).toContain('Merge the parser change?')
    expect(row(first)!.textContent).toContain('Default applied: Hold · example/atlas')
    expect(row(first)!.querySelector('[aria-label="resolved 2h ago"]')?.textContent).toBe('2h')
    // The default is not offered as its own override.
    expect(buttons(first).map((button) => button.textContent)).toEqual(['Merge', 'Drop'])
    const group = row(first)!.querySelector('[role="group"]')!
    expect(group.getAttribute('aria-label')).toBe('Answer “Merge the parser change?” instead')
    expect(document.getElementById(group.getAttribute('aria-describedby')!)?.textContent).toBe(COPY)
  })

  it('hides the override when Fractal no longer offers it, or this device cannot send it', async () => {
    const { action_parameters: _closed, ...expired } = first
    await show(seen([expired]), async () => ({ _tag: 'Sent' }))
    expect(row(expired)!.textContent).toContain('Default applied: Hold')
    expect(buttons(expired)).toEqual([])
    expect(host.textContent).not.toContain(COPY)
    await show(seen([first]))
    expect(buttons(first)).toEqual([])
  })

  it('shows the answer on its way at once, and the refusal in words when Fractal refuses it', async () => {
    let settle: (result: AlertActionResult) => void = () => {}
    const onOverride = vi.fn((_ask: AutoResolvedAsk, _answer: string) => {
      const { promise, resolve } = Promise.withResolvers<AlertActionResult>()
      settle = resolve
      return promise
    })
    await show(seen([first]), onOverride)
    await press(first, 'Merge')
    expect(onOverride).toHaveBeenCalledWith(first, 'merge')
    expect(row(first)!.querySelector('[role="status"]')?.textContent).toBe('Sending “Merge”…')
    expect(row(first)!.dataset.overrideState).toBe('sending')
    // A second click while it is on its way sends nothing more.
    await press(first, 'Drop')
    expect(onOverride).toHaveBeenCalledTimes(1)
    await act(async () => settle({ _tag: 'Refused', detail: 'The override window closed.' }))
    expect(row(first)!.querySelector('[role="alert"]')?.textContent).toBe('The override window closed.')
    await press(first, 'Drop')
    expect(onOverride).toHaveBeenLastCalledWith(first, 'drop')
    await act(async () => settle({ _tag: 'Sent' }))
    expect(row(first)!.querySelector('[role="status"]')?.textContent).toBe('Sent “Drop”; example/atlas is told.')
  })

  it('says nothing until read, for a session with no asks of its own, or when none were auto-resolved; a failed read says so', async () => {
    await show(undefined)
    expect(host.textContent).toBe('')
    await show({ _tag: 'Unavailable', reason: 'absent', detail: 'forbidden' })
    expect(host.textContent).toBe('')
    await show(seen([]))
    expect(host.textContent).toBe('')
    await show({ _tag: 'Unavailable', reason: 'failed', detail: 'network' })
    expect(host.querySelector('[role="status"]')?.textContent).toBe('Auto-resolved asks could not be loaded; reconnect and try again.')
    await show({ _tag: 'Observed', asks: [first], complete: false })
    expect(host.textContent).toContain('Older auto-resolved asks may be missing from this list.')
  })

  it('moves focus to the next row when an overridden one leaves, then to the heading', async () => {
    await show(seen([first, second]), async () => ({ _tag: 'Sent' }))
    buttons(first)[0]!.focus()
    await show(seen([second]), async () => ({ _tag: 'Sent' }))
    expect(document.activeElement).toBe(buttons(second)[0])
    await show(seen([]), async () => ({ _tag: 'Sent' }))
    expect(document.activeElement).toBe(host.querySelector('h3'))
    expect(host.textContent).toContain('Nothing else was auto-resolved.')
  })

  it('sits below the open alerts in the Alerts tab', async () => {
    await act(async () => root.render(<AlertsList feed={observed({ value: [] })} now={now} onOpen={() => {}} autoResolved={seen([first])} />))
    expect(host.textContent).toContain('Nothing needs you.')
    expect(row(first)).not.toBeNull()
  })
})

describe('Alerts tab auto-resolved reads', () => {
  it('reads on open, again when attention changes and after an accepted override, never on a timer', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'setInterval'] })
    try {
      const autoResolved = vi.fn(async (): Promise<AutoResolvedRead> => seen([first]))
      const override = vi.fn(async (): Promise<AlertActionResult> => ({ _tag: 'Sent' }))
      data.port = { act: vi.fn(), autoResolved, override }
      data.attention = { _tag: 'Observed', value: [], freshness: 'live' }
      await act(async () => root.render(<AlertsTab onOpen={() => {}} />))
      expect(autoResolved).toHaveBeenCalledTimes(1)
      await act(async () => { await vi.advanceTimersByTimeAsync(10 * 60_000) })
      expect(autoResolved).toHaveBeenCalledTimes(1)
      // The same collection turning stale is not a change.
      data.attention = { ...(data.attention as object), freshness: 'stale' }
      await act(async () => root.render(<AlertsTab onOpen={() => {}} />))
      expect(autoResolved).toHaveBeenCalledTimes(1)
      data.attention = { _tag: 'Observed', value: [], freshness: 'live' }
      await act(async () => root.render(<AlertsTab onOpen={() => {}} />))
      expect(autoResolved).toHaveBeenCalledTimes(2)
      await press(first, 'Merge')
      expect(override).toHaveBeenCalledWith({ ask: first, action: { type: 'work.override', answer: { id: 'merge' } } })
      expect(autoResolved).toHaveBeenCalledTimes(3)
    } finally {
      vi.useRealTimers()
    }
  })

  it('offers no override without an action grant', async () => {
    data.port = { act: vi.fn(), autoResolved: vi.fn(async () => seen([first])), override: vi.fn() }
    data.actions = 'ungranted'
    try {
      await act(async () => root.render(<AlertsTab onOpen={() => {}} />))
      expect(row(first)).not.toBeNull()
      expect(buttons(first)).toEqual([])
    } finally {
      data.actions = 'granted'
    }
  })
})
