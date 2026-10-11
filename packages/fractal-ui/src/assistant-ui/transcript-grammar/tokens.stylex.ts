import * as stylex from '@stylexjs/stylex'
import { geometryVars, spaceVars, typeVars, statusVars } from '../composition-tokens.stylex'

/** Transcript roles derive from the composition scale; geometry stays invariant across themes. */
export const transcriptVars = stylex.defineVars({
  markerHeight: spaceVars.xxl,
  rowHeight: geometryVars.toolRow,
  labelSize: typeVars.uiSize,
  labelLeading: typeVars.uiLeading,
  proseSize: typeVars.bodySize,
  proseLeading: typeVars.bodyLeading,
  metaSize: typeVars.metaSize,
  metaLeading: typeVars.metaLeading,
  tagSize: typeVars.denseSize,
  tagLeading: typeVars.metaLeading,
  outputSize: typeVars.metaSize,
  outputLeading: typeVars.metaLeading,
  itemGap: spaceVars.sm,
  groupGap: spaceVars.zero,
  glyphSize: geometryVars.status,
  glyphBox: geometryVars.icon,
  chevronSize: geometryVars.iconSm,
  attentionBorder: statusVars.attention,
  storyWidthCompact: '440px',
  storyWidthWide: '1440px',
})
