import { describe, expect, it, vi } from 'vitest'

// Node rendering exercises the real module, not the CSS compiler.
vi.mock('@stylexjs/stylex', () => ({
  create: (styles: unknown) => styles,
  defineVars: (variables: unknown) => variables,
  createTheme: () => ({}),
  keyframes: () => 'test-animation',
  props: () => ({}),
}))
vi.mock('../../data/react.tsx', () => ({ useAttention: vi.fn(), useDataSource: vi.fn() }))

import { dockBlockSize } from './AgentAlerts.tsx'

/** A docked three-option decision (meta, heading, context, More, three options with rationale, the quiet row) at 1280×800. */
const THREE_OPTION_CARD = 440

describe('docked decision room', () => {
  it('shows every option of a three-option decision without scrolling at an 800px window', () => {
    // The conversation column above the composer at 1280×800 is about 700px.
    expect(dockBlockSize(700)).toBeGreaterThanOrEqual(THREE_OPTION_CARD)
    expect(dockBlockSize(760)).toBeGreaterThanOrEqual(THREE_OPTION_CARD)
  })

  it('still leaves the transcript a visible strip, and never shrinks below the floor', () => {
    expect(700 - dockBlockSize(700)).toBeGreaterThanOrEqual(160)
    expect(dockBlockSize(300)).toBe(220)
  })
})
