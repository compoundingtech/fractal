import * as stylex from '@stylexjs/stylex'
import { elevationVars as elevation, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as text, typeVars as t } from '../composition-tokens.stylex'

/**
 * Shared tooltip surface; portal ownership and trigger behavior stay with each caller. It sizes to its text
 * (up to the cap) rather than to the room left of the viewport edge, so a long reason near the edge shifts
 * inward instead of wrapping word by word. Height is capped too and the label clamps, so a tooltip fed a long
 * string stays a label-sized box inside the viewport rather than a column over the page.
 */
export const tooltipStyles = stylex.create({
  tooltip: { zIndex: 50, borderRadius: r.control, backgroundColor: text.fg, paddingInline: s.row, paddingBlock: s.sm, fontSize: t.denseSize, fontFamily: t.fontSans, color: surface.raised, boxShadow: elevation.popover, boxSizing: 'border-box', width: 'max-content', maxWidth: `min(${g.tooltipMax}, calc(100vw - ${s.lg}))`, whiteSpace: 'normal', overflowWrap: 'break-word', textAlign: 'start', maxHeight: `min(${g.tooltipMaxBlock}, calc(100vh - ${s.lg} * 2))`, overflow: 'hidden' },
  /** A label longer than the cap ends in an ellipsis at the last whole line. */
  label: { display: '-webkit-box', WebkitBoxOrient: 'vertical', WebkitLineClamp: 6, overflow: 'hidden' },
})
