// The left sidebar is the app's one global area: what belongs to the person, the app or the connection lives in its
// header, under the header, in its footer, or on its 48px rail when collapsed. Panes keep only their own things, plus a
// band in the focused pane once the connection is gone.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { ChromeControl } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl.tsx'
import { Icon } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { ConnectionLine, ConnectionStatus, connectionTitle } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { SidebarTabBadge, sidebarTabName, type SidebarTab } from '../../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarTabs.tsx'
import { colorVars as c, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { paneBandShown, useConnectionView } from './connection.ts'

/** The footer's first line: nothing while live, otherwise the state, its detail and Reconnect when that can help. */
export const ConnectionFooterLine = () => {
  const view = useConnectionView()
  return <ConnectionLine state={view.state} {...(view.text === undefined ? {} : { text: view.text })} {...(view.onReconnect === undefined ? {} : { onReconnect: view.onReconnect })} />
}

/** At the top of the focused pane, only once the connection is gone (offline or failed); recovering is the footer's to say. */
export const PaneConnectionBand = () => {
  const view = useConnectionView()
  if (!paneBandShown(view.state)) return null
  return <ConnectionStatus state={view.state} {...(view.text === undefined ? {} : { text: view.text })} {...(view.onReconnect === undefined ? {} : { onReconnect: view.onReconnect })} />
}

/** Under the sidebar's header, beside the space switcher it is about: the person's glasses could not be reached, so this layout is this page's alone. */
export const LayoutSyncNote = ({ retry }: { readonly retry: () => void }) => <div data-layout-sync="" {...stylex.props(styles.syncNote)}>
  <span aria-hidden="true" {...stylex.props(styles.syncIcon)}><Icon name="alert" size={14} /></span>
  <span role="status" {...stylex.props(styles.syncWords)}>{layoutSyncWords}</span>
  <Aria.Button onPress={retry} {...stylex.props(styles.syncRetry)}>Retry</Aria.Button>
</div>

/** Words for the note on the collapsed rail's space control. */
export const layoutSyncWords = 'Layout not syncing'

type Tone = 'attention' | 'danger'

/** A 6px status dot for a control's corner; the control says what it means in words. */
const StatusDot = ({ tone }: { readonly tone: Tone }) => <span data-status-dot={tone} {...stylex.props(styles.dot, tone === 'danger' ? styles.danger : styles.attention)} />

/**
 * The phone's drawer button, the global area's door while the drawer is closed: a dot while the connection is lost or
 * alerts need the person, said in its description, so nothing global hides behind a closed drawer.
 */
export const PhoneDrawerOpener = ({ ref, expanded, controls, alerts, onPress }: {
  readonly ref: React.Ref<HTMLButtonElement>
  readonly expanded: boolean
  readonly controls: string | undefined
  /** The Alerts tab's marker in words ("Alerts, 2 need you") when alerts need the person. */
  readonly alerts: string | undefined
  readonly onPress: () => void
}) => {
  const connection = useConnectionView()
  const words = [...(connection.state === 'live' ? [] : [connectionTitle[connection.state]]), ...(alerts === undefined ? [] : [alerts])]
  const tone: Tone | undefined = connection.state === 'failed' || connection.state === 'offline' ? 'danger' : words.length > 0 ? 'attention' : undefined
  return <ChromeControl ref={ref} icon="panel-left" label="Expand sidebar" tooltip={words.length === 0 ? 'Show agents' : `Show agents · ${words.join(' · ')}`} edge="lead" popup="dialog" expanded={expanded}
    {...(controls === undefined ? {} : { controls })} onPress={onPress}
    {...(tone === undefined ? {} : { badge: <StatusDot tone={tone} />, description: words.join(' · ') })} />
}

/** The rail's connection: absent while live; otherwise a dot and the state, and pressing it opens the sidebar on the footer that says more. */
const RailConnectionControl = ({ onExpand }: { readonly onExpand: () => void }) => {
  const view = useConnectionView()
  if (view.state === 'live') return null
  const title = connectionTitle[view.state]
  return <ChromeControl icon="alert" label={`Connection: ${title}`} tooltip={view.text === undefined ? title : `${title} · ${view.text}`} tooltipPlacement="right" onPress={onExpand}
    badge={<StatusDot tone={view.state === 'failed' || view.state === 'offline' ? 'danger' : 'attention'} />} />
}

/**
 * The collapsed sidebar's contents, top to bottom: the space and the palette, the sidebar's tabs as icons with their
 * markers, then the connection while it is lost, Usage and Settings. Tooltips carry the detail; a tab opens the
 * sidebar on itself. The selected tab keeps a fill, never an outline, which is the focus ring's.
 */
export const SidebarRail = ({ top, tabs, selectedId, onShowTab, onExpand, bottom }: {
  /** The space switcher's rail form and the palette. */
  readonly top: React.ReactNode
  readonly tabs: readonly SidebarTab[]
  readonly selectedId: string
  readonly onShowTab: (id: string) => void
  readonly onExpand: () => void
  /** Usage and Settings. */
  readonly bottom: React.ReactNode
}) => <>
  {top}
  <div role="group" aria-label="Sidebar" data-rail-tabs="" {...stylex.props(styles.group)}>
    {tabs.map(tab => <ChromeControl key={tab.id} icon={tab.icon ?? 'dot'} label={sidebarTabName(tab)} tooltipPlacement="right" active={tab.id === selectedId} onPress={() => onShowTab(tab.id)} badge={<SidebarTabBadge tab={tab} />} />)}
  </div>
  <span aria-hidden="true" {...stylex.props(styles.spacer)} />
  <RailConnectionControl onExpand={onExpand} />
  {bottom}
</>

const styles = stylex.create({
  syncNote: { display: 'flex', alignItems: 'center', gap: s.sm, flexShrink: 0, marginInline: s.md, marginBlockEnd: s.xs, paddingInline: s.md, minHeight: g.controlLg, boxSizing: 'border-box', borderRadius: r.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: `color-mix(in srgb, ${c.attention} 24%, transparent)`, backgroundColor: `color-mix(in srgb, ${c.attention} 10%, ${c.sidebar})`, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  syncIcon: { display: 'inline-flex', flexShrink: 0, color: c.attention },
  syncWords: { flex: '1 1 0', minWidth: 0 },
  syncRetry: {
    flexShrink: 0, minHeight: g.controlSm, paddingInline: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: { default: 'transparent', ':hover': c.rowHover }, color: c.fg,
    fontFamily: t.fontSans, fontSize: t.metaSize, textDecoration: 'underline', cursor: 'pointer',
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: interaction.focusRing, outlineOffset: g.focusOffset,
  },
  dot: { display: 'block', width: s.sm, height: s.sm, borderRadius: r.full },
  attention: { backgroundColor: c.attention },
  danger: { backgroundColor: c.dangerFg },
  group: { display: 'flex', flexDirection: 'column', alignItems: 'center', gap: s.xs, paddingBlock: s.sm, marginBlock: s.xs, borderBlockWidth: g.hairline, borderBlockStyle: 'solid', borderBlockColor: c.border, alignSelf: 'stretch' },
  spacer: { flex: '1 1 0', minHeight: 0 },
})
