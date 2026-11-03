import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { SidebarStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarStatus.tsx'
import { accentVars as accent, borderVars as border, geometryVars, radiusVars as r, spaceVars, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { ObservedAge, useConversationLastObservedAt } from './lastObserved.tsx'
import type { LastSeenActivity } from './conversationTranscript.ts'

export { ThreadUnavailable, LastSeen }
/**
 * The conversation read failed or was refused. Fixed copy only, inline in the reading lane: the reason (danger tone
 * only for a failure), its consequence on its own line, and Retry only where the source can re-acquire the follow.
 */
function ThreadUnavailable({ classification, reason, detail, onRetry }: { readonly classification: string; readonly reason: string; readonly detail: string | undefined; readonly onRetry: (() => void) | undefined }) {
  return <div data-thread-unavailable {...stylex.props(styles.lane, styles.notice)}>
    <section aria-label={reason} data-testid="transcript-unavailable" data-system-state="error" {...stylex.props(styles.unavailable)}>
      <p role="status" {...stylex.props(styles.unavailableText)}>
        <span {...stylex.props(styles.unavailableTitle, classification === 'failed' && styles.dangerTitle)}>{reason}</span>
        {detail === undefined ? null : <span {...stylex.props(styles.unavailableDetail)}>{detail}</span>}
      </p>
      {onRetry === undefined ? null : <Button aria-label={`Retry: ${reason}`} onPress={onRetry} {...stylex.props(styles.actionButton)}>Retry</Button>}
    </section>
  </div>
}

/**
 * Last-known activity at the stream tail: labelled "Last seen", muted (never red), no spinner. The age counts from the
 * conversation's last actual observation on the shared clock; without one, no age is shown.
 */
function LastSeen({ agentRef, lastSeen }: { readonly agentRef: string; readonly lastSeen: LastSeenActivity }) {
  const observedAt = useConversationLastObservedAt(agentRef)
  const sentence = `Last seen ${lastSeen.activity}`
  return <div {...stylex.props(styles.lane)}>
    <p role="status" data-last-seen={lastSeen.activity} {...stylex.props(styles.lastSeen)}>
      {/* The sentence says "Last seen"; the glyph is decorative so the live region reads only the sentence. */}
      <span aria-hidden="true" {...stylex.props(styles.glyph)}><SidebarStatus status="stale" statusLabel="Last seen" freshness="stale" iconOnly /></span>
      <span>{observedAt === undefined ? sentence : <ObservedAge prefix={`${sentence} · `} at={observedAt} />}</span>
      {lastSeen.detail === undefined ? null : <><span aria-hidden="true">·</span><code {...stylex.props(styles.code)}>{lastSeen.detail}</code></>}
    </p>
  </div>
}

const focusRing = { outlineWidth: geometryVars.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '-2px' } as const
const styles = stylex.create({
  lane: { width: '100%', maxWidth: geometryVars.lane, boxSizing: 'border-box', paddingInline: spaceVars.lg, marginInline: 'auto', flexShrink: 0 },
  notice: { paddingBlock: spaceVars.lg },
  unavailable: { display: 'flex', alignItems: 'center', gap: spaceVars.md, minWidth: 0 },
  unavailableText: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs2, flex: '1 1 0', minWidth: 0, margin: 0, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  unavailableTitle: { color: ink.fg, fontWeight: t.weightMedium },
  dangerTitle: { color: tone.dangerFg },
  unavailableDetail: { color: ink.fgSoft, overflowWrap: 'anywhere' },
  actionButton: { flexShrink: 0, height: geometryVars.controlSm, paddingInline: spaceVars.md, borderRadius: r.sm, borderWidth: geometryVars.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.denseSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  lastSeen: { display: 'flex', alignItems: 'center', gap: spaceVars.sm, minWidth: 0, height: geometryVars.toolRow, margin: 0, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', overflow: 'hidden' },
  glyph: { display: 'inline-flex', flexShrink: 0 },
  code: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', fontFamily: t.fontMono, fontSize: t.denseSize },
})
