import * as stylex from '@stylexjs/stylex'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, statusVars as status, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'
export const deliveryStyles = stylex.create({
  card: { borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, backgroundColor: surface.raised, padding: s.lg, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, minWidth: 0 },
  row: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.sm, minHeight: g.toolRow, minWidth: 0 },
  stack: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0 },
  button: { borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, padding: s.xs, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary }, ':disabled': { cursor: 'default', color: ink.fgFaint } },
  summary: { textAlign: 'left', display: 'flex', gap: s.sm, alignItems: 'center', width: '100%', minHeight: g.toolRow },
  label: { minWidth: 0, flex: '1', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  mono: { fontFamily: t.fontMono, fontSize: t.metaSize },
  muted: { color: ink.fgMuted, fontSize: t.metaSize },
  chip: { paddingInline: s.sm, borderRadius: r.sm, backgroundColor: surface.rowActive, color: ink.fgSoft, fontSize: t.metaSize },
  danger: { color: status.dangerFg },
  excerpt: { margin: 0, color: ink.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden', overflowWrap: 'anywhere' },
  terminal: { margin: 0, padding: s.md, backgroundColor: surface.terminal, borderRadius: r.sm, fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', color: ink.fgSoft },
  line: { display: 'block', minHeight: t.metaLeading },
  tree: { maxHeight: '340px', overflow: 'auto', fontFamily: t.fontMono, fontSize: t.metaSize, backgroundColor: surface.codeBg, padding: s.md, borderRadius: r.sm },
  node: { paddingLeft: s.lg },
  footer: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border, paddingTop: s.md },
})
