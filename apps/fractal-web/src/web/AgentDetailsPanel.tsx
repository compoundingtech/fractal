import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tab, TabList, TabPanel, Tabs } from 'react-aria-components'
import { RegistryContext } from '@effect/atom-react'
import { DetailsPane, DetailsWhy } from '@fractal/ui/assistant-ui'
import type { DetailsLayoutIntent, DetailsVariants } from '@fractal/ui/assistant-ui'
import { SystemNotice } from '@fractal/ui/assistant-ui/shell'
import type { SyncStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sync-presentation/sync-status.ts'
import { colorVars as c, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { PaneHeaderAccessoryHost } from '../../../../packages/fractal-ui/src/assistant-ui/workbench/PaneLayer.tsx'
import { CountPill } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarTabs.tsx'
import { useClockDeadline, useConversation, useDataSource, useGatewaySync, useNowSnapshot } from '../data/react.tsx'
import { syncLineNextChange } from '@fractal/ui/assistant-ui/sync-status'
import { earliestChange, formatRelative, nextRelativeChange } from '@fractal/ui/human-time/format'
import { panelSectionContributions } from '../extensions/build.ts'
import type { AgentPanelSectionContribution, AgentPanelSectionData, AgentPanelSectionQuery, DetailsSupplement } from '../extensions/contract.ts'
import { credentialExpiredMessage, credentialNotice, type Feed } from '../data/source.ts'
import { useNativeDetails, type NativeDetailsObservation } from './useNativeDetails.ts'
import { detailsWithSupplement, mergePanelSections, type SupplementedDetails } from './detailsSupplement.ts'
import { traceSubagentSection } from '../../../../packages/fractal-ui/src/assistant-ui/details/DetailsResources.tsx'
import type { OpenOperation, UxTelemetry } from '../telemetry/ux.ts'
import { AgentResourcesView, retainedResourceAtoms, useResourcesCount } from './AgentResourcesPanel.tsx'
import { capturedChanges } from './capturedChanges.ts'
import { ChangesInspector } from './ChangesInspector.tsx'
import { markStart } from './perfMarks.ts'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { makeWorkspaceKinds, workspaceKinds } from '../workspace/kinds.ts'

/** Deployed defaults, individually replaceable without ratifying exploration alternatives. */
export const liveDetailsVariants: DetailsVariants = {
  overview: 'O2', context: 'C3', spend: 'S2', empty: 'E3',
}
export type InspectorView = 'details' | 'changes' | 'resources'

/** Details is about the agent, never the things it works with (Resources). */
const aboutAgent: Readonly<Record<string, true>> = { Workspace: true, Attention: true }

export interface AgentDetailsPanelProps {
  readonly agentRef: string
  /** A number sizes a standalone panel; `dock` fills a pane's side panel, and `fill` flows in the phone's bottom sheet. */
  readonly width: number | 'fill' | 'dock'
  /** The host shell applies the light/dark tokens; the inspector carries no theme control of its own. */
  readonly scheme: 'dark' | 'light'
  readonly onClose: () => void
  readonly onOpen: (uri: string, split: boolean) => void
  readonly variants?: DetailsVariants
  /** Compiled-in panel readers; each reader has its own keyed hook boundary. */
  readonly sectionContributions?: readonly AgentPanelSectionContribution[]
  /** The shown tab when the host owns it; otherwise the panel uses the global last-used tab. */
  readonly view?: InspectorView
  readonly onViewChange?: (view: InspectorView) => void
  /** Reports the tab for the dock separator's name. Resources has no separator of its own yet. */
  readonly onSelectedViewChange?: (view: 'changes' | 'details') => void
  readonly ux?: UxTelemetry
}

/**
 * One fixed row: Details, Changes and Resources tabs, Details' status/section controls, then the dock's close.
 * In the phone sheet (`fill`) the sheet header owns the only close. Hidden tabs unmount, releasing reader demand.
 */
export function AgentDetailsPanel({ agentRef, width, onClose, onOpen, variants = liveDetailsVariants, sectionContributions = panelSectionContributions, view, onViewChange, onSelectedViewChange, ux }: AgentDetailsPanelProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const source = useDataSource()
  const registry = React.useContext(RegistryContext)
  // Hosts own the tab through the app-wide right panel setting; a host that does not keeps it for this mount.
  const [localView, setLocalView] = React.useState<InspectorView>('details')
  const selected = view ?? localView
  const [intent, setIntent] = React.useState<DetailsLayoutIntent>({ hiddenSections: [] })
  // Details portals its sync status and Visible sections into this row, as it does into a kit pane header.
  const [accessoryHost, setAccessoryHost] = React.useState<HTMLDivElement | null>(null)
  const [opening, setOpening] = React.useState<{ readonly operation: OpenOperation | undefined; readonly cache: 'retained' | 'cold' }>()
  const resourcesCount = useResourcesCount(agentRef)
  const changesCount = useChangesCount(agentRef)
  const select = (next: InspectorView) => {
    if (next === selected) return
    setLocalView(next)
    onViewChange?.(next)
    if (next !== 'resources') onSelectedViewChange?.(next)
    opening?.operation?.cancel()
    if (next === 'resources') {
      markStart('resources_panel', { key: agentRef })
      setOpening({ operation: ux?.beginOpen({ surface: 'resources' }), cache: registry.get(retainedResourceAtoms(source).resources(agentRef))._tag === 'Observed' ? 'retained' : 'cold' })
    } else setOpening(undefined)
  }
  // The remembered tab may not be the one the dock separator assumes at mount.
  React.useEffect(() => { if (selected !== 'resources') onSelectedViewChange?.(selected) }, [])
  return <aside aria-label="Agent inspector" data-testid="agent-details-panel" style={typeof width === 'number' ? { width } : undefined} {...stylex.props(styles.root, typeof width !== 'number' && styles.fillRoot)}>
    {/* data-testid="tab-group" lets Details return focus to the selected tab after Retry. */}
    <Tabs selectedKey={selected} onSelectionChange={key => select(key === 'changes' || key === 'resources' ? key : 'details')} data-testid="tab-group" {...stylex.props(styles.tabs)}>
      <div data-agent-inspector-tabs {...stylex.props(styles.row, narrow && styles.rowTouch)}>
        <TabList aria-label="Agent inspector views" {...stylex.props(styles.tabList)}>
          <Tab id="details" {...stylex.props(styles.tab, narrow && styles.tabTouch)}>Details</Tab>
          <Tab id="changes" aria-label={changesCount === 0 ? 'Changes' : `Changes, ${changesCount}`} {...stylex.props(styles.tab, narrow && styles.tabTouch)}>Changes{changesCount === 0 ? null : <CountPill count={changesCount} />}</Tab>
          <Tab id="resources" aria-label={resourcesCount === 0 ? 'Resources' : `Resources, ${resourcesCount}`} {...stylex.props(styles.tab, narrow && styles.tabTouch)}>Resources{resourcesCount === 0 ? null : <CountPill count={resourcesCount} />}</Tab>
        </TabList>
        {/* Only Details portals controls here; its seat is reserved up front so the tabs scroll instead of sitting under them. */}
        <div ref={setAccessoryHost} data-agent-inspector-accessory {...stylex.props(styles.accessory, selected === 'details' && (narrow ? styles.accessorySeatTouch : styles.accessorySeat))} />
        {width === 'fill' ? null : <Button aria-label="Close agent inspector" data-agent-inspector-close onPress={onClose} {...stylex.props(styles.close, narrow && styles.closeTouch)}><Icon name="x" /></Button>}
      </div>
      <TabPanel id="details" {...stylex.props(styles.panel)}>
        <PaneHeaderAccessoryHost value={accessoryHost ?? undefined}>
          {selected === 'details' && <AgentPanelData agentRef={agentRef} contributions={sectionContributions} tab="details">{(native, contributions) =>
            <NativeDetailsPane native={native} contributions={contributions} variants={variants} intent={intent} onIntentChange={setIntent} onOpen={onOpen} />
          }</AgentPanelData>}
        </PaneHeaderAccessoryHost>
      </TabPanel>
      <TabPanel id="changes" {...stylex.props(styles.panel)}><ChangesInspector agentRef={agentRef} width={width} /></TabPanel>
      <TabPanel id="resources" {...stylex.props(styles.panel)}>
        {selected === 'resources' && <AgentPanelData agentRef={agentRef} contributions={sectionContributions} tab="resources">{(native, contributions) => {
          const supplemented = projectContributions(native, contributions)
          const sections = mergePanelSections([], contributions.flatMap(data => data.sections))
          const contributedSections = contributions.some(data => data.supplement !== undefined)
            ? mergePanelSections(sections, [traceSubagentSection(supplemented.trace)]) : sections
          return <AgentResourcesView agentRef={agentRef} onOpen={onOpen} contributedSections={contributedSections} {...(opening?.operation === undefined ? {} : { opening: opening.operation })} {...(opening === undefined ? {} : { cache: opening.cache })} />
        }}</AgentPanelData>}
      </TabPanel>
    </Tabs>
  </aside>
}

/** Read off the conversation feed the panel already shares; a feed nothing has loaded yet shows no count. */
const useChangesCount = (agentRef: string): number => {
  const feed = useConversation(agentRef)
  return React.useMemo(() => feed._tag === 'Observed' ? capturedChanges(feed.value).length : 0, [feed])
}

type ContributionContent = (native: NativeDetailsObservation, data: readonly AgentPanelSectionData[]) => React.ReactNode

const projectContributions = (native: NativeDetailsObservation, contributions: readonly AgentPanelSectionData[]): SupplementedDetails => {
  const scopedRoot = native.trace.nativeSessionId._tag === 'Known' ? native.trace.nativeSessionId.value : undefined
  return contributions.reduce<SupplementedDetails>((current, contribution) => {
    if (contribution.supplement === undefined) return current
    const next = detailsWithSupplement({
      trace: current.trace, overview: current.overview, supplement: contribution.supplement,
      ...(native.harness === undefined ? {} : { harness: native.harness }),
      ...(scopedRoot === undefined ? {} : { nativeSessionId: scopedRoot }),
    })
    return { ...current, ...next }
  }, detailsWithSupplement({ trace: native.trace, overview: native.overview, ...(native.harness === undefined ? {} : { harness: native.harness }) }))
}

/** Readers are components, not a variable-length loop of hooks in the panel. */
const ContributionReader = ({ contribution, query, children }: {
  readonly contribution: AgentPanelSectionContribution
  readonly query: AgentPanelSectionQuery
  readonly children: (data: AgentPanelSectionData) => React.ReactNode
}) => children(contribution.view.useData(query))

const ContributionReaders = ({ contributions, query, data, children }: {
  readonly contributions: readonly AgentPanelSectionContribution[]
  readonly query: AgentPanelSectionQuery
  readonly data: readonly AgentPanelSectionData[]
  readonly children: (data: readonly AgentPanelSectionData[]) => React.ReactNode
}): React.ReactNode => {
  const [contribution, ...remaining] = contributions
  return contribution === undefined ? children(data) : <ContributionReader key={contribution.id} contribution={contribution} query={query}>{value =>
    <ContributionReaders contributions={remaining} query={query} data={[...data, value]}>{children}</ContributionReaders>
  }</ContributionReader>
}

const AgentPanelData = ({ agentRef, contributions, tab, children }: {
  readonly agentRef: string
  readonly contributions: readonly AgentPanelSectionContribution[]
  readonly tab: 'details' | 'resources'
  readonly children: ContributionContent
}) => {
  const native = useNativeDetails({ agentRef })
  const scopedRoot = native.trace.nativeSessionId._tag === 'Known' ? native.trace.nativeSessionId.value : undefined
  const query: AgentPanelSectionQuery = {
    agentRef,
    ...(scopedRoot === undefined ? {} : { nativeSessionId: scopedRoot }),
    canRefresh: native.nativeSessionId !== undefined,
    nativeRootPending: native.nativeRootPending,
    ...(native.incarnationId === undefined ? {} : { incarnationId: native.incarnationId }),
    ...(native.host === undefined ? {} : { host: native.host }),
    ...(native.harness === undefined ? {} : { harness: native.harness }),
  }
  const kinds = React.useMemo(() => contributions === panelSectionContributions ? workspaceKinds : makeWorkspaceKinds(contributions), [contributions])
  return <ContributionReaders contributions={kinds.sectionsFor({ _tag: 'Conversation', agent: agentRef }, tab)} query={query} data={[]}>{data => children(native, data)}</ContributionReaders>
}

/** Plain reasons for a refused or failed subagent-activity read; the source's own detail is diagnostic only. */
const supplementUnavailableReason: Record<Extract<Feed<unknown>, { _tag: 'Unavailable' }>['reason'], string> = {
  unauthorized: credentialExpiredMessage,
  ungranted: "This device doesn't have access to subagent activity.",
  unsupported: "Subagent activity isn't reported for this session.",
  failed: "Subagent activity couldn't be loaded.",
}
/** Why retained subagent activity is shown instead of a newer read. */
const supplementStaleReason: Record<Extract<Feed<unknown>, { _tag: 'Unavailable' }>['reason'], string> = {
  unauthorized: credentialExpiredMessage,
  ungranted: 'This device no longer has access to newer subagent activity, so earlier activity is shown.',
  unsupported: "Newer subagent activity isn't reported for this session, so earlier activity is shown.",
  failed: "The latest update couldn't be loaded, so earlier activity is shown.",
}

function NativeDetailsPane({ native, contributions, variants, intent, onIntentChange, onOpen }: {
  readonly native: NativeDetailsObservation
  readonly contributions: readonly AgentPanelSectionData[]
  readonly variants: DetailsVariants
  readonly intent: DetailsLayoutIntent
  readonly onIntentChange: (intent: DetailsLayoutIntent) => void
  readonly onOpen: (uri: string, split: boolean) => void
}) {
  const suppliedSections = mergePanelSections(native.sections.filter(section => aboutAgent[section.title]), contributions.flatMap(data => data.sections))
  const now = useNowSnapshot()
  const [requestedAt] = React.useState(now)
  const gateway = useGatewaySync()
  const supplements = contributions.flatMap(data => data.supplement === undefined ? [] : [data.supplement])
  const data = projectContributions(native, contributions)
  const status: SyncStatus = native.syncStatus ?? (native.state === 'loading'
    ? { _tag: 'Requested', since: requestedAt }
    : native.state === 'error'
      ? { _tag: 'Failed', cause: { _tag: 'Unknown' } }
      : native.stale
        ? gateway?.status._tag === 'Stale' ? gateway.status : { _tag: 'Stale', reason: { _tag: 'Unknown' }, ...(native.observedAt === undefined ? {} : { lastLiveAt: native.observedAt }) }
        : gateway?.status ?? { _tag: 'Live', since: native.observedAt ?? requestedAt })
  useClockDeadline(syncLineNextChange({ status, label: 'agent details', now, observedAt: native.observedAt ?? requestedAt }))
  if (native.credentialNotice) return <SystemNotice kind="error" connection title={native.credentialNotice} />
  const { mission: _mission, ...overview } = data.overview
  return <div {...stylex.props(styles.content)}>
    {supplements.map((supplement, index) => <SupplementNotice key={index} supplement={supplement} harness={native.harness} />)}
    {data.accountingNote !== undefined && <div data-details-accounting-note {...stylex.props(styles.note)}>
      <span data-details-accounting-summary {...stylex.props(styles.noteSummary)}>{data.accountingNote.summary}</span>
      <DetailsWhy explains="Agent and total spend" reasons={data.accountingNote.reasons} />
    </div>}
    <DetailsPane overview={overview} sections={suppliedSections} trace={{ ...data.trace, subagents: { _tag: 'Unknown', reason: 'not-covered', detail: 'Per-subagent spend is shown in Resources.' } }} now={now}
      variants={variants} state={native.state} intent={intent} onIntentChange={onIntentChange}
      onOpen={onOpen} onRetry={native.onRetry} syncStatus={status}
      hasPriorContent={native.state !== 'loading' && native.state !== 'error'}
      {...(native.observedAt === undefined ? {} : { observedAt: native.observedAt })}
      {...(native.spendRead === undefined ? {} : { spendRead: native.spendRead })}
      {...(native.sessionFailure === undefined ? {} : { sessionFailure: native.sessionFailure })}
      sessionPending={native.sessionPending}
    />
  </div>
}

const SupplementNotice = ({ supplement, harness }: { readonly supplement: Feed<DetailsSupplement>; readonly harness: string | undefined }) => {
  const now = useNowSnapshot()
  const receivedAt = supplement._tag === 'Observed' && supplement.freshness === 'stale' ? supplement.value.receivedAtMs : undefined
  useClockDeadline(earliestChange(now, [receivedAt === undefined ? undefined : nextRelativeChange(receivedAt, now)]))
  const activity = credentialNotice(supplement) ? { summary: credentialExpiredMessage, reasons: [] } : harness !== 'omp' ? undefined
    : supplement._tag === 'Waiting' ? { summary: 'Loading subagent activity…', reasons: [] }
      : supplement._tag === 'Unavailable' ? { summary: "Subagent activity isn't available for this session.", reasons: [supplementUnavailableReason[supplement.reason]] }
        : supplement.freshness === 'stale' ? {
          summary: `Subagent activity may be out of date${receivedAt === undefined ? '' : ` (last updated ${formatRelative(receivedAt, now)})`}.`,
          reasons: [supplement.error === undefined ? 'An update is still loading, so earlier activity is shown.' : supplementStaleReason[supplement.error.reason]],
        } : undefined
  return activity === undefined ? null : <div data-details-supplement data-details-supplement-state={supplement._tag} {...stylex.props(styles.note)}>
    <span role="status" data-details-supplement-summary {...stylex.props(styles.noteSummary)}>{activity.summary}</span>
    <DetailsWhy explains="Subagent activity" reasons={activity.reasons} />
  </div>
}

const styles = stylex.create({
  root: { flexShrink: 0, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, overflow: 'hidden', backgroundColor: c.canvas, color: c.fg },
  fillRoot: { width: '100%', flexGrow: 1 },
  tabs: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  // Fixed height in every state: Details' status folds by the accessory's width (`pane-header`), never wraps.
  // The accessory never shrinks below its controls' seat; the tab list gives way and scrolls, so nothing paints over a tab.
  row: { display: 'flex', alignItems: 'center', gap: s.xs, height: g.controlLg, flexShrink: 0, boxSizing: 'border-box', paddingInline: s.sm, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: c.border, minWidth: 0, containerType: 'inline-size', containerName: 'pane-header' },
  rowTouch: { height: `calc(${g.touchTarget} + 2 * ${s.xs})` },
  tabList: { display: 'flex', alignItems: 'center', gap: s.xs2, minWidth: 0, flexShrink: 1, overflowX: 'auto', overscrollBehaviorX: 'contain', scrollbarWidth: 'none' },
  tab: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0, height: g.controlSm, paddingInline: s.sm, borderRadius: r.control, color: c.fgMuted, fontSize: t.metaSize, whiteSpace: 'nowrap', cursor: 'pointer', outline: 'none', ':hover': { color: c.fg }, ':is([data-selected])': { backgroundColor: c.rowActive, color: c.fg }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusInset } },
  // Below 280px of header content (a 280px phone sheet) touch tabs tighten so more of the strip shows beside Details' controls.
  tabTouch: { height: g.touchTarget, paddingInline: { default: s.lg, '@container pane-header (width < 280px)': s.sm } },
  accessory: { marginInlineStart: 'auto', display: 'flex', justifyContent: 'flex-end', alignItems: 'center', gap: s.xs2, flexGrow: 1, flexShrink: 1, flexBasis: 0, alignSelf: 'stretch', minWidth: 0, containerType: 'inline-size', containerName: 'pane-header' },
  // Details' narrowest fold: the status glyph and Visible sections, each one control wide (PaneSyncStatus, DetailsPane).
  accessorySeat: { minWidth: `calc(2 * ${g.controlSm} + ${s.xs2})` },
  accessorySeatTouch: { minWidth: `calc(2 * ${g.touchTarget} + ${s.xs2})` },
  close: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, height: g.controlSm, flexShrink: 0, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgMuted, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusInset } },
  closeTouch: { width: g.touchTarget, height: g.touchTarget },
  panel: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0, outline: 'none', containerType: 'inline-size', containerName: 'pane-mount' },
  content: { display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, flexGrow: 1, overflowY: 'auto' },
  // A wrapping line: the summary takes a full row, Why? and its panel start beneath it.
  note: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.xs, margin: 0, paddingInline: s.sm, paddingBlock: s.xs, fontSize: t.metaSize, color: c.fgMuted, flexShrink: 0 },
  noteSummary: { flexBasis: '100%', minWidth: 0, overflowWrap: 'normal' },
})
