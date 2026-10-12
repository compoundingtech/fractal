import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, statusVars as status, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'
import { Icon } from '../composition/Icons'
import type { WorkAgent, WorkKind, WorkLogCall, WorkLogTurn } from './work-log'
import { formatWorkDuration } from './work-log'
import { workCallGlyph, workCallWord, workRowStyles as grammar, type WorkLogCallDetailRenderer } from './WorkLogV1'

/** Consecutive calls that ran under the same plan item. Calls without one form an untitled run that renders without phase chrome. */
interface WorkPhase {
  readonly key: string
  readonly title: string | undefined
  readonly calls: readonly WorkLogCall[]
}
function workPhases(calls: readonly WorkLogCall[]): readonly WorkPhase[] {
  const phases: { key: string; title: string | undefined; calls: WorkLogCall[] }[] = []
  for (const call of calls) {
    const last = phases.at(-1)
    if (last !== undefined && last.title === call.phase) last.calls.push(call)
    else phases.push({ key: `phase:${phases.length}`, title: call.phase, calls: [call] })
  }
  return phases
}

const kindOrder: readonly WorkKind[] = ['run', 'eval', 'edit', 'read', 'agent']
const kindNouns: Readonly<Record<WorkKind, readonly [string, string]>> = { run: ['command', 'commands'], eval: ['eval', 'evals'], edit: ['edit', 'edits'], read: ['read', 'reads'], agent: ['agent', 'agents'] }
/** Kinds that change state lead; a delegation counts the agents it named. */
function workKindCounts(calls: readonly WorkLogCall[]): string {
  const counts = new Map<WorkKind, number>()
  for (const call of calls) counts.set(call.kind, (counts.get(call.kind) ?? 0) + (call.kind === 'agent' ? Math.max(call.agents?.length ?? 0, 1) : 1))
  return kindOrder.flatMap(kind => {
    const count = counts.get(kind)
    return count === undefined ? [] : [`${count} ${kindNouns[kind][count === 1 ? 0 : 1]}`]
  }).join(' · ')
}
const kindVerbs: Readonly<Record<WorkKind, string>> = { read: 'Read', run: 'Run', edit: 'Edit', eval: 'Eval', agent: 'Delegate' }

/** Observed span; a running call counts up to the host clock and shows nothing without one. */
function spanMs(startedAt: string, endedAt: string | undefined, now: number): number | undefined {
  const start = Date.parse(startedAt), end = endedAt === undefined ? now : Date.parse(endedAt)
  return Number.isFinite(start) && Number.isFinite(end) && end >= start ? end - start : undefined
}
const callMs = (call: WorkLogCall, now: number) => call.status === 'running' || call.endedAt !== undefined ? spanMs(call.startedAt, call.endedAt, now) : undefined
const live = (call: WorkLogCall) => call.status === 'running' || call.agents?.some(agent => agent.status === 'running') === true
const counted = (count: number, noun: string) => `${count} ${noun}${count === 1 ? '' : 's'}`

const Expansion = React.createContext<{ readonly isOpen: (key: string, auto: boolean) => boolean; readonly toggle: (key: string, open: boolean) => void; readonly now: number; readonly renderDetail: WorkLogCallDetailRenderer | undefined }>({ isOpen: (_, auto) => auto, toggle: () => {}, now: NaN, renderDetail: undefined })

/**
 * The work of one turn as one table: a summary row that folds everything, plan phases on demand, and the
 * steps of an opened phase on shared column tracks. The fold always works; incomplete history only adds a tag.
 * Live work and the failure that ended the turn open themselves; a failure the seat moved past stays shut.
 */
