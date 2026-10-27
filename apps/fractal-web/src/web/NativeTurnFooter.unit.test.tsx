import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { TranscriptTurn } from '@fractal/ui/assistant-ui'
import type { TextItem } from '../conversation/model.ts'
import { NativeTurnFooter, formatCostUsd, nativeTurnFooter } from './NativeTurnFooter.tsx'

// Match the app's node-rendering lane: exercise semantic DOM without the CSS compiler.
vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))

describe('native turn footer', () => {
  it('labels reported model, token and cost facts as the last response, never a turn total', () => {
    expect(renderToStaticMarkup(createElement(NativeTurnFooter, { facts: { model: 'example-model', tokens: 4200, costUsd: 0.025 } }))).toBe('<footer data-testid="native-turn-footer">Last response: example-model · 4,200 tokens · $0.025</footer>')
    expect(renderToStaticMarkup(createElement(NativeTurnFooter, { facts: { model: 'example-model', tokens: 0, costUsd: 0 } }))).toBe('<footer data-testid="native-turn-footer">Last response: example-model · 0 tokens · $0</footer>')
    expect(renderToStaticMarkup(createElement(NativeTurnFooter, { facts: { tokens: 84 } }))).toBe('<footer data-testid="native-turn-footer">Last response: 84 tokens</footer>')
  })
  it('formats cost without trailing zeros and never rounds a reported positive cost to zero', () => {
    expect(formatCostUsd(0.025)).toBe('$0.025')
    expect(formatCostUsd(2.5)).toBe('$2.5')
    expect(formatCostUsd(0.1)).toBe('$0.1')
    expect(formatCostUsd(1.23467)).toBe('$1.2347')
    expect(formatCostUsd(12)).toBe('$12')
    expect(formatCostUsd(0)).toBe('$0')
    expect(formatCostUsd(0.00001)).toBe('<$0.0001')
  })
  it('never synthesizes facts when none are reported', () => {
    expect(renderToStaticMarkup(createElement(NativeTurnFooter, { facts: {} }))).toBe('')
  })
  it('the transcript renderer uses native turn facts for finished turns and omits turns without them', () => {
    const item = {
      _tag: 'Text', id: 'assistant', role: 'assistant', text: 'Done.', attachments: [], streaming: false, at: '2026-01-15T12:00:00Z',
      assistantMetadata: { model: 'native-model', usage: { input: 1000, output: 200, cost_usd: 0.012 } },
    } satisfies TextItem
    const turn: TranscriptTurn = {
      id: 'native-turn', items: [item],
      work: { calls: [], running: false, failed: false, interrupted: false, durationMs: undefined },
    }
    expect(renderToStaticMarkup(nativeTurnFooter(turn))).toBe('<footer data-testid="native-turn-footer">Last response: native-model · 1,200 tokens · $0.012</footer>')
    expect(renderToStaticMarkup(nativeTurnFooter({ ...turn, items: [] }))).toBe('')
    const withoutFacts = { ...item, assistantMetadata: {} } satisfies TextItem
    expect(renderToStaticMarkup(nativeTurnFooter({ ...turn, items: [withoutFacts] }))).toBe('')
  })
  it('renders nothing below a running turn even when an earlier response reported facts', () => {
    const item = {
      _tag: 'Text', id: 'assistant', role: 'assistant', text: 'Checking.', attachments: [], streaming: false, at: '2026-01-15T12:00:00Z',
      assistantMetadata: { model: 'native-model', usage: { input: 10, output: 5, cost_usd: 0.001 } },
    } satisfies TextItem
    const running: TranscriptTurn = {
      id: 'running-turn', items: [item],
      work: { calls: [], running: true, failed: false, interrupted: false, durationMs: undefined },
    }
    expect(renderToStaticMarkup(nativeTurnFooter(running))).toBe('')
    expect(renderToStaticMarkup(nativeTurnFooter({ ...running, work: { ...running.work, running: false } }))).toContain('Last response: native-model · 15 tokens · $0.001')
  })
})
