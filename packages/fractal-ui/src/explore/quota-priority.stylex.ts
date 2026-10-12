import * as stylex from '@stylexjs/stylex'

/** Primary slot: 64 fits '1w ▬ 100%' with the bar at its 8 minimum; 52 fits '1w 100%' once the bar is dropped. */
export const quotaPriorityGeometry = stylex.defineVars({
  sidebar200: '200px', sidebar240: '240px', sidebar320: '320px',
  host1440: '1440px', host900: '900px',
  hiddenTracks: 'minmax(0, 1fr) 64px 44px',
  collapsedTracks: 'minmax(0, 1fr) 64px 44px 24px',
  fullTracks: 'minmax(0, 1fr) 64px 44px 56px 24px',
  truncateTracks: 'minmax(0, 2fr) 64px 44px minmax(0, 1fr) minmax(0, 1fr)',
  hiddenBareTracks: 'minmax(0, 1fr) 52px 44px',
  collapsedBareTracks: 'minmax(0, 1fr) 52px 44px 24px',
  truncateBareTracks: 'minmax(0, 2fr) 52px 44px minmax(0, 1fr) minmax(0, 1fr)',
})