export function WorkBlock({ turn, now, ariaLabel = 'Work', renderCallDetail }: {
  readonly turn: WorkLogTurn
  /** Host clock (ISO) for live durations; without it running rows show no time. */
  readonly now?: string
  readonly ariaLabel?: string
  readonly renderCallDetail?: WorkLogCallDetailRenderer
}) {
  const [overrides, setOverrides] = React.useState<ReadonlyMap<string, boolean>>(() => new Map())
  const clock = now === undefined ? NaN : Date.parse(now)
  const toggle = React.useCallback((key: string, open: boolean) => setOverrides(previous => new Map(previous).set(key, !open)), [])
  const expansion = React.useMemo(() => ({ isOpen: (key: string, auto: boolean) => overrides.get(key) ?? auto, toggle, now: clock, renderDetail: renderCallDetail }), [overrides, toggle, clock, renderCallDetail])
  const last = turn.calls.at(-1)
  const ending = turn.failed && last?.status === 'error' ? last : undefined
  const phases = workPhases(turn.calls)
  const failures = turn.calls.filter(call => call.status === 'error').length
  const started = turn.startedAt === undefined ? NaN : Date.parse(turn.startedAt)
  const elapsed = turn.running && Number.isFinite(started) && clock >= started ? clock - started : turn.durationMs
  const duration = formatWorkDuration(elapsed)
  const label = `${turn.running ? 'Working' : 'Worked'}${duration === '' ? '' : ` for ${duration}`}`
  const word = turn.running ? 'Running' : failures > 0 ? `${failures} failed` : turn.failed ? 'Failed' : turn.interrupted ? 'Interrupted' : undefined
  const blockOpen = expansion.isOpen('block', turn.running || turn.failed)
  const autoCalls = new Set<string>([...ending === undefined ? [] : [ending.id], ...turn.calls.filter(call => call.kind === 'agent' && live(call)).map(call => call.id)])
  return <Expansion.Provider value={expansion}><section aria-label={ariaLabel} data-testid="work-block" {...stylex.props(styles.block)}>
    {/* Nesting wrappers are plain subgrid containers that only carry the rails; every row stays owned by the table. */}
    <div role="table" aria-label={ariaLabel} {...stylex.props(styles.table)}>
      <GroupRow rowKey="block" kind="summary" depth={0} open={blockOpen} glyph={turn.running ? 'running' : 'chevron'} title={label} titleTone="label" counts={counted(turn.calls.length, 'step')} tag={turn.foldable === false ? 'partial history' : undefined} word={word} tone={turn.running ? 'running' : failures > 0 || turn.failed ? 'danger' : 'quiet'} />
      {blockOpen && <div data-work-children="block" {...stylex.props(styles.group, styles.rail0)}>
        {phases.map((phase, index) => phase.title === undefined
          ? phase.calls.map(call => <WorkRow key={call.id} call={call} depth={1} autoOpen={autoCalls.has(call.id)} />)
          : <PhaseRows key={phase.key} phase={phase} index={index} autoOpen={phase.calls.some(call => call === ending || turn.running && live(call))} autoCalls={autoCalls} />)}
      </div>}
    </div>
  </section></Expansion.Provider>
}

function PhaseRows({ phase, index, autoOpen, autoCalls }: { readonly phase: WorkPhase; readonly index: number; readonly autoOpen: boolean; readonly autoCalls: ReadonlySet<string> }) {
  const { isOpen, now } = React.useContext(Expansion)
  const open = isOpen(phase.key, autoOpen)
  const failures = phase.calls.filter(call => call.status === 'error').length
  const running = phase.calls.some(live)
  const known = phase.calls.map(call => callMs(call, now)).filter((value): value is number => value !== undefined)
  return <>
    <GroupRow rowKey={phase.key} kind="phase" depth={1} open={open} glyph={running ? 'running' : 'chevron'} title={phase.title ?? ''} titleTone="subject" counts={workKindCounts(phase.calls)} word={failures > 0 ? `${failures} failed` : running ? 'Running' : undefined} tone={failures > 0 ? 'danger' : running ? 'running' : 'quiet'} time={known.length === 0 ? undefined : known.reduce((sum, value) => sum + value, 0)} phaseIndex={index} />
    {open && <div data-work-children={phase.key} {...stylex.props(styles.group, styles.rail1)}>{phase.calls.map(call => <WorkRow key={call.id} call={call} depth={2} autoOpen={autoCalls.has(call.id)} />)}</div>}
  </>
}

