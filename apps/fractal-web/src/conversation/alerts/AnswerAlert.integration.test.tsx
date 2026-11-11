// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'
import type * as Native from '@fractal/client/schema'
import { DateTime } from 'effect'

vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))

import { AlertAnswersContext, createAlertAnswers } from './alertAnswers.ts'
import { AnswerAlert } from './AnswerAlert.tsx'
import type { AlertAction } from './alert-action-port.ts'

const alert = {
  id: 'attention/ask',
  kind: 'attention',
  revision: 'r1',
  attention_kind: 'person-step',
  title: 'Merge the parser change?',
  detail: '',
  priority: 'high',
  state: 'open',
  requested_at: DateTime.makeUnsafe('2026-10-14T10:00:00Z'),
  person_id: 'person/avery',
  source_id: 'step-run/example/build',
  requester_id: 'agent/example/atlas',
  episode: 'claim/example',
  actions: ['work.done'],
  request: {
    version: 1,
    type: 'decision',
    question: 'Merge the parser change?',
    why_person: 'You own the release.',
    answers: [
      { id: 'merge', label: 'Merge', consequence: 'It joins the merge queue.', outcome: 'accept' },
      { id: 'change', label: 'Change it', consequence: 'The builder revises it.', outcome: 'request_changes' },
    ],
  },
} as unknown as Native.Attention

const button = (container: HTMLElement, label: string) =>
  [...container.querySelectorAll('button')].find((each) => each.textContent === label)
const radio = (container: HTMLElement, label: string) =>
  [...container.querySelectorAll('label')].find((each) => each.textContent?.startsWith(label) || each.textContent?.includes(label))!.querySelector('input')!
const type = async (container: HTMLElement, text: string) => {
  const words = container.querySelector('textarea')!
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(words, text)
    words.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

let cleanup: (() => void) | undefined
afterEach(() => cleanup?.())

const mount = async (onAct: (alert: Native.Attention, action: AlertAction) => Promise<{ _tag: 'Sent' }>, shown: Native.Attention = alert) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', undefined)
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const store = createAlertAnswers()
  const render = (next: Native.Attention) => root.render(<AlertAnswersContext.Provider value={store}><AnswerAlert alert={next} onAct={onAct} /></AlertAnswersContext.Provider>)
  cleanup = () => { act(() => root.unmount()); container.remove() }
  await act(async () => render(shown))
  return { container, store, render }
}

it('sends nothing on choosing; Send is its own act, and sends once', async () => {
  const onAct = vi.fn(async (_alert: Native.Attention, _action: AlertAction) => ({ _tag: 'Sent' as const }))
  const { container, store } = await mount(onAct)
  expect(button(container, 'Send')!.disabled).toBe(true)
  await act(async () => radio(container, 'Merge').click())
  expect(onAct).not.toHaveBeenCalled()
  await act(async () => button(container, 'Send')!.click())
  expect(onAct).toHaveBeenCalledTimes(1)
  expect(onAct.mock.calls[0]).toEqual([alert, { type: 'work.done', summary: 'Merge', answer: { id: 'merge' } }])
  expect(store.getSnapshot().pending.get(alert.id)?.label).toBe('Merge')
  // A second press on a card still mounted answers nothing more.
  await act(async () => button(container, 'Send')!.click())
  expect(onAct).toHaveBeenCalledTimes(1)
})

it('drops a choice when the alert is replaced, and keeps the person\'s words', async () => {
  const onAct = vi.fn(async (_alert: Native.Attention, _action: AlertAction) => ({ _tag: 'Sent' as const }))
  const { container, render } = await mount(onAct)
  await act(async () => radio(container, 'Change it').click())
  await type(container, 'Only after the fix lands')
  expect(radio(container, 'Change it').checked).toBe(true)
  // The same alert id comes back at a new revision, carrying a new question.
  const replaced = { ...alert, revision: 'r2', request: { ...(alert as unknown as { request: object }).request, question: 'Merge the reworked parser change?' } } as unknown as Native.Attention
  await act(async () => render(replaced))
  expect(container.querySelector('textarea')!.value).toBe('Only after the fix lands')
  // The words still reach the answer that carries them, against the revision now on screen.
  await act(async () => button(container, 'Send')!.click())
  expect(onAct.mock.calls[0]![0]).toBe(replaced)
  expect(onAct.mock.calls[0]![1]).toEqual({ type: 'work.done', summary: 'Only after the fix lands', answer: { id: 'change', text: 'Only after the fix lands' } })
})

