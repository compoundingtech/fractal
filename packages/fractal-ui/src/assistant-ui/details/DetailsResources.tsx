import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button as AriaButton } from 'react-aria-components'
import { ResourceEntityCard } from '../entity/ResourceCard'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t, interactionVars } from '../composition-tokens.stylex'
import { Icon, type IconName } from '../composition/Icons'
import { AgentHoverCard } from '../sidebar/DeferredRowActions.tsx'
import { needsYouLabel, rowAttention } from '../sidebar/model'
import { agentPaneKey } from '../workbench/agent-drag'
import { useEntityDrag } from '../../workspace/react/mod'
import { ThingCard, ThingRow } from '../things/Thing'
import type { Observed, ThingAct, ThingFace, ThingMark, ThingOpenable } from '../things/face'
import type { DetailsResource, DetailsSection, DetailsState, DetailsValue } from './model'
import type { TraceSeries, TraceSubagent } from './trace-series'
import { DetailsInk } from './DetailsSkeleton'
import { accumulationParts, TraceCoverageNote } from './DetailsCharts'
import { DetailsAvailabilityState, DetailsUnavailable, DetailsUnreportedSections, DetailsWhy, MissingDetailsValue } from './DetailsUnavailable'
import { DetailsReadFailureLine } from './DetailsReadState'
import { detailsReasonText, traceFactReason, traceReasonText } from './trace-reasons'
import { traceCoverageState } from './trace-series'

export type DetailsEmptyRule = 'E1' | 'E2' | 'E3'
export interface DetailsResourcesProps {
  readonly state: DetailsState
  readonly sections: readonly DetailsSection[]
  /** Breakdown facts are independent of the Fractal resource snapshot. */
  readonly trace: TraceSeries
  readonly now: number
  readonly empty?: DetailsEmptyRule
  readonly onOpen: (uri: string, split: boolean) => void
  /** Synced layout intent (q45): section names the parent keeps hidden. */
  readonly hiddenSections?: readonly string[]
  readonly styles?: stylex.StyleXStyles
}
export const DetailsLabelContext=React.createContext('')

