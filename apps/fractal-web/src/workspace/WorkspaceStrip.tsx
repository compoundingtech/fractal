import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { Menu, MenuItem, MenuPopover, MenuTrigger } from '../ui-compat/components.tsx'
import { EditorTabs, type EditorTabItem } from '../workbench-kit/EditorTabs.tsx'
import { IconButton } from '../workbench-kit/IconButton.tsx'
import type { Command } from './model.ts'

const PlusIcon = () => <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="M6 2v8M2 6h8" /></svg>

const openable = [
  { id: 'inbox', ref: 'inbox', label: 'Inbox' },
  { id: 'missions', ref: 'missions', label: 'Missions' },
  { id: 'usage', ref: 'monitor/quota', label: 'Usage' },
] as const

/** The tab strip at every width. The panel it hosts keeps one identity, so no activation remounts a surface. */
export const WorkspaceStrip = ({ items, active, group, narrow, dispatch, open, trailing, children }: {
  readonly items: readonly EditorTabItem[]
  readonly active: string | null
  readonly group: string
  readonly narrow: boolean
  readonly dispatch: (command: Command) => void
  readonly open: (ref: string) => void
  readonly trailing?: React.ReactNode
  readonly children: React.ReactNode
}) => {
  const [menu, setMenu] = React.useState<{ readonly id: string; readonly target: React.RefObject<Element | null> }>()
  // Tabs belong to the kit; a right-click reaches them here by the id they already expose.
  const onContextMenu = (event: React.MouseEvent<HTMLDivElement>) => {
    const target = event.target instanceof Element ? event.target.closest('[data-tab-id]') : null
    const id = target?.getAttribute('data-tab-id')
    if (target === null || id === null || id === undefined) return
    event.preventDefault()
    setMenu({ id, target: { current: target } })
  }
  // The kit's drag wiring swallows a focused tab's own Delete; the strip closes it here, before the drag sees the key.
  const onKeyDownCapture = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Delete' && event.key !== 'Backspace') return
    const id = event.target instanceof Element ? event.target.closest('[data-tab-id]')?.getAttribute('data-tab-id') : null
    if (id === null || id === undefined) return
    event.preventDefault()
    event.stopPropagation()
    dispatch({ _tag: 'Close', id })
  }
  return <div onContextMenu={onContextMenu} onKeyDownCapture={onKeyDownCapture} {...stylex.props(styles.host)}>
    <EditorTabs
      label="Workspace tabs"
      groupId={group}
      tabs={items}
      activeId={active}
      variant="compact"
      isGroupFocused
      dragType="application/x-fractal-workspace-tab"
      stablePanel
      touch={narrow}
      onActivate={id => dispatch({ _tag: 'Activate', id })}
      onClose={id => dispatch({ _tag: 'Close', id })}
      onMove={({ source, toIndex }) => dispatch({ _tag: 'Move', id: source.tabId, to: { _tag: 'Strip', group, index: toIndex } })}
      actions={<>
        <MenuTrigger>
          <IconButton label="Open a tab" icon={<PlusIcon />} />
          <MenuPopover>
            <Menu aria-label="Open a tab" onAction={key => {
              const row = openable.find(item => item.id === key)
              if (row !== undefined) open(row.ref)
            }}>
              {openable.map(item => <MenuItem key={item.id} id={item.id}>{item.label}</MenuItem>)}
            </Menu>
          </MenuPopover>
        </MenuTrigger>
        {trailing}
      </>}
    >
      {children}
    </EditorTabs>
    {menu === undefined ? null : <Aria.Popover isOpen triggerRef={menu.target} placement="bottom start" onOpenChange={isOpen => { if (!isOpen) setMenu(undefined) }} {...stylex.props(styles.popover)}>
      <Aria.Menu aria-label="Tab actions" autoFocus="first" onClose={() => setMenu(undefined)} onAction={key => {
        if (key === 'close') dispatch({ _tag: 'Close', id: menu.id })
        else if (key === 'close-others') dispatch({ _tag: 'CloseOthers', id: menu.id })
      }} {...stylex.props(styles.menu)}>
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
  menuItem: { padding: '4px 8px', borderRadius: 4, cursor: 'pointer', fontSize: t.metaSize, outline: 'none', backgroundColor: { default: 'transparent', ':hover': c.rowHover, ':focus': c.rowHover }, opacity: { default: 1, '[data-disabled]': 0.5 } },
  empty: { position: 'relative', zIndex: 2, margin: 'auto', padding: s.section, color: c.fgMuted, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: s.md },
  emptyTitle: { margin: 0 },
  emptyActions: { display: 'flex', gap: s.sm },
  emptyAction: { padding: '4px 8px', backgroundColor: 'transparent', color: c.fgMuted, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 6, cursor: 'pointer', fontSize: t.metaSize, minHeight: 28 },
})
