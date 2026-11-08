import { Schema } from 'effect'
import * as Workspace from '@fractal/ui/workspace'
import { isAgentTerminalPane, paneIdentity } from './subject.ts'

/**
 * Fractal's binding of the shared workspace primitives (`@fractal/ui/workspace`): a tab's `ref` is the Fractal
 * TUI's pane key (`crates/fractal-tui/src/ui/pane.rs`), an agent and its terminal are one tab, and each tab keeps
 * the view state below. Spaces are the person's glasses when the gateway keeps them, and the shared body is the
 * glass body (`glassSync.ts`). Everything else, the reducer included, is the primitives'.
 */
export const TabView = Schema.Struct({
  scrollTop: Schema.optional(Schema.Finite),
  /** Which face an agent's tab shows; a key naming the terminal itself shows it until told otherwise. */
  terminal: Schema.optional(Schema.Boolean),
  /** Holds an unsent draft. The kit keeps drafts only in the mounted composer, so this pins the surface. */
  dirty: Schema.optional(Schema.Boolean),
})
export type TabView = typeof TabView.Type

export type WorkspaceState = Workspace.WorkspaceState<TabView>
export type Space = Workspace.Space<TabView>
export type Command = Workspace.Command<TabView>
export type Tab = Workspace.Tab
export type Pane = Workspace.Pane
export type Layout = Workspace.Layout
export type Drop = Workspace.Drop
export type Side = Workspace.Side

/** The name a person's first space gets, as the TUI names its first glass. */
export const defaultSpaceName = 'Main'

export const workspaceConfig = (newId: () => string = () => globalThis.crypto.randomUUID()): Workspace.WorkspaceConfig =>
  ({ newId, identity: paneIdentity, spaceName: defaultSpaceName })

export const workspaceSchema = Workspace.makeWorkspaceSchema(TabView)

export const emptyWorkspace = (config: Workspace.WorkspaceConfig): WorkspaceState =>
  Workspace.emptyWorkspace<TabView>(config, { name: defaultSpaceName })

export const currentSpace = (state: WorkspaceState): Space => Workspace.currentSpace(state)

/** The tab with the focus in the current space: the one the URL names and the header follows. */
export const activeTab = (state: WorkspaceState): Tab | undefined => Workspace.activeTab(currentSpace(state))

export const tabsOf = (state: WorkspaceState): readonly Tab[] => Workspace.tabsOf(currentSpace(state).layout)

/** One tab per pane is on screen; only the zoomed one while zoomed. */
export const visibleTabIds = (state: WorkspaceState): ReadonlySet<string> =>
  new Set(Workspace.shownPanes(currentSpace(state)).flatMap(pane => pane.active === null ? [] : [pane.active]))

/** Whether this tab shows its terminal face. */
export const showsTerminal = (tab: Pick<Tab, 'ref'>, view: TabView | undefined): boolean =>
  view?.terminal ?? isAgentTerminalPane(tab.ref)

/**
 * A terminal link opens the agent's one tab, on its terminal face: the shared reducer opens the agent's key,
 * and the face is this tab's view. Every other command passes through unchanged.
 */
export const transition = (state: WorkspaceState, command: Command, config: Workspace.WorkspaceConfig): WorkspaceState => {
  if (command._tag !== 'Open' || !isAgentTerminalPane(command.ref)) return Workspace.transition(state, command, config)
  const opened = Workspace.transition(state, { ...command, ref: paneIdentity(command.ref) }, config)
  const tab = Workspace.locateTab(currentSpace(opened).layout, row => paneIdentity(row.ref) === paneIdentity(command.ref))
  return tab === undefined ? opened : Workspace.transition(opened, { _tag: 'SetView', tab: tab.tab.id, view: { terminal: true } }, config)
}

// ---------------------------------------------------------------------------------------------------
// The document before the shared primitives: one layout of groups, with pane keys. It is read once and becomes
// this client's first space; nothing writes it any more.

const LegacyTab = Schema.Struct({ id: Schema.String, pane: Schema.NonEmptyString, title: Schema.optional(Schema.String) })
const LegacyGroup = Schema.Struct({ id: Schema.String, tabs: Schema.Array(LegacyTab), active: Schema.NullOr(Schema.String) })
interface LegacySplit {
  readonly split: 'right' | 'below'
  readonly children: readonly LegacyLayout[]
  readonly ratio?: number | undefined
}
type LegacyLayout = typeof LegacyGroup.Type | LegacySplit
const LegacyLayout: Schema.Codec<LegacyLayout> = Schema.Union([
  LegacyGroup,
  Schema.Struct({
    split: Schema.Literals(['right', 'below']),
    children: Schema.Array(Schema.suspend((): Schema.Codec<LegacyLayout> => LegacyLayout)),
    ratio: Schema.optional(Schema.Finite),
  }),
])
const LegacyDocument = Schema.Struct({
  layout: LegacyLayout,
  focus: Schema.String,
  mru: Schema.Array(Schema.String),
  closed: Schema.Array(Schema.String),
  views: Schema.Record(Schema.String, TabView),
})

const fromLegacyLayout = (layout: LegacyLayout, newId: () => string): Layout => 'tabs' in layout
  ? { id: layout.id, active: layout.active, tabs: layout.tabs.map(tab => tab.title === undefined ? { id: tab.id, ref: tab.pane } : { id: tab.id, ref: tab.pane, title: tab.title }) }
  : {
    id: newId(),
    axis: layout.split === 'right' ? 'row' : 'column',
    children: layout.children.map(child => fromLegacyLayout(child, newId)),
    ...(layout.ratio === undefined || layout.children.length !== 2 ? {} : { sizes: [layout.ratio, 1 - layout.ratio] }),
  }

export const legacyDecoders = (config: Workspace.WorkspaceConfig): readonly Workspace.LegacyDecoder<TabView>[] => [
  Workspace.legacyDecoder(LegacyDocument, (legacy): WorkspaceState => {
    const space: Space = {
      id: config.newId(),
      name: defaultSpaceName,
      layout: fromLegacyLayout(legacy.layout, config.newId),
      local: { ...Workspace.emptyLocal<TabView>(legacy.focus), mru: legacy.mru, closed: legacy.closed, views: legacy.views },
    }
    return { version: 1, current: space.id, spaces: [space], retired: [] }
  }),
]
