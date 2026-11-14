// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type * as Native from '@fractal/client/schema'
import { DateTime } from 'effect'

const data = vi.hoisted(() => ({ attention: [] as unknown[], act: vi.fn(), actions: 'granted' }))
vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))
vi.mock('../../data/react.tsx', () => ({
  useAttention: () => ({ _tag: 'Observed', value: data.attention, freshness: 'live' }),
  useDataSource: () => ({ alertActions: { act: data.act } }),
  useGrants: () => ({ actions: data.actions }),
}))

import { LiveAgentAlerts } from './AgentAlerts.tsx'
import { AlertAnswersContext, createAlertAnswers } from './alertAnswers.ts'
import type { UxTelemetry } from '../../telemetry/ux.ts'

const seat = 'agent/example/atlas'
const ask = (id: string): Native.Attention =>
  ({
    id, kind: 'attention', revision: 'r1', attention_kind: 'person-step', title: `${id}?`, detail: '',
    priority: 'high', state: 'open', requested_at: DateTime.makeUnsafe('2026-10-14T10:00:00Z'),
    person_id: 'person/avery', source_id: `step-run/${id}`, requester_id: seat, conversation_id: seat,
    conversation_ids: [seat], alert: true, actions: ['work.done'],
  }) as unknown as Native.Attention

const press = async (container: HTMLElement, label: string) =>
  act(async () => [...container.querySelectorAll('button')].find((each) => each.textContent === label)!.click())
const choose = async (container: HTMLElement, label: string) =>
  act(async () => [...container.querySelectorAll('label')].find((each) => each.textContent?.includes(label))!.querySelector('input')!.click())

const mount = (ux?: UxTelemetry) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const store = createAlertAnswers()
  const render = () => act(async () => root.render(<AlertAnswersContext.Provider value={store}><LiveAgentAlerts agentRef={seat} {...(ux === undefined ? {} : { ux })} /></AlertAnswersContext.Provider>))
  return { container, root, store, render, dispose: async () => { await act(async () => root.unmount()); container.remove() } }
}

it('advances to the next alert before the server replies, keeps stale frames hidden, and rolls back without taking focus or drafts', async () => {
  const landed = vi.fn(() => () => {})
  const cancel = vi.fn()
  const ux = { beginAlertAnswer: vi.fn(() => ({ landed, cancel })) } as unknown as UxTelemetry
  const first = ask('attention/one')
  const second = ask('attention/two')
  data.attention = [first, second]
  let reply!: (result: unknown) => void
  data.act.mockReturnValueOnce(new Promise((resolve) => { reply = resolve }))
  const { container, store, render, dispose } = mount(ux)
  await render()
  expect(container.textContent).toContain('1 of 2')

  // Answer: withhold the server's reply, deliberately.
  await choose(container, 'Yes')
  await press(container, 'Send')
  expect(data.act).toHaveBeenLastCalledWith({ alert: first, action: { type: 'work.done', summary: 'Yes' } })
  expect(container.querySelector('section[aria-label="attention/two?"]')).not.toBeNull()
  expect(container.querySelector('section[aria-label="attention/one?"]')).toBeNull()
  expect(container.textContent).toContain('You answered Yes')

  // A stale frame still holding the answered alert does not bring it back.
  data.attention = [first, second]
  await render()
  expect(container.querySelector('section[aria-label="attention/one?"]')).toBeNull()

  // The person starts on the next alert; the refusal arrives.
  store.setWords(second.id, 'new draft')
  const words = container.querySelector('textarea')!
  words.focus()
  await act(async () => { reply({ _tag: 'Refused', detail: 'Fractal no longer offers this answer here.' }) })
  expect(cancel).toHaveBeenCalledOnce()
  expect(document.activeElement).toBe(words)
  expect(container.querySelector('section[aria-label="attention/two?"]')).not.toBeNull()
  expect(store.words(second.id)).toBe('new draft')
  expect(container.textContent).toContain('2 of 2')
  expect(store.getSnapshot().flash).toMatchObject({ id: first.id, detail: 'Fractal no longer offers this answer here.' })
  expect(store.failure(first)).toBe('Fractal no longer offers this answer here.')
  expect(store.getSnapshot().pending.size).toBe(0)
  await dispose()
})

it('confirms only when the action was accepted and the feed dropped the alert, and times that as alert_landed', async () => {
  const landed = vi.fn(() => () => {})
  const ux = { beginAlertAnswer: vi.fn(() => ({ landed, cancel: vi.fn() })) } as unknown as UxTelemetry
  const first = ask('attention/one')
  data.attention = [first, ask('attention/two')]
  data.act.mockResolvedValueOnce({ _tag: 'Sent' })
  const { container, store, render, dispose } = mount(ux)
  await render()
  await choose(container, 'Yes')
  await press(container, 'Send')
  // Accepted, but the feed still holds it: hidden, not done, not landed.
  expect(store.getSnapshot().pending.get(first.id)?.phase).toBe('sent')
  expect(landed).not.toHaveBeenCalled()
  data.attention = [ask('attention/two')]
  await render()
  expect(landed).toHaveBeenCalledOnce()
  expect(store.getSnapshot().pending.get(first.id)?.phase).toBe('confirmed')
  expect(container.textContent).toContain('You answered Yes')
  await dispose()
})

it('shows options disabled on a device without the action grant', async () => {
  data.actions = 'ungranted'
  data.attention = [ask('attention/one')]
  const { container, render, dispose } = mount()
  await render()
  expect(container.textContent).toContain('This device is read-only')
  expect(container.querySelector<HTMLInputElement>('input[type="radio"]')!.disabled).toBe(true)
  data.actions = 'granted'
  await dispose()
})

it('removes a served prompt card when its authoritative attention row closes', async () => {
  data.attention = [{ ...ask('attention/question'), attention_kind: 'harness-prompt', title: 'Choose a color' }]
  const { container, render, dispose } = mount()
  await render()
  expect(container.textContent).toContain('Choose a color')
  data.attention = [{ ...data.attention[0] as Native.Attention, state: 'resolved' }]
  await render()
  expect(container.querySelector('[data-testid="conversation-alerts"]')).toBeNull()
  await dispose()
})
