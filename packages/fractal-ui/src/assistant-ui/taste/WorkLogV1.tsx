import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tooltip, TooltipTrigger } from 'react-aria-components'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, statusVars as status, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'
import { Icon } from '../composition/Icons'
import { ErrorOverlay } from '../composition/ErrorOverlay'
import { HighlightedSource } from '../composition/Markdown'
import type { WorkLogCall, WorkLogTurn } from './work-log'
import { errorReason, failedCommandSource, formatWorkDuration, readableCommand, renderedToolOutput, workLogOutputLanguage } from './work-log'
import { workKindPresentation, workServicePresentation } from './work-row-presentation'

export type WorkLogCallDetailRenderer = (call: WorkLogCall) => React.ReactNode
const CallDetailRenderer = React.createContext<WorkLogCallDetailRenderer | undefined>(undefined)
const CallPresentation = React.createContext({ interactive: true })
const timeFormat = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' })
/** Host-owned facts and actions; incomplete, running and failed work never folds. */
export function WorkLogV1({ turn, listStyle, renderCallDetail, interactiveCalls = true, hideLiveRow = false, expandedBody, ariaLabel = 'Work log', onRetry, onOpenOutput }: {
  readonly turn: WorkLogTurn
  readonly listStyle?: stylex.StyleXStyles
  readonly renderCallDetail?: WorkLogCallDetailRenderer
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
  const [showSystemEvents, setShowSystemEvents] = React.useState(false)
  const systemCalls = turn.calls.filter(call => call.kind === 'unknown')
  const failedCall = turn.calls.find(call => call.status === 'error')
  const failed = turn.failed && !turn.interrupted
  const workLabel = turn.running ? 'Working' : settled ? `Worked${durationLabel ? ` for ${durationLabel}` : ''}` : failed ? 'Run failed' : turn.interrupted ? 'Run interrupted' : 'Work · completion unverified'
  const summary = [workLabel, settled && turn.commands ? `${turn.commands} ${turn.commands === 1 ? 'command' : 'commands'}` : undefined, settled && turn.messagesSent ? `${turn.messagesSent} ${turn.messagesSent === 1 ? 'message sent' : 'messages sent'}` : undefined, settled && turn.changedFiles ? `${turn.changedFiles} ${turn.changedFiles === 1 ? 'file changed' : 'files changed'}` : undefined].filter(value => value !== undefined).join(' · ')
  const started = turn.running ? NaN : Date.parse(turn.startedAt ?? '')
  const failedCommand = failedCall?.kind === 'run' ? failedCall.argsSummary ?? failedCall.title : undefined
  const detail = [failedCommand, failedCall?.detail, turn.failureNote].filter((value, index, values) => value !== undefined && values.indexOf(value) === index).join('\n') || undefined
  const presentation = React.useMemo(() => ({ interactive: interactiveCalls }), [interactiveCalls])
  const summaryContent = <><span {...stylex.props(styles.summaryLabel)}>{summary}</span>{Number.isFinite(started) && <time dateTime={new Date(started).toISOString()} {...stylex.props(styles.time)}>{timeFormat.format(started)}</time>}</>
  return <CallDetailRenderer.Provider value={renderCallDetail}><CallPresentation.Provider value={presentation}><section aria-label={ariaLabel} data-testid="work-log">
    {turn.running && !hideLiveRow && <div data-testid="live-work" role="status" aria-label="Response in progress" {...stylex.props(styles.live)}><Icon name="spinner" spinning /><span>Working</span></div>}
    {interactiveCalls ? <Button data-work-log-summary isDisabled={!settled} aria-expanded={!settled || open} onPress={() => setOpen(value => !value)} {...stylex.props(styles.summary)}><Icon name={!settled || open ? 'chevron-down' : 'chevron-right'} size={12} />{summaryContent}</Button> : <div data-work-log-summary {...stylex.props(styles.summary, styles.staticCall)}>{summaryContent}</div>}
    {(!interactiveCalls || !settled || open) && <><div {...stylex.props(styles.list, listStyle)}>{turn.calls.filter(call => call.kind !== 'unknown').map(call => <TimelineCall key={call.id} call={call} />)}{systemCalls.length > 0 && <><Button aria-expanded={showSystemEvents} onPress={() => setShowSystemEvents(value => !value)} {...stylex.props(styles.call)}><Icon name={showSystemEvents ? 'chevron-down' : 'chevron-right'} size={12} />{showSystemEvents ? 'Hide system events' : 'Show system events'} · {systemCalls.length}</Button>{showSystemEvents && systemCalls.map(call => <TimelineCall key={call.id} call={call} />)}</>}</div>{expandedBody}<hr data-testid="work-log-divider" {...stylex.props(styles.divider)} /></>}
    {failed && <ErrorOverlay id={`work-failure-${turn.calls[0]?.id ?? 'turn'}`} title={failedCommand === undefined ? 'Run failed.' : 'Command did not complete'} detail={errorReason(failedCall?.detail ?? turn.failureNote ?? '')} rawDetail={detail} command={failedCall === undefined ? failedCommandSource(turn.failureNote ?? '') : readableCommand(failedCall)} onRetry={onRetry} onOpenOutput={failedCall === undefined || onOpenOutput === undefined ? undefined : () => onOpenOutput(failedCall)} />}
    {turn.interrupted && <div {...stylex.props(styles.promoted)}><Icon name="stop" size={14} /><span>Run interrupted.</span></div>}
  </section></CallPresentation.Provider></CallDetailRenderer.Provider>
}
const TimelineCall = React.memo(function TimelineCall({ call }: { call: WorkLogCall }) {
  const [open, setOpen] = React.useState(false)
  const renderDetail = React.useContext(CallDetailRenderer)
  const { interactive } = React.useContext(CallPresentation)
  // Expandability follows rendered text: empty and image-only native results have nothing to read.
  const hasOutput = renderedToolOutput(call) !== undefined
  // Input-only rows still open an intent summary and the explicit raw disclosure.
  const canInspect = hasOutput || call.rawInput !== undefined || renderDetail !== undefined && (call.input !== undefined || call.command !== undefined)
  const displayed = canInspect && open
  const presentation = workKindPresentation[call.kind]
  const service = workServicePresentation(call)
  const verb = service === undefined ? presentation.verb : 'Service'
  const args = service?.name ?? call.argsSummary
  const secondary = <>{verb}{args === undefined ? null : <>{' '}<code {...stylex.props(styles.mono)}>{args}</code></>}</>
  const label = [call.intent, verb, args].filter(value => value !== undefined).join(' · ')
  const start = Date.parse(call.startedAt), end = Date.parse(call.endedAt ?? '')
  const duration = Number.isFinite(start) && Number.isFinite(end) && end >= start ? formatWorkDuration(end - start) || '<1s' : undefined
  const state = call.skipped ? 'Skipped' : call.status === 'running' ? 'Running' : call.status === 'error' ? 'Failed' : call.status === 'interrupted' ? 'Interrupted' : 'OK'
  const row = <><span data-tool-kind={service === undefined ? call.kind === 'unknown' ? 'system' : call.kind : 'service'} data-kind-icon={service === undefined ? presentation.icon : 'gear'}><Icon name={service === undefined ? presentation.icon : 'gear'} size={16} /></span><span {...stylex.props(styles.callLabel)}>{call.intent === undefined ? <span data-tool-title>{secondary}</span> : <><span data-tool-title data-tool-intent>{call.intent}</span><span data-tool-secondary {...stylex.props(styles.secondary)}>{' · '}{secondary}</span></>}</span>{canInspect && interactive && <span data-row-disclosure={displayed ? 'open' : 'closed'} {...stylex.props(styles.disclosure)}><Icon name={displayed ? 'chevron-down' : 'chevron-right'} size={12} /></span>}<span data-tool-state {...stylex.props(styles.callState)}>{call.status === 'running' && <Icon name="spinner" spinning size={12} />}{call.status === 'error' && !call.skipped && <span data-tool-error-mark {...stylex.props(styles.errorMark)}><Icon name="x" size={12} /></span>}{service === undefined ? state : <>{service.state}<span data-service-readiness={service.readiness} data-service-tone={service.warning ? 'warning' : 'neutral'} {...stylex.props(styles.serviceChip, service.warning && styles.serviceWarning)}>{service.readiness}</span></>}{duration === undefined ? '' : ` · ${duration}`}</span></>
  return <div data-item-id={call.id} data-tool-status={call.status}>{canInspect && interactive ? <TooltipTrigger delay={150}><Button aria-description="Show raw input and output" aria-expanded={displayed} onPress={() => setOpen(value => !value)} {...stylex.props(styles.call, open && styles.expanded)}>{row}</Button><Tooltip {...stylex.props(styles.hover)}><div>{label}</div></Tooltip></TooltipTrigger> : <div {...stylex.props(styles.call, styles.staticCall)}>{row}</div>}{displayed && (renderDetail === undefined ? <DefaultCallDetail call={call} /> : renderDetail(call))}</div>
})
/** The row disclosure is the sole route to exact input and full output. */
function DefaultCallDetail({ call }: { readonly call: WorkLogCall }) {
  const output = renderedToolOutput(call)
  return <>
    <p>{call.summary ?? call.title} · {call.status}</p>
    {call.status === 'error' && call.detail !== undefined && <p>{errorReason(call.detail)}</p>}
    {call.rawInput !== undefined && <pre data-testid="tool-raw-input" {...stylex.props(styles.output)}><code>{call.rawInput}</code></pre>}
    {output !== undefined && <pre data-testid="work-call-output" {...stylex.props(styles.output)}><code><HighlightedSource code={output} language={workLogOutputLanguage(call)} /></code></pre>}
  </>
}
const styles = stylex.create({
  summary: { display: 'flex', alignItems: 'center', gap: s.sm, width: '100%', minHeight: g.toolRow, borderWidth: 0, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.uiSize, padding: 0, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  summaryLabel: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  list: { maxHeight: g.workExpandedMax, overflowY: 'auto', overscrollBehavior: 'contain' },
  call: { display: 'flex', alignItems: 'center', gap: s.sm, width: '100%', minHeight: g.toolRow, paddingInline: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.uiSize, textAlign: 'left', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  callLabel: { flex: '1 1 auto', minWidth: 0, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }, callState: { display: 'inline-flex', alignItems: 'center', gap: s.xs, marginLeft: 'auto', flexShrink: 0, color: 'inherit', fontSize: t.metaSize },
  expanded: { backgroundColor: surface.rowActive, color: ink.fg }, staticCall: { cursor: 'default' }, disclosure: { display: 'inline-flex', flexShrink: 0 },
  mono: { fontFamily: t.fontMono, fontSize: t.metaSize },
  output: { marginBlock: 0, paddingInline: s.xxl, paddingBlock: s.xs, color: ink.fgMuted, fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  divider: { height: 0, margin: 0, borderWidth: 0, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  time: { flexShrink: 0, color: ink.fgMuted, fontSize: t.metaSize, marginLeft: 'auto' },
  live: { minHeight: g.toolRow, display: 'flex', alignItems: 'center', gap: s.md, color: ink.fgMuted, fontSize: t.metaSize },
  promoted: { display: 'flex', alignItems: 'center', gap: s.md, marginTop: s.md, padding: s.md, backgroundColor: surface.washSubtle, borderRadius: r.md, color: ink.fgSoft, fontSize: t.metaSize, lineHeight: t.uiLeading },
  errorMark: { display: 'inline-flex', color: status.dangerFg, opacity: 0.65 },
  serviceChip: { display: 'inline-flex', paddingInline: s.xs, borderRadius: r.sm, backgroundColor: surface.washSubtle, color: ink.fgMuted },
  secondary: { fontSize: t.metaSize, color: ink.fgMuted },
  serviceWarning: { color: status.attention },
  hover: { maxWidth: g.tooltipMax, padding: s.md, borderRadius: r.md, backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.uiLeading, zIndex: 10 },
})
