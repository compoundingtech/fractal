import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { useDrag } from 'react-aria'
import { Button as AriaButton, Tooltip as AriaTooltip, TooltipTrigger } from 'react-aria-components'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { Icon, type IconName } from '../composition/Icons'
import { SidebarAgentRow } from '../sidebar/SidebarAgentRow'
import { ResourceCardV1, ResourceHoverV1, WholeResourceMeta } from '../resources/ResourceCardV1'
import { AgentHoverCard } from '../sidebar/SidebarAgentRow'
import { AGENT_DRAG_MIME, agentPaneKey } from '../workbench/agent-drag'
import type { DetailsResource, DetailsSection, DetailsState, DetailsValue } from './model'
import type { TraceSeries, TraceSubagent, TraceUnknownReason } from './trace-series'
import { DetailsInk } from './DetailsSkeleton'
import { TraceCoverageNote } from './DetailsCharts'

export type DetailsResourceForm = 'R1' | 'R2' | 'R3'
export type DetailsEmptyRule = 'E1' | 'E2' | 'E3'
export interface DetailsResourcesProps {
  readonly state: DetailsState
  readonly sections: readonly DetailsSection[]
  /** Breakdown facts are independent of the st resource snapshot. */
  readonly trace: TraceSeries
  readonly now: number
  readonly form?: DetailsResourceForm
  readonly empty?: DetailsEmptyRule
  readonly onOpen: (uri: string, split: boolean) => void
  /** Synced layout intent (q45): section names the parent keeps hidden. */
  readonly hiddenSections?: readonly string[]
  readonly styles?: stylex.StyleXStyles
}
export const DetailsLabelContext=React.createContext('')

/** One pane-level column; collapse, hover and skeletons stay local, never synced. */
export function DetailsResources({ state, sections: suppliedSections, trace, now, form = 'R1', empty = 'E1', onOpen, hiddenSections = [], styles: callerStyles }: DetailsResourcesProps) {
  const labelPrefix=React.useContext(DetailsLabelContext)
  // Pending geometry derives only from the host's supplied observations.
  const observedSubagents = suppliedSections.find(section => section.title === 'Subagents')
  const hasObservedSubagents = observedSubagents?.items._tag === 'Known' && observedSubagents.items.value.length > 0
  const [collapsed, setCollapsed] = React.useState<Readonly<Record<string, boolean>>>({})
  const hidden = React.useMemo(() => new Set(hiddenSections), [hiddenSections])
  const toggle = React.useCallback((title: string) => setCollapsed(current => ({ ...current, [title]: !current[title] })), [])
  const open = React.useCallback((uri: string, split: boolean) => onOpen(agentPaneKey(uri), split), [onOpen])
  const sections = state === 'error' ? [] : suppliedSections
    .map(section => section.title === 'Subagents' ? { ...section, ...traceSubagentSection(trace) } : section.title === 'History' ? { ...section, ...traceHistorySection(trace) } : section)
    .filter(section => state !== 'new' || empty !== 'E3' || (section.items._tag === 'Known' && section.items.value.length > 0) || (section.title === 'Subagents' && hasObservedSubagents))
    .filter(section => visible(section.items, empty) || (section.title === 'Subagents' && hasObservedSubagents))
    .filter(section => !hidden.has(section.title))
  return <div data-details-state={state} data-details-form={form} data-details-empty={empty} {...stylex.props(styles.root, callerStyles)}>
    {sections.map(section => {
      const isCollapsed = collapsed[section.title] === true
      return <section key={section.title} aria-label={`${labelPrefix}${section.title}`} {...stylex.props(styles.section)}>
        <div {...stylex.props(styles.header)}>
          <AriaButton aria-expanded={!isCollapsed} onPress={() => toggle(section.title)} {...stylex.props(styles.headerButton)}>
            <Icon name={isCollapsed?'chevron-right':'chevron-down'}/>
            <span data-details-heading {...stylex.props(styles.headerTitle)}>{section.title}</span>
            {section.title !== 'Subagents' && section.items._tag === 'Known' && section.items.value.length > 0 && (state === 'loading'
              ? <span aria-hidden="true" data-details-skeleton-count {...stylex.props(styles.count)}><DetailsInk ch={String(section.items.value.length).length} /></span>
              : <span {...stylex.props(styles.count)}>{section.items.value.length}</span>)}
          </AriaButton>
        </div>
        {!isCollapsed && <div {...stylex.props(styles.body)}>
          <SectionContent section={section} observedSubagents={observedSubagents} trace={trace} now={now} form={form} empty={empty} open={open} pending={state === 'loading'} />
        </div>}
      </section>
    })}
    {sections.length === 0 && state !== 'error' && !(state === 'new' && empty === 'E3') && <p {...stylex.props(styles.metaLine, styles.allHidden)}>Every section is empty or hidden.</p>}
  </div>
}

