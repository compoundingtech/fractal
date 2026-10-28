import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, Heading, Modal, ModalOverlay, Tab, TabList, TabPanel, Tabs } from 'react-aria-components'
import { DetailsPane, DetailsWhy } from '@fractal/ui/assistant-ui'
import type { DetailsLayoutIntent, DetailsVariants } from '@fractal/ui/assistant-ui'
import type { SyncStatus } from '../../../../packages/fractal-ui/src/assistant-ui/sync-presentation/sync-status.ts'
import { colorVars as c, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { PaneHeaderAccessoryHost } from '../../../../packages/fractal-ui/src/assistant-ui/workbench/PaneLayer.tsx'
import { useClockDeadline, useGatewaySync, useNowSnapshot } from '../data/react.tsx'
import { syncLineNextChange } from '@fractal/ui/assistant-ui/sync-status'
import { earliestChange, formatRelative, nextRelativeChange } from '@fractal/ui/human-time/format'
import { useDetailsSections, useDetailsSupplement } from '../extensions/build.ts'
import type { UseDetailsSupplement } from '../extensions/contract.ts'
import type { Feed } from '../data/source.ts'
import { useNativeDetails } from './useNativeDetails.ts'
import { detailsWithSupplement } from './detailsSupplement.ts'
import { ChangesInspector } from './ChangesInspector.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'

/** Deployed defaults, individually replaceable without ratifying exploration alternatives. */
export const liveDetailsVariants: DetailsVariants = {
  overview: 'O2', context: 'C3', spend: 'S2', resources: 'R1', empty: 'E3',
}
type InspectorView = 'changes' | 'details'

export interface AgentDetailsPanelProps {
  readonly agentRef: string
  /** A number sizes a standalone panel; `dock` fills DockedSplit, and `fill` flows in the narrow sheet. */
  readonly width: number | 'fill' | 'dock'
  /** The host shell applies the light/dark tokens; the inspector carries no theme control of its own. */
  readonly scheme: 'dark' | 'light'
  readonly onClose: () => void
  readonly onOpen: (uri: string, split: boolean) => void
  readonly variants?: DetailsVariants
  /** The composition's subagent-activity feed; defaults to the build-time extension. Fixed for the panel's lifetime. */
  readonly detailsSupplement?: UseDetailsSupplement
  /** Reports each user tab selection; the panel still owns its selection, starting on Details at every mount. */
  readonly onSelectedViewChange?: (view: InspectorView) => void
}

/**
 * One fixed row: Changes and Details tabs, Details' status/section controls, then the dock's close.
 * In the phone sheet (`fill`) the sheet header owns the only close. Hidden tabs unmount, releasing reader demand.
 */
export function AgentDetailsPanel({ agentRef, width, onClose, onOpen, variants = liveDetailsVariants, detailsSupplement = useDetailsSupplement, onSelectedViewChange }: AgentDetailsPanelProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const [selected, setSelected] = React.useState<InspectorView>('details')
  const [intent, setIntent] = React.useState<DetailsLayoutIntent>({ hiddenSections: [] })
  // Details portals its sync status and Visible sections into this row, as it does into a kit pane header.
  const [accessoryHost, setAccessoryHost] = React.useState<HTMLDivElement | null>(null)
  return <aside aria-label="Agent inspector" data-testid="agent-details-panel" style={typeof width === 'number' ? { width } : undefined} {...stylex.props(styles.root, typeof width !== 'number' && styles.fillRoot)}>
    {/* data-testid="tab-group" lets Details return focus to the selected tab after Retry. */}
    <Tabs selectedKey={selected} onSelectionChange={key => { const view = key === 'changes' ? 'changes' : 'details'; setSelected(view); onSelectedViewChange?.(view) }} data-testid="tab-group" {...stylex.props(styles.tabs)}>
      <div data-agent-inspector-tabs {...stylex.props(styles.row, narrow && styles.rowTouch)}>
        <TabList aria-label="Agent inspector views" {...stylex.props(styles.tabList)}>
          <Tab id="changes" {...stylex.props(styles.tab, narrow && styles.tabTouch)}>Changes</Tab>
          <Tab id="details" {...stylex.props(styles.tab, narrow && styles.tabTouch)}>Details</Tab>
        </TabList>
        <div ref={setAccessoryHost} data-agent-inspector-accessory {...stylex.props(styles.accessory)} />
        {width === 'fill' ? null : <Button aria-label="Close agent inspector" data-agent-inspector-close onPress={onClose} {...stylex.props(styles.close, narrow && styles.closeTouch)}><Icon name="x" /></Button>}
      </div>
      <TabPanel id="changes" {...stylex.props(styles.panel)}><ChangesInspector agentRef={agentRef} width={width} /></TabPanel>
      <TabPanel id="details" {...stylex.props(styles.panel)}>
        <PaneHeaderAccessoryHost value={accessoryHost ?? undefined}>
          <NativeDetailsPane agentRef={agentRef} variants={variants} intent={intent} onIntentChange={setIntent} onOpen={onOpen} useSupplement={detailsSupplement} />
        </PaneHeaderAccessoryHost>
      </TabPanel>
    </Tabs>
  </aside>
}

/** Plain reasons for a refused or failed subagent-activity read; the source's own detail is diagnostic only. */
const supplementUnavailableReason: Record<Extract<Feed<unknown>, { _tag: 'Unavailable' }>['reason'], string> = {
  ungranted: "This device doesn't have access to subagent activity.",
  unsupported: "Subagent activity isn't reported for this session.",
  failed: "Subagent activity couldn't be loaded.",
}
/** Why retained subagent activity is shown instead of a newer read. */
const supplementStaleReason: Record<Extract<Feed<unknown>, { _tag: 'Unavailable' }>['reason'], string> = {
  ungranted: 'This device no longer has access to newer subagent activity, so earlier activity is shown.',
  unsupported: "Newer subagent activity isn't reported for this session, so earlier activity is shown.",
  failed: "The latest update couldn't be loaded, so earlier activity is shown.",
}

function NativeDetailsPane({ agentRef, variants, intent, onIntentChange, onOpen, useSupplement }: {
  readonly agentRef: string
  readonly variants: DetailsVariants
  readonly intent: DetailsLayoutIntent
  readonly onIntentChange: (intent: DetailsLayoutIntent) => void
  readonly onOpen: (uri: string, split: boolean) => void
  readonly useSupplement: UseDetailsSupplement
}) {
  const narrow = React.useContext(NarrowLayoutContext)
  const native = useNativeDetails({ agentRef })
  const protectedSections = useDetailsSections({ agentRef })
  const suppliedSections = native.sections.map(section => protectedSections.find(contribution => contribution.title === section.title) ?? section)
  const [inspectedUri, setInspectedUri] = React.useState<string>()
  const portalContainer = React.useRef<HTMLDivElement>(null)
  const resource = suppliedSections.flatMap(section => section.items._tag === 'Known' ? section.items.value : []).find(item => item.uri === inspectedUri)
  const now = useNowSnapshot()
  const [requestedAt] = React.useState(now)
  const gateway = useGatewaySync()
  const scopedRoot = native.trace.nativeSessionId._tag === 'Known' ? native.trace.nativeSessionId.value : undefined
  const supplement = useSupplement({
    agentRef,
    ...(scopedRoot === undefined ? {} : { nativeSessionId: scopedRoot }),
    canRefresh: native.nativeSessionId !== undefined,
    nativeRootPending: native.nativeRootPending,
    ...(native.incarnationId === undefined ? {} : { incarnationId: native.incarnationId }),
    ...(native.host === undefined ? {} : { host: native.host }),
    ...(native.harness === undefined ? {} : { harness: native.harness }),
  })
  const data = detailsWithSupplement({
    trace: native.trace, overview: native.overview, supplement,
    ...(native.harness === undefined ? {} : { harness: native.harness }),
    ...(scopedRoot === undefined ? {} : { nativeSessionId: scopedRoot }),
  })
  const subagent = inspectedUri?.startsWith('session:') && data.trace.subagents._tag === 'Known'
    ? data.trace.subagents.value.find(child => `session:${child.sessionId}` === inspectedUri) : undefined
  const inspection = resource !== undefined ? { label: resource.card.label, detail: resource.card.detail }
    : subagent === undefined ? undefined : {
      label: subagent.label._tag === 'Known' ? subagent.label.value : 'Subagent',
      detail: `Session ${subagent.sessionId} · ${subagent.requests} requests · $${subagent.usd.toFixed(2)} · ${subagent.model._tag === 'Known' ? subagent.model.value : 'model not reported'}`,
    }
  const status: SyncStatus = native.syncStatus ?? (native.state === 'loading'
    ? { _tag: 'Requested', since: requestedAt }
    : native.state === 'error'
      ? { _tag: 'Failed', cause: { _tag: 'Unknown' } }
      : native.stale
        ? gateway?.status._tag === 'Stale' ? gateway.status : { _tag: 'Stale', reason: { _tag: 'Unknown' }, ...(native.observedAt === undefined ? {} : { lastLiveAt: native.observedAt }) }
        : gateway?.status ?? { _tag: 'Live', since: native.observedAt ?? requestedAt })
  const receivedAt = supplement._tag === 'Observed' && supplement.freshness === 'stale' ? supplement.value.receivedAtMs : undefined
  useClockDeadline(earliestChange(now, [
    syncLineNextChange({ status, label: 'agent details', now, observedAt: native.observedAt ?? requestedAt }),
    receivedAt === undefined ? undefined : nextRelativeChange(receivedAt, now),
  ]))
  const activity = native.harness !== 'omp' ? undefined
    : supplement._tag === 'Waiting' ? { summary: 'Loading subagent activity…', reasons: [] }
      : supplement._tag === 'Unavailable' ? { summary: "Subagent activity isn't available for this session.", reasons: [supplementUnavailableReason[supplement.reason]] }
        : supplement.freshness === 'stale' ? {
          summary: `Subagent activity may be out of date${receivedAt === undefined ? '' : ` (last updated ${formatRelative(receivedAt, now)})`}.`,
          reasons: [supplement.error === undefined ? 'An update is still loading, so earlier activity is shown.' : supplementStaleReason[supplement.error.reason]],
        } : undefined
  return <div ref={portalContainer} {...stylex.props(styles.content)}>
    {activity !== undefined && <div data-details-supplement data-details-supplement-state={supplement._tag} {...stylex.props(styles.note)}>
      <span role="status" data-details-supplement-summary {...stylex.props(styles.noteSummary)}>{activity.summary}</span>
      <DetailsWhy explains="Subagent activity" reasons={activity.reasons} />
    </div>}
    {data.accountingNote !== undefined && <div data-details-accounting-note {...stylex.props(styles.note)}>
      <span data-details-accounting-summary {...stylex.props(styles.noteSummary)}>{data.accountingNote.summary}</span>
      <DetailsWhy explains="Agent and total spend" reasons={data.accountingNote.reasons} />
    </div>}
    <DetailsPane overview={data.overview} sections={suppliedSections} trace={data.trace} now={now}
      variants={variants} state={native.state} intent={intent} onIntentChange={onIntentChange}
      onOpen={(uri, split) => {
        if (uri.startsWith('details:') || uri.startsWith('session:')) setInspectedUri(uri)
        else onOpen(uri, split)
      }} onRetry={native.onRetry} syncStatus={status}
      hasPriorContent={native.state !== 'loading' && native.state !== 'error'}
      {...(native.observedAt === undefined ? {} : { observedAt: native.observedAt })}
      {...(native.spendRead === undefined ? {} : { spendRead: native.spendRead })}
      {...(native.sessionFailure === undefined ? {} : { sessionFailure: native.sessionFailure })}
      sessionPending={native.sessionPending}
    />
    <ModalOverlay isOpen={inspection !== undefined} onOpenChange={open => { if (!open) setInspectedUri(undefined) }} isDismissable UNSTABLE_portalContainer={portalContainer.current ?? undefined} {...stylex.props(styles.overlay)}>
      <Modal {...stylex.props(styles.modal)}><Dialog aria-label="Reported resource details" {...stylex.props(styles.inspection)}>
        <Heading slot="title">{inspection?.label}</Heading>
        <p {...stylex.props(styles.inspectionText)}>{inspection?.detail}</p>
        <p {...stylex.props(styles.inspectionText)}>Reported details only; this resource can't be opened from here.</p>
        <Button onPress={() => setInspectedUri(undefined)} {...stylex.props(styles.dialogClose, narrow && styles.dialogCloseTouch)}>Close</Button>
      </Dialog></Modal>
    </ModalOverlay>
  </div>
}

const styles = stylex.create({
  root: { flexShrink: 0, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, overflow: 'hidden', backgroundColor: c.canvas, color: c.fg },
  fillRoot: { width: '100%', flexGrow: 1 },
  tabs: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  // Fixed height in every state: Details' status folds by this row's width (`pane-header`), never wraps.
  row: { display: 'flex', alignItems: 'center', gap: s.xs, height: g.controlLg, flexShrink: 0, boxSizing: 'border-box', paddingInline: s.sm, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: c.border, minWidth: 0, containerType: 'inline-size', containerName: 'pane-header' },
  rowTouch: { height: `calc(${g.touchTarget} + 2 * ${s.xs})` },
  tabList: { display: 'flex', alignItems: 'center', gap: s.xs2, minWidth: 0, flexShrink: 0 },
  tab: { display: 'inline-flex', alignItems: 'center', height: g.controlSm, paddingInline: s.sm, borderRadius: r.control, color: c.fgMuted, fontSize: t.metaSize, whiteSpace: 'nowrap', cursor: 'pointer', outline: 'none', ':hover': { color: c.fg }, ':is([data-selected])': { backgroundColor: c.rowActive, color: c.fg }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '-2px' } },
  tabTouch: { height: g.touchTarget, paddingInline: s.lg },
  accessory: { marginInlineStart: 'auto', display: 'flex', alignItems: 'center', gap: s.xs2, flexShrink: 0, alignSelf: 'stretch', minWidth: 0 },
  close: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, height: g.controlSm, flexShrink: 0, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fgMuted, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover, color: c.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '-2px' } },
  closeTouch: { width: g.touchTarget, height: g.touchTarget },
  panel: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0, outline: 'none', containerType: 'inline-size', containerName: 'pane-mount' },
  content: { display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, flexGrow: 1, overflowY: 'auto' },
  // A wrapping line: the summary takes a full row, Why? and its panel start beneath it.
  note: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: s.xs, margin: 0, paddingInline: s.sm, paddingBlock: s.xs, fontSize: t.metaSize, color: c.fgMuted, flexShrink: 0 },
  noteSummary: { flexBasis: '100%', minWidth: 0, overflowWrap: 'normal' },
  overlay: { position: 'fixed', inset: 0, zIndex: 100, display: 'flex', alignItems: 'center', justifyContent: 'center', backgroundColor: 'rgba(0, 0, 0, 0.4)' },
  modal: { maxWidth: 'min(520px, 90vw)', maxHeight: '90vh', overflowY: 'auto', backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  inspection: { backgroundColor: c.canvas, color: c.fg, padding: s.lg, outline: 'none' },
  inspectionText: { whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  // A plain text action at dense height; the phone sheet raises it to the 44px touch floor.
  dialogClose: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', height: g.controlSm, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.control, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', outline: 'none', ':hover': { backgroundColor: c.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '2px' } },
  dialogCloseTouch: { minWidth: g.touchTarget, height: g.touchTarget, paddingInline: s.lg },
})
