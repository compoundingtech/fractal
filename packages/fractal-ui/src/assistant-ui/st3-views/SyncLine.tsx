// The host owns decoded facts and the clock.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, DialogTrigger, Popover, Tooltip, TooltipTrigger } from 'react-aria-components'
import { syncLine, type SyncLineInput, type SyncLineValue } from './sync-line'
import { interactionVars as interaction, borderVars as border, geometryVars as g, motionVars as m, radiusVars as r, spaceVars as s, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { useClockLabel } from '../../time/clock'

/** The host owns the clock and decodes its own transport into the portable tagged union. */
export interface SyncLineProps extends Omit<SyncLineInput, 'now'> { readonly now?: number; readonly onRetry?: () => void; readonly compact?: boolean; readonly pill?: boolean; /** Host-owned header: occupy only visible status/actions, not the standalone reserved slot. */ readonly inline?: boolean }
const pulse = stylex.keyframes({ from: { opacity: 0.5 }, to: { opacity: 1 } })
/** Standalone lines reserve their existing 24px slot; inline host headers collapse absent status. */
export const SyncLine = React.memo(function SyncLine({ status: observation, label, now, observedAt, gateway, socket, onRetry, compact = false, pill = false, inline = false }: SyncLineProps) {
  // A primitive snapshot lets unchanged live/hidden status lines skip clock renders.
  const snapshot = useClockLabel(now => JSON.stringify(syncLine({ status: observation, label, now, observedAt, gateway, socket }) ?? null), now)
  const line: SyncLineValue | undefined = JSON.parse(snapshot) ?? undefined
  const failed = observation._tag === 'Failed' && line !== undefined
  const retryable = failed && !(observation.cause._tag === 'Server' && (observation.cause.code === 'unsupported' || observation.cause.code === 'forbidden'))
  const fit = pill || inline
  const fullReason = inline && observation._tag === 'Failed'
    ? [line?.text, observation.cause._tag === 'Server' ? observation.cause.message : observation.cause._tag === 'Local' ? observation.cause.detail?.message : undefined].filter(Boolean).join(' · ')
    : line?.text
  return <div data-testid="sync-line" data-sync-state={observation._tag} data-sync-visible={line !== undefined} aria-busy={observation._tag !== 'Live' && observation._tag !== 'Failed'} {...stylex.props(styles.root, compact && styles.compact, fit && styles.fitContent, inline && styles.inline, pill && line !== undefined && styles.pill, inline && line === undefined && styles.noActions)}>
    <TooltipTrigger delay={150}><Button aria-label={fullReason ?? `${label} sync status not shown`} isDisabled={line === undefined} {...stylex.props(styles.textButton, pill && styles.fitText)}>
      <span role="status" aria-live="polite" aria-label={line?.announce ?? ''} {...stylex.props(styles.status, pill && styles.fitStatus)}><span aria-hidden="true" {...stylex.props(styles.dot, line?.tone === 'warning' && styles.warningDot, line?.tone === 'error' && styles.errorDot, line?.animate && styles.pulsing, line === undefined && styles.hidden)} /><span aria-hidden="true" data-testid={inline ? 'inline-sync-text' : undefined} {...stylex.props(styles.text, line?.tone === 'warning' && styles.warning, line?.tone === 'error' && styles.error, inline && failed && styles.fullInlineText)}>{line?.text ?? ''}</span>{inline && failed ? <span aria-hidden="true" {...stylex.props(styles.text, styles.shortInlineText)}>Sync failed</span> : null}</span>
    </Button>{line !== undefined && <Tooltip {...stylex.props(styles.tooltip)}>{fullReason}</Tooltip>}</TooltipTrigger>
    <div {...stylex.props(styles.actions, inline && styles.inlineActions, !failed && styles.hidden, fit && !failed && styles.noActions)}>
      <Button data-command-id="sync.retry" aria-label={`Retry loading ${label}`} isDisabled={!retryable || onRetry === undefined} onPress={onRetry} {...stylex.props(styles.button, inline && styles.inlineButton, !retryable && styles.hidden)}>Retry</Button>
      <span {...stylex.props(inline && styles.inlineDetails)}>
      <DialogTrigger><Button aria-label={`Sync details for ${label}`} isDisabled={!failed} {...stylex.props(styles.button)}>Details</Button><Popover {...stylex.props(styles.popover)}><Dialog aria-label={`Sync details for ${label}`} {...stylex.props(styles.dialog)}>{failed && <>{observation.cause._tag === 'Server' ? <><strong>{observation.cause.code}</strong><p {...stylex.props(styles.detail)}>{observation.cause.message}</p></> : observation.cause._tag === 'Local' ? <><strong>{observation.cause.kind}</strong>{observation.cause.detail?.cap !== undefined && <p {...stylex.props(styles.detail)}>Cap: {observation.cause.detail.cap}</p>}{observation.cause.detail?.message !== undefined && <p {...stylex.props(styles.detail)}>{observation.cause.detail.message}</p>}</> : <p {...stylex.props(styles.detail)}>Failure cause unknown.</p>}{gateway !== undefined && <p {...stylex.props(styles.detail)}>Gateway: {gateway}</p>}{socket !== undefined && <p {...stylex.props(styles.detail)}>Socket: {String(socket)}</p>}</>}</Dialog></Popover></DialogTrigger>
      </span>
    </div>
  </div>
})
const styles = stylex.create({
  root: { display: 'flex', alignItems: 'center', gap: s.xs, width: '100%', minWidth: 0, height: g.toolRow, minHeight: g.toolRow, maxHeight: g.toolRow, flex: '1 1 0', boxSizing: 'border-box', color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums' }, compact: { maxWidth: g.tooltipMax },
  fitContent: { width: 'max-content', maxWidth: '100%', flex: '0 1 auto' }, fitText: { flex: '0 1 auto' }, fitStatus: { width: 'auto' }, noActions: { display: 'none' },
  inline: { width: '100%', flex: '0 1 auto', minWidth: 0 },
  inlineActions: { width: 'auto' },
  inlineButton: { minWidth: 24, flexShrink: 0 },
  fullInlineText: { display: { default: 'block', '@container thread-header (max-width: 440px)': 'none' } },
  shortInlineText: { display: { default: 'none', '@container thread-header (max-width: 440px)': 'block' } },
  inlineDetails: { display: { default: 'contents', '@container thread-header (max-width: 440px)': 'none' } },
  pill: { paddingInline: s.md, borderRadius: r.full, backgroundColor: surface.controlFill },
  textButton: { display: 'flex', alignItems: 'center', flex: '1 1 0', minWidth: 0, height: g.toolRow, padding: s.zero, borderWidth: 0, backgroundColor: surface.transparent, color: 'inherit', fontFamily: 'inherit', fontSize: 'inherit', textAlign: 'left', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  status: { display: 'flex', alignItems: 'center', gap: s.sm, width: '100%', minWidth: 0, height: t.metaLeading }, text: { display: 'block', flex: '1 1 0', minWidth: 0, height: t.metaLeading, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }, dot: { width: g.statusDot, height: g.statusDot, flexShrink: 0, borderRadius: r.full, backgroundColor: ink.fgMuted }, warningDot: { backgroundColor: status.attention }, errorDot: { backgroundColor: status.dangerFg }, warning: { color: ink.fgSoft }, error: { color: status.dangerFg }, pulsing: { animationName: pulse, animationDuration: m.pulse, animationIterationCount: 'infinite', animationDirection: 'alternate', '@media (prefers-reduced-motion: reduce)': { animationName: 'none' } }, hidden: { visibility: 'hidden' },
  actions: { display: 'flex', alignItems: 'center', gap: s.xs, flexShrink: 0, width: `calc(${g.controlSm} * 4)` },
  button: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', height: g.toolRow, minHeight: g.toolRow, paddingInline: s.xs, paddingBlock: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  tooltip: { maxWidth: g.tooltipMax, padding: s.md, borderRadius: r.sm, backgroundColor: surface.raised, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong }, popover: { maxWidth: g.tooltipMax, backgroundColor: surface.raised, color: ink.fg, borderRadius: r.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, padding: s.md }, dialog: { outline: 'none', fontFamily: t.fontSans, fontSize: t.metaSize }, detail: { margin: 0, marginTop: s.md, overflowWrap: 'anywhere' },
})
