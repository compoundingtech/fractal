import * as React from 'react'
import type { ReactNode } from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { SidebarStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarStatus.tsx'
import type { AgentStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import { accentVars as accent, borderVars as border, geometryVars, radiusVars as r, spaceVars, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { readingColumnStyles } from '../../../../packages/fractal-ui/src/assistant-ui/reading-column.stylex.ts'
import { ObservedAge, useConversationLastObservedAt } from './lastObserved.tsx'
import type { LastSeenActivity } from './conversationTranscript.ts'
import type { OutboxRecoveryItem as RecoveryItem, OutboxRecoveryPort } from '../data/source.ts'

export { ThreadUnavailable, LastSeen, SeatAtRest, SendUnprotected, OutboxRecovery }
/**
 * The conversation read failed or was refused. Fixed copy only, inline in the reading lane: the reason (danger tone
 * only for a failure), its consequence on its own line, and Retry only where the source can re-acquire the follow.
 * Content that still reads without the conversation (the agent's threads) follows in the shared reading column.
 */
function ThreadUnavailable({ classification, reason, detail, onRetry, children }: { readonly classification: string; readonly reason: string; readonly detail: string | undefined; readonly onRetry: (() => void) | undefined; readonly children?: ReactNode }) {
  return <>
    <div data-thread-unavailable {...stylex.props(styles.lane, styles.notice)}>
      <section aria-label={reason} data-testid="transcript-unavailable" data-system-state="error" {...stylex.props(styles.unavailable)}>
        <p role="status" {...stylex.props(styles.unavailableText)}>
          <span {...stylex.props(styles.unavailableTitle, classification === 'failed' && styles.dangerTitle)}>{reason}</span>
          {detail === undefined ? null : <span {...stylex.props(styles.unavailableDetail)}>{detail}</span>}
        </p>
        {onRetry === undefined ? null : <Button aria-label={`Retry: ${reason}`} onPress={onRetry} {...stylex.props(styles.actionButton)}>Retry</Button>}
      </section>
    </div>
    {children == null ? null : <div {...stylex.props(readingColumnStyles.column)}>{children}</div>}
  </>
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

/**
 * The seat reports no turn in progress while its transcript still holds an open run: the seat's own observed state,
 * as its roster row names it, with no spinner and no elapsed counter.
 */
function SeatAtRest({ status, label }: { readonly status: AgentStatus; readonly label: string }) {
  return <div {...stylex.props(styles.lane)}>
    <p role="status" data-seat-at-rest={status} {...stylex.props(styles.lastSeen)}>
      <span aria-hidden="true" {...stylex.props(styles.glyph)}><SidebarStatus status={status} statusLabel={label} freshness="live" iconOnly /></span>
      <span>{label}</span>
    </p>
  </div>
}

/**
 * Sending works, but this browser keeps no durable copy of a message before it leaves: the daemon's verified
 * issuer identity, which scopes the on-device outbox, is unavailable. Text carries the meaning; the tone only marks it.
 */
function SendUnprotected() {
  return <div {...stylex.props(styles.lane)}>
    <p role="status" data-send-protection="unprotected" {...stylex.props(styles.protection)}>
      <span {...stylex.props(styles.protectionLabel)}>Unprotected:</span> messages are not saved on this device before sending.
    </p>
  </div>
}

/** One list answer, held against the port that gave it: a changed port reads as not yet listed. */
type RecoveryRead =
  | { readonly port: OutboxRecoveryPort; readonly _tag: 'Listed'; readonly items: readonly RecoveryItem[] }
  | { readonly port: OutboxRecoveryPort; readonly _tag: 'Unavailable' }

const RECOVERY_TITLE = 'Outbox on this device'
const RECOVERY_UNAVAILABLE = 'Outbox on this device is unavailable; retry later.'

/**
 * Messages this browser's outbox can no longer send. Each row says the outbox's own status verbatim and offers only
 * Discard; nothing here sends, copies or retries a message. A list that cannot be read says storage is unavailable
 * (never that it is damaged) with a Retry that lists again. Lists once per port and after each Discard; no polling.
 */
function OutboxRecovery({ port }: { readonly port: OutboxRecoveryPort }) {
  const [read, setRead] = React.useState<RecoveryRead>()
  const [attempt, setAttempt] = React.useState(0)
  const [discarding, setDiscarding] = React.useState<string>()
  const [discardFailed, setDiscardFailed] = React.useState<string>()
  // The port a Discard answer must still belong to; a later port or an unmount drops it.
  const current = React.useRef<OutboxRecoveryPort | undefined>(port)
  React.useEffect(() => {
    current.current = port
    let live = true
    port.list().then(
      (items) => { if (live) setRead({ port, _tag: 'Listed', items }) },
      () => { if (live) setRead({ port, _tag: 'Unavailable' }) },
    )
    return () => { live = false; current.current = undefined }
  }, [port, attempt])
  const relist = () => setAttempt((n) => n + 1)
  const discard = (id: string) => {
    setDiscarding(id)
    setDiscardFailed(undefined)
    port.discard(id).then(
      () => { if (current.current === port) { setDiscarding(undefined); relist() } },
      () => { if (current.current === port) { setDiscarding(undefined); setDiscardFailed(id); relist() } },
    )
  }
  const shown = read?.port === port ? read : undefined
  if (shown === undefined) return null
  if (shown._tag === 'Unavailable') return <div {...stylex.props(styles.lane)}>
    <section aria-label={RECOVERY_TITLE} data-outbox-recovery="unavailable" {...stylex.props(styles.recovery, styles.recoveryRow)}>
      <p role="status" {...stylex.props(styles.recoveryText)}>{RECOVERY_UNAVAILABLE}</p>
      <Button aria-label={`Retry: ${RECOVERY_UNAVAILABLE}`} onPress={relist} {...stylex.props(styles.actionButton)}>Retry</Button>
    </section>
  </div>
  if (shown.items.length === 0) return null
  return <div {...stylex.props(styles.lane)}>
    <section aria-label={RECOVERY_TITLE} data-outbox-recovery="list" {...stylex.props(styles.recovery)}>
      <p aria-hidden="true" {...stylex.props(styles.recoveryTitle)}>{RECOVERY_TITLE}</p>
      <ul {...stylex.props(styles.recoveryList)}>
        {shown.items.map((item) => <OutboxRecoveryRow key={item.id} item={item} discarding={discarding === item.id} failed={discardFailed === item.id} onDiscard={discard} />)}
      </ul>
    </section>
  </div>
}

function OutboxRecoveryRow({ item, discarding, failed, onDiscard }: { readonly item: RecoveryItem; readonly discarding: boolean; readonly failed: boolean; readonly onDiscard: (id: string) => void }) {
  const id = React.useId()
  return <li data-outbox-item={item.id} {...stylex.props(styles.recoveryRow)}>
    <span {...stylex.props(styles.recoveryText, styles.recoveryStatus)}>
      <span id={`${id}-status`} data-outbox-status>{item.status}</span>
      {failed ? <span role="status" {...stylex.props(styles.recoveryFailure)}>Discard did not finish: outbox unavailable; retry later.</span> : null}
    </span>
    {/* The outbox grants Discard alone; no other action is drawn for a recovery item. */}
    <Button id={`${id}-discard`} aria-labelledby={`${id}-discard ${id}-status`} isDisabled={discarding} onPress={() => onDiscard(item.id)} {...stylex.props(styles.actionButton)}>{discarding ? 'Discarding…' : 'Discard'}</Button>
  </li>
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
  protection: { margin: 0, paddingBlock: spaceVars.xs2, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  protectionLabel: { color: tone.attention, fontWeight: t.weightMedium },
  code: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', fontFamily: t.fontMono, fontSize: t.denseSize },
  recovery: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs2, minWidth: 0, margin: 0, paddingBlock: spaceVars.xs2, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  recoveryTitle: { margin: 0, color: ink.fgMuted, fontWeight: t.weightMedium },
  recoveryList: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs2, margin: 0, padding: 0, listStyle: 'none' },
  recoveryRow: { display: 'flex', flexDirection: 'row', alignItems: 'center', gap: spaceVars.md, minWidth: 0 },
  recoveryText: { flex: '1 1 0', minWidth: 0, margin: 0, color: ink.fgMuted, overflowWrap: 'anywhere' },
  recoveryStatus: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs2, color: ink.fgSoft },
  recoveryFailure: { color: tone.attention },
})
