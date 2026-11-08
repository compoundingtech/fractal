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
    expect(dockBlockSize(700, false)).toBeGreaterThanOrEqual(THREE_OPTION_CARD)
  })

  it('leaves a phone more, since its tab bar, find line and taller composer share the same column', () => {
    // A 440×900 phone column is about 770px; a 4-option card is about 565px of it.
    expect(771 - dockBlockSize(771, true)).toBeGreaterThanOrEqual(280)
    expect(dockBlockSize(771, true)).toBeLessThan(dockBlockSize(771, false))
  })

  it('still leaves the transcript a visible strip, and never shrinks below the floor', () => {
    expect(700 - dockBlockSize(700, false)).toBeGreaterThanOrEqual(160)
    expect(dockBlockSize(300, false)).toBe(220)
  })
})
