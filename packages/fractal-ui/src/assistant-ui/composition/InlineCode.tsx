import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { radiusVars as r, spaceVars as s, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

/**
 * A Markdown code span (backticks). Every transcript Markdown surface renders spans through this one style;
 * plain-text surfaces never produce it, so no prose is guessed to be code.
 */
export function InlineCode({ children }: { readonly children?: React.ReactNode }) {
  return <code data-inline-code="" {...stylex.props(styles.code)}>{children}</code>
}

const styles = stylex.create({
  // An atomic box: a span that fits the line moves to the next line whole rather than wrapping inside;
  // only a span longer than the line wraps, within the line, instead of overflowing it.
  // The ink is the host's own C18 ink; the fill mixes the C18 muted ink, so both schemes follow without a new grey.
  // 10% keeps fgMuted spans in semantic rows at AA on the canvas (4.65:1 dark, 4.86:1 light); 14% drops dark below 4.5.
  // The size follows the host at 0.9em but never drops below the dense size (11px), so meta rows stay legible.
  code: {
    display: 'inline-block',
    maxWidth: '100%',
    boxSizing: 'border-box',
    verticalAlign: 'baseline',
    paddingInline: s.xs,
    borderRadius: r.inlineCode,
    backgroundColor: `color-mix(in srgb, ${ink.fgMuted} 10%, transparent)`,
    color: 'inherit',
    fontFamily: t.fontMono,
    fontSize: `max(${t.denseSize}, ${t.inlineCodeSize})`,
    lineHeight: 'normal',
    whiteSpace: 'pre-wrap',
    overflowWrap: 'anywhere',
  },
})
