import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { TranscriptTurn } from '@smalltalk/fractal-ui/assistant-ui'
import type { TextItem } from '../conversation/model.ts'
import { NativeTurnFooter, nativeTurnFooter } from './NativeTurnFooter.tsx'

// Match the app's node-rendering lane: exercise semantic DOM without the CSS compiler.
vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))

describe('native turn footer', () => {
  it('renders only reported model, token and cost facts', () => {
    expect(renderToStaticMarkup(createElement(NativeTurnFooter, { facts: { model: 'example-model', tokens: 4200, costUsd: 0.025 } }))).toBe('<footer data-testid="native-turn-footer">example-model · 4,200 tokens · $0.0250</footer>')
    expect(renderToStaticMarkup(createElement(NativeTurnFooter, { facts: { model: 'example-model', tokens: 0, costUsd: 0 } }))).toContain('example-model · 0 tokens · $0.0000')
  })
  it('never synthesizes facts when none are reported', () => {
    expect(renderToStaticMarkup(createElement(NativeTurnFooter, { facts: {} }))).toBe('')
  })
  it('the transcript renderer uses native turn facts and omits turns without them', () => {
    const item = {
      _tag: 'Text', id: 'assistant', role: 'assistant', text: 'Done.', attachments: [], streaming: false, at: '2026-01-15T12:00:00Z',
      assistantMetadata: { model: 'native-model', usage: { input: 1000, output: 200, cost_usd: 0.012 } },
    } satisfies TextItem
    const turn: TranscriptTurn = {
      id: 'native-turn', items: [item],
      work: { calls: [], running: false, failed: false, interrupted: false, durationMs: undefined },
    }
    expect(renderToStaticMarkup(nativeTurnFooter(turn))).toBe('<footer data-testid="native-turn-footer">native-model · 1,200 tokens · $0.0120</footer>')
    expect(renderToStaticMarkup(nativeTurnFooter({ ...turn, items: [] }))).toBe('')
    const withoutFacts = { ...item, assistantMetadata: {} } satisfies TextItem
    expect(renderToStaticMarkup(nativeTurnFooter({ ...turn, items: [withoutFacts] }))).toBe('')
  })
})
