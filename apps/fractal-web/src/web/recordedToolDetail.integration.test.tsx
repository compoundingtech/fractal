// @vitest-environment jsdom
/**
 * Semantic wait, message and skipped cards stand in for the kit summary line only: through the real
 * app projection and kit transcript, each call's recorded input, command and output stay one raw
 * disclosure away, and the host Open action stays reachable.
 */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import type { WorkLogCall } from '@fractal/ui/assistant-ui/work-log'
import type { ConversationItem, ToolCallItem } from '../conversation/model.ts'
import { EmbraceRuntimeProvider } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceRuntime.tsx'
import { Transcript } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'
import { mapConversationFeed } from './conversationTranscript.ts'
import { recordedToolDetail } from './semanticTool.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const at = (seconds: number) => `2026-10-08T12:00:${String(seconds).padStart(2, '0')}.000Z`
const send = 'st conversations send agent/example/Reviewer --from agent/example/Builder --idempotency-key example-send-1 --subject "Recorded review" --body "Please inspect the **recorded review**." </dev/null 2>&1 | tail -1'
const tools = {
  send: { _tag: 'ToolCall', id: 'example/send', callId: 'call/send', name: 'bash', input: { command: send }, status: 'success', callSeen: true, at: at(1), result: { content: 'Message sent.', isError: false, at: at(2) } },
  wait: { _tag: 'ToolCall', id: 'example/wait', callId: 'call/wait', name: 'wait', input: { jobs: ['bg_example'], timeout: 30 }, status: 'success', callSeen: true, at: at(3), result: { content: 'bg_example completed', isError: false, at: at(4), metadata: { jobs: [{ jobId: 'bg_example', status: 'completed' }] } } },
  skipped: { _tag: 'ToolCall', id: 'example/skipped', callId: 'call/skipped', name: 'bash', input: { command: 'printf ready --verbose' }, status: 'success', skipped: true, callSeen: true, at: at(5), result: { content: 'Interrupted before execution.', isError: false, at: at(6) } },
} satisfies Record<string, ToolCallItem>
const items: readonly ConversationItem[] = [
  { _tag: 'Text', id: 'example/prompt', role: 'user', text: 'Ask the reviewer and wait for the job.', attachments: [], streaming: false, at: at(0) },
  tools.send, tools.wait, tools.skipped,
  { _tag: 'Status', id: 'example/status', status: 'completed', detail: 'Finished', at: at(7) },
]

let root: Root | undefined
const container = document.createElement('div')
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => {})
  document.body.appendChild(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root?.unmount())
  container.remove()
  container.textContent = ''
  vi.unstubAllGlobals()
})

it('keeps raw input, the literal send command with its flags, and output beside semantic wait, message and skipped cards', async () => {
  const state = mapConversationFeed({ _tag: 'Observed', freshness: 'live', value: { items, hasOlder: false, observation: { empty: false } } }, {})
  if (state._tag !== 'Observed') throw new Error('expected an observed transcript')
  const opened: WorkLogCall[] = []
  const onOpenTool = (call: WorkLogCall) => { opened.push(call) }
  await act(async () => root!.render(<EmbraceRuntimeProvider options={{ messages: state.items, isRunning: false, onNew: async () => {} }}>
    <Transcript title="Recorded tools" turns={state.turns} sync={{ _tag: 'Live', since: 1000 }} now={1000} observedAt={1000} onOpenTool={onOpenTool}
      renderToolDetail={(item: ToolCallItem, call: WorkLogCall) => recordedToolDetail(item, call, () => onOpenTool(call))} />
  </EmbraceRuntimeProvider>))
  await act(async () => { await Promise.resolve() })
  await act(async () => container.querySelector<HTMLButtonElement>('[data-testid="work-log"] button')!.click())

  const preview = (id: string) => container.querySelector<HTMLElement>(`[data-scroll-anchor-id="${id}"] [data-testid="tool-detail-preview"]`)!
  const showRaw = async (id: string) => {
    const raw = [...preview(id).querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === 'Show raw input/output')!
    expect(raw.getAttribute('aria-expanded')).toBe('false')
    await act(async () => raw.click())
  }
  const rawInput = (id: string) => preview(id).querySelector('[data-testid="tool-raw-input"] code')?.textContent

  // Semantic cards render inside the kit preview, never instead of it.
  expect(preview(tools.send.id).querySelector('[data-testid="outgoing-message-card"]')?.textContent).toContain('Recorded review')
  expect(preview(tools.wait.id).querySelector('[data-testid="wait-row"]')?.textContent).toContain('Wait finished')
  expect(preview(tools.skipped.id).querySelector('[data-testid="skipped-tool"]')).not.toBeNull()
  // The message card reads the send; the literal invocation stays behind the raw disclosure.
  expect(preview(tools.send.id).querySelector('[data-testid="tool-command"]')).toBeNull()

  await showRaw(tools.send.id)
  const command = preview(tools.send.id).querySelector('[data-testid="tool-command"] code')?.textContent
  expect(command).toBe(send)
  for (const flag of ['--from agent/example/Builder', '--idempotency-key example-send-1', '--subject "Recorded review"', '--body "Please inspect the **recorded review**."']) expect(command).toContain(flag)
  expect(rawInput(tools.send.id)).toBe(JSON.stringify(tools.send.input, null, 2))
  expect(preview(tools.send.id).textContent).toContain('Message sent.')

  await showRaw(tools.wait.id)
  expect(rawInput(tools.wait.id)).toBe(JSON.stringify(tools.wait.input, null, 2))
  expect(preview(tools.wait.id).textContent).toContain('bg_example completed')

  await showRaw(tools.skipped.id)
  expect(rawInput(tools.skipped.id)).toBe(JSON.stringify(tools.skipped.input, null, 2))
  expect(preview(tools.skipped.id).querySelector('[data-testid="tool-command"] code')?.textContent).toBe('printf ready --verbose')
  expect(preview(tools.skipped.id).textContent).toContain('Interrupted before execution.')
  // A skipped card has no inspector button of its own; the kit Open action still reaches the host.
  await act(async () => preview(tools.skipped.id).querySelector<HTMLButtonElement>('button[aria-label="Open Skipped tool detail"]')!.click())
  expect(opened.map(call => call.id)).toEqual([tools.skipped.id])
})
