import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { Menu, MenuItem, MenuPopover, MenuTrigger } from '../ui-compat/components.tsx'
import { EditorTabs, type EditorTabItem } from '../workbench-kit/EditorTabs.tsx'
import { IconButton } from '../workbench-kit/IconButton.tsx'
import { useCurrentSpace, useDragState, useWorkspaceContext } from '@fractal/ui/workspace/react'
import type { Command } from './model.ts'

const PlusIcon = () => <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="M6 2v8M2 6h8" /></svg>

/** Surfaces a strip can open as a tab: its "Open a tab" menu, a phone's More actions and the empty pane offer them. */
export const openable = [
  { id: 'inbox', ref: 'inbox', label: 'Inbox' },
  { id: 'missions', ref: 'missions', label: 'Missions' },
  { id: 'usage', ref: 'monitor/quota', label: 'Usage' },
] as const

/** The tab strip at every width. The panel it hosts keeps one identity, so no activation remounts a surface. */
export const WorkspaceStrip = ({ items, refs, active, group, focused = true, narrow, dispatch, open, leading, trailing, bar = false, children }: {
  readonly items: readonly EditorTabItem[]
  /** Each tab's ref, which a drag of it carries. */
  readonly refs: ReadonlyMap<string, string>
  readonly active: string | null
  /** The pane this strip belongs to. */
  readonly group: string
  /** Whether its pane has the focus: its shown tab carries the accent, another split's a grey line. */
  readonly focused?: boolean
  readonly narrow: boolean
  readonly dispatch: (command: Command) => void
  readonly open: (ref: string) => void
  readonly leading?: React.ReactNode
  readonly trailing?: React.ReactNode
  /** The strip is the window's one top bar: it leads with `leading` and carries the conversation's controls in `trailing`. */
  readonly bar?: boolean
  readonly children: React.ReactNode
}) => {
  const [menu, setMenu] = React.useState<{ readonly id: string; readonly target: React.RefObject<Element | null> }>()
  // The kit's drag wiring swallows a focused tab's own Delete; the strip closes it here, before the drag sees the key.
  const onKeyDownCapture = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Delete' && event.key !== 'Backspace') return
    const id = event.target instanceof Element ? event.target.closest('[data-tab-id]')?.getAttribute('data-tab-id') : null
    if (id === null || id === undefined) return
    event.preventDefault()
    event.stopPropagation()
    dispatch({ _tag: 'Close', tab: id })
  }
  // Tabs drag on the workspace's pointer drag: within and between strips, and onto a pane's centre or edges.
  const { drag, describe } = useWorkspaceContext()
  const space = useCurrentSpace().id
  const dropIndex = useDragState(state => state.over?._tag === 'Strip' && state.over.pane === group ? state.over.slot : null)
  const hostDrag = React.useMemo(() => ({
    stripProps: { 'data-drop-strip': group },
    dropIndex,
    tabProps: (tab: EditorTabItem) => ({
      'data-workspace-tab': '',
      onPointerDown: (event: React.PointerEvent) => {
        const ref = refs.get(tab.id)
        // A phone shows one pane: a touch press scrolls the strip.
        if (event.button !== 0 || event.pointerType === 'touch' || ref === undefined) return
        drag.press(event, () => ({ entity: describe(ref), ref, tab: { space, id: tab.id } }))
      },
    }),
  }), [group, dropIndex, refs, drag, describe, space])
  return <div onKeyDownCapture={onKeyDownCapture} {...stylex.props(styles.host)}>
    <EditorTabs
      label="Workspace tabs"
      groupId={group}
      tabs={items}
      activeId={active}
      variant={bar ? "bar" : "compact"}
      leading={leading}
      end={bar ? trailing : undefined}
      isGroupFocused={focused}
      dragType="application/x-fractal-workspace-tab"
      stablePanel
      touch={narrow}
      hostDrag={hostDrag}
      onActivate={id => dispatch({ _tag: 'Activate', tab: id })}
      onClose={id => dispatch({ _tag: 'Close', tab: id })}
      // A tab's actions are on the tab: a right-click, Shift+F10 or the ContextMenu key opens them.
      onTabMenu={(id, tab) => setMenu({ id, target: { current: tab } })}
      onMove={({ source, toIndex }) => dispatch({ _tag: 'Move', tab: source.tabId, to: { _tag: 'Strip', pane: group, index: toIndex } })}
      actions={<>
        {/* "Open" then "Go to tab"; a phone folds both into the bar's More actions, keeping its targets 44px. */}
        {narrow ? null : <MenuTrigger>
          <IconButton label="Open a tab" icon={<PlusIcon />} />
          <MenuPopover>
            <Menu aria-label="Open a tab">
              <Aria.MenuSection aria-label="Open">
                <Aria.Header {...stylex.props(styles.menuHeading)}>Open</Aria.Header>
                {openable.map(item => <MenuItem key={item.id} id={`open:${item.id}`} onAction={() => open(item.ref)}>{item.label}</MenuItem>)}
              </Aria.MenuSection>
              <Aria.MenuSection aria-label="Go to tab" selectionMode="single" selectedKeys={active === null ? [] : [`tab:${active}`]}>
                <Aria.Header {...stylex.props(styles.menuHeading)}>Go to tab</Aria.Header>
                {items.map(item => <MenuItem key={item.id} id={`tab:${item.id}`} textValue={item.title} onAction={() => dispatch({ _tag: 'Activate', tab: item.id })}>{item.title}</MenuItem>)}
              </Aria.MenuSection>
            </Menu>
          </MenuPopover>
        </MenuTrigger>}
        {bar ? null : trailing}
      </>}
    >
      {children}
    </EditorTabs>
    {menu === undefined ? null : <Aria.Popover isOpen triggerRef={menu.target} placement="bottom start" onOpenChange={isOpen => { if (!isOpen) setMenu(undefined) }} {...stylex.props(styles.popover)}>
      <Aria.Menu aria-label="Tab actions" autoFocus="first" onClose={() => setMenu(undefined)} onAction={key => {
        if (key === 'close') dispatch({ _tag: 'Close', tab: menu.id })
        else if (key === 'close-others') dispatch({ _tag: 'CloseOthers', tab: menu.id })
        // One tab per entity, so a split from a tab moves it into the new pane.
        else if (key === 'split-right') dispatch({ _tag: 'Split', pane: group, side: 'right', tab: menu.id })
        else if (key === 'split-down') dispatch({ _tag: 'Split', pane: group, side: 'bottom', tab: menu.id })
      }} {...stylex.props(styles.menu)}>
        {narrow ? null : <>
          <Aria.MenuItem id="split-right" isDisabled={items.length < 2} {...stylex.props(styles.menuItem)}>Split right</Aria.MenuItem>
          <Aria.MenuItem id="split-down" isDisabled={items.length < 2} {...stylex.props(styles.menuItem)}>Split down</Aria.MenuItem>
        </>}
        <Aria.MenuItem id="close" {...stylex.props(styles.menuItem)}>Close</Aria.MenuItem>
        <Aria.MenuItem id="close-others" isDisabled={items.length < 2} {...stylex.props(styles.menuItem)}>Close others</Aria.MenuItem>
      </Aria.Menu>
    </Aria.Popover>}
  </div>
}

