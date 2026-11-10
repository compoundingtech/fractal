// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'
import type * as Native from '@fractal/client/schema'
import { DateTime } from 'effect'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'

vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))

import { AnswerAlert } from './AnswerAlert.tsx'
import type { AlertAction } from './alert-action-port.ts'
import { isFixturePlainText } from '../../client-control/fixtureText.ts'
import { LiveAgentAlerts } from './AgentAlerts.tsx'
import type { AlertActionPort } from './alert-action-port.ts'
import type { PromptAnswerPort } from './prompt-answer-port.ts'
import { fixtureSource } from '../../data/fixtureSource.ts'
import { DataSourceProvider } from '../../data/react.tsx'
import { handleGrantedDeviceCommand } from '../../client-control/grantBoundary.ts'
import { testCommand, testGrant, testLifecycle } from '../../client-control/command.test-support.ts'
import type { ClientControlRouteTarget } from '../../client-control/navigation.ts'

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

let cleanup: (() => void) | undefined
afterEach(() => cleanup?.())

const mount = async (onAct: (alert: Native.Attention, action: AlertAction) => Promise<{ _tag: 'Sent' }>) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  cleanup = () => { act(() => root.unmount()); container.remove() }
  await act(async () => root.render(<AnswerAlert alert={alert} onAct={onAct} />))
  return container
}

it('confirms an answer before it sends it, and sends it once', async () => {
  const onAct = vi.fn(async (_alert: Native.Attention, _action: AlertAction) => ({ _tag: 'Sent' as const }))
  const container = await mount(onAct)
  await act(async () => button(container, 'Merge')!.click())
  expect(onAct).not.toHaveBeenCalled()
  expect(container.querySelector('[aria-label="Confirm your answer"]')?.textContent).toContain('Send “Merge” to example/atlas?')
  await act(async () => button(container, 'Cancel')!.click())
  expect(container.querySelector('[aria-label="Confirm your answer"]')).toBeNull()
  await act(async () => button(container, 'Merge')!.click())
  await act(async () => button(container, 'Send')!.click())
  expect(onAct).toHaveBeenCalledTimes(1)
  expect(onAct.mock.calls[0]).toEqual([alert, { type: 'work.done', summary: 'Merge', answer: { id: 'merge' } }])
  expect(container.querySelector('[role="status"]')?.textContent).toBe('Answered. The alert clears once Fractal records it.')
})

it('drops a pending confirmation when the alert is replaced, and keeps the person\'s words', async () => {
  const onAct = vi.fn(async (_alert: Native.Attention, _action: AlertAction) => ({ _tag: 'Sent' as const }))
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  cleanup = () => { act(() => root.unmount()); container.remove() }
  await act(async () => root.render(<AnswerAlert alert={alert} onAct={onAct} />))
  const words = container.querySelector('textarea')!
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(words, 'Only after the fix lands')
    words.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => button(container, 'Merge')!.click())
  expect(container.querySelector('[aria-label="Confirm your answer"]')).not.toBeNull()
  // The same alert id comes back at a new revision, carrying a new question.
  const replaced = { ...alert, revision: 'r2', request: { ...(alert as unknown as { request: object }).request, question: 'Merge the reworked parser change?' } } as unknown as Native.Attention
  await act(async () => root.render(<AnswerAlert alert={replaced} onAct={onAct} />))
  expect(container.querySelector('[aria-label="Confirm your answer"]')).toBeNull()
  expect(button(container, 'Send')).toBeUndefined()
  expect(container.querySelector('textarea')!.value).toBe('Only after the fix lands')
  expect(onAct).not.toHaveBeenCalled()
  // A fresh choice is confirmed and sent against the revision now on screen.
  await act(async () => button(container, 'Merge')!.click())
  await act(async () => button(container, 'Send')!.click())
  expect(onAct).toHaveBeenCalledTimes(1)
  expect(onAct.mock.calls[0]![0]).toBe(replaced)
})

it('sends a request for changes only with the person\'s words', async () => {
  const onAct = vi.fn(async (_alert: Native.Attention, _action: AlertAction) => ({ _tag: 'Sent' as const }))
  const container = await mount(onAct)
  expect(button(container, 'Change it')!.disabled).toBe(true)
  const words = container.querySelector('textarea')!
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(words, 'Rename the flag.')
    words.dispatchEvent(new Event('input', { bubbles: true }))
  })
  expect(button(container, 'Change it')!.disabled).toBe(false)
  await act(async () => button(container, 'Change it')!.click())
  await act(async () => button(container, 'Send')!.click())
  expect(onAct.mock.calls[0]![1]).toEqual({ type: 'work.done', summary: 'Rename the flag.', answer: { id: 'change', text: 'Rename the flag.' } })
})

