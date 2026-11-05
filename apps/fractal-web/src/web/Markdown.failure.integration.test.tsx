// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type * as LazyChunkModule from '../../../../packages/fractal-ui/src/lazy-chunk.ts'
import type { ConversationPage, Feed } from '../data/source.ts'
import { ConversationPane } from './ConversationPane.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const recovery = vi.hoisted(() => ({ load: vi.fn(), retry: vi.fn(), reload: vi.fn() }))
vi.mock('../../../../packages/fractal-ui/src/lazy-chunk.ts', async importOriginal => {
  const original = await importOriginal<typeof LazyChunkModule>()
  return {
    ...original,
    createLazyChunk: ((options: Parameters<typeof original.createLazyChunk>[0]) => {
      const chunk = original.createLazyChunk({ ...options, load: () => { recovery.load(); return options.load() } })
      return { ...chunk, retry: recovery.retry, reload: recovery.reload }
    }) as typeof original.createLazyChunk,
  }
})
const source = vi.hoisted(() => ({ feed: { _tag: 'Waiting' } as Feed<ConversationPage> }))
const failedImport = vi.hoisted(() => {
  let release!: () => void
  const ready = new Promise<void>(resolve => { release = resolve })
  return { ready, release, attempts: 0 }
})
vi.mock('@pierre/highlights', async () => {
  failedImport.attempts++
  await failedImport.ready
  throw new Error('Highlighter chunk unavailable')
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

it('contains a rejected highlighter import to sticky plain code across transcript reopen without reload', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => setTimeout(() => callback(0), 0))
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
    await act(async () => { await vi.waitFor(() => expect(failedImport.attempts).toBe(1)) })
    const transcript = container.querySelector('[data-testid="transcript-scroll"]')!
    const composer = container.querySelector<HTMLTextAreaElement>('textarea')!
    const code = transcript.querySelector('code')!
    const pre = code.closest('pre')!
    const wrap = transcript.querySelector<HTMLButtonElement>('[aria-label="Wrap code"]')!
    const copy = transcript.querySelector<HTMLButtonElement>('[aria-label="Copy code"]')!
    expect(pre.getAttribute('role')).toBe('group')
    expect(pre.tabIndex).toBe(0)
    expect(copy).not.toBeNull()
    await act(async () => wrap.click())
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
    expect(transcript.querySelector('code')).toBe(code)
    expect(code.closest('pre')).toBe(pre)
    expect(transcript.querySelector('[aria-label="Copy code"]')).toBe(copy)
    expect(wrap.getAttribute('aria-pressed')).toBe('true')
    vi.doMock('@pierre/highlights', async importOriginal => {
      failedImport.attempts++
      return importOriginal()
    })
    await show('const answer = 43')
    expect(container.querySelector('[data-testid="transcript-scroll"]')).toBe(transcript)
    expect(container.querySelector('textarea')).toBe(composer)
    await act(async () => { await vi.dynamicImportSettled() })
    expect(document.activeElement).toBe(composer)
    expect(code.textContent).toBe('const answer = 43\n')
    expect(failedImport.attempts).toBe(1)
    expect(code.querySelector('[data-syntax-token]')).toBeNull()
    await act(async () => root.render(null))
    await show('const answer = 43')
    await act(async () => { await vi.dynamicImportSettled() })
    expect(failedImport.attempts).toBe(1)
    expect(recovery.load).toHaveBeenCalledTimes(1)
    expect(recovery.retry).not.toHaveBeenCalled()
    expect(recovery.reload).not.toHaveBeenCalled()
    expect(container.querySelector('[data-syntax-token]')).toBeNull()
    expect(container.textContent).not.toMatch(/Reload and retry|Try again/)
    expect(container.querySelector('code')?.textContent).toBe('const answer = 43\n')
  } finally {
    failedImport.release()
    await act(async () => root.unmount())
    container.remove()
    vi.unstubAllGlobals()
  }
})
