// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Attention, decodeUnknownSync } from '@fractal/client/schema'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
vi.mock('../data/react.tsx', () => ({ useAttention: vi.fn(), useAttentionDecode: vi.fn(), useDataSource: vi.fn(), useNow: vi.fn() }))

import { credentialExpiredMessage, observed, unavailable } from '../data/source.ts'
import { AlertsList } from './AlertsTab.tsx'

const now = Date.parse('2026-11-01T12:00:00Z')

/** An open ask whose served request names two answers. */
const ask = (id: string, title: string) => decodeUnknownSync(Attention)({
  id, kind: 'attention', revision: 'r1', attention_kind: 'person-step', title, detail: '', priority: 'high', state: 'open', alert: true,
  person_id: 'person/avery', source_id: `step-run/release/${id.split('/')[1]}`, requester_id: 'agent/example/atlas',
  requested_at: '2026-11-01T11:00:00Z', updated_at: '2026-11-01T11:00:00Z', actions: ['work.done'],
  presentation: { tier: 'stopped', kind: 'request', agent: null, raised_by: null, waiting: null },
  request: {
    version: 1, type: 'decision', question: title, why_person: 'You own the release.',
    answers: [
      { id: 'merge', label: 'Merge', consequence: 'It joins the merge queue.', outcome: 'accept' },
      { id: 'hold', label: 'Hold', consequence: 'The builder waits.', outcome: 'decline' },
    ],
  },
})

const first = ask('attention/first', 'Merge the parser change?')
const second = ask('attention/second', 'Merge the lexer change?')
const third = ask('attention/third', 'Merge the docs change?')

let host: HTMLDivElement
let outside: HTMLButtonElement
let root: Root
const opened: string[] = []

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  opened.length = 0
  outside = document.createElement('button')
  outside.textContent = 'Elsewhere in the workspace'
  host = document.createElement('div')
  document.body.append(outside, host)
  root = createRoot(host)
})
afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  outside.remove()
  vi.unstubAllGlobals()
})

const show = (cards: readonly Attention[], selected?: string) =>
  act(async () => root.render(<AlertsList feed={observed({ value: cards })} now={now} onOpen={() => {}} onSelect={(id) => opened.push(id)} {...(selected === undefined ? {} : { selected })} />))
const row = (card: Attention) => host.querySelector<HTMLElement>(`[data-alert-id="${card.id}"]`)
const rowButton = (card: Attention) => row(card)!.querySelector<HTMLButtonElement>('button[data-alert-open]')!

describe('AlertsList rows', () => {
  it('opens an alert from its row and marks the one the main area shows', async () => {
    await show([first, second], second.id)
    await act(async () => rowButton(first).click())
    expect(opened).toEqual([first.id])
    expect(rowButton(first).getAttribute('aria-current')).toBeNull()
    expect(rowButton(second).getAttribute('aria-current')).toBe('true')
    expect(rowButton(first).hasAttribute('aria-expanded')).toBe(false)
  })
})

describe('AlertsList, after a row leaves the feed', () => {
  it('explains rejected credentials quietly without reconnect advice or Retry', async () => {
    await act(async () => root.render(<AlertsList feed={unavailable({ reason: 'unauthorized', detail: 'private diagnostic' })} now={now} onOpen={() => {}} onSelect={() => {}} />))
    expect(host.querySelector('[role="status"]')?.textContent).toBe(credentialExpiredMessage)
    expect(host.querySelector('[role="alert"], button')).toBeNull()
    expect(host.textContent).not.toMatch(/private diagnostic|reconnect/)
  })

  it('moves focus to the row that took its place', async () => {
    await show([first, second, third])
    rowButton(second).focus()
    await show([first, third])
    expect(row(second)).toBeNull()
    expect(document.activeElement).toBe(rowButton(third))
  })

  it('moves focus to the row before it when it was last', async () => {
    await show([first, second, third])
    rowButton(third).focus()
    await show([first, second])
    expect(document.activeElement).toBe(rowButton(second))
  })

  it('moves focus to the empty state when it was the only one', async () => {
    await show([first])
    rowButton(first).focus()
    await show([])
    const message = [...host.querySelectorAll('p')].find((each) => each.textContent === 'Nothing needs you. Asks, approvals and harness questions show here.')
    expect(message).toBeDefined()
    expect(document.activeElement).toBe(message)
  })

  it('leaves focus where the person moved it', async () => {
    await show([first, second, third])
    rowButton(second).focus()
    outside.focus()
    await show([first, third])
    expect(document.activeElement).toBe(outside)
  })

  it('does not take focus back after the person pointed elsewhere', async () => {
    await show([first, second, third])
    rowButton(second).focus()
    rowButton(second).blur()
    document.body.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    await show([first, third])
    expect(document.activeElement).toBe(document.body)
  })

  it('does not take focus when another row leaves while focus is elsewhere in the list', async () => {
    await show([first, second, third])
    rowButton(first).focus()
    await show([first, third])
    expect(document.activeElement).toBe(rowButton(first))
  })

  it('does not take focus when a row leaves that never held it', async () => {
    await show([first, second, third])
    outside.focus()
    await show([first, third])
    expect(document.activeElement).toBe(outside)
  })
})

