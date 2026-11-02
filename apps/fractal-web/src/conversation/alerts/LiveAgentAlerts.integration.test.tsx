// @vitest-environment jsdom
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type * as Native from '@fractal/client/schema'
import { DateTime } from 'effect'

const data = vi.hoisted(() => ({ attention: [] as unknown[], act: vi.fn() }))
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
}))

import { LiveAgentAlerts } from './AgentAlerts.tsx'
import type { UxTelemetry } from '../../telemetry/ux.ts'

const seat = 'agent/example/atlas'
const ask = (id: string): Native.Attention =>
  ({
    id, kind: 'attention', revision: 'r1', attention_kind: 'person-step', title: `${id}?`, detail: '',
    priority: 'high', state: 'open', requested_at: DateTime.makeUnsafe('2026-10-14T10:00:00Z'),
    person_id: 'person/avery', source_id: `step-run/${id}`, requester_id: seat, conversation_id: seat,
    conversation_ids: [seat], alert: true, actions: ['work.done'],
  }) as unknown as Native.Attention

const click = async (container: HTMLElement, label: string) =>
  act(async () => [...container.querySelectorAll('button')].find((each) => each.textContent === label)!.click())

it('times each answered alert until the strip no longer shows it, and drops refused answers', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  const landed = vi.fn(() => () => {})
  const cancel = vi.fn()
  const ux = { beginAlertAnswer: vi.fn(() => ({ landed, cancel })) } as unknown as UxTelemetry
  data.attention = [ask('attention/one'), ask('attention/two')]
  data.act.mockResolvedValueOnce({ _tag: 'Refused', detail: 'Fractal no longer offers this answer here.' }).mockResolvedValueOnce({ _tag: 'Sent' })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const render = () => act(async () => root.render(<LiveAgentAlerts agentRef={seat} items={[]} ux={ux} />))
  await render()
  expect(container.textContent).toContain('◆ 2 alerts')

  await click(container, 'Yes')
  await click(container, 'Send')
  expect(container.querySelector('[role="status"]')?.textContent).toBe('Fractal no longer offers this answer here.')
  expect(cancel).toHaveBeenCalledOnce()

  await click(container, 'Yes')
  await click(container, 'Send')
  expect(data.act).toHaveBeenLastCalledWith({ alert: data.attention[0], action: { type: 'work.done', summary: 'Yes' } })
  expect(landed).not.toHaveBeenCalled()

  // Fractal records the answer: the alert leaves the attention collection and the strip.
  data.attention = [ask('attention/two')]
  await render()
  expect(landed).toHaveBeenCalledOnce()
  expect(container.textContent).not.toContain('attention/one?')
  expect(ux.beginAlertAnswer).toHaveBeenCalledTimes(2)
  act(() => root.unmount())
  container.remove()
})

it('groups simultaneous native questions without creating duplicate navigation landmarks', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  data.attention = []
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const calls = ['first', 'second'].map(id => ({
    _tag: 'ToolCall', id: `item/${id}`, callId: id, name: 'ask', status: 'running', callSeen: true,
    input: { questions: [{ id: 'storage', question: 'Storage?', options: [{ label: 'SQLite' }] }] },
  }))
  await act(async () => root.render(<LiveAgentAlerts agentRef={seat} items={calls} />))
  const alerts = container.querySelectorAll('section[aria-label="The agent has a question"]')
  expect(alerts).toHaveLength(2)
  for (const alert of alerts) expect(alert.getAttribute('role')).toBe('group')
  expect(container.querySelector('[role="region"]')).toBeNull()
  await act(async () => root.unmount())
  container.remove()
})

it('shows native OMP asks without attention, opens the terminal and clears the draft on native settlement', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  data.attention = []
  const openTerminal = vi.fn()
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const call = {
    _tag: 'ToolCall', id: 'item/native', callId: 'native', name: 'ask', status: 'running', callSeen: true,
    input: { questions: [{ id: 'storage', question: 'Storage?', options: [{ label: 'SQLite', preview: '<script>text only</script>' }] }] },
  }
  const render = (items: readonly typeof call[]) =>
    act(async () => root.render(<LiveAgentAlerts agentRef={seat} items={items} onOpenTerminal={openTerminal} />))
  await render([call])
  expect(container.querySelector('[data-testid="conversation-alerts"]')).not.toBeNull()
  expect(container.querySelector('script')).toBeNull()
  expect(container.querySelector('pre')?.textContent).toBe('<script>text only</script>')
  expect(container.querySelector<HTMLInputElement>('input[type="radio"]')?.checked).toBe(false)
  await act(async () => container.querySelector<HTMLInputElement>('input[type="radio"]')!.click())
  expect(container.querySelector<HTMLInputElement>('input[type="radio"]')?.checked).toBe(true)
  expect([...container.querySelectorAll('button')].find(button => button.textContent === 'Send answers')?.disabled).toBe(true)
  await click(container, 'Open terminal')
  expect(openTerminal).toHaveBeenCalledOnce()
  await render([{ ...call, input: { questions: [{ id: 'storage', question: 'Changed?', options: [{ label: 'New choice', preview: '' }] }] } }])
  expect(container.querySelector<HTMLInputElement>('input[type="radio"]')?.checked).toBe(false)
  await render([{ ...call, status: 'success' }])
  expect(container.querySelector('[data-testid="conversation-alerts"]')).toBeNull()
  await act(async () => root.unmount())
  container.remove()
})