/** E1 hides empty and Unknown sections; E3 hides only empty; E2 shows everything. */
function visible(items: DetailsValue<readonly DetailsResource[]>, rule: DetailsEmptyRule): boolean {
  if (rule === 'E2') return true
  if (items._tag === 'Unknown') return rule === 'E3'
  return items.value.length > 0
}


/** E3 explains a section without known values once, in human language, never as an Unknown value. */
const humanSubagentReasons: Readonly<Record<TraceUnknownReason, string>> = {
  'no-provider': 'This source doesn’t report subagent breakdowns.',
  'not-covered': 'Subagent breakdowns aren’t tracked for this session.',
  'not-attributed': 'Subagent breakdowns aren’t attributed to this session.',
  'out-of-retention': 'Subagent breakdowns are no longer retained.',
  loading: 'Subagent breakdowns are still loading.',
  failed: 'Subagent breakdowns couldn’t be loaded.',
}
const humanSectionReasons: Readonly<Partial<Record<DetailsSection['title'], string>>> = {
  Reviews: 'This source doesn’t report reviews.',
  'Declared bindings': 'This source doesn’t report declared bindings.',
  History: 'This source doesn’t report session history.',
}
function unavailableReason(section: DetailsSection, trace: TraceSeries, empty: DetailsEmptyRule): string {
  if (section.items._tag === 'Unknown' && section.unavailableReason !== undefined) return section.unavailableReason
  const subagentReason = trace.subagents._tag === 'Unknown' ? trace.subagents.reason : 'no-provider'
  if (empty !== 'E3') return section.title === 'Subagents' ? humanSubagentReasons[subagentReason] : `This source doesn’t report ${section.title.toLowerCase()} yet.`
  return section.title === 'Subagents' ? humanSubagentReasons[subagentReason] : humanSectionReasons[section.title] ?? `This source doesn’t report ${section.title.toLowerCase()}.`
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

/** No native-session-to-st-agent identity is supplied; traced sessions stay neutral. */
export function traceSubagentSection(trace: TraceSeries): DetailsSection {
  if (trace.subagents._tag !== 'Known') return { title: 'Subagents', items: { _tag: 'Unknown' } }
  return {
    title: 'Subagents',
    items: { _tag: 'Known', value: trace.subagents.value.map(subagent => {
      const label = subagent.label._tag === 'Known' ? subagent.label.value : subagent.sessionId
      const detail = traceSubagentDetail(subagent)
      return {
        uri: `session:${subagent.sessionId}`,
        glyph: '◇',
        card: { kind: 'file', label, detail: `Traced session · ${detail}`, fileCount: { _tag: 'Unknown' }, files: [] },
      }
    }) },
  }
}
/** Missing descendant facts are omitted, never fabricated parentage or timing. */
const traceSubagentDetail = (subagent: TraceSubagent): string => [
  `$${subagent.usd.toFixed(2)}`,
  `${subagent.requests} requests`,
  subagent.model._tag === 'Known' ? subagent.model.value : undefined,
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
      label:`Incarnation · ${incarnation.nativeSessionId}`,
      detail:`Started ${historyTimestamp.format(incarnation.startedMs)} UTC${incarnation.endedMs===null?'':` · Ended ${historyTimestamp.format(incarnation.endedMs)} UTC`} · $${incarnation.usd.toFixed(2)} · ${incarnation.compactions} ${incarnation.compactions === 1 ? 'compaction' : 'compactions'}`,
      fileCount:{_tag:'Unknown'},
      files:[],
    },
  }))}}
}

