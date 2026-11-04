// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type { ConversationPage, Feed } from '../data/source.ts'
import { ConversationPane } from './ConversationPane.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const source = vi.hoisted(() => ({ feed: { _tag: 'Waiting' } as Feed<ConversationPage> }))
const failedImport = vi.hoisted(() => {
  let release!: () => void
  const ready = new Promise<void>(resolve => { release = resolve })
  return { ready, release, attempts: 0 }
})
vi.mock('../../../../packages/fractal-ui/src/assistant-ui/composition/syntaxGrammar', async () => {
  failedImport.attempts++
  await failedImport.ready
  throw new Error('Grammar chunk unavailable')
})
vi.mock('../data/react.tsx', async () => {
  const Atom = await import('effect/reactivity/Atom')
  const agents = Atom.make({ _tag: 'Observed' as const, freshness: 'live' as const, value: [] })
  return {
    useConversation: () => source.feed,
    useConversationSync: () => ({ status: { _tag: 'Live', since: 1000 }, observedAt: 1000 }),
    useConnection: () => ({ _tag: 'Live' }),
    useDataSource: () => ({ agents, attachments: {} }),
    useFeedInterest: () => {},
    useNowSnapshot: () => 1000,
    usePausedNow: () => undefined,
    useGrants: () => ({ actions: 'ungranted', messageSend: 'granted', attachmentUpload: 'ungranted' }),
    useAttention: () => ({ _tag: 'Waiting' }),
  }
})

it('contains a rejected grammar import to plain code and retries on a later transcript render', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => {})
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const show = async (code: string) => {
    source.feed = { _tag: 'Observed', freshness: 'live', value: {
      items: [{ _tag: 'Text', id: 'answer', role: 'assistant', text: `Readable **answer**.\n\n\`\`\`typescript\n${code}\n\`\`\``, attachments: [], streaming: false, at: '2026-11-02T00:00:00.000Z' }],
      hasOlder: false, observation: { empty: false },
    } }
    await act(async () => root.render(<ConversationPane agentRef="agent/example" agentName="Example" onOpenTool={() => {}} />))
  }
  try {
    await show('const answer = 42')
    const transcript = container.querySelector('[data-testid="transcript-scroll"]')!
    const composer = container.querySelector<HTMLTextAreaElement>('textarea')!
    const code = transcript.querySelector('code')!
    expect(composer).not.toBeNull()
    expect(code.textContent).toBe('const answer = 42\n')
    await act(async () => composer.focus())
    await act(async () => { failedImport.release(); await vi.dynamicImportSettled() })
    expect(failedImport.attempts).toBe(1)
    expect(container.querySelector('[data-testid="transcript-scroll"]')).toBe(transcript)
    expect(container.querySelector('textarea')).toBe(composer)
    expect(document.activeElement).toBe(composer)
    expect(transcript.textContent).toContain('Readable answer.')
    expect(code.textContent).toBe('const answer = 42\n')
    expect(code.querySelector('[data-syntax-token]')).toBeNull()
    vi.doMock('../../../../packages/fractal-ui/src/assistant-ui/composition/syntaxGrammar', async importOriginal => importOriginal())
    await show('const answer = 43')
    expect(container.querySelector('[data-testid="transcript-scroll"]')).toBe(transcript)
    expect(container.querySelector('textarea')).toBe(composer)
    await act(async () => { await vi.dynamicImportSettled() })
    expect(document.activeElement).toBe(composer)
    expect(code.textContent).toBe('const answer = 43\n')
    expect(code.querySelector('[data-syntax-token="keyword"]')?.textContent).toBe('const')
  } finally {
    failedImport.release()
    await act(async () => root.unmount())
    container.remove()
    vi.unstubAllGlobals()
  }
})
