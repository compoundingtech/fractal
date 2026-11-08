import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Fragment } from 'react'
import { colorVars as c, spaceVars as s } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { SystemNotice } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { useDataSource } from '../data/react.tsx'
import type * as AlertsTabModule from '../web/AlertsTab.tsx'
import { ConversationPaneFallback } from '../web/ConversationPaneFallback.tsx'
import type * as ConversationPaneModule from '../web/ConversationPane.tsx'
import { conversationChunk, conversationPane, loadedTerminalDetail, retryConversationPane, terminalChunk, terminalDetail, type LazyConversationPane } from './chunks.ts'
import type { SurfaceProps } from './kinds.ts'
import { subjectAgent } from './subject.ts'

/** The first recovery re-imports in this document; after a second rejected import, reload with fresh feature assets. Drafts and tabs are already persisted. */
const LazyChunkRecoveryAction = (props: { readonly chunk: { readonly recovery: 'retry' | 'reload'; readonly reload: () => Promise<void> }; readonly onRetry: () => void }) =>
  props.chunk.recovery === 'reload'
    ? <button type="button" onClick={() => void props.chunk.reload()}>Reload and retry</button>
    : <button type="button" onClick={props.onRetry}>Try again</button>

class ConversationPaneCodeBoundary extends React.Component<React.ComponentProps<typeof ConversationPaneModule.ConversationPane>> {
  state: { readonly failed: boolean; readonly Pane: LazyConversationPane } = { failed: false, Pane: conversationPane() }
  static getDerivedStateFromError() {
    return { failed: true }
  }
  render() {
    if (this.state.failed) return <div role="alert" aria-hidden={!this.props.visible} {...stylex.props(styles.notice)}>
      <p>This conversation view could not load.</p>
      <LazyChunkRecoveryAction chunk={conversationChunk} onRetry={() => this.setState({ failed: false, Pane: retryConversationPane() })} />
    </div>
    const Pane = this.state.Pane
    return <React.Suspense fallback={<ConversationPaneFallback agentName={this.props.agentName} visible={this.props.visible} />}>
      <Pane {...this.props} />
    </React.Suspense>
  }
}

// Load the tab on demand, not with the shell.
const makeAlertsTab = () => React.lazy(() => import('../web/AlertsTab.tsx').then(module => ({ default: module.AlertsTab })))
const AlertsTab = makeAlertsTab()
export class AlertsTabCodeBoundary extends React.Component<React.ComponentProps<typeof AlertsTabModule.AlertsTab>> {
  state = { failed: false, Tab: AlertsTab }
  static getDerivedStateFromError() { return { failed: true } }
  render() {
    if (this.state.failed) return <div role="alert" {...stylex.props(styles.notice)}>
      <p>Alerts could not load.</p>
      <button type="button" onClick={event => {
        // The retry control unmounts during Suspense; keep keyboard focus on the stable selected tab.
        event.currentTarget.closest('[data-sidebar-tabs]')?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus()
        this.setState({ failed: false, Tab: makeAlertsTab() })
      }}>Try again</button>
    </div>
    const Tab = this.state.Tab
    return <React.Suspense fallback={<SystemNotice kind="loading" title="Loading alerts" />}>
      <Tab {...this.props} />
    </React.Suspense>
  }
}

/** The terminal grid, fonts and native key encoder stay outside the eager shell; a fresh lazy identity on retry avoids React's permanently rejected module cache. */
class TerminalCodeBoundary extends React.Component<{ readonly terminalRef: `terminal/${string}`; readonly shown: boolean; readonly connected: boolean }> {
  state = { failed: false, Detail: terminalDetail(() => terminalChunk.load()) }
  static getDerivedStateFromError() { return { failed: true } }
  render() {
    if (this.state.failed) return <div role="alert">
      <p>This terminal view could not load.</p>
      <LazyChunkRecoveryAction chunk={terminalChunk} onRetry={() => this.setState({ failed: false, Detail: terminalDetail(() => terminalChunk.retry()) })} />
    </div>
    const Detail = loadedTerminalDetail() ?? this.state.Detail
    return <React.Suspense fallback={<SystemNotice kind="loading" title="Loading terminal" />}>
      <Detail address={{ ref: this.props.terminalRef, presentation: 'detail' }} visibility={this.props.shown ? 'visible' : 'hidden'} keepConnected={this.props.connected} />
    </React.Suspense>
  }
}

const AlertDetailsPane = React.lazy(() => import('../web/AlertDetailsPane.tsx').then(module => ({ default: module.AlertDetailsPane })))
const MissionsPane = React.lazy(() => import('../missions/MissionsPane.tsx').then(module => ({ default: module.MissionsPane })))
const MissionDetailView = React.lazy(() => import('../missions/MissionDetailView.tsx').then(module => ({ default: module.MissionDetailView })))
const DocumentPane = React.lazy(() => import('../documents/DocumentPane.tsx').then(module => ({ default: module.DocumentPane })))
const DeclarationPane = React.lazy(() => import('../web/DeclarationDialog.tsx').then(module => ({ default: module.DeclarationPane })))
const makeUsagePane = () => React.lazy(() => import('../monitor/UsagePane.tsx'))
class UsageCodeBoundary extends React.Component {
  state = { failed: false, Pane: makeUsagePane() }
  static getDerivedStateFromError() { return { failed: true } }
  render() {
    if (this.state.failed) return <div role="alert" {...stylex.props(styles.paneNotice)}>
      <p>This usage view could not load.</p>
      <button type="button" onClick={() => this.setState({ failed: false, Pane: makeUsagePane() })}>Try again</button>
    </div>
    const Pane = this.state.Pane
    return <React.Suspense fallback={<div {...stylex.props(styles.paneNotice)}><SystemNotice kind="loading" title="Loading usage" /></div>}>
      <Pane />
    </React.Suspense>
  }
}

