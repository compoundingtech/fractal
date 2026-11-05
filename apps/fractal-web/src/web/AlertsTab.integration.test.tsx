// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Attention, decodeUnknownSync } from '@fractal/client/schema'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
vi.mock('../data/react.tsx', () => ({ useAttention: vi.fn(), useDataSource: vi.fn(), useNow: vi.fn() }))

import type { AlertAction } from '../conversation/alerts/alert-action-port.ts'
import { observed } from '../data/source.ts'
import { AlertsList } from './AlertsTab.tsx'

const now = Date.parse('2026-11-01T12:00:00Z')

/** An open ask whose served request names two answers, so the Alerts row expands to the real answer controller. */
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
const sent: Array<{ id: string; action: AlertAction }> = []
// The served answer port: it records the answer and reports it sent. The feed drops the card later, as Fractal does.
const onAct = async (alert: Attention, action: AlertAction) => {
  sent.push({ id: alert.id, action })
  return { _tag: 'Sent' } as const
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  sent.length = 0
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

const show = (cards: readonly Attention[]) =>
  act(async () => root.render(<AlertsList feed={observed({ value: cards })} now={now} onOpen={() => {}} onAct={onAct} />))
const row = (card: Attention) => host.querySelector<HTMLElement>(`[data-alert-id="${card.id}"]`)
const rowButton = (card: Attention) => row(card)!.querySelector<HTMLButtonElement>('button[aria-expanded]')!
const answerButton = (card: Attention, label: string) =>
  [...row(card)!.querySelectorAll<HTMLButtonElement>('button')].find((button) => button.textContent === label)!

/** Expands the row from the keyboard's point of view, chooses Merge, then focuses and presses Send: the confirmation unmounts on send. */
const answer = async (card: Attention) => {
  const button = rowButton(card)
  button.focus()
  await act(async () => button.click())
  expect(button.getAttribute('aria-expanded')).toBe('true')
  expect(row(card)!.contains(document.activeElement)).toBe(true)
  await act(async () => answerButton(card, 'Merge').click())
  const send = answerButton(card, 'Send')
  send.focus()
  await act(async () => send.click())
  expect(sent.at(-1)).toEqual({ id: card.id, action: { type: 'work.done', summary: 'Merge', answer: { id: 'merge' } } })
  expect(row(card)!.textContent).toContain('Answered. The alert clears once Fractal records it.')
}

describe('AlertsList, after an answer leaves the feed', () => {
  it('moves focus to the row that took the answered row\'s place', async () => {
    await show([first, second, third])
    await answer(second)
    await show([first, third])
    expect(row(second)).toBeNull()
    expect(document.activeElement).toBe(rowButton(third))
  })

  it('moves focus to the row before it when the answered row was last', async () => {
    await show([first, second, third])
    await answer(third)
    await show([first, second])
    expect(document.activeElement).toBe(rowButton(second))
  })

  it('moves focus to the empty state when the answered row was the only one', async () => {
    await show([first])
    await answer(first)
    await show([])
    const message = [...host.querySelectorAll('p')].find((each) => each.textContent === 'Nothing needs you. Asks, approvals and harness questions show here.')
    expect(message).toBeDefined()
    expect(document.activeElement).toBe(message)
  })

  it('leaves focus where the person moved it before the feed caught up', async () => {
    await show([first, second, third])
    await answer(second)
    outside.focus()
    await show([first, third])
    expect(document.activeElement).toBe(outside)
  })

  it('does not take focus back after the person pointed elsewhere', async () => {
    await show([first, second, third])
    await answer(second)
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
