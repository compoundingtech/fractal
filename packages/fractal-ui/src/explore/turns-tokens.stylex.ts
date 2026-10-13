import * as stylex from '@stylexjs/stylex'

/** Story-only geometry; production hierarchy dimensions remain locked. */
export const turnsGeometry = stylex.defineVars({
  narrow: '900px', metaRoom: '168px',
  // A gutter column holds the widest clock ('11:47 AM' at 12px) above a 24px Copy.
  metaGutter: '64px',
})