/** One pane-level column; collapse, hover and skeletons stay local, never synced. Every row is the thing kit's. */
export function DetailsResources({ state, sections: suppliedSections, trace, now, empty = 'E1', onOpen, hiddenSections = [], styles: callerStyles }: DetailsResourcesProps) {
  const labelPrefix=React.useContext(DetailsLabelContext)
  // Phone hosts: section toggles are 44px touch targets and unavailable reasons paint inline.
  const narrow = React.useContext(NarrowLayoutContext)
  // Pending geometry derives only from the host's supplied observations.
  const observedSubagents = suppliedSections.find(section => section.title === 'Subagents')
  // Known [] is an observed empty Fractal resource list, distinct from an unreported one; it is evidence once the agent has started.
  const hasObservedSubagents = observedSubagents?.items._tag === 'Known'
  const hasObservedSubagentRows = observedSubagents?.items._tag === 'Known' && observedSubagents.items.value.length > 0
  const hasNoTracedSubagents = knownZeroTracedSubagents(trace)
  const [collapsed, setCollapsed] = React.useState<Readonly<Record<string, boolean>>>({})
  const hidden = React.useMemo(() => new Set(hiddenSections), [hiddenSections])
  const toggle = React.useCallback((title: string) => setCollapsed(current => ({ ...current, [title]: !current[title] })), [])
  const open = React.useCallback((uri: string, split: boolean) => onOpen(agentPaneKey(uri), split), [onOpen])
  const candidates = state === 'error' ? [] : suppliedSections
    .map(section => section.title === 'Subagents' ? { ...section, ...traceSubagentSection(trace) } : section.title === 'History' ? { ...section, ...traceHistorySection(trace) } : section)
    .filter(section => !hidden.has(section.title))
  const hasSubagentEvidence = hasObservedSubagents || hasNoTracedSubagents || trace.subagentCount._tag === 'Known'
  // A section whose own read has been in flight or failed stays a mounted section for the pane's life, even when the
  // read settles refused or unsupported (Unknown, no failure): its heading is the focus target the read line hands off to.
  const [readSections, setReadSections] = React.useState<ReadonlySet<string>>(() => new Set())
  const reading = suppliedSections.filter(section => section.pending === true || section.failure !== undefined).map(section => section.title)
  if (reading.some(title => !readSections.has(title))) setReadSections(new Set([...readSections, ...reading]))
  const ownsRead = (section: DetailsSection): boolean => section.pending === true || section.failure !== undefined || readSections.has(section.title)
  const unreported = empty === 'E3' && state !== 'new'
    ? candidates.filter(section => !ownsRead(section) && section.items._tag === 'Unknown' && !(section.title === 'Subagents' && hasSubagentEvidence))
    : []
  // Pane-level loading keeps the settled footprint: the unreported-sections tail is reserved, unpainted and inert, so recovery never grows the slot.
  const reserveUnreported = state === 'loading'
  const sections = candidates
    // First reads stay pending, never masquerading as settled unavailable data; a read section keeps its own heading and line.
    .filter(section => ownsRead(section) || state !== 'new' || empty !== 'E3' || (section.items._tag === 'Known' && section.items.value.length > 0) || (section.title === 'Subagents' && hasObservedSubagentRows))
    .filter(section => ownsRead(section) || visible(section.items, empty) || (section.title === 'Subagents' && hasSubagentEvidence))
  return <div data-details-state={state} data-details-empty={empty} {...stylex.props(styles.root, callerStyles)}>
    {sections.map(section => {
      const isCollapsed = collapsed[section.title] === true
      // The Subagents badge is the reported traced total, never the retained list length; an unknown total has no badge.
      const count = section.title === 'Subagents' ? trace.subagentCount._tag === 'Known' ? trace.subagentCount.value : undefined
        : section.items._tag === 'Known' && section.items.value.length > 0 ? section.items.value.length : undefined
      const pending = state === 'loading' || section.pending === true
      // Nothing read yet and the read failed: the heading names the section and its failure line is the whole body.
      const failedOnly = !pending && section.failure !== undefined && section.items._tag === 'Unknown'
      // Settled with nothing reported (including a refused or unsupported read): the heading carries the availability copy.
      const unavailable = !failedOnly && !pending && section.items._tag === 'Unknown' && !(section.title === 'Subagents' && hasSubagentEvidence)
      const expandable = !failedOnly && !unavailable
      // Every state shares one tree with one heading button, so the read line and whichever control holds focus survive
      // every transition; with nothing to expand the button stays focusable but `aria-disabled`.
      return <section key={section.title} aria-label={`${labelPrefix}${section.title}`} data-details-section-state={failedOnly ? 'failed' : unavailable ? 'unavailable' : undefined} aria-busy={pending || undefined} {...stylex.props(styles.section)}>
        <div {...stylex.props(styles.header, unavailable && narrow && styles.headerWrap)}>
          <AriaButton data-details-section-control aria-expanded={expandable ? !isCollapsed : undefined} aria-disabled={expandable ? undefined : true} onPress={() => { if (expandable) toggle(section.title) }} {...stylex.props(styles.headerButton, narrow && styles.touchHeight, !expandable && styles.headerButtonStatic, unavailable && styles.headerButtonFit)}>
            {expandable ? <Icon name={isCollapsed?'chevron-right':'chevron-down'}/> : <span {...stylex.props(styles.chevronSpace)} />}
            <span data-details-heading {...stylex.props(unavailable ? styles.metaLine : styles.headerTitle)}>{section.title}</span>
            {count !== undefined && (pending
              ? <span aria-hidden="true" data-details-skeleton-count {...stylex.props(styles.count)}><DetailsInk ch={String(count).length} /></span>
              : <span data-details-section-count {...stylex.props(styles.count)}>{count}</span>)}
          </AriaButton>
          {unavailable && <><span {...stylex.props(styles.metaLine)}>·</span><DetailsAvailabilityState subject={section.title} reason={unavailableReason(section, trace, empty)} /></>}
        </div>
        <div {...stylex.props(styles.body, ((isCollapsed && expandable) || unavailable) && styles.bodyCollapsed)}>
          {/* Pane-level loading has no section read in flight; a section's own pending read keeps its held failure mounted. */}
          {state !== 'loading' && <DetailsReadFailureLine failure={pending ? undefined : section.failure} pending={section.pending === true} />}
          {expandable && !isCollapsed && <SectionContent section={section} observedSubagents={observedSubagents} trace={trace} now={now} empty={empty} open={open} pending={pending} />}
        </div>
      </section>
    })}
    {/* One summary line for every unreported section; their reasons wait behind a collapsed Why? on desktop and phone alike. */}
    {unreported.length > 0 && <div data-details-unreported-sections={reserveUnreported ? undefined : true} data-details-unreported-reserved={reserveUnreported || undefined} aria-hidden={reserveUnreported || undefined} inert={reserveUnreported} {...stylex.props(styles.metaLine, styles.allHidden, styles.unreportedTail, reserveUnreported && styles.skeletonHidden)}>
      <DetailsUnreportedSections sections={unreported.map(section => ({ name: section.title, reason: unavailableReason(section, trace, empty) }))} />
    </div>}
    {sections.length === 0 && unreported.length === 0 && state !== 'error' && !(state === 'new' && empty === 'E3') && <p {...stylex.props(styles.metaLine, styles.allHidden)}>Every section is empty or hidden.</p>}
  </div>
}

