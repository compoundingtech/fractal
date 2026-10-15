// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { useAuiState } from '@assistant-ui/react'
import { expect, it, vi } from 'vitest'
import { EmbraceRuntimeProvider } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceRuntime.tsx'
import { Transcript, type TranscriptTurn } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const at = '2026-01-15T12:00:00Z'
const first = { _tag: 'Text' as const, id: 'first', role: 'user' as const, text: 'First window', attachments: [], streaming: false, at }
const second = { ...first, id: 'second', text: 'Second window' }
const missing = { ...first, id: 'missing', role: 'assistant' as const, text: 'Settled missing answer' }
const work = { calls: [], running: false, failed: false, interrupted: false }
const firstTurns: readonly TranscriptTurn[] = [{ id: 'turn/first', prompt: first, items: [], work }]
const secondTurns: readonly TranscriptTurn[] = [{ id: 'turn/second', prompt: second, items: [missing], work }]
const messages = [first, second]
function RuntimeIds() {
  const ids = useAuiState(state => state.thread.messages.map(message => message.id).join(' '))
  return <output data-testid="ids">{ids}</output>
}
function Windows({ secondVisible }: { secondVisible: boolean }) {
  return <EmbraceRuntimeProvider virtualizeMessages options={{ messages, onNew: async () => {} }}>
    <RuntimeIds />
    <Transcript title="First" turns={firstTurns} history={{ _tag: 'Complete' }} sync={{ _tag: 'Live', since: 0 }} now={0} observedAt={0} />
    {secondVisible && <Transcript title="Second" turns={secondTurns} history={{ _tag: 'Complete' }} sync={{ _tag: 'Live', since: 0 }} now={0} observedAt={0} />}
  </EmbraceRuntimeProvider>
}
it('unions two real viewport demands, renders settled missing rows, and releases only the unmounted window', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () => root.render(<Windows secondVisible />))
    await expect.poll(async () => {
      await act(async () => {})
      return container.querySelector('[data-testid="ids"]')?.textContent
    }).toBe('first second')
    expect(container.querySelector('[data-testid="transcript-stranded"][data-item-id="missing"]')?.textContent).toBe('Settled missing answer')
    await act(async () => root.render(<Windows secondVisible={false} />))
    await expect.poll(async () => {
      await act(async () => {})
      return container.querySelector('[data-testid="ids"]')?.textContent
    }).toBe('first')
    expect(container.querySelector('[data-testid="user-message"][data-item-id="first"]')).not.toBeNull()
  } finally {
    await act(async () => root.unmount())
    container.remove()
    vi.unstubAllGlobals()
  }
})

it('never commits a newly appended answer as stranded against the previous source epoch', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const appended = { ...missing, id: 'appended', text: 'Newly appended answer' }
  const nextMessages = [first, appended]
  const nextTurns: readonly TranscriptTurn[] = [{ ...firstTurns[0]!, items: [appended] }]
  const strandedCommits: boolean[] = []
  const promptCommits: (Element | null)[] = []
  let appending = false
  const probe: React.ProfilerOnRenderCallback = () => {
    if (!appending) return
    strandedCommits.push(container.querySelector('[data-testid="transcript-stranded"][data-item-id="appended"]') !== null)
    promptCommits.push(container.querySelector('[data-testid="user-message"][data-item-id="first"]'))
  }
  const view = (append: boolean) => <React.Profiler id="adoption-commits" onRender={probe}>
    <EmbraceRuntimeProvider virtualizeMessages options={{ messages: append ? nextMessages : [first], onNew: async () => {} }}>
      <Transcript title="Append" turns={append ? nextTurns : firstTurns} history={{ _tag: 'Complete' }} sync={{ _tag: 'Live', since: 0 }} now={0} observedAt={0} />
    </EmbraceRuntimeProvider>
  </React.Profiler>
  try {
    await act(async () => root.render(view(false)))
    await expect.poll(async () => {
      await act(async () => {})
      return container.querySelector('[data-testid="user-message"][data-item-id="first"]') !== null
    }).toBe(true)
    const promptNode = container.querySelector('[data-testid="user-message"][data-item-id="first"]')
    appending = true
    await act(async () => root.render(view(true)))
    await expect.poll(async () => {
      await act(async () => {})
      return container.querySelector('[data-testid="agent-message"][data-item-id="appended"]')?.textContent
    }).toContain('Newly appended answer')
    expect(strandedCommits.length).toBeGreaterThan(0)
    expect(strandedCommits).not.toContain(true)
    expect(promptCommits.every(node => node === promptNode)).toBe(true)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    vi.unstubAllGlobals()
  }
})
