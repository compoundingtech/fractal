import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { surfaceVars, textVars, borderVars, accentVars, statusVars, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g, motionVars as m } from './composition-tokens.stylex'
import { Icon } from './composition/Icons'

/** One request waiting on the person. The host decides what is still waiting and owns any answer form. */
export interface NeedsYouRequest {
  readonly id: string
  readonly agentLabel: string
  readonly prompt: string
}

/** Pinned attention dock; hides once the host reports the in-thread request at least 60% visible. */
export const NeedsYouBar = ({ requests, ariaLabel = 'Needs you', expanded, onToggle, transcriptVisibility = 0, children }: { readonly requests: readonly NeedsYouRequest[]; readonly ariaLabel?: string; readonly expanded: boolean; readonly onToggle: () => void; readonly transcriptVisibility?: number; readonly children?: React.ReactNode }) => {
  const first = requests[0]
  if (first === undefined || transcriptVisibility >= 0.6) return null
  return <aside data-fractal-card="" {...stylex.props(styles.card)} aria-label={ariaLabel}><Button {...stylex.props(styles.button, styles.dockButton)} aria-expanded={expanded} onPress={onToggle}><span aria-hidden="true" {...stylex.props(styles.glyph)}><Icon name="question" scale="status" /></span><span {...stylex.props(styles.dockLabel)}>{first.agentLabel} is waiting on you · {first.prompt}{requests.length > 1 ? ` · +${requests.length - 1}` : ''}</span><span {...stylex.props(styles.dockAction)}>{expanded ? 'Hide' : 'Answer'}</span></Button>{expanded ? children : null}</aside>
}

/** Two-line disclosures per waiting request; `children` renders the host-owned body for an open entry. */
export const NeedsYouInbox = ({ requests, ariaLabel = 'Needs you inbox', onShowInThread, onOpenTerminal, children }: { readonly requests: readonly NeedsYouRequest[]; readonly ariaLabel?: string; readonly onShowInThread: (id: string) => void; readonly onOpenTerminal?: (id: string) => void; readonly children?: (id: string) => React.ReactNode }) => <section aria-label={ariaLabel} {...stylex.props(styles.body)}>{requests.map(request => <details data-fractal-card="" key={request.id} {...stylex.props(styles.card)}>
  <summary {...stylex.props(styles.inboxSummary)}><span aria-hidden="true" data-needs-you-chevron {...stylex.props(styles.chevron)}><Icon name="chevron-right" size={12} /></span><span data-needs-you-label {...stylex.props(styles.inboxLabel)}>{request.agentLabel} · {request.prompt}</span></summary>
  <div {...stylex.props(styles.body)}>
    {children?.(request.id)}
    <footer {...stylex.props(styles.actions)}><Button {...stylex.props(styles.button)} onPress={() => onShowInThread(request.id)}>Show in thread</Button>{onOpenTerminal !== undefined ? <Button {...stylex.props(styles.button)} onPress={() => onOpenTerminal(request.id)}>Open terminal</Button> : null}</footer>
  </div>
</details>)}</section>

const styles = stylex.create({
  card: { display: 'flex', flexDirection: 'column', gap: s.md, paddingBlock: s.md, paddingInline: s.lg, maxWidth: g.lane, width: '100%', boxSizing: 'border-box', borderWidth: g.hairline, borderStyle: 'solid', borderColor: statusVars.attention, borderRadius: r.md, backgroundColor: surfaceVars.raised, color: textVars.fg, minWidth: 0, overflowWrap: 'anywhere', fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  inboxSummary: { cursor: 'pointer', display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.controlSm, listStyle: 'none', '::-webkit-details-marker': { display: 'none' }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accentVars.primary, outlineOffset: g.focusOffset } },
  inboxLabel: { minWidth: 0, overflow: 'hidden', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical' },
  chevron: { width: g.icon, height: g.icon, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, color: textVars.fgMuted, transform: { default: 'rotate(0deg)', ':is(details[open] *)': 'rotate(90deg)' }, transitionProperty: 'transform', transitionDuration: { default: m.fast, '@media (prefers-reduced-motion: reduce)': '0s' }, transitionTimingFunction: m.ease },
  body: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0 },
  actions: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md },
  glyph: { width: g.icon, height: g.icon, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, color: statusVars.attention },
  button: { paddingBlock: s.xs, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.sm, backgroundColor: surfaceVars.rowActive, color: textVars.fg, cursor: 'pointer', outlineStyle: 'none', outlineOffset: g.focusOffset, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accentVars.primary }, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  dockButton: { display: 'flex', alignItems: 'center', gap: s.md, height: g.controlSm, minHeight: g.controlSm, maxWidth: '100%', boxSizing: 'border-box', paddingBlock: 0, fontWeight: t.weightMedium },
  dockLabel: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  dockAction: { flex: '0 0 auto' },
})