/** E1 and E3 hide empty and Unknown bodies; E3 names settled unavailable categories once. E2 shows everything. */
function visible(items: DetailsValue<readonly DetailsResource[]>, rule: DetailsEmptyRule): boolean {
  if (rule === 'E2') return true
  if (items._tag === 'Unknown') return false
  return items.value.length > 0
}

/** An explicitly reported zero is data, not an empty-list inference. */
const knownZeroTracedSubagents = (trace: TraceSeries): boolean => trace.subagentCount._tag === 'Known'
  && trace.subagentCount.value === 0 && trace.subagents._tag === 'Known' && trace.subagents.value.length === 0

const humanSectionReasons: Readonly<Partial<Record<DetailsSection['title'], string>>> = {
  Reviews: 'This session doesn’t report reviews.',
  'Declared bindings': 'This session doesn’t report declared bindings.',
  History: 'This session doesn’t report its history.',
}
function unavailableReason(section: DetailsSection, trace: TraceSeries, empty: DetailsEmptyRule): string {
  if (section.items._tag === 'Unknown' && section.unavailableReason !== undefined) return detailsReasonText(section.unavailableReason)
  if (section.title === 'Subagents') return trace.subagents._tag === 'Unknown' ? traceFactReason(trace.subagents) : traceReasonText('no-provider')
  if (section.title === 'History' && trace.history._tag === 'Unknown' && trace.history.detail !== undefined) return traceFactReason(trace.history)
  if (empty !== 'E3') return `This session doesn’t report ${section.title.toLowerCase()} yet.`
  return humanSectionReasons[section.title] ?? `This session doesn’t report ${section.title.toLowerCase()}.`
}

/** Name only missing row metrics; unrelated unreported facts need no section note. */
function missingLineCountsNote(section: DetailsSection): string | undefined {
  if (section.items._tag !== 'Known') return undefined
  let added = false, removed = false
  for (const resource of section.items.value) {
    added ||= resource.file?.added._tag === 'Unknown'
    removed ||= resource.file?.removed._tag === 'Unknown'
  }
  if (!added && !removed) return undefined
  return `${added && removed ? 'Added- and removed-line' : added ? 'Added-line' : 'Removed-line'} counts aren't reported for some files.`
}

/** No session-to-agent identity is supplied; subagent sessions stay neutral. */
export function traceSubagentSection(trace: TraceSeries): DetailsSection {
  if (trace.subagents._tag !== 'Known') return { title: 'Subagents', items: { _tag: 'Unknown' } }
  return {
    title: 'Subagents',
    items: { _tag: 'Known', value: trace.subagents.value.map(subagent => {
      // Reported role, known model and full native session identity are visible before Open; a generic role stays generic.
      const label = [subagent.label._tag === 'Known' ? subagent.label.value : 'role unreported', subagent.model._tag === 'Known' ? subagent.model.value : undefined, subagent.sessionId].filter(value => value !== undefined).join(' · ')
      const detail = traceSubagentDetail(subagent)
      return {
        uri: `session:${subagent.sessionId}`,
        glyph: '◇',
        card: { kind: 'file', label, detail: `Subagent session · ${detail}`, fileCount: { _tag: 'Unknown' }, files: [] },
      }
    }) },
  }
}
/** Missing descendant facts are omitted, never fabricated parentage or timing; the model is part of the title. */
const traceSubagentDetail = (subagent: TraceSubagent): string => [
  `$${subagent.usd.toFixed(2)}`,
  `${subagent.requests} requests`,
  subagent.parentSessionId._tag === 'Known' ? `Parent · ${subagent.parentSessionId.value}` : undefined,
  subagent.firstMs._tag === 'Known' ? `First observed ${historyTimestamp.format(subagent.firstMs.value)} UTC` : undefined,
  subagent.lastMs._tag === 'Known' ? `Last observed ${historyTimestamp.format(subagent.lastMs.value)} UTC` : undefined,
].filter(value => value !== undefined).join(' · ')