type Tone = 'quiet' | 'danger' | 'running'
/** Summary and phase rows: the title and kind counts span name and detail; the chevron leads in the glyph slot. */
function GroupRow({ rowKey, kind, depth, open, glyph, title, titleTone, counts, tag, word, tone, time, phaseIndex }: {
  readonly rowKey: string; readonly kind: 'summary' | 'phase'; readonly depth: 0 | 1; readonly open: boolean; readonly glyph: 'chevron' | 'running'
  readonly title: string; readonly titleTone: 'label' | 'subject'; readonly counts: string; readonly tag?: string; readonly word?: string; readonly tone: Tone
  readonly time?: number; readonly phaseIndex?: number
}) {
  const { toggle } = React.useContext(Expansion)
  const id = React.useId()
  return <div role="row" data-work-row={kind} data-depth={depth} data-phase-index={phaseIndex} {...stylex.props(stylex.defaultMarker(), styles.row)}>
    <div role="rowheader" data-cell="name" {...stylex.props(styles.nameCell, styles.spanDetail, depths[depth])}>
      <Button aria-expanded={open} aria-labelledby={[`${id}-title`, `${id}-counts`, tag === undefined ? '' : `${id}-tag`, word === undefined ? '' : `${id}-word`].filter(Boolean).join(' ')} onPress={() => toggle(rowKey, open)} {...stylex.props(styles.hit)} />
      <span aria-hidden="true" {...stylex.props(grammar.slot, glyph === 'running' && styles.runningInk)}>{glyph === 'running' ? <Icon name="spinner" spinning size={14} /> : <span {...stylex.props(grammar.chevron, open && grammar.chevronOpen)}><Icon name="chevron-right" size={12} /></span>}</span>
      <span id={`${id}-title`} data-work-title {...stylex.props(styles.text, titleTone === 'subject' ? styles.subject : styles.verb)}>{title}</span>
      <span id={`${id}-counts`} data-work-counts {...stylex.props(styles.counts, kind === 'phase' && styles.phaseCounts)}>{counts}</span>
      {tag !== undefined && <span id={`${id}-tag`} data-work-tag {...stylex.props(styles.tag)}>{tag}</span>}
    </div>
    <div role="cell" data-cell="status" {...stylex.props(styles.wordCell, tones[tone])}><span id={`${id}-word`}>{word}</span></div>
    <div role="cell" data-cell="disclosure" {...stylex.props(styles.disclosureCell)} />
    <div role="cell" data-cell="time" {...stylex.props(styles.timeCell)}>{formatWorkDuration(time)}</div>
  </div>
}

/**
 * One step on the block's shared tracks: indent, glyph and intent (or the verb when the seat wrote none), the raw
 * command as quiet mono detail, the status word, the disclosure and the duration. Durations under a second stay blank.
 */