it('sends a request for changes only with the person\'s words', async () => {
  const onAct = vi.fn(async (_alert: Native.Attention, _action: AlertAction) => ({ _tag: 'Sent' as const }))
  const { container } = await mount(onAct)
  await act(async () => radio(container, 'Change it').click())
  expect(button(container, 'Send')!.disabled).toBe(true)
  await type(container, 'Rename the flag.')
  expect(button(container, 'Send')!.disabled).toBe(false)
  await act(async () => button(container, 'Send')!.click())
  expect(onAct.mock.calls[0]![1]).toEqual({ type: 'work.done', summary: 'Rename the flag.', answer: { id: 'change', text: 'Rename the flag.' } })
})

it('moves words typed on an answer that cannot carry them, and says so', async () => {
  const onAct = vi.fn(async (_alert: Native.Attention, _action: AlertAction) => ({ _tag: 'Sent' as const }))
  const { container } = await mount(onAct)
  await act(async () => radio(container, 'Merge').click())
  await type(container, 'Rename the flag.')
  expect(radio(container, 'Change it').checked).toBe(true)
  expect(container.querySelector('[role="status"]')?.textContent).toBe('Your words go with “Change it”.')
  // Clearing them puts the person's own choice back.
  await type(container, '')
  expect(radio(container, 'Merge').checked).toBe(true)
})

it('holds Send when nothing here takes words, and says why', async () => {
  const named = { ...alert, request: { ...(alert as unknown as { request: { answers: { id: string }[] } }).request, answers: [{ id: 'merge', label: 'Merge', outcome: 'accept' }] } } as unknown as Native.Attention
  const { container } = await mount(vi.fn(async () => ({ _tag: 'Sent' as const })), named)
  await act(async () => radio(container, 'Merge').click())
  await type(container, 'Hm')
  expect(button(container, 'Send')!.disabled).toBe(true)
  expect(container.querySelector('[role="status"]')?.textContent).toContain('your words can\'t be sent')
})

it('takes a second press to dismiss, so a stray click answers nothing', async () => {
  const onAct = vi.fn(async (_alert: Native.Attention, _action: AlertAction) => ({ _tag: 'Sent' as const }))
  const { container } = await mount(onAct)
  await act(async () => button(container, 'Dismiss')!.click())
  expect(onAct).not.toHaveBeenCalled()
  await act(async () => button(container, 'Confirm dismiss')!.click())
  expect(onAct.mock.calls[0]![1]).toEqual({ type: 'work.done', summary: 'Dismissed without an answer.', answer: { id: 'dismiss' } })
})

it('answers with the keyboard inside the card: a digit chooses, Enter sends, Shift+Enter is a newline, Escape clears', async () => {
  const onAct = vi.fn(async (_alert: Native.Attention, _action: AlertAction) => ({ _tag: 'Sent' as const }))
  const { container } = await mount(onAct)
  const section = container.querySelector('section')!
  const key = (target: Element, init: KeyboardEventInit) => act(async () => { target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })) })
  await key(section, { key: '2' })
  expect(radio(container, 'Change it').checked).toBe(true)
  await key(section, { key: 'Escape' })
  expect(radio(container, 'Change it').checked).toBe(false)
  await key(section, { key: '1' })
  expect(radio(container, 'Merge').checked).toBe(true)
  await type(container, 'Rename it')
  const words = container.querySelector('textarea')!
  await key(words, { key: 'Enter', shiftKey: true })
  expect(onAct).not.toHaveBeenCalled()
  await key(words, { key: 'Escape' })
  expect(words.value).toBe('')
  await key(section, { key: 'Enter' })
  expect(onAct.mock.calls[0]![1]).toEqual({ type: 'work.done', summary: 'Merge', answer: { id: 'merge' } })
})
