import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { SidebarAgentRow } from './assistant-ui/sidebar/SidebarAgentRow'
import type { AgentStatus, SidebarAgentRow as Row } from './assistant-ui/sidebar/model'
import { ThreadHeader } from './assistant-ui/composition/Shell'
import { ConnectionStatus, SystemNotice, type ConnectionState } from './assistant-ui/composition/SystemState'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { darkTheme, liveAccentTheme } from './assistant-ui/embrace-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { borderVars as border, surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t, geometryVars as g } from './assistant-ui/composition-tokens.stylex'

const now = Date.parse('2026-01-15T12:00:00Z')
const minutes = (count: number) => now - count * 60_000
/** The roster as last observed before the connection dropped: the app marks every row stale. */
const agent = (name: string, host: string, status: AgentStatus, since: number): Row => ({
  ref: `agent/${name}`, id: `agent/${name}`, title: name, host, status, statusLabel: status, freshness: 'stale', harness: 'omp',
  statusSince: minutes(since), lastActivityAt: minutes(since), lastTurn: { _tag: 'Known', kind: 'turn-completed', at: minutes(since) },
  childrenKnown: false, children: [], usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' },
})
const rows: readonly Row[] = [
  agent('fractal-web', 'dev5', 'working', 12),
  agent('st-gateway', 'dev3', 'idle', 95),
  agent('ui-design', 'dev5', 'waiting', 4),
  agent('hypermerge', 'dev3', 'working', 31),
  agent('nix-infra', 'dev4', 'idle', 240),
  agent('pr-review', 'dev4', 'waiting', 18),
]
const countdown = 'Next try in 2s · attempt 1'

/**
 * The workspace as the app composes its system states: one amber band under the thread header owns the gateway
 * connection; the roster (last observed, stale), Missions (empty), Changes (loading) and the terminal (ended, with
 * Retry) keep quiet one-line rows and never repeat the connection. Reconnect and Retry are live for the plays.
 */
function SystemStatesStory({ scheme, connection: initial }: { scheme: Scheme; connection: ConnectionState }) {
  const [connection, setConnection] = React.useState<ConnectionState>(initial)
  const [terminal, setTerminal] = React.useState<'ended' | 'loading'>('ended')
  const reconnect = () => setConnection('reconnecting')
  return <main aria-label="Workspace" data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'dark' && darkTheme, liveAccentTheme, scheme === 'light' && lightTheme)}>
    <aside aria-label="Agents" {...stylex.props(styles.sidebar)}>
      <header {...stylex.props(styles.brand)}><strong>Fractal</strong></header>
      <nav aria-label="Agent roster" {...stylex.props(styles.roster)}>
        {rows.map(row => <SidebarAgentRow key={row.ref} item={row} now={now} active={row.ref === 'agent/fractal-web'} onOpen={() => {}} />)}
      </nav>
      <SystemNotice scope="rail" kind="stale" connection title="Last observed roster" detail="Live updates pending" />
    </aside>
    <section aria-label="Agent workspace" {...stylex.props(styles.workspace)}>
      <ThreadHeader folder="dev5" title="fractal-web" status="working" statusLabel="working" statusSince={minutes(12)} freshness="stale" now={now} terminalAvailable panelOpen drawerOpen onTogglePanel={() => {}} onToggleDrawer={() => {}} />
      <ConnectionStatus state={connection} text={countdown} {...(connection === 'offline' || connection === 'reconnecting' ? { onReconnect: reconnect } : {})} />
      <div {...stylex.props(styles.body)}>
        <section aria-label="Missions" {...stylex.props(styles.missions)}>
          <header {...stylex.props(styles.paneHeader)}>Missions</header>
          <div {...stylex.props(styles.paneBody)}>
            <SystemNotice kind="empty" title="No missions" />
          </div>
        </section>
        <aside aria-label="Changes" {...stylex.props(styles.changes)}>
          <SystemNotice kind="loading" title="Loading captured changes" />
        </aside>
      </div>
      <section aria-label="Terminal drawer" {...stylex.props(styles.drawer)}>
        {terminal === 'ended'
          ? <SystemNotice kind="error" title="Terminal session ended" detail="The shell exited with code 137" onRetry={() => setTerminal('loading')} />
          : <SystemNotice kind="loading" title="Loading terminal" />}
      </section>
    </section>
  </main>
}

const meta = {
  title: 'Fractal UI/System states',
  component: SystemStatesStory,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', connection: 'offline' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, connection: { options: ['live', 'offline', 'reconnecting', 'stale', 'failed'], control: 'radio' } },
} satisfies Meta<typeof SystemStatesStory>
export default meta
type Story = StoryObj<typeof meta>