it.each([
  ['A short native ask', 'client-control fixture 0001'],
  ['Plain ASCII punctuation: yes?', 'Read this single paragraph.'],
  ['T'.repeat(60), 'Q'.repeat(60)],
])('preserves approved plain ask text as one unsplit element: %s', async (title, question) => {
  expect(isFixturePlainText(title)).toBe(true)
  expect(isFixturePlainText(question)).toBe(true)
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  cleanup = () => { act(() => root.unmount()); container.remove() }
  let answers = 0
  await act(async () => root.render(<AnswerAlert
    alert={{ ...alert, title, request: { ...alert.request!, question } }}
    onAct={async () => { answers += 1; return { _tag: 'Sent' } }}
  />))
  const titles = container.querySelectorAll('[data-ask-title]')
  const questions = container.querySelectorAll('[data-ask-question] p')
  expect(titles).toHaveLength(1)
  expect(questions).toHaveLength(1)
  expect(titles[0]!.childElementCount).toBe(0)
  expect(questions[0]!.childElementCount).toBe(0)
  expect(titles[0]!.textContent).toBe(title)
  expect(questions[0]!.textContent).toBe(question)
  // Observe only: answer buttons are deliberately never pressed.
  expect(answers).toBe(0)
})

it.each(['decision', 'feedback'] as const)(
  'D-5 keeps shipped %s answer controls visible and untouched across the complete command sequence',
  async requestType => {
    const shown = {
      ...alert, alert: true, conversation_id: alert.requester_id, conversation_ids: [alert.requester_id],
      request: { ...alert.request!, type: requestType },
    } as Native.Attention
    // Record at the real source answer-port seam used by LiveAgentAlerts (and later useAlertAnswers).
    // No data-hook/module mock: the provider, live controller and answer renderer are real.
    const actionRequests: Parameters<AlertActionPort['act']>[0][] = []
    const promptRequests: Parameters<PromptAnswerPort['respond']>[0][] = []
    const source = {
      ...fixtureSource({ world: {
        now: Date.parse('2026-11-01T12:00:00Z'), agents: [], missions: [], attention: [shown],
        events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
      } }),
      alertActions: { act: async (request: Parameters<AlertActionPort['act']>[0]) => {
        actionRequests.push(request)
        return { _tag: 'Sent' as const }
      } },
      prompts: { respond: async (request: Parameters<PromptAnswerPort['respond']>[0]) => {
        promptRequests.push(request)
        return { _tag: 'Sent' as const }
      } },
    }
    const registry = AtomRegistry.make()
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    const host = document.createElement('div')
    document.body.append(host)
    const root = createRoot(host)
    cleanup = () => { act(() => root.unmount()); registry.dispose(); host.remove(); vi.unstubAllGlobals() }
    await act(async () => root.render(
      <DataSourceProvider source={source} registry={registry}>
        <LiveAgentAlerts agentRef={shown.requester_id} />
      </DataSourceProvider>,
    ))
    const merge = button(host, 'Merge')!
    const changes = button(host, 'Change it')!
    const words = host.querySelector('textarea')!
    expect(merge).toBeDefined()
    expect(merge.disabled).toBe(false)
    expect(changes).toBeDefined()
    expect(words).not.toBeNull()
    if (requestType === 'feedback') expect(button(host, 'Send')).toBeDefined()
    const activations: string[] = []
    for (const name of ['click', 'submit', 'input', 'change', 'keydown']) {
      host.addEventListener(name, event => {
        if ((event.target as Element).closest('button, textarea, form') !== null) activations.push(name)
      }, { capture: true })
    }
    let target: ClientControlRouteTarget = { _tag: 'NowRoot' }
    let signatures = 0
    const permission = testGrant(shown.id)
    const lifecycle = testLifecycle(permission)
    const commandFor = (verb: string, args: unknown = {}) => testCommand(verb, args, permission)
    const commands = [
      { command: commandFor('phone.inspect'), refused: false },
      { command: commandFor('phone.navigate', { screen: 'Attention', params: { id: shown.id } }), refused: false },
      { command: commandFor('phone.inspect'), refused: false },
      { command: commandFor('phone.navigate', { screen: 'Attention', params: { id: 'attention/not-bound' } }), refused: true },
      ...['work.done', 'prompt.respond', 'attention.answer', 'attention.close', 'phone.message.read', 'read-position.advance']
        .map(verb => ({ command: commandFor(verb, verb === 'phone.message.read' ? { message_id: 'message/unit' } : {}), refused: true })),
      { command: commandFor('phone.navigate', { screen: 'NowRoot' }), refused: false },
      { command: commandFor('phone.inspect'), refused: false },
    ]
    for (const { command, refused } of commands) {
      const before = signatures
      const previous = target
      await act(async () => {
        expect(await handleGrantedDeviceCommand({
          command, permission: () => permission, lifecycle, currentTarget: () => target,
          foreground: () => true, navigate: async next => { target = next },
          signVisibleState: async () => { signatures += 1 },
        })).toBe(refused ? 'Refused' : 'Applied')
      })
      if (refused) { expect(target).toBe(previous); expect(signatures).toBe(before) }
      expect(actionRequests).toEqual([])
      expect(promptRequests).toEqual([])
      expect(activations).toEqual([])
      expect(button(host, 'Merge')).toBe(merge)
      expect(button(host, 'Change it')).toBe(changes)
      expect(host.querySelector('textarea')).toBe(words)
      for (const control of host.querySelectorAll<HTMLElement>('button, textarea')) {
        expect(control.isConnected).toBe(true)
        expect(control.hidden).toBe(false)
        expect(control.closest('[hidden], [aria-hidden="true"]')).toBeNull()
      }
      expect(words.value).toBe('')
      expect(host.querySelector('[aria-label="Confirm your answer"]')).toBeNull()
      if (requestType === 'feedback') expect(button(host, 'Send')).toBeDefined()
    }
  },
)
