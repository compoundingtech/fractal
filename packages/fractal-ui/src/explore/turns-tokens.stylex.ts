import * as stylex from '@stylexjs/stylex'

/** Story-only geometry; production hierarchy dimensions remain locked. */
export const turnsGeometry = stylex.defineVars({
  narrow: '900px', metaRoom: '168px', trailingWidth: '6em', trailingOverlap: '-6em',
  // The overlay needs the reading lane, canvas padding and panel overhang margins.
  metaPosition: { default: 'static', '@media (min-width: 56rem)': 'absolute' },
})