const band = (canvasElement: HTMLElement) => canvasElement.querySelectorAll('[data-connection-state]')
/** One band under the thread header, none in the sidebar; every pane keeps its own quiet row. */
const oneBand = (state: ConnectionState) => async ({ canvasElement }: { canvasElement: HTMLElement }) => {
  await expect(band(canvasElement)).toHaveLength(1)
  const workspace = within(canvasElement).getByRole('region', { name: 'Agent workspace' })
  await expect(workspace.querySelector(`[data-connection-state="${state}"]`)).not.toBeNull()
  await expect(canvasElement.querySelector('aside[aria-label="Agents"] [data-connection-state]')).toBeNull()
  await expect(canvasElement.querySelectorAll('[data-system-state]')).toHaveLength(4)
  await expect(canvasElement.querySelector('[data-system-state="stale"]')).toHaveTextContent('Last observed roster')
  await expect(canvasElement.querySelector('[data-system-state="error"]')).toHaveTextContent('Terminal session ended. The shell exited with code 137')
}

/** Offline: the band names it once and offers Reconnect; the roster row says only what it still shows. */
export const ConnectionBanner: Story = { play: async context => {
  await oneBand('offline')(context)
  await expect(band(context.canvasElement)[0]).toHaveTextContent('You are offline')
} }
export const ConnectionBannerLight: Story = { ...ConnectionBanner, args: { scheme: 'light' } }
/** Reconnecting: the band carries the host's countdown. */
export const Reconnecting: Story = { args: { connection: 'reconnecting' }, play: async context => {
  await oneBand('reconnecting')(context)
  await expect(band(context.canvasElement)[0]).toHaveTextContent(countdown)
} }
/** Failed: no automatic retry is pending, so the band offers no Reconnect. */
export const ConnectionFailed: Story = { args: { connection: 'failed' }, play: async context => {
  await oneBand('failed')(context)
  await expect(within(context.canvasElement).queryByRole('button', { name: 'Reconnect' })).toBeNull()
} }
/** Live: no band at all; the panes' own rows stay. */
export const Live: Story = { args: { connection: 'live' }, play: async ({ canvasElement }) => {
  await expect(band(canvasElement)).toHaveLength(0)
  await expect(canvasElement.querySelectorAll('[data-system-state]')).toHaveLength(4)
} }
/** Reconnect moves the band to reconnecting; Retry on the terminal row loads the terminal again. */
export const ConnectionBannerRecovers: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await userEvent.click(canvas.getByRole('button', { name: 'Reconnect' }))
  await expect(canvasElement.querySelector('[data-connection-state="reconnecting"]')).toHaveTextContent(`Reconnecting·${countdown}`)
  await userEvent.click(canvas.getByRole('button', { name: 'Retry: Terminal session ended' }))
  const drawer = canvas.getByRole('region', { name: 'Terminal drawer' })
  await expect(drawer.querySelector('[data-system-state="loading"]')).toHaveTextContent('Loading terminal')
} }

// Narrow: the sidebar is closed and the Changes panel steps aside, so the workspace fills the phone.
const styles = stylex.create({
  root: { height: '100vh', width: '100%', boxSizing: 'border-box', display: 'flex', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  sidebar: { display: { default: 'flex', '@media (max-width: 600px)': 'none' }, flexDirection: 'column', flexShrink: 0, minHeight: 0, overflow: 'hidden', width: g.sidebarDefault, backgroundColor: surface.sidebar },
  brand: { height: g.band, paddingInline: s.lg, display: 'flex', alignItems: 'center', flexShrink: 0 },
  roster: { display: 'flex', flexDirection: 'column', overflowY: 'auto', padding: s.md, minHeight: 0, flexGrow: 1, gap: s.xs2 },
  workspace: { flex: '1 1 0', minWidth: 0, minHeight: 0, display: 'flex', flexDirection: 'column' },
  body: { flex: '1 1 0', minHeight: 0, display: 'flex' },
  missions: { flex: '1 1 0', minWidth: 0, display: 'flex', flexDirection: 'column' },
  paneHeader: { height: g.controlLg, display: 'flex', alignItems: 'center', paddingInline: s.lg, flexShrink: 0, fontSize: t.metaSize, fontWeight: t.weightMedium, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  paneBody: { flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column' },
  changes: { display: { default: 'flex', '@media (max-width: 900px)': 'none' }, flexDirection: 'column', width: '380px', flexShrink: 0, borderLeftWidth: g.hairline, borderLeftStyle: 'solid', borderLeftColor: border.border },
  drawer: { height: g.drawerDefault, flexShrink: 0, display: 'flex', flexDirection: 'column', borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border, backgroundColor: surface.terminal },
})
