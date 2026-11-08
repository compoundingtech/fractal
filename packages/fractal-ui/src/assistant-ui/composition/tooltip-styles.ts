import * as stylex from '@stylexjs/stylex'
import { elevationVars as elevation, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../composition-tokens.stylex'

/**
 * Shared tooltip surface; portal ownership and trigger behavior stay with each caller. It sizes to its text
 * (up to the cap) rather than to the room left of the viewport edge, so a long reason near the edge shifts
 * inward instead of wrapping word by word.
 */
export const tooltipStyles = stylex.create({
  tooltip: { zIndex: 50, borderRadius: r.control, backgroundColor: text.fg, paddingInline: s.row, paddingBlock: s.sm, fontSize: t.denseSize, fontFamily: t.fontSans, color: surface.raised, boxShadow: elevation.popover, boxSizing: 'border-box', width: 'max-content', maxWidth: `min(${g.tooltipMax}, calc(100vw - ${s.lg}))`, whiteSpace: 'normal', overflowWrap: 'break-word', textAlign: 'start' },
})
