import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tooltip, TooltipTrigger } from 'react-aria-components'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, statusVars as status, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g, motionVars as m } from '../composition-tokens.stylex'
import { Icon } from '../composition/Icons'
import { ErrorOverlay } from '../composition/ErrorOverlay'
import { HighlightedSource } from '../composition/Markdown'
import type { WorkLogCall, WorkLogTurn } from './work-log'
import { errorReason, failedCommandSource, formatWorkDuration, readableCommand, workLogOutputLanguage } from './work-log'
import { OutgoingMessageRow, type OutgoingMessageRowProps } from '../AgentMessage'

export type WorkLogCallDetailRenderer = (call: WorkLogCall) => React.ReactNode
const CallDetailRenderer = React.createContext<WorkLogCallDetailRenderer | undefined>(undefined)
const CallPresentation = React.createContext({ preview: false, interactive: true })
const timeFormat = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' })
/** Host-owned facts and actions; incomplete, running and failed work never folds. */
export function WorkLogV1({ turn, outgoingMessages, listStyle, renderCallDetail, previewCallDetail = false, interactiveCalls = true, hideLiveRow = false, expandedBody, ariaLabel = 'Work log', onRetry, onOpenOutput }: {
  readonly turn: WorkLogTurn
  /** Host-normalized semantic sends replace their underlying tool call, keyed by call id. */
  readonly outgoingMessages?: Readonly<Record<string, OutgoingMessageRowProps>>
  readonly listStyle?: stylex.StyleXStyles
  readonly renderCallDetail?: WorkLogCallDetailRenderer
  readonly previewCallDetail?: boolean
  readonly interactiveCalls?: boolean
  readonly hideLiveRow?: boolean
  readonly expandedBody?: React.ReactNode
  readonly ariaLabel?: string
  readonly onRetry?: () => void
  readonly onOpenOutput?: (call: WorkLogCall) => void
}) {
  const settled = !turn.running && !turn.failed && !turn.interrupted && turn.foldable !== false
  const durationLabel = formatWorkDuration(turn.durationMs)
  const [open, setOpen] = React.useState(false)
  const failedCall = turn.calls.find(call => call.status === 'error')
  const failed = turn.failed || failedCall !== undefined
  const workLabel = turn.running ? 'Working' : settled ? `Worked${durationLabel ? ` for ${durationLabel}` : ''}` : failed ? 'Run failed' : turn.interrupted ? 'Run interrupted' : 'Work · completion unverified'
  const messagesSent = turn.calls.filter(call => outgoingMessages?.[call.id] !== undefined).length
  const commands = turn.commands === undefined ? undefined : Math.max(0, turn.commands - turn.calls.filter(call => call.countedAsCommand === true && outgoingMessages?.[call.id] !== undefined).length)
  const summary = [workLabel, settled && commands ? `${commands} ${commands === 1 ? 'command' : 'commands'}` : undefined, settled && messagesSent ? `${messagesSent} ${messagesSent === 1 ? 'message sent' : 'messages sent'}` : undefined, settled && turn.changedFiles ? `${turn.changedFiles} ${turn.changedFiles === 1 ? 'file changed' : 'files changed'}` : undefined].filter(value => value !== undefined).join(' · ')
  const started = turn.running ? NaN : Date.parse(turn.startedAt ?? '')
  const failedCommand = failedCall?.kind === 'run' ? failedCall.argsSummary ?? failedCall.title : undefined
  const detail = [failedCommand, failedCall?.detail, turn.failureNote].filter((value, index, values) => value !== undefined && values.indexOf(value) === index).join('\n') || undefined
  const summaryContent = <><span {...stylex.props(styles.summaryLabel)}>{summary}</span>{Number.isFinite(started) && <time dateTime={new Date(started).toISOString()} {...stylex.props(styles.time)}>{timeFormat.format(started)}</time>}</>
  return <CallDetailRenderer.Provider value={renderCallDetail}><CallPresentation.Provider value={{ preview: previewCallDetail, interactive: interactiveCalls }}><section aria-label={ariaLabel} data-testid="work-log">
    {turn.running && !hideLiveRow && <div data-testid="live-work" role="status" aria-label="Response in progress" {...stylex.props(styles.live)}><Icon name="spinner" spinning /><span>Working</span></div>}
    {interactiveCalls ? <Button isDisabled={!settled} aria-expanded={!settled || open} onPress={() => setOpen(value => !value)} {...stylex.props(styles.summary, failed && styles.failureInk)}><span aria-hidden="true" {...stylex.props(styles.slot)}><span {...stylex.props(styles.chevron, (!settled || open) && styles.chevronOpen)}><Icon name="chevron-right" size={12} /></span></span>{summaryContent}</Button> : <div {...stylex.props(styles.summary, styles.staticCall, failed && styles.failureInk)}><span aria-hidden="true" {...stylex.props(styles.slot)} />{summaryContent}</div>}
    {(!interactiveCalls || !settled || open) && <div data-testid="work-log-children" {...stylex.props(styles.children)}><div {...stylex.props(styles.list, listStyle)}>{turn.calls.map(call => {
      const message = outgoingMessages?.[call.id]
      return message === undefined ? <TimelineCall key={call.id} call={call} /> : <div key={call.id} data-testid="work-log-outgoing-message" data-item-id={call.id}><OutgoingMessageRow {...message} /></div>
    })}{expandedBody}</div></div>}
    {failed && <ErrorOverlay id={`work-failure-${turn.calls[0]?.id ?? 'turn'}`} title={failedCommand === undefined ? 'Run failed.' : 'Command did not complete'} detail={errorReason(failedCall?.detail ?? turn.failureNote ?? '')} rawDetail={detail} command={failedCall === undefined ? failedCommandSource(turn.failureNote ?? '') : readableCommand(failedCall)} onRetry={onRetry} onOpenOutput={failedCall === undefined || onOpenOutput === undefined ? undefined : () => onOpenOutput(failedCall)} />}
    {turn.interrupted && <div {...stylex.props(styles.promoted)}><Icon name="stop" size={14} /><span>Run interrupted.</span></div>}
  </section></CallPresentation.Provider></CallDetailRenderer.Provider>
}
function TimelineCall({ call }: { call: WorkLogCall }) {
  const [open, setOpen] = React.useState(false)
  const renderDetail = React.useContext(CallDetailRenderer)
  const { preview, interactive } = React.useContext(CallPresentation)
  const hasOutput = call.detail !== undefined && call.detail.length > 0
  const displayed = hasOutput && (!interactive || open || preview && call.status !== 'error')
  const result = call.detail ?? (call.status === 'running' ? 'Running…' : 'No output')
  const label = call.argsSummary === undefined ? call.title : `${call.title} ${call.argsSummary}`
  const start = Date.parse(call.startedAt), end = Date.parse(call.endedAt ?? '')
  const duration = Number.isFinite(start) && Number.isFinite(end) && end >= start ? formatWorkDuration(end - start) || '<1s' : undefined
  // A non-success outcome stays visible as a word beside the pinned disclosure; running rows say so in the duration column until they end.
  const word = call.status === 'error' ? 'Failed' : call.status === 'interrupted' ? 'Interrupted' : undefined
  const row = <><span aria-hidden="true" {...stylex.props(styles.slot)}><Icon name={call.status === 'running' ? 'spinner' : call.status === 'error' ? 'alert' : call.status === 'interrupted' ? 'stop-circle' : call.kind === 'read' ? 'search' : call.kind === 'run' ? 'drawer' : 'pencil'} spinning={call.status === 'running'} size={14} /></span><span data-call-title {...stylex.props(styles.callLabel)}>{call.kind === 'run' ? <code {...stylex.props(styles.mono)}><HighlightedSource code={label} language="bash" /></code> : <>{call.title}{call.argsSummary === undefined ? null : <>{' '}<code {...stylex.props(styles.mono)}>{call.argsSummary}</code></>}</>}</span><span data-call-word {...stylex.props(styles.callWord)}>{word}</span><span aria-hidden="true" data-row-disclosure={hasOutput && interactive ? displayed ? 'open' : 'closed' : undefined} {...stylex.props(styles.slot, styles.disclosure)}>{hasOutput && interactive && <span {...stylex.props(styles.chevron, displayed && styles.chevronOpen)}><Icon name="chevron-right" size={12} /></span>}</span><span data-call-duration {...stylex.props(styles.duration)}>{duration ?? (call.status === 'running' ? 'Running' : '')}</span></>
  return <div data-tool-status={call.status}>{hasOutput && interactive ? <TooltipTrigger delay={150}><Button aria-expanded={displayed} onPress={() => setOpen(value => !value)} {...stylex.props(stylex.defaultMarker(), styles.call, open && styles.expanded, call.status === 'error' && styles.failureInk)}>{row}</Button><Tooltip {...stylex.props(styles.hover)}><div>{label}</div><div>{result}</div></Tooltip></TooltipTrigger> : <div {...stylex.props(styles.call, styles.staticCall, call.status === 'error' && styles.failureInk)}>{row}</div>}{displayed && (renderDetail === undefined ? <pre data-testid="work-call-output" {...stylex.props(styles.output)}><code><HighlightedSource code={result} language={workLogOutputLanguage(call)} /></code></pre> : renderDetail(call))}</div>
}
const styles = stylex.create({
  summary: { display: 'flex', alignItems: 'center', gap: s.sm, width: '100%', minHeight: g.toolRow, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.uiSize, padding: 0, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  summaryLabel: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  slot: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, flexShrink: 0 },
  chevron: { display: 'inline-flex', transitionProperty: 'transform', transitionDuration: m.fast, transitionTimingFunction: m.ease, '@media (prefers-reduced-motion: reduce)': { transitionDuration: '0s' } },
  chevronOpen: { transform: 'rotate(90deg)' },
  // Children sit one text column in; the rail runs on the header chevron's centre and stops short of the last child.
  children: { position: 'relative', paddingInlineStart: g.textColumn, '::before': { content: '""', position: 'absolute', insetInlineStart: g.railInset, top: 0, bottom: s.xs, width: g.hairline, backgroundColor: border.rail } },
  // Tool rows and Thinking share this one scroller, so a classic scrollbar narrows every child row alike.
  list: { maxHeight: g.workExpandedMax, overflowY: 'auto', overscrollBehavior: 'contain' },
  call: { display: 'grid', gridTemplateColumns: g.workRowColumns, alignItems: 'center', gap: s.sm, width: '100%', minHeight: g.toolRow, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.uiSize, textAlign: 'left', cursor: 'pointer', transitionProperty: 'background-color, color', transitionDuration: m.fast, transitionTimingFunction: m.ease, '@media (prefers-reduced-motion: reduce)': { transitionDuration: '0s' }, ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  callLabel: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', lineHeight: t.uiLeading },
  callWord: { maxWidth: g.statusWordMax, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, lineHeight: t.metaLeading },
  duration: { overflow: 'hidden', whiteSpace: 'nowrap', fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', textAlign: 'right' },
  disclosure: { opacity: { default: 0.5, [stylex.when.ancestor(':hover')]: 1, [stylex.when.ancestor(':focus-visible')]: 1 }, transitionProperty: 'opacity', transitionDuration: m.fast, transitionTimingFunction: m.ease, '@media (prefers-reduced-motion: reduce)': { transitionDuration: '0s' } },
  expanded: { backgroundColor: surface.rowActive, color: ink.fg }, staticCall: { cursor: 'default' },
  mono: { fontFamily: t.fontMono, fontSize: t.metaSize },
  output: { marginBlockStart: s.xs2, marginBlockEnd: 0, marginInlineStart: g.textColumn, paddingInline: s.lg, paddingBlock: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, color: ink.fgMuted, fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  time: { flexShrink: 0, color: ink.fgMuted, fontSize: t.metaSize, marginLeft: 'auto' },
  live: { minHeight: g.toolRow, display: 'flex', alignItems: 'center', gap: s.md, color: ink.fgMuted, fontSize: t.metaSize },
  promoted: { display: 'flex', alignItems: 'center', gap: s.md, marginTop: s.md, padding: s.md, backgroundColor: surface.washSubtle, borderRadius: r.md, color: ink.fgSoft, fontSize: t.metaSize, lineHeight: t.uiLeading },
  failureInk: { color: status.dangerFg },
  hover: { maxWidth: g.tooltipMax, padding: s.md, borderRadius: r.md, backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.uiLeading, zIndex: 10 },
})
