import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'
/** Theme and viewport framing only; every child is a production kit component. */
export function deliveryStoryMatrix(render: () => React.ReactNode) {
  return <div {...stylex.props(styles.matrix)}>{(['dark', 'light'] as const).flatMap(scheme => ([440, 1440] as const).map(width => <section key={`${scheme}-${width}`} data-scheme={scheme} data-width={width} {...stylex.props(styles.frame, width === 440 ? styles.narrow : styles.wide, ...baselineTheme, scheme === 'light' && lightTheme)}><h2 {...stylex.props(styles.label)}>{scheme} · {width}px</h2>{render()}</section>))}</div>
}
const styles = stylex.create({ matrix: { display: 'flex', flexDirection: 'column', gap: s.xl, alignItems: 'flex-start' }, frame: { boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans }, narrow: { width: '440px' }, wide: { width: '1440px' }, label: { margin: 0, marginBottom: s.lg, fontSize: t.metaSize, color: ink.fgMuted } })
