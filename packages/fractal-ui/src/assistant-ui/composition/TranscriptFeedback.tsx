import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import type { SendState } from '../embrace-data/model'
import { Icon } from './Icons'
import { interactionVars as interaction, surfaceVars as surface, textVars as ink, statusVars as status, typeVars as t, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'

export interface TranscriptEmptyState { readonly title: string; readonly body?: string }
const sendFailureCopy: Readonly<Record<Extract<SendState, { _tag: 'Failed' }>['reason']['_tag'], string>> = {
  Rejected: 'Message was rejected',
  Ungranted: "You don't have permission to send here",
  Invalid: "Message couldn't be sent: it isn't valid",
  Failed: "Couldn't send",
  StaleFence: "Couldn't send: the conversation changed",
  SnapshotUnavailable: "Couldn't send: conversation not loaded yet. Nothing was sent.",
}

/** Failed send: danger reason line; host-supplied detail and native code open on disclosure. */
export function SendFailure({ state, onRetry }: { readonly state: Extract<SendState, { _tag: 'Failed' }>; readonly onRetry?: () => void }) {
  const [open, setOpen] = React.useState(false)
  const detailId = React.useId()
  // Source-controlled retryability wins; terminal invalid/expired/revoked rows never offer a blind Retry.
  const retryable = state.retryable ?? (state.reason._tag === 'Failed' || state.reason._tag === 'StaleFence' || state.reason._tag === 'SnapshotUnavailable')
  const hasDetail = state.detail !== undefined || state.code !== undefined
  return <div role="alert" data-testid="send-failure" data-send-failure-reason={state.reason._tag} data-send-failure-code={state.code} {...stylex.props(styles.sendFailure)}>
    <Button aria-expanded={open} aria-controls={hasDetail ? detailId : undefined} onPress={() => setOpen(value => !value)} {...stylex.props(styles.sendFailureLine)}>{sendFailureCopy[state.reason._tag]}{hasDetail && <Icon name={open ? 'chevron-down' : 'chevron-right'} size={12} />}</Button>
    {open && hasDetail ? <div id={detailId} {...stylex.props(styles.sendFailureDetail)}>{[state.detail, state.code === undefined ? undefined : `Code ${state.code}`].filter(Boolean).join(' · ')}</div> : null}
    {retryable && onRetry !== undefined && <Button onPress={onRetry} {...stylex.props(styles.sendFailureLine)}>Retry</Button>}
  </div>
}

/** Host-observed send lifecycle under a user prompt. Sent means accepted, never delivered or read. */
export function SendStatus({ state, onRetry }: { readonly state: SendState | undefined; readonly onRetry?: () => void }) {
  if (state === undefined) return null
  if (state._tag === 'Failed') return <SendFailure state={state} onRetry={onRetry} />
  if (state._tag === 'Unconfirmed') return <SendUnconfirmed state={state} onRetry={onRetry} />
  return <span role="status" data-testid="send-status" {...stylex.props(styles.sendStatus)}>{state._tag === 'Pending' ? 'Sending…' : 'Sent'}</span>
}

/** Unconfirmed send: a neutral status, not a failure; Fractal may already have it, so Send again repeats it. */
export function SendUnconfirmed({ state, onRetry }: { readonly state: Extract<SendState, { _tag: 'Unconfirmed' }>; readonly onRetry?: () => void }) {
  const [open, setOpen] = React.useState(false)
  const detailId = React.useId()
  return <div role="status" data-testid="send-unconfirmed" {...stylex.props(styles.sendFailure)}>
    <Button aria-expanded={open} aria-controls={state.detail === undefined ? undefined : detailId} onPress={() => setOpen(value => !value)} {...stylex.props(styles.sendFailureLine, styles.sendUnconfirmedLine)}>Not confirmed yet{state.detail !== undefined && <Icon name={open ? 'chevron-down' : 'chevron-right'} size={12} />}</Button>
    {open && state.detail !== undefined ? <div id={detailId} {...stylex.props(styles.sendFailureDetail)}>{state.detail}</div> : null}
    {onRetry !== undefined && <Button onPress={onRetry} {...stylex.props(styles.sendFailureLine, styles.sendUnconfirmedLine)}>Send again</Button>}
  </div>
}

/** Host-supplied empty copy; the default stays neutral for read-only and fresh conversations. */
export function TranscriptEmptyContent({ emptyState }: { readonly emptyState?: React.ReactNode | TranscriptEmptyState }) {
  if (typeof emptyState === 'object' && emptyState !== null && 'title' in emptyState) return <><p {...stylex.props(styles.emptyTitle)}>{emptyState.title}</p>{emptyState.body !== undefined && <p {...stylex.props(styles.emptyDetail)}>{emptyState.body}</p>}</>
  return emptyState ?? <p {...stylex.props(styles.emptyTitle)}>No messages yet</p>
}

const styles = stylex.create({
  emptyTitle: { margin: 0, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium, color: ink.fgSoft },
  emptyDetail: { margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  sendFailure: { display: 'flex', flexDirection: 'column', gap: s.xs2, marginTop: s.xs2 },
  sendFailureLine: { display: 'inline-flex', alignItems: 'center', gap: s.xs, alignSelf: 'flex-start', minHeight: g.controlSm, padding: 0, borderWidth: 0, backgroundColor: surface.transparent, color: status.dangerFg, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  sendUnconfirmedLine: { color: ink.fgMuted },
  sendFailureDetail: { color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  sendStatus: { display: 'block', marginTop: s.xs2, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
})
