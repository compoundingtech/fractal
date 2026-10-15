// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import type { TranscriptProps } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'
import { DataSourceProvider } from '../data/react.tsx'
import { createHistoryFixture, historyFixtureRef } from '../data/historyFixture.ts'
import { ConversationPane } from './ConversationPane.tsx'

let history: TranscriptProps['history']
vi.mock('../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx', () => ({ Transcript: (props: TranscriptProps) => { history = props.history; return <div>{props.turns.flatMap(turn => turn.items).map(item => item._tag === 'Text' ? <p key={item.id}>{item.text}</p> : undefined)}</div> } }))
vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (values: unknown) => values, createTheme: () => ({}), keyframes: () => '', props: () => ({}) }))

it('maps native paging and recovery to the durable virtual history contract after source rerenders', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  const fixture = createHistoryFixture()
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () => root.render(<DataSourceProvider {...fixture}><ConversationPane agentRef={historyFixtureRef} agentName="Row review" onOpenTool={() => {}} /></DataSourceProvider>))
    expect(history?._tag).toBe('More')
    let loading: Promise<void> | undefined
    await act(async () => { if (history?._tag === 'More') loading = history.loadOlder() })
    expect(history?._tag).toBe('Loading')
    await act(async () => { fixture.settle(false); await loading })
    expect(history?._tag).toBe('Failed')
    expect(container.textContent).toContain('Selection is retained in the newest result.')
    expect(container.textContent).not.toContain('The earlier review identified the stable row keys.')
    await act(async () => { if (history?._tag === 'Failed') loading = history.retry() })
    expect(history?._tag).toBe('Loading')
    await act(async () => { fixture.settle(true); await loading })
    expect(history?._tag).toBe('Complete')
    expect(container.textContent).toContain('The earlier review identified the stable row keys.')
    expect(container.textContent).toContain('Selection is retained in the newest result.')
  } finally {
    await act(async () => root.unmount())
    fixture.registry.dispose()
    container.remove()
    vi.unstubAllGlobals()
  }
})
