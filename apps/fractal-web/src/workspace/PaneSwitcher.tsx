import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { currentSpace, panesOf, type WorkspaceState } from '@fractal/ui/workspace'
import { useWorkspace } from '@fractal/ui/workspace/react'
import { colorVars as c, geometryVars as g, interactionVars as interaction, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'

const layoutOf = (state: WorkspaceState) => currentSpace(state)

/**
 * A phone keeps the splits but shows the focused pane alone; this reaches the others. It reads as the
 * focused pane's place among them ("1/2") and lists each pane by what it shows. Hidden with one pane.
 */
export const PaneSwitcher = ({ titleOf, onFocusPane }: {
  /** A tab's title, as its strip shows it. */
  readonly titleOf: (tab: string) => string | undefined
  readonly onFocusPane: (pane: string) => void
}) => {
  const space = useWorkspace(layoutOf)
  const panes = panesOf(space.layout)
  if (panes.length < 2) return null
  const index = panes.findIndex(pane => pane.id === space.local.focus)
  const describe = (pane: (typeof panes)[number]) => {
    const shown = pane.active === null ? 'Empty' : titleOf(pane.active) ?? 'Tab'
    return pane.tabs.length > 1 ? `${shown} +${pane.tabs.length - 1}` : shown
  }
  return <Aria.MenuTrigger>
    <Aria.Button aria-label={`Pane ${index + 1} of ${panes.length}`} data-testid="pane-switcher" {...stylex.props(styles.trigger)}>
      <span>{index + 1}/{panes.length}</span>
      <Icon name="chevron-down" size={12} />
    </Aria.Button>
    <Aria.Popover placement="bottom start" {...stylex.props(styles.popover)}>
      <Aria.Menu aria-label="Panes" autoFocus="first" selectionMode="single" selectedKeys={[space.local.focus]} onAction={key => onFocusPane(String(key))} {...stylex.props(styles.menu)}>
        {panes.map((pane, at) => <Aria.MenuItem key={pane.id} id={pane.id} textValue={describe(pane)} {...stylex.props(styles.item)}>
          <span {...stylex.props(styles.place)}>{at + 1}</span>
          <span {...stylex.props(styles.title)}>{describe(pane)}</span>
        </Aria.MenuItem>)}
      </Aria.Menu>
    </Aria.Popover>
  </Aria.MenuTrigger>
}

const styles = stylex.create({
  // A 44px touch target, as the phone's other top-bar controls.
  trigger: {
    display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: 2, flexShrink: 0, minWidth: 44, minHeight: 44, paddingInline: s.xs,
    borderWidth: 0, borderRadius: 6, backgroundColor: { default: 'transparent', ':hover': c.rowHover }, color: c.fgMuted,
    fontFamily: t.fontSans, fontSize: t.metaSize, fontVariantNumeric: 'tabular-nums', cursor: 'pointer',
    outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: g.focusRing, outlineColor: interaction.focusRing, outlineOffset: g.focusInset,
  },
  popover: { backgroundColor: c.raised, color: c.fg, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: 8, fontFamily: t.fontSans, fontSize: t.uiSize },
  menu: { padding: s.xs, outline: 'none', minWidth: 200, maxWidth: 'calc(100vw - 32px)' },
  item: {
    display: 'flex', alignItems: 'center', gap: s.sm, minHeight: 44, paddingInline: s.sm, borderRadius: 4, cursor: 'pointer', outline: 'none',
    backgroundColor: { default: 'transparent', ':hover': c.rowHover, '[data-focused]': c.rowHover, '[data-selected]': c.rowActive },
  },
  place: { color: c.fgMuted, fontVariantNumeric: 'tabular-nums', width: 16, flexShrink: 0 },
  title: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
})