function WorkRow({ call, depth, autoOpen = false }: { readonly call: WorkLogCall; readonly depth: 1 | 2; readonly autoOpen?: boolean }) {
  const { isOpen, toggle, now, renderDetail } = React.useContext(Expansion)
  const id = React.useId()
  // The raw command, path or arguments: second-level content, shown whole only when the row is opened.
  const detail = call.command ?? call.changedPath ?? call.argsSummary
  const agents = call.kind === 'agent' ? call.agents ?? [] : []
  const expandable = agents.length > 0 || detail !== undefined || (call.detail ?? '') !== ''
  const open = expandable && isOpen(`call:${call.id}`, autoOpen)
  const word = workCallWord(call) ?? (call.status === 'running' ? 'Running' : undefined)
  const tone: Tone = call.status === 'error' ? 'danger' : call.status === 'running' ? 'running' : 'quiet'
  const notable = call.status === 'error' || agents.length > 0
  return <>
    <div role="row" data-work-row="step" data-depth={depth} data-tool-status={call.status} data-call-id={call.id} {...stylex.props(stylex.defaultMarker(), styles.row)}>
      <div role="rowheader" data-cell="name" {...stylex.props(styles.nameCell, depths[depth])}>
        {expandable && <Button aria-expanded={open} aria-labelledby={word === undefined ? `${id}-title` : `${id}-title ${id}-word`} onPress={() => toggle(`call:${call.id}`, open)} {...stylex.props(styles.hit)} />}
        <span aria-hidden="true" {...stylex.props(grammar.slot, tones[tone])}><Icon name={workCallGlyph(call)} spinning={call.status === 'running'} size={14} /></span>
        <span id={`${id}-title`} data-call-title {...stylex.props(styles.text, call.intent === undefined ? styles.verb : styles.subject)}>{call.intent ?? kindVerbs[call.kind]}</span>
      </div>
      <div role="cell" data-cell="detail" data-call-detail {...stylex.props(styles.detailCell, agents.length === 0 && call.kind !== 'agent' && styles.code)}>{agents.length > 0 ? counted(agents.length, 'agent') : detail?.split('\n').find(line => line.trim() !== '')?.trim()}</div>
      <div role="cell" data-cell="status" data-call-word {...stylex.props(styles.wordCell, tones[tone])}><span id={`${id}-word`}>{word}</span></div>
      <div role="cell" data-cell="disclosure" data-row-disclosure={expandable ? open ? 'open' : 'closed' : undefined} {...stylex.props(styles.disclosureCell)}>{expandable && <span aria-hidden="true" {...stylex.props(grammar.slot, styles.chevronQuiet, (notable || open) && styles.chevronNotable)}><span {...stylex.props(grammar.chevron, open && grammar.chevronOpen)}><Icon name="chevron-right" size={12} /></span></span>}</div>
      <div role="cell" data-cell="time" data-call-duration {...stylex.props(styles.timeCell)}>{formatWorkDuration(callMs(call, now))}</div>
    </div>
    {open && (agents.length > 0
      ? <div data-work-children={call.id} {...stylex.props(styles.group, depth === 1 ? styles.rail1 : styles.rail2)}>{agents.map(agent => <AgentRow key={agent.id} agent={agent} depth={depth === 1 ? 2 : 3} />)}</div>
      : <div role="row" data-work-row="body" data-call-id={call.id} {...stylex.props(styles.bodyRow)}><div role="cell" {...stylex.props(styles.bodyCell, bodyDepths[depth])}><div data-opened-body {...stylex.props(grammar.output, styles.frame, call.status === 'error' && styles.frameDanger)}>{renderDetail === undefined ? <OpenedDetail call={call} detail={detail} /> : renderDetail(call)}</div></div></div>)}
  </>
}

/** A named child of a delegation: name, assignment, status and its own run time. */
function AgentRow({ agent, depth }: { readonly agent: WorkAgent; readonly depth: 2 | 3 }) {
  const word = workCallWord({ status: agent.status }) ?? (agent.status === 'running' ? 'Running' : undefined)
  const tone: Tone = agent.status === 'error' ? 'danger' : agent.status === 'running' ? 'running' : 'quiet'
  return <div role="row" data-work-row="agent" data-depth={depth} data-tool-status={agent.status} {...stylex.props(stylex.defaultMarker(), styles.row)}>
    <div role="rowheader" data-cell="name" {...stylex.props(styles.nameCell, depths[depth])}>
      <span aria-hidden="true" {...stylex.props(grammar.slot, tones[tone])}><Icon name={agent.status === 'running' ? 'spinner' : agent.status === 'error' ? 'alert' : 'agent'} spinning={agent.status === 'running'} size={14} /></span>
      <span data-call-title {...stylex.props(styles.text, styles.subject)}>{agent.name}</span>
      {agent.task !== undefined && <span data-agent-task {...stylex.props(styles.counts, styles.narrowOnly)}>{agent.task}</span>}
    </div>
    <div role="cell" data-cell="detail" data-call-detail {...stylex.props(styles.detailCell)}>{agent.task}</div>
    <div role="cell" data-cell="status" data-call-word {...stylex.props(styles.wordCell, tones[tone])}>{word}</div>
    <div role="cell" data-cell="disclosure" {...stylex.props(styles.disclosureCell)} />
    <div role="cell" data-cell="time" data-call-duration {...stylex.props(styles.timeCell)}>{formatWorkDuration(agent.durationMs)}</div>
  </div>
}

