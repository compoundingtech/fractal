import * as stylex from '@stylexjs/stylex'

/** Story-only geometry; production hierarchy dimensions remain locked. */
export const turnsGeometry = stylex.defineVars({
  narrow: '900px', metaRoom: '168px', trailingWidth: '6em', trailingOverlap: '-6em',
  metaPosition: { default: 'static', '@media (min-width: 48rem)': 'absolute' },
})