/** The loading shadow and populated body share markup, wrapping and intrinsic metadata fit. */
function SectionContent({ section, observedSubagents, trace, now, form, empty, open, pending }: {
  section: DetailsSection; observedSubagents: DetailsSection | undefined; trace: TraceSeries; now: number;
  form: DetailsResourceForm; empty: DetailsEmptyRule; open: (uri: string, split: boolean) => void; pending: boolean
}) {
  const hasObserved = observedSubagents?.items._tag === 'Known' && observedSubagents.items.value.length > 0
  const showBreakdown = section.title !== 'Subagents' || visible(section.items, empty)
  const hasUnavailableNote = showBreakdown && section.items._tag === 'Unknown'
  const missingStatus = section.title === 'Subagents' && observedSubagents?.items._tag === 'Known'
    && observedSubagents.items.value.some(resource => resource.agent?.status === 'unobserved')
  const partialNote = hasUnavailableNote ? undefined
    : missingStatus ? "Status isn't reported for st resources."
    : empty === 'E3' ? missingLineCountsNote(section) : undefined
  return <div aria-hidden={pending || undefined} inert={pending} data-details-skeleton={pending ? section.title : undefined} {...stylex.props(styles.contentShell)}>
    <div {...stylex.props(styles.content, pending && styles.skeletonHidden)}>
      {section.title === 'Subagents' && hasObserved && observedSubagents !== undefined && <>
        <SkeletonLine pending={pending}><p {...stylex.props(styles.metaLine)}>Observed st resources</p></SkeletonLine>
        <SectionItems section={observedSubagents} now={now} form={form} open={open} pending={pending} />
      </>}
      {showBreakdown && <>
        {section.title === 'Subagents' && <SkeletonLine pending={pending}><p {...stylex.props(styles.metaLine)}>Traced-session breakdown</p></SkeletonLine>}
        {section.title === 'Subagents' && trace.subagents._tag === 'Known' && <SkeletonLine pending={pending}><p data-details-subagent-count {...stylex.props(styles.metaLine)}>{trace.subagentCount._tag === 'Known' ? `Showing ${trace.subagents.value.length} of ${trace.subagentCount.value} traced sessions` : `Showing ${trace.subagents.value.length} traced sessions · Total count is unreported`}</p></SkeletonLine>}
        {section.items._tag === 'Unknown'
          ? <SkeletonLine pending={pending}><p data-details-reason={section.title} {...stylex.props(styles.metaLine, styles.reason)}>{unavailableReason(section, trace, empty)}</p></SkeletonLine>
          : section.items.value.length === 0
            ? <SkeletonLine pending={pending}><p {...stylex.props(styles.metaLine)}>{section.title === 'Subagents' && trace.coverage._tag !== 'Complete' ? 'No traced sessions retained.' : 'None'}</p></SkeletonLine>
            : <SectionItems section={section} now={now} form={form} open={open} pending={pending} />}
      </>}
      {partialNote !== undefined && <SkeletonLine pending={pending}><p data-details-partial-note {...stylex.props(styles.metaLine, styles.reason)}>{partialNote}</p></SkeletonLine>}
      {section.title === 'Subagents' && <SkeletonLine pending={pending}><TraceCoverageNote trace={trace} /></SkeletonLine>}
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

function wrapsIdentity(resource: DetailsResource): boolean {
  return resource.uri.startsWith('branch:') || resource.uri.startsWith('pr:') || resource.uri.startsWith('mission:')
}

function SectionItems({ section, now, form, open, pending }: { section: DetailsSection; now: number; form: DetailsResourceForm; open: (uri: string, split: boolean) => void; pending: boolean }) {
  const [expanded,setExpanded]=React.useState(false)
  const items = section.items._tag === 'Known' ? section.items.value : []
  const capped = form === 'R3' && items.length > 3
  const shown = capped && !expanded ? items.slice(0, 3) : items
  // An absent agent status must not fall back to the card's projected status text.
  const row = (resource: DetailsResource, child: React.ReactNode) => <SkeletonRow key={resource.uri} pending={pending} label={resource.card.label} meta={resource.agent === undefined ? resource.card.detail : resource.agent.status === 'unobserved' ? undefined : resource.agent.statusLabel}>{child}</SkeletonRow>
  return <>
    {form === 'R2' && <div role="list" aria-label={section.title} {...stylex.props(styles.dense)}>{shown.map(resource => row(resource, <DenseRow resource={resource} now={now} open={open} />))}</div>}
    {form !== 'R2' && shown.map(resource => row(resource, <ResourceEntry resource={resource} now={now} open={open} />))}
    {capped && <SkeletonLine pending={pending}><AriaButton onPress={()=>setExpanded(value=>!value)} {...stylex.props(styles.splitButton)}>{expanded?'Show fewer':`+${items.length - shown.length} more`}</AriaButton></SkeletonLine>}
  </>
}

/** Terminal previews are observed lines; diff and file previews reuse the canonical hover. */
function HoverContent({ resource, now }: { resource: DetailsResource; now: number }) {
  if (resource.agent !== undefined) return <AgentHoverCard row={resource.agent} reportedStatusOnly now={now} />
  return <div {...stylex.props(styles.hover)}>
    <strong {...stylex.props(styles.hoverTitle)}>{resource.card.label}</strong>
    {resource.terminal !== undefined
      ? <pre {...stylex.props(styles.terminalPreview)}>{resource.terminal.join('\n')}</pre>
      : <ResourceHoverV1 resource={resource.card} file={resource.file} />}
  </div>
}

function ResourceEntry({ resource, now, open }: { resource: DetailsResource; now: number; open: (uri: string, split: boolean) => void }) {
  const paneKey = agentPaneKey(resource.uri)
  const getItems = React.useCallback(() => [{ [AGENT_DRAG_MIME]: paneKey, 'text/plain': paneKey }], [paneKey])
  const { dragProps } = useDrag({ getItems, getAllowedDropOperations: () => ['copy'] })
  const splitPress=React.useRef(false)
  const capture=(event:React.MouseEvent|React.PointerEvent|React.KeyboardEvent)=>{splitPress.current=event.metaKey||event.ctrlKey}
  if (resource.agent !== undefined) {
    return <div {...dragProps} onPointerDownCapture={capture} onKeyDownCapture={capture} onClickCapture={capture} {...stylex.props(styles.agentEntry)}>
      <div {...stylex.props(styles.agentRow)}><SidebarAgentRow item={resource.agent} variant="SR-2" reportedStatusOnly now={now} onOpen={row => open(agentPaneKey(row.ref), splitPress.current)} /></div>
      <AriaButton aria-label={`Open ${resource.card.label} in split`} onPress={() => open(paneKey, true)} {...stylex.props(styles.splitButton, styles.inlineSplit)}>Open in split</AriaButton>
    </div>
  }
  return <div {...dragProps} onPointerDownCapture={capture} onKeyDownCapture={capture} onClickCapture={capture} {...stylex.props(styles.entry)}>
    <ResourceCardV1 resource={resource.card} variant="RC-1" titlePriority titleLines={wrapsIdentity(resource) ? 'wrap' : 1} icon={resourceIcon(resource)} onOpen={() => open(paneKey, splitPress.current)} />
  </div>
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
    case 'terminal': return 'chevron-right'
    case 'agent':
    case 'mission': return 'message'
    case 'attention': return 'alert'
    case 'session': return resource.card.label.startsWith('Incarnation · ') ? 'clock' : 'message'
    default: return resource.card.kind === 'diff' ? 'pencil' : 'attach'
  }
}