const historyTimestamp=new Intl.DateTimeFormat('en-GB',{dateStyle:'medium',timeStyle:'short',timeZone:'UTC'})
/** Incarnations retain their provider identity and observed lifetime facts. */
export function traceHistorySection(trace: TraceSeries): DetailsSection {
  if(trace.history._tag!=='Known')return {title:'History',items:{_tag:'Unknown'}}
  return {title:'History',items:{_tag:'Known',value:trace.history.value.map(incarnation=>({
    uri:`session:${incarnation.nativeSessionId}`,
    glyph:'◷',
    card:{
      kind:'file',
      label:`Session · ${incarnation.nativeSessionId}`,
      detail:`Started ${historyTimestamp.format(incarnation.startedMs)} UTC${incarnation.endedMs===null?'':` · Ended ${historyTimestamp.format(incarnation.endedMs)} UTC`} · $${incarnation.usd.toFixed(2)} · ${incarnation.compactions} ${incarnation.compactions === 1 ? 'compaction' : 'compactions'}`,
      fileCount:{_tag:'Unknown'},
      files:[],
    },
  }))}}
}

/** The loading shadow and populated body share markup, wrapping and intrinsic metadata fit. */
function SectionContent({ section, observedSubagents, trace, now, empty, open, pending }: {
  section: DetailsSection; observedSubagents: DetailsSection | undefined; trace: TraceSeries; now: number;
  empty: DetailsEmptyRule; open: (uri: string, split: boolean) => void; pending: boolean
}) {
  const hasObserved = observedSubagents?.items._tag === 'Known'
  // A first-read Subagents section reserves its breakdown; pane-level loading keeps the settled footprint.
  const showBreakdown = section.pending === true || section.title !== 'Subagents' || visible(section.items, empty) || trace.subagentCount._tag === 'Known'
  const subagentWindow=section.title==='Subagents'?accumulationParts(trace.subagentsWindow):undefined
  const collectionReason = showBreakdown && section.items._tag === 'Unknown' ? unavailableReason(section, trace, empty) : undefined
  const observedReason = section.title === 'Subagents' && observedSubagents?.items._tag === 'Unknown'
    ? observedSubagents.unavailableReason ?? 'This session doesn’t report its resources.' : undefined
  const missingStatus = section.title === 'Subagents' && observedSubagents?.items._tag === 'Known'
    && observedSubagents.items.value.some(resource => resource.agent?.status === 'unobserved')
  const partialNote = missingStatus ? "Status isn't reported for some resources." : missingLineCountsNote(section)
  const windowReason = section.title === 'Subagents' && showBreakdown && subagentWindow === undefined
    ? trace.subagentsWindow._tag === 'Unknown' ? traceFactReason(trace.subagentsWindow) : 'The period for this subagent list isn’t reported.'
    : undefined
  const countReason = section.title === 'Subagents' && showBreakdown && trace.subagents._tag === 'Known' && trace.subagentCount._tag === 'Unknown'
    ? traceFactReason(trace.subagentCount) : undefined
  const coverage = traceCoverageState(trace.coverage)
  // Which session spend belongs to is explained behind the Spend group's Why?, never as a missing field here.
  const gaps = [
    { subject: 'Subagent list', reason: collectionReason, partial: false },
    { subject: 'Resources', reason: observedReason, partial: false },
    { subject: missingStatus ? 'Status' : 'Line counts', reason: partialNote, partial: missingStatus
      ? observedSubagents?.items._tag === 'Known' && observedSubagents.items.value.some(resource => resource.agent !== undefined && resource.agent.status !== 'unobserved')
      : section.items._tag === 'Known' && section.items.value.some(resource => resource.file?.added._tag === 'Known' || resource.file?.removed._tag === 'Known') },
    { subject: 'Subagent list window', reason: windowReason, partial: false },
    { subject: 'Subagent total', reason: countReason, partial: false },
  ].filter((gap): gap is { subject: string; reason: string; partial: boolean } => gap.reason !== undefined)
  const allShown = trace.subagents._tag === 'Known' && trace.subagentCount._tag === 'Known' && trace.subagentCount.value === trace.subagents.value.length
  return <div aria-hidden={pending || undefined} inert={pending} data-details-skeleton={pending ? section.title : undefined} {...stylex.props(styles.contentShell)}>
    <div {...stylex.props(styles.content, pending && styles.skeletonHidden)}>
      {section.title === 'Subagents' && hasObserved && observedSubagents !== undefined && observedSubagents.items._tag === 'Known' && (observedSubagents.items.value.length === 0
        ? <SkeletonLine pending={pending}><p data-details-observed-subagent-count {...stylex.props(styles.metaLine)}>No resources for this session.</p></SkeletonLine>
        : <SectionItems section={observedSubagents} now={now} open={open} pending={pending} />)}
      {showBreakdown && <>
        {section.title === 'Subagents' && <SkeletonLine pending={pending}><p data-details-subagent-window {...stylex.props(styles.metaLine)}>Subagents · {subagentWindow === undefined ? <MissingDetailsValue label="Subagent list window" reason={windowReason ?? 'The period for this subagent list isn’t reported.'} /> : subagentWindow.join('')}</p></SkeletonLine>}
        {/* A complete list needs no count; a bounded or unknown total says how many are shown. */}
        {section.title === 'Subagents' && trace.subagents._tag === 'Known' && !allShown && <SkeletonLine pending={pending}><p data-details-subagent-count {...stylex.props(styles.metaLine)}>Showing {trace.subagents.value.length} of {trace.subagentCount._tag === 'Known' ? trace.subagentCount.value : <MissingDetailsValue label="Total subagent count" reason={countReason ?? 'The total number of subagents isn’t reported.'} />} subagents</p></SkeletonLine>}
        {section.title === 'Subagents' && (coverage._tag === 'Partial' || coverage._tag === 'NotCovered' || coverage._tag === 'PartialUnreported') && <SkeletonLine pending={pending}><TraceCoverageNote trace={trace} /></SkeletonLine>}
        {section.items._tag === 'Unknown'
          ? null
          : section.items.value.length === 0
            ? <SkeletonLine pending={pending}><p {...stylex.props(styles.metaLine)}>None</p></SkeletonLine>
            : <SectionItems section={section} now={now} open={open} pending={pending} />}
      </>}
      {gaps.length > 0 && <SkeletonLine pending={pending}>{gaps.length === 1 && gaps[0]!.subject === 'Resources'
        ? <p data-details-unavailable data-details-unavailable-subject="Resources" {...stylex.props(styles.whyLine)}><span data-details-resources-unavailable {...stylex.props(styles.whySummary)}>Resources aren’t available for this session.</span><DetailsWhy explains="Resources" reasons={[detailsReasonText(gaps[0]!.reason)]} /></p>
        : <DetailsUnavailable subjects={gaps.map(gap => gap.subject)} state={gaps.every(gap => gap.partial) ? 'Partial' : 'Unavailable'} reason={gaps.map(gap => gap.reason).join(' ')} />}</SkeletonLine>}
    </div>
  </div>
}

