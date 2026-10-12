import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button as AriaButton } from 'react-aria-components'
import { accentVars as accent, textVars as text, borderVars as border, surfaceVars as surface, spaceVars as s, geometryVars as g, typeVars as t, motionVars as m } from '../composition-tokens.stylex'
import { Markdown } from './Markdown'
import { Icon } from './Icons'

const reasoningType = stylex.createTheme(t, { bodySize: t.codeSize, bodyLeading: t.uiLeading })
const reasoningInk = stylex.createTheme(text, { fgSoft: text.fgMuted, fg: text.fgMuted })

/** Muted reasoning disclosure: collapsed by default, streaming-aware, prose rendered through the shared Markdown seam. */
export function ThinkingEntry({ text: value, streaming = false }: { text: string; streaming?: boolean }) {
  const [open, setOpen] = React.useState(false)
  const contentId = React.useId()
  return <div data-testid="thinking-entry" data-streaming={streaming} {...stylex.props(styles.thinking)}>
    <AriaButton aria-expanded={open} aria-controls={contentId} onPress={() => setOpen(value => !value)} {...stylex.props(styles.thinkingControl)}>
      <Icon name={streaming ? 'spinner' : open ? 'chevron-down' : 'chevron-right'} spinning={streaming} /><span>Thinking</span>
    </AriaButton>
    {open ? <div id={contentId} data-testid="thinking-body" {...stylex.props(styles.body, reasoningType, reasoningInk)}><Markdown text={value} streaming={streaming} /></div> : null}
  </div>
}

const styles = stylex.create({
  thinking: { color: text.fgMuted, fontSize: t.uiSize, lineHeight: t.uiLeading },
  thinkingControl: { display: 'flex', alignItems: 'center', gap: s.sm, padding: 0, minHeight: g.toolRow, borderWidth: 0, backgroundColor: surface.transparent, color: text.fgMuted, fontFamily: t.fontSans, fontSize: t.uiSize, cursor: 'pointer', transitionProperty: 'background-color, color', transitionDuration: m.fast, transitionTimingFunction: m.ease, '@media (prefers-reduced-motion: reduce)': { transitionDuration: '0s' }, ':hover': { color: text.fgSoft, backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  body: { borderLeftWidth: g.hairline, borderLeftStyle: 'solid', borderLeftColor: border.border, paddingLeft: s.lg, color: text.fgMuted, fontSize: t.codeSize, lineHeight: t.uiLeading },
})
