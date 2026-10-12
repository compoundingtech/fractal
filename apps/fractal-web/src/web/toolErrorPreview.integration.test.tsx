// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { EmbraceToolItem } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceToolCall.tsx'
import { Transcript, ToolDetailPreview } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'
import { WorkLogV1 } from '../../../../packages/fractal-ui/src/assistant-ui/taste/WorkLogV1.tsx'
import { EmbraceRuntimeProvider } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceRuntime.tsx'
import { arrivingSuccess, scopedToolCases, scopedToolItems, toolArrivalInitial, toolFixtureAt } from '../../../../packages/fractal-ui/src/transcript-tool-fixtures.ts'
import type { ConversationItem } from '../conversation/model.ts'
import { prepareTranscriptTurns } from './conversationTranscript.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

let root: Root
let container: HTMLDivElement
const diagnostic = 'The requested change could not be applied.'
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  // These tests exercise disclosure DOM, not viewport geometry (covered by the kit scroll suite).
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => {})
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

it.each(['plain', 'markdown', 'diff'] as const)('shows failed tool diagnostics before raw disclosure alongside %s content', async kind => {
  const input = kind === 'markdown' ? { path: 'notes.md', content: 'Proposed **note**.' }
    : kind === 'diff' ? { patch: '--- rows.ts\n+++ rows.ts\n@@ -1 +1 @@\n-old\n+new' } : { command: 'check rows' }
  await act(async () => root.render(<EmbraceToolItem variant="cards" item={{
    _tag: 'ToolCall', id: 'failed-call', callId: 'failed-call', name: kind === 'diff' ? 'edit' : kind === 'markdown' ? 'write' : 'run',
    input, status: 'error', callSeen: true, at: '2026-01-15T12:00:00Z',
    result: { content: diagnostic, isError: true, at: '2026-01-15T12:00:01Z' },
  }} />))
  const raw = container.querySelector('details')!
  expect(raw.open).toBe(false)
  expect([...container.querySelectorAll('pre')].filter(pre => !raw.contains(pre)).map(pre => pre.textContent)).toContain(diagnostic)
})

it('shows the full failed tool diagnostic inside the row-owned raw detail without a second disclosure', async () => {
  const fixturePath = ['', 'fixture-workspace', 'src', 'rows.ts'].join('/')
  const dump = `Error: ENOENT: no such file or directory, open '${fixturePath}'\n    at open (${['', 'fixture-workspace', 'node_modules', 'loader', 'index.js'].join('/')}:12:3)`
  await act(async () => root.render(<ToolDetailPreview call={{ id: 'failed-call', kind: 'read', title: 'Reading rows', summary: 'Read rows.ts', status: 'error', startedAt: '2026-01-15T12:00:00Z', detail: dump }} />))
  expect(container.querySelector('button')).toBeNull()
  expect(container.querySelector('[data-testid="tool-error-reason"]')?.textContent).toBe("Error: ENOENT: no such file or directory, open '…/rows.ts'")

  expect(container.querySelector('pre')?.textContent).toBe(dump)
})

it('covers the semantic-kind story play: all kinds, success/failure/running and a single raw route', async () => {
  const kinds = ['read', 'search', 'run', 'change', 'subagent', 'web', 'decision', 'todo'] as const
  const targets = ['src/rows.ts', 'visibleRows in src/', 'pnpm test rows', 'src/rows.ts', 'Review selection', 'https://example.com/guide', 'Choose a row order', 'Verify selection']
  await act(async () => root.render(<WorkLogV1 turn={{ calls: kinds.map((kind, index) => ({ id: `semantic/${kind}`, kind, title: `Inspect ${kind} operation`, argsSummary: targets[index], status: index === 2 ? 'running' : index === 3 ? 'error' : 'success', startedAt: '2026-01-15T12:00:00Z', detail: index === 3 ? 'Selection assertion failed.' : `Observed ${kind} result.` })), running: false, failed: false, interrupted: false, foldable: false, durationMs: undefined }} />))
  for (const kind of kinds) expect(container.querySelector(`[data-tool-kind="${kind}"]`)).not.toBeNull()
  expect(container.querySelector('[data-testid="work-call-output"]')).toBeNull()
  expect(container.querySelector('[data-tool-title] [data-syntax-token]')).toBeNull()
  const raw = container.querySelector<HTMLButtonElement>('[aria-label="Inspect read operation src/rows.ts: raw input/output"]')!
  await act(async () => raw.click())
  expect(container.querySelector('[data-testid="work-call-output"]')?.textContent).toBe('Observed read result.')
  expect([...container.querySelectorAll('button')].filter(button => button.textContent === 'Show raw input/output' && !button.closest('[data-error-overlay]'))).toHaveLength(0)
  await act(async () => raw.click())
  expect(container.querySelector('[data-testid="work-call-output"]')).toBeNull()
})

const renderToolSource = async (items: readonly ConversationItem[]) => {
  const now = Date.parse(toolFixtureAt)
  const turns = prepareTranscriptTurns(items, { firstTurnComplete: false })
  const messages = turns.flatMap(turn => turn.items)
  await act(async () => root.render(<EmbraceRuntimeProvider options={{ messages, isRunning: turns.some(turn => turn.work.running), onNew: async () => {} }}><Transcript turns={turns} title="Row selection" sync={{ _tag: 'Live', since: now }} now={now} observedAt={now} onOpenTool={() => {}} /></EmbraceRuntimeProvider>))
}

it('SuccessfulToolArrival story-equivalent assertions keep an appended success collapsed and open 0→1→0', async () => {
  await renderToolSource(toolArrivalInitial)
  expect(container.querySelector('[data-tool-status="success"]')).toBeNull()
  await renderToolSource([...toolArrivalInitial, arrivingSuccess])
  await vi.waitFor(() => expect(container.querySelector('[data-item-id="arrival/run"][data-tool-status]')).not.toBeNull())
  const row = container.querySelector('[data-item-id="arrival/run"][data-tool-status]')!
  const trigger = row.querySelector<HTMLButtonElement>(':scope > button')!
  expect(trigger.getAttribute('aria-expanded')).toBe('false')
  expect(row.querySelectorAll('[data-testid="tool-detail-preview"]')).toHaveLength(0)
  await act(async () => trigger.click())
  expect(trigger.getAttribute('aria-expanded')).toBe('true')
  expect(row.querySelectorAll(':scope > [data-testid="tool-detail-preview"]')).toHaveLength(1)
  expect(row.textContent).toContain('Selection retained. Three checks passed.')
  await act(async () => trigger.click())
  expect(trigger.getAttribute('aria-expanded')).toBe('false')
  expect(row.querySelectorAll('[data-testid="tool-detail-preview"]')).toHaveLength(0)
})

it('ScopedToolTargets story-equivalent assertions show six kinds with source-derived targets in collapsed rows', async () => {
  await renderToolSource(scopedToolItems)
  await vi.waitFor(() => expect(container.querySelectorAll('[data-tool-status]').length).toBe(6))
  for (const example of scopedToolCases) {
    const row = container.querySelector(`[data-item-id="scoped/${example.kind}"][data-tool-status]`)!
    expect(row.querySelector('[data-tool-kind]')?.textContent).toBe(example.kind)
    expect(row.querySelector('[data-tool-title]')?.parentElement?.querySelector('code')?.textContent).toBe(example.target)
    expect(row.querySelector(':scope > button')?.getAttribute('aria-expanded')).toBe('false')
    expect(row.querySelectorAll('[data-testid="tool-detail-preview"]')).toHaveLength(0)
  }
})
