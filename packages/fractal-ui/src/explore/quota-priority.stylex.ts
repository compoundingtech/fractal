import * as stylex from '@stylexjs/stylex'

export const quotaPriorityGeometry = stylex.defineVars({
  sidebar200: '200px', sidebar240: '240px', sidebar320: '320px',
  host1440: '1440px', host900: '900px',
  hiddenTracks: 'minmax(0, 1fr) 56px 5ch',
  collapsedTracks: 'minmax(0, 1fr) 56px 5ch 24px',
  fullTracks: 'minmax(0, 1fr) 56px 5ch 56px 24px',
  truncateTracks: 'minmax(0, 2fr) 56px 5ch minmax(0, 1fr) minmax(0, 1fr)',
})