/** The opened body: the whole command first, then the output's last lines. */
function OpenedDetail({ call, detail }: { readonly call: WorkLogCall; readonly detail: string | undefined }) {
  const output = (call.detail ?? '').replace(/\n+$/, '')
  return <>
    {detail !== undefined && <pre data-testid="work-step-command" {...stylex.props(styles.command)}><code>{detail}</code></pre>}
    {output !== '' && <TerminalTail text={output} />}
  </>
}

const TAIL_LINES = 3
/** The last lines of output; earlier lines stay one press away. */
function TerminalTail({ text }: { readonly text: string }) {
  const [full, setFull] = React.useState(false)
  const [copied, setCopied] = React.useState(false)
  const lines = text.split('\n')
  const hidden = full ? 0 : Math.max(0, lines.length - TAIL_LINES)
  const copy = () => { void navigator.clipboard?.writeText(text).then(() => setCopied(true), () => setCopied(false)) }
  return <div data-terminal-tail>
    <div {...stylex.props(styles.tailMeta)}>
      {lines.length > TAIL_LINES && <>
        {hidden > 0 && <><span data-tail-hidden>… {counted(hidden, 'earlier line')}</span><span aria-hidden="true">·</span></>}
        {/* One toggle that stays mounted, so keyboard focus survives the switch. */}
        <Button onPress={() => setFull(value => !value)} {...stylex.props(styles.tailAction)}>{full ? 'Last lines' : 'Full output'}</Button>
        <span aria-hidden="true">·</span>
      </>}
      <Button onPress={copy} data-copied={copied || undefined} {...stylex.props(styles.tailAction)}>{copied ? 'Copied' : 'Copy'}</Button>
    </div>
    <pre data-testid="work-step-output" {...stylex.props(styles.tailLines)}><code>{lines.slice(hidden).join('\n')}</code></pre>
  </div>
}

