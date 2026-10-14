import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { NativeTurnFooter } from './NativeTurnFooter.tsx'

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
})