const Loading = ({ title }: { readonly title: string }) => <div {...stylex.props(styles.paneNotice)}><SystemNotice kind="loading" title={title} /></div>

/**
 * One agent's tab, on either face. The conversation stays retained behind its terminal. The terminal
 * mounts when first shown, then stays connected until the tab is left, so the toggle only reveals it.
 */
export const AgentSurface = ({ subject, name, visible, terminal, onOpenTool, onOpen, ux }: SurfaceProps) => {
  const source = useDataSource()
  const agent = subjectAgent(subject) ?? ''
  // The resource manager owns this demand. Passing the atom stops the pane acquiring it again.
  const interest = React.useMemo(() => source.conversationInterest?.(agent), [source, agent])
  const [terminalMounted, setTerminalMounted] = React.useState(terminal && visible)
  if (terminal && visible && !terminalMounted) setTerminalMounted(true)
  const conversationShown = visible && !terminal
  const terminalShown = visible && terminal
  return <>
    <div data-wf-shown-pane={conversationShown ? '' : undefined} {...stylex.props(styles.face, conversationShown ? styles.faceShown : styles.faceHidden)}>
      <ConversationPaneCodeBoundary agentRef={agent} agentName={name ?? agent.split('/').at(-1) ?? 'Conversation'} onOpenTool={onOpenTool} onOpenDocument={onOpen} ux={ux} visible={conversationShown} interest={interest} />
    </div>
    {terminalMounted ? <section aria-label="Terminal" data-testid="terminal-pane" data-terminal-shown={terminalShown} tabIndex={-1} inert={!terminalShown} aria-hidden={!terminalShown} {...stylex.props(styles.face, styles.terminal, terminalShown ? styles.faceShown : styles.faceHidden)}>
      <TerminalCodeBoundary terminalRef={`terminal/${agent.slice('agent/'.length)}`} shown={terminalShown} connected={visible} />
    </section> : null}
  </>
}

export const InboxSurface = ({ onOpen }: SurfaceProps) => <section aria-label="Inbox" {...stylex.props(styles.column)}>
  <AlertsTabCodeBoundary onOpen={onOpen} onSelect={id => onOpen(`alert/${id}`)} />
</section>

/** Answering gives this tab to the next alert, and with none left closes it. */
export const AlertSurface = ({ subject, onOpen, onReplace, onClose, onBackToAlerts, ux }: SurfaceProps) => <React.Suspense fallback={<Loading title="Loading alert" />}>
  {subject._tag === 'Alert' ? <AlertDetailsPane id={subject.id} onOpenAlert={id => onReplace(`alert/${id}`)} onOpenRef={onOpen} onClose={onClose} onBack={onBackToAlerts} {...(ux === undefined ? {} : { ux })} /> : <Fragment />}
</React.Suspense>

/** The Missions screen, or one mission as its own tab: header, banner, plan, agents, activity, linked work and actions. */
export const MissionsSurface = ({ subject, visible, onOpen }: SurfaceProps) => <React.Suspense fallback={<Loading title="Loading missions" />}>
  {subject._tag === 'Mission'
    ? <MissionDetailView missionRef={subject.ref} visible={visible} onOpen={ref => onOpen(ref)} />
    : <MissionsPane onOpenAgent={onOpen} onFocusMission={ref => onOpen(ref ?? 'missions')} />}
</React.Suspense>

export const UsageSurface = () => <UsageCodeBoundary />

export const DocumentSurface = ({ subject, onOpen }: SurfaceProps) => <React.Suspense fallback={<Loading title="Loading document" />}>
  {subject._tag === 'Document' ? <DocumentPane name={subject.ref} onOpen={onOpen} /> : <Fragment />}
</React.Suspense>

export const DeclarationSurface = ({ subject }: SurfaceProps) => <React.Suspense fallback={<Loading title="Loading declaration" />}>
  {subject._tag === 'Declaration' ? <DeclarationPane subject={subject.ref} /> : <Fragment />}
</React.Suspense>

/** Any ref no kind claims still opens: the tab is durable, and a real surface can arrive for it later. */
export const ResourceSurface = ({ subject }: SurfaceProps) => <div {...stylex.props(styles.paneNotice)}>
  <SystemNotice kind="empty" title={subject._tag === 'Resource' ? subject.ref : 'Resource'} detail="This web client has no dedicated view for this resource yet" />
</div>

export const UnsupportedSurface = ({ subject }: SurfaceProps) => <div {...stylex.props(styles.paneNotice)}>
  <SystemNotice kind="empty" title="View not available here" detail={`This web client does not render ${subject._tag === 'Unsupported' ? subject.key : 'this view'} yet. The tab stays so it still reaches your other clients`} />
</div>

const styles = stylex.create({
  // Above retained surfaces.
  paneNotice: { position: 'relative', zIndex: 2 },
  notice: { position: 'relative', zIndex: 2, margin: 'auto', padding: s.section, color: c.fgMuted },
  column: { display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0, minHeight: 0 },
  // Identical geometry on either face: revealing one must not invalidate its layout.
  face: { position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, backgroundColor: c.canvas },
  faceShown: { zIndex: 1 },
  faceHidden: { contentVisibility: 'hidden' },
  terminal: { overflow: 'hidden', outline: 'none' },
})