/**
 * Pending rows keep their populated markup for exact geometry and paint one
 * row-shaped skeleton over it, like Context and Spend ink, instead of a section
 * block. The wrapper re-shows only the skeleton inside the hidden content.
 */
function SkeletonRow({ pending, label, meta, children }: { pending: boolean; label: string; meta?: string; children: React.ReactNode }) {
  if (!pending) return children
  return <div data-details-skeleton-row {...stylex.props(styles.skeletonRow)}>
    <div {...stylex.props(styles.skeletonHidden)}>{children}</div>
    <div {...stylex.props(styles.skeletonInk)}>
      <DetailsInk ch={2} />
      <span {...stylex.props(styles.skeletonLabel)}><DetailsInk ch={Math.min(label.length, 32)} /></span>
      {meta !== undefined && meta !== '' && <DetailsInk ch={Math.min(meta.length, 16)} />}
    </div>
  </div>
}

function SkeletonLine({ pending, children }: { pending: boolean; children: React.ReactNode }) {
  if (!pending) return children
  return <div data-details-skeleton-row {...stylex.props(styles.skeletonRow)}>
    <div {...stylex.props(styles.skeletonHidden)}>{children}</div>
    <div {...stylex.props(styles.skeletonInk)}><DetailsInk ch={18} /></div>
  </div>
}