/** Dense rows retain aligned metadata while branch and PR identities get two lines. */
function DenseRow({ resource, now, open }: { resource: DetailsResource; now: number; open: (uri: string, split: boolean) => void }) {
  const paneKey = agentPaneKey(resource.uri)
  const getItems=React.useCallback(()=>[{[AGENT_DRAG_MIME]:paneKey,'text/plain':paneKey}],[paneKey])
  const {dragProps}=useDrag({getItems,getAllowedDropOperations:()=>['copy']})
  const meta = resource.terminal !== undefined ? 'terminal'
    : resource.agent !== undefined ? resource.agent.status === 'unobserved' ? undefined : resource.agent.statusLabel
    : resource.file !== undefined
      ? [resource.file.added._tag === 'Known' && resource.file.added.value !== 0 ? `+${resource.file.added.value}` : '', resource.file.removed._tag === 'Known' && resource.file.removed.value !== 0 ? `−${resource.file.removed.value}` : ''].filter(Boolean).join(' ')
    : resource.card.detail
  const isSubagent = resource.agent !== undefined || resource.uri.startsWith('session:')
  return <div data-resource-card role="listitem" {...dragProps} {...stylex.props(styles.denseEntry)}><TooltipTrigger delay={150}>
    <AriaButton data-details-resource-row aria-label={`Open ${resource.card.label}`} onPress={event => open(paneKey, event.metaKey || event.ctrlKey)} {...stylex.props(styles.denseRow)}>
      <span data-details-resource-icon={resourceIcon(resource)} {...stylex.props(styles.resourceIcon)}><Icon name={resourceIcon(resource)} /></span>
      <span data-details-resource-title data-resource-title-lines={wrapsIdentity(resource) ? 'wrap' : 1} {...stylex.props(styles.denseLabel, wrapsIdentity(resource) && styles.wrappedLabel)}>{resource.card.label}</span>
      <WholeResourceMeta numeric={resource.file !== undefined} styles={styles.denseMeta}>{meta}</WholeResourceMeta>
    </AriaButton>
    <AriaTooltip placement="left" {...stylex.props(styles.tooltip)}><HoverContent resource={resource} now={now} /></AriaTooltip>
  </TooltipTrigger>{isSubagent && <AriaButton aria-label={`Open ${resource.card.label} in split`} onPress={() => open(paneKey, true)} {...stylex.props(styles.denseSplit)}><Icon name="panel" /></AriaButton>}</div>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, fontFamily: t.fontSans, color: ink.fg },
  section: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  header: { minHeight: g.toolRow, display: 'flex', alignItems: 'center', gap: s.xs, flexShrink: 0, paddingInline: s.md },
  headerButton: { display: 'flex', alignItems: 'center', gap: s.xs, minWidth: 0, flexGrow: 1, height: g.toolRow, padding: s.zero, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'left', cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outline: `2px solid ${border.focusBorder}`, outlineOffset: -2 } },
  headerTitle: { color: ink.fg, fontWeight: t.weightSemibold, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  count: { fontFamily: t.fontMono, fontSize: t.microSize, color: ink.fgFaint, flexShrink: 0 },
  body: { display: 'flex', flexDirection: 'column', gap: s.sm, paddingInline: s.xxl, paddingBottom: s.md, minWidth: 0 },
  metaLine: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  reason: { color: ink.fgSoft },
  allHidden: { padding: s.md },
  entry: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  agentEntry: { display: 'flex', alignItems: 'center', gap: s.xs, minWidth: 0 },
  agentRow: { flex: 1, minWidth: 0 },
  inlineSplit: { flexShrink: 0, whiteSpace: 'nowrap' },
  splitButton: { height: g.controlSm, paddingInline: s.sm, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgFaint, fontFamily: t.fontSans, fontSize: t.microSize, borderRadius: r.sm, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': { outline: `2px solid ${border.focusBorder}` } },
  dense: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  denseEntry: { position: 'relative', display: 'flex', alignItems: 'center', minWidth: 0 },
  denseSplit: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: g.toolRow, height: g.toolRow, padding: s.zero, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': { outline: `2px solid ${border.focusBorder}`, outlineOffset: -2 } },
  denseRow: { display: 'flex', alignItems: 'center', gap: s.md, width: '100%', minWidth: 0, flex: 1, boxSizing: 'border-box', minHeight: g.toolRow, paddingInline: s.sm, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'left', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, borderRadius: r.sm, color: ink.fg }, ':focus-visible': { outline: `2px solid ${border.focusBorder}`, outlineOffset: -2 } },
  resourceIcon: { display: 'inline-flex', flexShrink: 0, color: ink.fgMuted },
  denseLabel: { flexBasis: 0, flexGrow: 1, flexShrink: 0, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  // Identity labels wrap in full (no clamp) with balanced lines: a long branch never leaves one orphaned word beside its badge.
  wrappedLabel: { whiteSpace: 'normal', overflow: 'visible', overflowWrap: 'anywhere', textWrap: 'balance' },
  denseMeta: { marginInlineStart: 'auto', color: ink.fgMuted, fontSize: t.microSize },
  contentShell: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  content: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0 },
  skeletonHidden: { visibility: 'hidden' },
  skeletonRow: { position: 'relative', minWidth: 0 },
  skeletonInk: { visibility: 'visible', position: 'absolute', inset: s.zero, display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.sm, overflow: 'hidden', pointerEvents: 'none' },
  skeletonLabel: { display: 'flex', flexGrow: 1, minWidth: 0, overflow: 'hidden' },
  hover: { maxWidth: 'min(360px, calc(100vw - 24px))', display: 'flex', flexDirection: 'column', gap: s.xs },
  hoverTitle: { fontSize: t.metaSize, fontWeight: t.weightMedium },
  tooltip: { padding: s.md, backgroundColor: surface.raised, color: ink.fg, borderRadius: r.md, borderWidth: 1, borderStyle: 'solid', borderColor: border.borderStrong, fontSize: t.metaSize, lineHeight: t.metaLeading, zIndex: 10 },
  terminalPreview: { margin: s.zero, padding: s.sm, backgroundColor: surface.codeBg, borderRadius: r.sm, fontFamily: t.fontMono, fontSize: t.metaSize, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
})
