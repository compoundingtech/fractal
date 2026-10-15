import * as stylex from '@stylexjs/stylex'
import { Button, Tooltip, TooltipTrigger } from 'react-aria-components'
import { accentVars as accent, borderVars as border, geometryVars as geometry, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from '../composition-tokens.stylex'

/** Missing observations stay terse; source provenance is available to keyboard and pointer users. */
export function MissingDetailsValue({ label, reason }: { readonly label: string; readonly reason: string }) {
  return <TooltipTrigger delay={0}><Button data-details-missing-value aria-label={`${label}: unavailable`} {...stylex.props(styles.trigger)}>—</Button><Tooltip {...stylex.props(styles.tooltip)}>{reason}</Tooltip></TooltipTrigger>
}
export function DetailsUnavailable({ reason }: { readonly reason: string }) {
  return <p data-details-unavailable {...stylex.props(styles.line)}>Unavailable <MissingDetailsValue label="Unavailable observations" reason={reason}/></p>
}
const styles = stylex.create({
  trigger: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', padding: space.zero, minWidth: geometry.icon, borderWidth: 0, backgroundColor: surface.transparent, color: text.fgMuted, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, cursor: 'help', ':focus-visible': { outline: `2px solid ${accent.primary}`, outlineOffset: geometry.focusOffset } },
  line: { display: 'flex', alignItems: 'center', gap: space.xs, margin: space.zero, color: text.fgMuted, whiteSpace: 'nowrap', fontSize: type.metaSize, lineHeight: type.metaLeading },
  tooltip: { maxWidth: geometry.tooltipMax, padding: space.md, color: text.fg, backgroundColor: surface.raised, borderWidth: geometry.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.sm, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading },
})