function SectionItems({ section, now, open, pending }: { section: DetailsSection; now: number; open: (uri: string, split: boolean) => void; pending: boolean }) {
  const items = section.items._tag === 'Known' ? section.items.value : []
  return <div role="list" aria-label={section.title} {...stylex.props(styles.list)}>
    {items.map(resource => {
      const face = resource.face ?? legacyFace(resource)
      const fact = isFact(resource)
      return <div role="listitem" key={resource.uri} data-details-resource={resource.uri}>
        <SkeletonRow pending={pending} label={face.title} meta={face.act?.label ?? face.mark.word}>
          {fact ? <ThingRow observed={observedNow(face, now)} now={now} /> : <OpenableResource resource={resource} face={face} now={now} open={open} />}
          {/* A fact row has no hover card, so its reported facts paint beneath it instead of hiding. */}
          {fact && face.summary !== undefined && <p data-details-resource-detail {...stylex.props(styles.factDetail)}>{face.summary}</p>}
        </SkeletonRow>
      </div>
    })}
  </div>
}

/** Reported facts (`details:` observations, traced sessions) have no ref to open yet: the kit draws them as fact rows. */
const isFact = (resource: DetailsResource): boolean => resource.uri.startsWith('details:') || resource.uri.startsWith('session:')

/** Details rows are a current snapshot; the host's read state speaks for staleness above the sections. */
const observedNow = (face: ThingFace, now: number): Observed<ThingFace> => ({ _tag: 'Known', value: face, observedAt: now, freshness: 'live' })

/**
 * A row with a ref: click opens, Cmd/Ctrl-click and Alt+Enter open in a split, Enter opens, a press drags it onto the
 * workspace, and resting on it shows its entity's canonical card.
 */
function OpenableResource({ resource, face, now, open }: { resource: DetailsResource; face: ThingFace; now: number; open: (uri: string, split: boolean) => void }) {
  const key = agentPaneKey(resource.agent === undefined ? resource.uri : resource.agent.ref)
  const entity = React.useMemo(() => resource.agent !== undefined
    ? { kind: 'agent' as const, id: resource.agent.ref, title: face.title }
    : { kind: 'resource' as const, id: resource.uri, title: face.title }, [resource.agent, resource.uri, face.title])
  const drag = useEntityDrag({ ref: key, entity })
  const openable = React.useMemo<ThingOpenable>(() => ({
    onClick: event => open(key, event.metaKey || event.ctrlKey),
    onKeyDown: event => {
      if (event.key !== 'Enter') return
      event.preventDefault()
      open(key, event.altKey || event.metaKey || event.ctrlKey)
    },
    onPointerDown: drag.onPointerDown,
  }), [key, open, drag.onPointerDown])
  const observed = observedNow(face, now)
  const card = () => resource.agent !== undefined ? <AgentHoverCard row={resource.agent} reportedStatusOnly now={now} />
    : resource.terminal !== undefined ? <ResourceEntityCard id={resource.uri} title={face.title} kind="Terminal" icon="terminal" text={resource.card.detail} preview={resource.terminal.slice(-6)} />
      : <ThingCard observed={observed} now={now} />
  return <ThingRow observed={observed} now={now} openable={openable} hoverCard={card} />
}

const kindLabels: Readonly<Record<string, string>> = {
  agent: 'Agent', mission: 'Mission', terminal: 'Terminal', worktree: 'Worktree', repository: 'Repository', branch: 'Branch',
  pr: 'Pull request', issue: 'Issue', 'ci-run': 'CI run', review: 'Review', diff: 'File', file: 'File', attention: 'Attention', session: 'Session', details: 'Detail',
}