describe('AlertsList decode notice', () => {
  it('counts rejected rows above an empty list, discloses IDs and clears on recovery', async () => {
    const decode = {
      decodeStale: false,
      undecodable: [
        { _tag: 'Rejected' as const, id: 'attention/broken', path: 'actions', reason: 'Expected an array' },
        { _tag: 'Rejected' as const, id: 'attention/broken', path: 'title', reason: 'Expected a string' },
        { _tag: 'Rejected' as const, id: 'attention/other', path: 'actions', reason: 'Expected an array' },
        { _tag: 'Degraded' as const, id: 'attention/kept', path: 'extra', reason: 'Unknown leaf' },
      ],
    }
    await act(async () => root.render(<AlertsList feed={observed({ value: [] })} decode={decode} now={now} onOpen={() => {}} />))
    const notice = host.querySelector('[data-wf-attention-decode]')!
    expect(notice.querySelector('[role="status"]')?.textContent).toBe('2 alerts could not be shown · their data does not match the expected format')
    expect(host.textContent).toContain('Nothing needs you. Asks, approvals and harness questions show here.')
    const details = notice.querySelector<HTMLButtonElement>('button')!
    await act(async () => details.click())
    expect(notice.querySelector('[aria-label="Alerts that could not be shown"]')?.textContent).toContain('attention/broken')
    expect(notice.textContent).toContain('attention/other')
    expect(notice.textContent).not.toContain('attention/kept')
    await show([])
    expect(host.querySelector('[data-wf-attention-decode]')).toBeNull()
    expect(host.textContent).toBe('Nothing needs you. Asks, approvals and harness questions show here.')
  })

  it('keeps valid alerts visible alongside rejected rows', async () => {
    await act(async () => root.render(<AlertsList
      feed={observed({ value: [first] })}
      decode={{ decodeStale: false, undecodable: [{ _tag: 'Rejected', id: 'attention/broken', path: 'actions', reason: 'Expected an array' }] }}
      now={now} onOpen={() => {}}
    />))
    expect(row(first)).not.toBeNull()
    expect(host.querySelector('[data-wf-attention-decode] [role="status"]')?.textContent).toContain('1 alert could not be shown')
  })

  it('hides retained decode evidence when alert read access is refused', async () => {
    const decode = {
      decodeStale: false,
      undecodable: [{ _tag: 'Rejected' as const, id: 'attention/broken', path: 'actions', reason: 'Expected an array' }],
    }
    await act(async () => root.render(<AlertsList feed={observed({ value: [] })} decode={decode} now={now} onOpen={() => {}} />))
    expect(host.querySelector('[data-wf-attention-decode]')).not.toBeNull()
    await act(async () => root.render(<AlertsList
      feed={unavailable({ reason: 'ungranted', detail: 'Read access refused.' })}
      decode={decode} now={now} onOpen={() => {}}
    />))
    expect(host.querySelector('[data-wf-attention-decode]')).toBeNull()
    expect(host.textContent).not.toContain('attention/broken')
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Read access to alerts has not been granted.')
  })
})
