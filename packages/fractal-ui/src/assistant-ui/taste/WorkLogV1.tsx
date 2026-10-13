import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Disclosure, DisclosurePanel, Tooltip, TooltipTrigger } from 'react-aria-components'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, statusVars as status, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g, motionVars as m } from '../composition-tokens.stylex'
import { Icon } from '../composition/Icons'
import { ErrorOverlay } from '../composition/ErrorOverlay'
import { HighlightedSource } from '../composition/Markdown'
import type { WorkLogCall, WorkLogTurn } from './work-log'
import { errorReason, failedCommandSource, formatWorkDuration, readableCommand, workLogOutputLanguage, workLogOutputText } from './work-log'
import { workKindPresentation, workServicePresentation } from './work-row-presentation.ts'

export type WorkLogCallDetailRenderer = (call: WorkLogCall) => React.ReactNode
const CallDetailRenderer = React.createContext<WorkLogCallDetailRenderer | undefined>(undefined)
const CallPresentation = React.createContext({ preview: false, interactive: true })
const timeFormat = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' })
/** Host-owned facts and actions; incomplete, running and failed work never folds. */
export function WorkLogV1({ turn, listStyle, renderCallDetail, previewCallDetail = false, interactiveCalls = true, hideLiveRow = false, expandedBody, ariaLabel = 'Work log', onRetry, onOpenOutput }: {
  readonly turn: WorkLogTurn
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
  const [showSystemEvents, setShowSystemEvents] = React.useState(false)
  // Unclassified results (orphans) stay out of the default rows; one disclosure reveals them.
  const systemCalls = turn.calls.filter(call => call.kind === 'unknown')
  // A skipped call is not a failure, so it never becomes the failed command.
  const failedCall = turn.calls.find(call => call.status === 'error' && !call.skipped)
  const failed = turn.failed && !turn.interrupted
  const workLabel = turn.running ? 'Working' : settled ? `Worked${durationLabel ? ` for ${durationLabel}` : ''}` : failed ? 'Run failed' : turn.interrupted ? 'Run interrupted' : 'Work · completion unverified'
  const summary = [workLabel, settled && turn.commands ? `${turn.commands} ${turn.commands === 1 ? 'command' : 'commands'}` : undefined, settled && turn.messagesSent ? `${turn.messagesSent} ${turn.messagesSent === 1 ? 'message sent' : 'messages sent'}` : undefined, settled && turn.changedFiles ? `${turn.changedFiles} ${turn.changedFiles === 1 ? 'file changed' : 'files changed'}` : undefined].filter(value => value !== undefined).join(' · ')
  const started = turn.running ? NaN : Date.parse(turn.startedAt ?? '')
  const failedCommand = failedCall?.kind === 'run' ? failedCall.argsSummary ?? failedCall.title : undefined
  const detail = [failedCommand, failedCall?.detail, turn.failureNote].filter((value, index, values) => value !== undefined && values.indexOf(value) === index).join('\n') || undefined
  const summaryContent = <><span {...stylex.props(styles.summaryLabel)}>{summary}</span>{Number.isFinite(started) && <time dateTime={new Date(started).toISOString()} {...stylex.props(styles.time)}>{timeFormat.format(started)}</time>}</>
  return <CallDetailRenderer.Provider value={renderCallDetail}><CallPresentation.Provider value={{ preview: previewCallDetail, interactive: interactiveCalls }}><section aria-label={ariaLabel} data-testid="work-log">
    {turn.running && !hideLiveRow && <div data-testid="live-work" role="status" aria-label="Response in progress" {...stylex.props(styles.live)}><Icon name="spinner" spinning /><span>Working</span></div>}
    {interactiveCalls ? <Button isDisabled={!settled} aria-expanded={!settled || open} onPress={() => setOpen(value => !value)} {...stylex.props(styles.summary, failed && styles.failureInk)}><span aria-hidden="true" {...stylex.props(styles.slot)}><span {...stylex.props(styles.chevron, (!settled || open) && styles.chevronOpen)}><Icon name="chevron-right" size={12} /></span></span>{summaryContent}</Button> : <div {...stylex.props(styles.summary, styles.staticCall, failed && styles.failureInk)}><span aria-hidden="true" {...stylex.props(styles.slot)} />{summaryContent}</div>}
    {(!interactiveCalls || !settled || open) && <div data-testid="work-log-children" {...stylex.props(styles.children)}><div {...stylex.props(styles.list, listStyle)}>{turn.calls.filter(call => call.kind !== 'unknown').map(call => <TimelineCall key={call.id} call={call} />)}{systemCalls.length > 0 && <>
      <Button data-system-events aria-expanded={showSystemEvents} onPress={() => setShowSystemEvents(value => !value)} {...stylex.props(styles.call)}><span aria-hidden="true" {...stylex.props(styles.slot)}><span {...stylex.props(styles.chevron, showSystemEvents && styles.chevronOpen)}><Icon name="chevron-right" size={12} /></span></span><span {...stylex.props(styles.callLabel)}>{showSystemEvents ? 'Hide system events' : 'Show system events'} · {systemCalls.length}</span></Button>
      {showSystemEvents && systemCalls.map(call => <TimelineCall key={call.id} call={call} />)}
    </>}{expandedBody}</div></div>}
    {failed && <ErrorOverlay id={`work-failure-${turn.calls[0]?.id ?? 'turn'}`} title={failedCommand === undefined ? 'Run failed.' : 'Command did not complete'} detail={errorReason(failedCall?.detail ?? turn.failureNote ?? '')} rawDetail={detail} command={failedCall === undefined ? failedCommandSource(turn.failureNote ?? '') : readableCommand(failedCall)} onRetry={onRetry} onOpenOutput={failedCall === undefined || onOpenOutput === undefined ? undefined : () => onOpenOutput(failedCall)} />}
    {turn.interrupted && <div {...stylex.props(styles.promoted)}><Icon name="stop" size={14} /><span>Run interrupted.</span></div>}
  </section></CallPresentation.Provider></CallDetailRenderer.Provider>
}
function TimelineCall({ call }: { call: WorkLogCall }) {
  const [open, setOpen] = React.useState(false)
  const renderDetail = React.useContext(CallDetailRenderer)
  const { preview, interactive } = React.useContext(CallPresentation)
  // Readable text, else the recorded semantic result (raw disclosure only); a skipped call's failure is not a failure ink.
  const hasOutput = workLogOutputText(call) !== undefined
  const failedInk = call.status === 'error' && !call.skipped
  // Input-only rows still open an intent summary and the explicit raw disclosure.
  const expandable = hasOutput || call.rawInput !== undefined
  const displayed = interactive ? expandable && (open || hasOutput && preview && call.status !== 'error') : hasOutput
  const presentation = workKindPresentation[call.kind]
  const service = workServicePresentation(call)
  // Recorded service facts name the row; otherwise a call's intent leads and the kind's verb and target follow.
  const intent = service === undefined ? call.intent : undefined
  const label = intent === undefined ? call.argsSummary === undefined ? call.title : `${call.title} ${call.argsSummary}` : [intent, presentation.verb, call.argsSummary].filter(value => value !== undefined).join(' · ')
  const start = Date.parse(call.startedAt), end = Date.parse(call.endedAt ?? '')
  const duration = Number.isFinite(start) && Number.isFinite(end) && end >= start ? formatWorkDuration(end - start) || '<1s' : undefined
  // A non-success outcome stays visible as a word beside the pinned disclosure; running rows say so in the duration column until they end.
  const word = call.skipped ? 'Skipped' : call.status === 'error' ? 'Failed' : call.status === 'interrupted' ? 'Interrupted' : service?.readiness
  const row = <>
    <span aria-hidden="true" data-tool-kind={call.kind} {...stylex.props(styles.slot, failedInk && styles.failureInk)}>
      <Icon name={call.skipped ? 'stop-circle' : call.status === 'running' ? 'spinner' : call.status === 'error' ? 'alert' : call.status === 'interrupted' ? 'stop-circle' : workKindPresentation[call.kind].icon} spinning={!call.skipped && call.status === 'running'} size={14} />
    </span>
    <span data-call-title title={label} {...stylex.props(styles.callLabel)}>
      {service !== undefined ? <>Service {service.name}{service.state === undefined ? null : <> · {service.state}</>}</>
        : intent !== undefined ? <><span data-tool-intent>{intent}</span><span data-tool-secondary {...stylex.props(styles.secondary)}>{' · '}{presentation.verb}{call.argsSummary === undefined ? null : <>{' '}<code {...stylex.props(styles.mono)}>{call.kind === 'run' ? <HighlightedSource code={call.argsSummary} language="bash" /> : call.argsSummary}</code></>}</span></>
        : call.kind === 'run' ? <code {...stylex.props(styles.mono)}><HighlightedSource code={label} language="bash" /></code> : <>{call.title}{call.argsSummary === undefined ? null : <>{' '}<code {...stylex.props(styles.mono)}>{call.argsSummary}</code></>}</>}
    </span>
    <span data-call-word data-service-readiness={service === undefined ? undefined : ''} data-service-tone={service === undefined ? undefined : service.warning ? 'warning' : 'neutral'} {...stylex.props(styles.callWord, failedInk && styles.failureInk, service?.warning && styles.serviceWarning)}>{word}</span>
    <span aria-hidden="true" data-row-disclosure={expandable && interactive ? displayed ? 'open' : 'closed' : undefined} {...stylex.props(styles.slot, styles.disclosure)}>{expandable && interactive && <span {...stylex.props(styles.chevron, displayed && styles.chevronOpen)}><Icon name="chevron-right" size={12} /></span>}</span>
    <span data-call-duration {...stylex.props(styles.duration)}>{duration ?? (call.status === 'running' ? 'Running' : '')}</span>
  </>
  return <div data-item-id={call.id} data-tool-status={call.status}>{expandable && interactive ? <TooltipTrigger delay={150}><Button aria-expanded={displayed} onPress={() => setOpen(value => !value)} {...stylex.props(stylex.defaultMarker(), styles.call, open && styles.expanded)}>{row}</Button><Tooltip {...stylex.props(styles.hover)}><div>{label}</div><div>{call.detail ?? ''}</div></Tooltip></TooltipTrigger> : <div {...stylex.props(styles.call, styles.staticCall)}>{row}</div>}{displayed && (renderDetail === undefined ? <DefaultCallDetail call={call} /> : renderDetail(call))}</div>
}
function DefaultCallDetail({ call }: { readonly call: WorkLogCall }) {
  const [rawOpen, setRawOpen] = React.useState(false)
  const output = workLogOutputText(call)
  return <>
    <p>{call.intent ?? call.summary ?? call.title} · {call.status}</p>
    {call.status === 'error' && call.detail !== undefined && <p>{errorReason(call.detail)}</p>}
    <Disclosure isExpanded={rawOpen} onExpandedChange={setRawOpen}>
      <Button slot="trigger" {...stylex.props(styles.call, styles.rawTrigger)}>Show raw input/output</Button>
      <DisclosurePanel>{rawOpen && <>
        {call.rawInput !== undefined && <pre data-testid="tool-raw-input" {...stylex.props(styles.output)}><code>{call.rawInput}</code></pre>}
        {output !== undefined && <pre data-testid="work-call-output" {...stylex.props(styles.output)}><code><HighlightedSource code={output} language={workLogOutputLanguage(call)} /></code></pre>}
      </>}</DisclosurePanel>
    </Disclosure>
  </>
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
  rawTrigger: { display: 'flex', paddingInline: s.xs },
  mono: { fontFamily: t.fontMono, fontSize: t.metaSize },
  secondary: { color: ink.fgMuted, fontSize: t.metaSize },
  output: { marginBlockStart: s.xs2, marginBlockEnd: 0, marginInlineStart: g.textColumn, paddingInline: s.lg, paddingBlock: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, color: ink.fgMuted, fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  time: { flexShrink: 0, color: ink.fgMuted, fontSize: t.metaSize, marginLeft: 'auto' },
  live: { minHeight: g.toolRow, display: 'flex', alignItems: 'center', gap: s.md, color: ink.fgMuted, fontSize: t.metaSize },
  promoted: { display: 'flex', alignItems: 'center', gap: s.md, marginTop: s.md, padding: s.md, backgroundColor: surface.washSubtle, borderRadius: r.md, color: ink.fgSoft, fontSize: t.metaSize, lineHeight: t.uiLeading },
  failureInk: { color: status.dangerFg },
  serviceWarning: { color: status.attention },
  hover: { maxWidth: g.tooltipMax, padding: s.md, borderRadius: r.md, backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.uiLeading, zIndex: 10 },
})
