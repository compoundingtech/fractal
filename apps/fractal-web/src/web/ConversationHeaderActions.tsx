import * as React from 'react'
import { RegisteredPortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/PortalContainer.tsx'
import { cancelMeasure, markStart } from './perfMarks.ts'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { RegistryContext, useAtom, useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import { prefetchWorkspaceMenus, WorkspaceMenuBody } from './workspaceMenus.tsx'
import { SidePanel } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SidePanel.tsx'
import { ChromeControl } from '../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl.tsx'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { Tooltip } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { SyncFailureDetails } from '../../../../packages/fractal-ui/src/assistant-ui/sync-presentation/SyncLine.tsx'
import { colorVars as c, typeVars as t, spaceVars as s, geometryVars as g } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useConversation, useConversationSync, useDataSource } from '../data/react.tsx'
import { unavailable, waiting, type DataSource, type Feed } from '../data/source.ts'
import type { ResourcePage } from '../resources/agent/model.ts'
import { systemEventsPreference } from './conversationPreferences.ts'
import type { OpenOperation, UxTelemetry } from '../telemetry/ux.ts'
import { DeclarationDialog } from './DeclarationDialog.tsx'

const unsupportedResources = Atom.make<Feed<ResourcePage>>(unavailable({ reason: 'unsupported', detail: '' }))
const unopenedResources = Atom.make<Feed<ResourcePage>>(waiting)

interface ResourceAtoms {
  readonly resources: (ref: string) => Atom.Atom<Feed<ResourcePage>>
  readonly queue: DataSource['subjectReads']['agentQueue']['feed']
}
const resourceAtomsBySource = new WeakMap<DataSource, ResourceAtoms>()

/** Retain authoritative reads only after explicit intent, never merely selecting an agent. */
export const retainedResourceAtoms = (source: DataSource): ResourceAtoms => {
  let atoms = resourceAtomsBySource.get(source)
  if (atoms === undefined) {
    atoms = {
      resources: Atom.family((ref: string) => Atom.make(get => get(source.resources?.byAgent(ref) ?? unsupportedResources)).pipe(Atom.setIdleTTL('60 seconds'))),
      queue: Atom.family((ref: string) => Atom.make(get => get(source.subjectReads.agentQueue.feed(ref))).pipe(Atom.setIdleTTL('60 seconds'))),
    }
    resourceAtomsBySource.set(source, atoms)
  }
  return atoms
}

/** Conversation controls stay in the responsive header; the workspace supplies the desktop dock. */
export const ConversationHeaderActions = ({ agentRef, ux, resourcesDock }: {
  readonly agentRef: string
  readonly ux?: UxTelemetry
  readonly resourcesDock?: { readonly host: HTMLElement | null; readonly open: boolean; readonly onOpenChange: (open: boolean) => void }
}) => {
  const source = useDataSource()
  const narrow = React.useContext(NarrowLayoutContext)
  const registry = React.useContext(RegistryContext)
  const atoms = retainedResourceAtoms(source)
  const prefetch = () => {
    prefetchWorkspaceMenus()
    registry.mount(atoms.resources(agentRef))()
    registry.mount(atoms.queue(agentRef))()
  }
  const [showSystemEvents, setShowSystemEvents] = useAtom(systemEventsPreference)
  const [standaloneResourcesOpen, setStandaloneResourcesOpen] = React.useState(false)
  const resourcesOpen = resourcesDock?.open ?? standaloneResourcesOpen
  const [declarationOpen, setDeclarationOpen] = React.useState(false)
  const [syncDetailsOpen, setSyncDetailsOpen] = React.useState(false)
  const conversation = useConversation(agentRef)
  const sync = useConversationSync(agentRef)
  const retainedFailure = conversation._tag === 'Observed' && conversation.value.items.length > 0 && sync?.status._tag === 'Failed'
  const [opening, setOpening] = React.useState<OpenOperation>()
  const [cache, setCache] = React.useState<'retained' | 'cold'>('cold')
  // A recovered failure must not leave the details open, or the next failure reopens them unasked.
  React.useEffect(() => { if (!retainedFailure) setSyncDetailsOpen(false) }, [retainedFailure])
  // The header control and the narrow overflow item share this one transition, so reopening an
  // already open panel from the menu never restarts its open measurement.
  const changeResourcesOpen = (open: boolean) => {
    if (open === resourcesOpen) return
    if (open) {
      markStart('resources_panel', { key: agentRef })
      setCache(registry.get(atoms.resources(agentRef))._tag === 'Observed' ? 'retained' : 'cold')
      setOpening(ux?.beginOpen({ surface: 'resources' }))
    }
    else { cancelMeasure('resources_panel'); opening?.cancel(); setOpening(undefined) }
    if (resourcesDock === undefined) setStandaloneResourcesOpen(open)
    else resourcesDock.onOpenChange(open)
  }
  const shellCommit = React.useCallback((node: HTMLElement | null) => {
    if (node === null || opening === undefined) return
    opening.phase('shell')
    return () => { opening.cancel(); cancelMeasure('resources_panel') }
  }, [opening])
  const panelId = React.useId()
  /** Closing releases UI demand, but the intent-created wrapper retains its poll for 60 seconds. */
  const resources = useAtomValue(source.resources === undefined ? unsupportedResources
    : resourcesOpen ? atoms.resources(agentRef) : unopenedResources)
  const disabledReason = resources._tag === 'Unavailable'
    ? resources.reason === 'ungranted' ? 'Read access to agent resources has not been granted.'
      : resources.reason === 'unsupported' ? 'This connection does not support agent resources.' : undefined
    : undefined
  const panel = resourcesOpen ? <SidePanel ref={shellCommit} id={panelId} placement={resourcesDock === undefined ? 'overlay' : 'dock'} aria-label="Agent resources" heading="Resources" actions={<Aria.Button aria-label="Close resources" onPress={() => changeResourcesOpen(false)} {...stylex.props(styles.button, narrow && styles.touchItem)}>Close resources</Aria.Button>} onClose={() => changeResourcesOpen(false)}>
    <WorkspaceMenuBody label="Agent resources" size={styles.contents}>
      {({ ResourceContents }) => <ResourceContents agentRef={agentRef} resources={resources} opening={opening} cache={cache} />}
    </WorkspaceMenuBody>
  </SidePanel> : null
  return <div {...stylex.props(styles.actions)}>
    <span {...stylex.props(styles.secondary, narrow && styles.secondaryHidden)}>
      <ChromeControl icon="folder" label="Resources" active={resourcesOpen} expanded={resourcesOpen} {...(resourcesOpen ? { controls: panelId } : {})} onPrefetch={prefetch} onPress={() => changeResourcesOpen(!resourcesOpen)} {...(disabledReason === undefined ? {} : { disabledReason })} />
    </span>
    {resourcesDock === undefined ? panel : resourcesDock.host === null ? null : <RegisteredPortal container={resourcesDock.host}>{panel}</RegisteredPortal>}
    <span {...stylex.props(styles.secondary, narrow && styles.secondaryHidden)}>
      <Tooltip label="View KDL">
        <Aria.Button aria-label="View KDL" onPress={() => setDeclarationOpen(true)} {...stylex.props(styles.button, styles.declarationButton)}>
          <span {...stylex.props(styles.declarationIcon)}><Icon name="checklist" /></span>
          <span {...stylex.props(styles.declarationLabel)}>View KDL</span>
        </Aria.Button>
      </Tooltip>
    </span>
    <DeclarationDialog subject={agentRef} isOpen={declarationOpen} onOpenChange={setDeclarationOpen} />
    <Aria.ModalOverlay isOpen={syncDetailsOpen && retainedFailure} onOpenChange={setSyncDetailsOpen} isDismissable {...stylex.props(styles.overlay)}>
      <Aria.Modal {...stylex.props(styles.modal)}>
        <Aria.Dialog aria-label="Sync details for conversation" {...stylex.props(styles.dialog)}>
          <header {...stylex.props(styles.heading)}><strong>Sync details</strong><Aria.Button aria-label="Close sync details" onPress={() => setSyncDetailsOpen(false)} {...stylex.props(styles.button, narrow && styles.touchItem)}>Close</Aria.Button></header>
          {sync !== undefined && <SyncFailureDetails status={sync.status} />}
        </Aria.Dialog>
      </Aria.Modal>
    </Aria.ModalOverlay>
    <Aria.MenuTrigger>
      <ChromeControl icon="gear" label="Conversation menu" onPrefetch={prefetchWorkspaceMenus} />
      <Aria.Popover {...stylex.props(styles.popover)}>
        <WorkspaceMenuBody label="Conversation settings" size={narrow ? styles.pendingTouchMenu : styles.pendingMenu}>
          {({ ConversationMenu }) => <ConversationMenu showSystemEvents={showSystemEvents} onShowSystemEventsChange={setShowSystemEvents} disabledReason={disabledReason} retainedFailure={retainedFailure}
            onAction={key => { if (key === 'resources') changeResourcesOpen(true); else if (key === 'declaration') setDeclarationOpen(true); else if (key === 'sync-details') setSyncDetailsOpen(true) }} />}
        </WorkspaceMenuBody>
      </Aria.Popover>
    </Aria.MenuTrigger>
  </div>
}

const styles = stylex.create({
  actions: { display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0 },
  secondary: { display: { default: 'inline-flex', '@container thread-header (max-width: 440px)': 'none' }, flexShrink: 0 },
  secondaryHidden: { display: 'none' },
  declarationButton: { height: 28, minWidth: 28, justifyContent: 'center', paddingInline: { default: 8, '@container thread-header (max-width: 680px)': 0 } },
  declarationIcon: { display: { default: 'none', '@container thread-header (max-width: 680px)': 'inline-flex' } },
  declarationLabel: { display: { default: 'inline', '@container thread-header (max-width: 680px)': 'none' } },
  button: { display: 'inline-flex', alignItems: 'center', gap: s.xs, whiteSpace: 'nowrap', flexShrink: 0, padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` }, ':disabled': { opacity: 0.5, cursor: 'default' } },
  overlay: { position: 'fixed', inset: 0, zIndex: 100, display: 'flex', justifyContent: 'flex-end', backgroundColor: 'rgba(0, 0, 0, 0.4)' },
  modal: { width: 380, maxWidth: 'calc(100vw - 24px)', height: '100%', backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  dialog: { display: 'flex', flexDirection: 'column', height: '100%', outline: 'none' },
  heading: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: s.lg, borderBottom: `1px solid ${c.border}` },
  contents: { flex: '1 1 0', minHeight: 0, padding: s.lg, overflowY: 'auto' },
  popover: { backgroundColor: c.raised, color: c.fg, border: `1px solid ${c.border}`, borderRadius: 6, fontFamily: t.fontSans, fontSize: t.metaSize, zIndex: 100 },
  pendingMenu: { width: 200, height: 120 },
  pendingTouchMenu: { width: 200, height: 140 },
  touchItem: { minHeight: g.touchTarget, display: 'flex', flexDirection: 'column', justifyContent: 'center', boxSizing: 'border-box' },
})
