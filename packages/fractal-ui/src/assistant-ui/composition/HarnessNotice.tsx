import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Disclosure, DisclosurePanel } from 'react-aria-components'
import type { EventItem, HarnessNotice as HarnessNoticeData } from '../embrace-data/model.ts'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'
import { Icon } from './Icons'
import { Markdown } from './Markdown'

/** Neutral presentation of typed harness facts; the app alone owns q101 transport recognition. */
export function HarnessNotice({ item }: { readonly item: EventItem & { readonly notice: HarnessNoticeData } }) {
  const notice = item.notice
  const compacted = notice._tag === 'Compaction'
  const time = item.at === undefined ? NaN : Date.parse(item.at)
  const action = compacted ? 'Read summary' : notice._tag === 'Irc' ? 'Read message' : 'Read result'
  const payload = /^[\s]*[\[{]/.test(notice.payload) ? `\`\`\`json\n${notice.payload}\n\`\`\`` : notice.payload
  return <Disclosure data-testid="harness-notice" data-harness-kind={item.kind} data-item-id={item.id} {...stylex.props(styles.card, compacted && styles.boundary)}>
    <div {...stylex.props(styles.header)}><Icon name={compacted ? 'panel' : notice._tag === 'Irc' ? 'message' : 'check'} size={14} /><span {...stylex.props(styles.label)}>{item.title}</span>{Number.isFinite(time) && <time dateTime={item.at} title={new Date(time).toLocaleString()} {...stylex.props(styles.time)}>{new Date(time).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}</time>}</div>
    <p data-testid="harness-excerpt" {...stylex.props(styles.excerpt)}>{item.text}</p>
    <Button slot="trigger" {...stylex.props(styles.trigger)}><Icon name="chevron-right" size={14} />{action}</Button>
    <DisclosurePanel {...stylex.props(styles.payload)}>
      <div data-testid={compacted ? 'compaction-summary' : 'harness-payload'}><Markdown text={payload} /></div>
      {notice._tag === 'SubagentResult' && <dl {...stylex.props(styles.facts)}><dt>Agent</dt><dd>{notice.agent}</dd>{notice.lines !== undefined && <><dt>Output</dt><dd>{notice.lines} lines · {notice.size}</dd></>}{notice.fullOutput !== undefined && <><dt>Full output</dt><dd>{notice.fullOutput}</dd></>}{notice.abortReason !== undefined && <><dt>Stop reason</dt><dd>{notice.abortReason}</dd></>}{notice.error !== undefined && <><dt>Error</dt><dd>{notice.error}</dd></>}{notice.mergeSummary !== undefined && <><dt>Merge</dt><dd>{notice.mergeSummary}</dd></>}</dl>}
      {notice._tag === 'Irc' && (notice.to !== undefined || notice.replyTo !== undefined) && <dl {...stylex.props(styles.facts)}>{notice.to !== undefined && <><dt>To</dt><dd>{notice.to}</dd></>}{notice.replyTo !== undefined && <><dt>Reply to</dt><dd>{notice.replyTo}</dd></>}</dl>}
    </DisclosurePanel>
  </Disclosure>
}

const styles = stylex.create({
  card: { minWidth: 0, padding: s.md, borderWidth: 1, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, backgroundColor: surface.raised, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  boundary: { borderInlineWidth: 0, borderRadius: 0, backgroundColor: 'transparent' },
  header: { display: 'flex', alignItems: 'center', gap: s.sm },
  label: { minWidth: 0, flex: 1, overflowWrap: 'anywhere', fontWeight: 500 },
  time: { flexShrink: 0, color: ink.fgMuted, fontVariantNumeric: 'tabular-nums' },
  excerpt: { marginBlock: s.xs, color: ink.fgMuted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  trigger: { display: 'flex', alignItems: 'center', gap: s.xs, padding: 0, minHeight: g.toolRow, borderWidth: 0, backgroundColor: 'transparent', color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  payload: { marginTop: s.sm, maxHeight: 'min(32rem, 60vh)', overflowY: 'auto', overflowWrap: 'anywhere' },
  facts: { display: 'grid', gridTemplateColumns: 'auto 1fr', gap: s.sm, color: ink.fgMuted, marginBottom: 0, marginInline: 0 },
})
