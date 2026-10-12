import * as stylex from '@stylexjs/stylex'

/** Story-only geometry; production hierarchy dimensions remain locked. */
export const turnsGeometry = stylex.defineVars({
  glyph: '24px', narrow: '900px', wide: '1440px', gutter: '76px', lift: '-16px',
  metaRoom: '168px', indent: '22px', railInset: '7.5px', trailingWidth: '6em', trailingOverlap: '-6em',
})