/** A face for hosts that supply only the legacy card: the card's label and detail, the agent's reported status, file counts. */
function legacyFace(resource: DetailsResource): ThingFace {
  const scheme = resource.uri.split(':', 1)[0] ?? ''
  const base = {
    kind: kindLabels[scheme] ?? 'Resource',
    icon: resourceIcon(resource),
    title: resource.card.label,
    ...(resource.file !== undefined ? { titleStyle: 'path' as const } : {}),
    ...(resource.card.detail === '' ? {} : { summary: resource.card.detail }),
  }
  const agent = resource.agent
  if (agent !== undefined) {
    const attention = rowAttention(agent)
    // An unobserved agent has no status to show: no mark, no word, never the card's projected text.
    const mark: ThingMark = agent.status === 'unobserved' ? { shape: 'none', word: '' }
      : { shape: agent.status === 'working' ? 'filled' : agent.status === 'failed' || agent.status === 'ended' || agent.status === 'retired' ? 'ended' : 'hollow', word: agent.statusLabel }
    const act: ThingAct | undefined = attention === 'failed' ? { tone: 'failed', label: agent.statusLabel }
      : attention === 'needs-you' ? { tone: 'act', label: needsYouLabel } : undefined
    return { ...base, mark, ...(act === undefined ? {} : { act }) }
  }
  const counts = resource.file === undefined ? '' : [
    resource.file.added._tag === 'Known' && resource.file.added.value !== 0 ? `+${resource.file.added.value}` : '',
    resource.file.removed._tag === 'Known' && resource.file.removed.value !== 0 ? `−${resource.file.removed.value}` : '',
  ].filter(Boolean).join(' ')
  return { ...base, mark: { shape: 'none', word: counts } }
}

/** Resource concepts use the same stroke kit as section controls, never text glyphs. */
function resourceIcon(resource: DetailsResource): IconName {
  switch (resource.uri.split(':', 1)[0]) {
    case 'worktree':
    case 'repository': return 'folder'
    case 'branch':
    case 'pr': return 'swap'
    case 'ci-run': return resource.card.detail.endsWith('running') ? 'clock' : 'check'
    case 'review': return 'check'
    case 'diff': return 'pencil'
    case 'terminal': return 'terminal'
    case 'agent':
    case 'mission': return 'message'
    case 'attention': return 'alert'
    case 'session': return resource.glyph === '◷' ? 'clock' : 'message'
    default: return resource.card.kind === 'diff' ? 'pencil' : 'info'
  }
}

const styles = stylex.create({
  // Phone hosts (NarrowLayoutContext): every Details action is a 44px touch target.
  touchHeight: { height: g.touchTarget },
  // The unavailable state's reason wraps onto its own row beneath the section title.
  headerWrap: { flexWrap: 'wrap', paddingBlock: s.xs },
  // The unreported-sections tail: its summary takes a full row, Why? and its panel start beneath it.
  unreportedTail: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.xs },
  root: { display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, fontFamily: t.fontSans, color: ink.fg },
  section: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  header: { minHeight: g.toolRow, display: 'flex', alignItems: 'center', gap: s.xs, flexShrink: 0, paddingInline: s.md },
  headerButton: { display: 'flex', alignItems: 'center', gap: s.xs, minWidth: 0, flexGrow: 1, height: g.toolRow, padding: s.zero, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'left', cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing, outlineOffset: g.focusInset } },
  // A heading with nothing to expand keeps its button (focus target) but reads as static text.
  headerButtonStatic: { cursor: 'default', ':hover': { color: ink.fgMuted } },
  // An unavailable heading sizes to its title so the availability copy follows it on the same row.
  headerButtonFit: { flexGrow: 0 },
  headerTitle: { color: ink.fg, fontWeight: t.weightSemibold, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  count: { fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted, flexShrink: 0 },
  body: { display: 'flex', flexDirection: 'column', gap: s.sm, paddingInlineStart: s.xxl, paddingInlineEnd: s.md, paddingBottom: s.md, minWidth: 0 },
  // A collapsed section keeps only a held read line in its body; with none, the body has no height.
  bodyCollapsed: { paddingBottom: s.zero },
  metaLine: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  // One plain sentence on its own row; its Why? starts beneath it.
  whyLine: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  whySummary: { flexBasis: '100%', minWidth: 0, overflowWrap: 'normal' },
  allHidden: { padding: s.md },
  chevronSpace: { width: g.icon, flexShrink: 0 },
  list: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  // Wraps in full under its fact row.
  factDetail: { margin: s.zero, paddingInline: s.sm, paddingBottom: s.xs, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  contentShell: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  content: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0 },
  skeletonHidden: { visibility: 'hidden' },
  skeletonRow: { position: 'relative', minWidth: 0 },
  skeletonInk: { visibility: 'visible', position: 'absolute', inset: s.zero, display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.sm, overflow: 'hidden', pointerEvents: 'none' },
  skeletonLabel: { display: 'flex', flexGrow: 1, minWidth: 0, overflow: 'hidden' },
})