export const WorkspaceEmpty = ({ open }: { readonly open: (ref: string) => void }) => <div data-testid="workspace-empty" {...stylex.props(styles.empty)}>
  <p {...stylex.props(styles.emptyTitle)}>Choose an agent to open its live thread.</p>
  <div {...stylex.props(styles.emptyActions)}>
    {openable.map(item => <Aria.Button key={item.id} onPress={() => open(item.ref)} {...stylex.props(styles.emptyAction)}>{item.label}</Aria.Button>)}
  </div>
</div>

const styles = stylex.create({
  host: { display: 'contents' },
  popover: { backgroundColor: c.raised, color: c.fg, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 6 },
  menu: { padding: s.xs, outline: 'none', minWidth: 140 },
  menuHeading: { padding: '6px 8px 2px', color: c.fgMuted, fontSize: t.metaSize, fontWeight: 600 },
  menuItem: { padding: '4px 8px', borderRadius: 4, cursor: 'pointer', fontSize: t.metaSize, outline: 'none', backgroundColor: { default: 'transparent', ':hover': c.rowHover, ':focus': c.rowHover }, opacity: { default: 1, '[data-disabled]': 0.5 } },
  empty: { position: 'relative', zIndex: 2, margin: 'auto', padding: s.section, color: c.fgMuted, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: s.md },
  emptyTitle: { margin: 0 },
  emptyActions: { display: 'flex', gap: s.sm },
  emptyAction: { padding: '4px 8px', backgroundColor: 'transparent', color: c.fgMuted, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 6, cursor: 'pointer', fontSize: t.metaSize, minHeight: 28 },
})
