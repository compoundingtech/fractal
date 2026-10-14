import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button as AriaButton } from 'react-aria-components'
import { accentVars as accent, textVars as text, spaceVars as s, geometryVars as g, typeVars as t } from '../composition-tokens.stylex'
import { Markdown } from './Markdown'
import { Icon } from './Icons'
import type { ReasoningItem } from '../embrace-data/model'

/** Muted reasoning disclosure: collapsed by default, streaming-aware, prose rendered through the shared Markdown seam. */
export function ThinkingEntry({ text: value, streaming = false }: { text: string; streaming?: boolean }) {
  return <ThinkingDisclosure streaming={streaming}><Markdown text={value} streaming={streaming} /></ThinkingDisclosure>
}

/** One disclosure per adjacent run; source ids and individual Markdown boundaries remain intact. */
export function ThinkingRun({ items }: { readonly items: readonly ReasoningItem[] }) {
  return <ThinkingDisclosure streaming={items.some(item => item.streaming)}>
    {items.map(item => <div key={item.id} data-conversation-entry-id={item.id}><Markdown text={item.text} streaming={item.streaming} /></div>)}
  </ThinkingDisclosure>
}

function ThinkingDisclosure({ children, streaming }: { readonly children: React.ReactNode; readonly streaming: boolean }) {
  const [open, setOpen] = React.useState(false)
  const contentId = React.useId()
  return <div data-testid="thinking-entry" data-streaming={streaming} {...stylex.props(styles.thinking)}>
    <AriaButton aria-expanded={open} aria-controls={contentId} onPress={() => setOpen(value => !value)} {...stylex.props(styles.thinkingControl)}>
      <Icon name={streaming ? 'spinner' : open ? 'chevron-down' : 'chevron-right'} spinning={streaming} /><span>Thinking</span>
    </AriaButton>
    {open ? <div id={contentId}>{children}</div> : null}
  </div>
}

const styles = stylex.create({
  thinking: { color: text.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  thinkingControl: { display: 'flex', alignItems: 'center', gap: s.sm, padding: 0, minHeight: g.toolRow, borderWidth: 0, backgroundColor: 'transparent', color: text.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: text.fgSoft }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
})
