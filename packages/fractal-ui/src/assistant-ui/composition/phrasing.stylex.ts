import * as stylex from '@stylexjs/stylex'
import { surfaceVars as surface, textVars as textColor, borderVars as border, accentVars as accent, statusVars as status, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'

/** The one source for inline phrasing: block `Markdown` and `InlineMarkdown` render code, strong, emphasis and links with these. */
export const phrasingStyles = stylex.create({
  code: { fontFamily: t.fontMono, fontSize: t.metaSize, color: textColor.fg, backgroundColor: surface.codeBg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, paddingInline: s.xs },
  strong: { fontWeight: t.weightBold, color: textColor.fg },
  emphasis: { fontStyle: 'italic' },
  link: { color: status.runningFg, textDecorationLine: 'none', ':hover': { textDecorationLine: 'underline' }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
})
