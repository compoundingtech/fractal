import * as stylex from '@stylexjs/stylex'

/** Story-only spinner geometry and motion; production status glyphs remain unchanged. */
export const spinnerGeometry = stylex.defineVars({
  size12: '12px', size14: '14px', size16: '16px',
  host440: '440px', host1440: '1440px',
  // S3: each dot is a fifth of the glyph box, so the two gaps are a fifth each.
  dot: '20%',
  // S4: ten braille frames at the TUI's 80ms cadence.
  brailleCycle: '800ms',
  // S3: negative delays start all three dots in phase order without a static first beat.
  dotLead: '-320ms', dotMid: '-160ms', dotTail: '0ms',
})
