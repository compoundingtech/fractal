// @vitest-environment jsdom
import { RegistryContext } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { ThreadEditor } from './ThreadEditor.tsx'
import type { ThreadDraft, ThreadSendResult } from './threadSource.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it('mounts the common composer with the exact quote and clears the draft on confirmation', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class {
    observe() {}
    unobserve() {}
    disconnect() {}
  })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const registry = AtomRegistry.make()
  const draft = Atom.make<ThreadDraft>({
    content: 'Run it once more.',
    quote: { id: 'message/one', sender: 'Atlas', excerpt: 'Worker passed' },
  })
  let complete!: (result: ThreadSendResult) => void
  const pending = new Promise<ThreadSendResult>(resolve => { complete = resolve })
  const send = vi.fn((_text: string, _quote?: string) => pending)
  try {
    await act(async () => root.render(<RegistryContext.Provider value={registry}>
      <ThreadEditor source={{ draft: () => draft }} destination="thread/one" label="PR #42" send={send} />
    </RegistryContext.Provider>))
    expect(host.querySelector('[aria-label="Message composer, PR #42"]')).not.toBeNull()
    expect(host.querySelector('textarea')?.value).toBe('Run it once more.')
    const button = Array.from(host.querySelectorAll('button')).find(button => button.textContent === 'Send' || button.getAttribute('aria-label') === 'Send')
    expect(button).toBeDefined()
    await act(async () => button?.click())
    expect(send).toHaveBeenCalledWith('Run it once more.', 'message/one')
    expect(host.querySelector('textarea')?.value).toBe('Run it once more.')
    await act(async () => complete({ _tag: 'Sent', threadId: 'thread/one' }))
    expect(host.querySelector('textarea')?.value).toBe('')
    expect(host.textContent).not.toContain('Worker passed')
  } finally {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
    vi.unstubAllGlobals()
  }
})
