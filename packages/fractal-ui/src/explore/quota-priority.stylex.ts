import * as stylex from '@stylexjs/stylex'

export const quotaPriorityGeometry = stylex.defineVars({
  sidebar200: '200px', sidebar240: '240px', sidebar320: '320px',
  host1440: '1440px', host900: '900px', row: '24px',
  compactGrid: 'minmax(0, 1fr) 64px 28px', mediumGrid: 'minmax(0, 1fr) 64px 48px 28px',
  fullGrid: 'minmax(0, 1fr) 64px 48px 64px 28px',
  truncateGrid: 'minmax(68px, 1fr) minmax(48px, 64px) minmax(0, 48px) minmax(0, 64px) minmax(0, 28px)',
})