// The lane width where the detail column appears; it matches the transcript lane minimum.
const wideLane = '@container (min-width: 40rem)'
const styles = stylex.create({
  // The block is the query container, so the table's tracks follow the lane, not the viewport.
  block: { containerType: 'inline-size', fontFamily: t.fontSans, color: ink.fgMuted },
  table: { display: 'grid', gridTemplateColumns: { default: g.workTableNarrowColumns, [wideLane]: g.workTableColumns }, columnGap: s.md, rowGap: 0 },
  group: { position: 'relative', display: 'grid', gridTemplateColumns: 'subgrid', gridColumn: '1 / -1' },
  // One rail per nesting level, on the parent's glyph centre, stopping short of the last child.
  rail0: { '::before': { content: '""', position: 'absolute', insetInlineStart: g.railInset, top: 0, bottom: s.xs, width: g.hairline, backgroundColor: border.rail } },
  rail1: { '::before': { content: '""', position: 'absolute', insetInlineStart: `calc(${g.railInset} + ${g.textColumn})`, top: 0, bottom: s.xs, width: g.hairline, backgroundColor: border.rail } },
  rail2: { '::before': { content: '""', position: 'absolute', insetInlineStart: `calc(${g.railInset} + 2 * ${g.textColumn})`, top: 0, bottom: s.xs, width: g.hairline, backgroundColor: border.rail } },
  row: { position: 'relative', display: 'grid', gridTemplateColumns: 'subgrid', gridColumn: '1 / -1', alignItems: 'center', height: g.toolRow, borderRadius: r.sm, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover } },
  // The whole row is the press target; the visible cells stay plain text under it.
  hit: { position: 'absolute', inset: 0, zIndex: 1, margin: 0, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, cursor: 'pointer', outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: accent.primary, outlineOffset: `calc(-1 * ${g.focusRing})` },
  nameCell: { gridColumn: 'name', display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0 },
  spanDetail: { gridColumn: 'name / status' },
  text: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.uiSize, lineHeight: t.uiLeading },
  subject: { flexShrink: 1, color: ink.fgSoft, fontWeight: t.weightMedium },
  verb: { flexShrink: 1, color: ink.fgMuted, fontWeight: t.weightRegular },
  counts: { flexShrink: 100, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  // In a narrow lane the phase title keeps the room; its kind counts stay in the accessible name.
  phaseCounts: { display: { default: 'none', [wideLane]: 'inline' } },
  // An agent's assignment rides after its name while the detail column is folded away.
  narrowOnly: { display: { default: 'inline', [wideLane]: 'none' } },
  tag: { flexShrink: 0, paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, fontSize: t.denseSize, lineHeight: t.metaLeading, color: ink.fgMuted, whiteSpace: 'nowrap' },
  detailCell: { gridColumn: 'detail', display: { default: 'none', [wideLane]: 'block' }, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  code: { fontFamily: t.fontMono },
  wordCell: { gridColumn: 'status', maxWidth: g.statusWordMax, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', textAlign: 'right', fontSize: t.metaSize, lineHeight: t.metaLeading },
  disclosureCell: { gridColumn: 'disclosure', display: 'flex', justifyContent: 'center', color: ink.fgMuted },
  timeCell: { gridColumn: 'time', overflow: 'hidden', whiteSpace: 'nowrap', textAlign: 'right', fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', color: ink.fgMuted },
  // Hidden at rest, half ink on rows with something notable to open, full on hover or focus.
  chevronQuiet: { opacity: { default: 0, [stylex.when.ancestor(':hover')]: 1, [stylex.when.ancestor(':focus-within')]: 1 } },
  chevronNotable: { opacity: { default: 0.5, [stylex.when.ancestor(':hover')]: 1, [stylex.when.ancestor(':focus-within')]: 1 } },
  runningInk: { color: status.runningFg },
  bodyRow: { display: 'grid', gridTemplateColumns: 'subgrid', gridColumn: '1 / -1', paddingBlockEnd: s.xs },
  bodyCell: { gridColumn: 'name / end', minWidth: 0 },
  frame: { marginInlineStart: 0, color: ink.fgMuted, fontFamily: t.fontSans },
  frameDanger: { borderColor: status.dangerMuted },
  command: { margin: 0, fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.codeLeading, color: ink.fgMuted, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  tailMeta: { display: 'flex', alignItems: 'center', gap: s.xs, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  tailAction: { padding: 0, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', textDecorationLine: { default: 'none', ':hover': 'underline' }, outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: accent.primary },
  tailLines: { margin: 0, fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.codeLeading, color: ink.fgSoft, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
})
const tones = stylex.create({
  quiet: { color: ink.fgMuted },
  danger: { color: status.dangerFg },
  running: { color: status.runningFg },
})
// Each nesting level steps one text column (glyph slot + gutter) inside the name column, so every other edge stays put.
const depthStyles = stylex.create({
  d0: { paddingInlineStart: 0 },
  d1: { paddingInlineStart: g.textColumn },
  d2: { paddingInlineStart: `calc(2 * ${g.textColumn})` },
  d3: { paddingInlineStart: `calc(3 * ${g.textColumn})` },
})
const depths = [depthStyles.d0, depthStyles.d1, depthStyles.d2, depthStyles.d3] as const
// An opened body starts at its step's text column, one level in from the step.
const bodyDepths = { 1: depthStyles.d2, 2: depthStyles.d3 } as const
