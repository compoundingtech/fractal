import * as stylex from '@stylexjs/stylex'

/**
 * Mission pill hues: five mid tones, each at least 3:1 against its own 14% wash on both the dark and the light sidebar,
 * so one value serves both schemes. None is the running blue, the attention amber or the danger red. Only the mission
 * glyph and the pill wash take the hue; text stays on the ink tokens.
 */
export const missionHueVars = stylex.defineVars({
  teal: '#0d9488',
  violet: '#8b5cf6',
  pink: '#db2777',
  olive: '#5b8a12',
  cyan: '#0891b2',
})
