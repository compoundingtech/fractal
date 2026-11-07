// Multiple-selection filter menu: checkbox entries with counts; selection semantics belong to the host.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Header, Menu, MenuItem, MenuSection, MenuTrigger, Popover, VisuallyHidden, type Selection } from 'react-aria-components'
import { Icon } from '../composition/Icons'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { accentVars as accent, borderVars as border, elevationVars as elevation, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

/** One checkbox entry; `id` is the host's selection key. */
export interface ShowMenuEntry {
  readonly id: string
  readonly label: string
  readonly count: number
}
export interface ShowMenuGroup {
  readonly id: string
  readonly title: string
  readonly entries: readonly ShowMenuEntry[]
  /** Rarer entries folded behind a "More" row in this group; open while one of them is selected. */
  readonly more?: readonly ShowMenuEntry[]
}
/** A one-shot command below the checkbox groups; pressing it closes the menu. */
export interface ShowMenuAction {
  readonly id: string
  readonly label: string
  readonly isDisabled?: boolean
  readonly onAction: () => void
}
export interface ShowMenuProps {
  readonly groups: readonly ShowMenuGroup[]
  readonly selectedKeys: ReadonlySet<string>
  readonly onSelectionChange: (keys: ReadonlySet<string>) => void
  /** Visible trigger text and menu name. */
  readonly label?: string
  /** The trigger's count when the host's active filters are not exactly the selected keys. Defaults to their number. */
  readonly activeCount?: number
  readonly actions?: readonly ShowMenuAction[]
  /** Theme for the portalled popover: themes on the host's subtree do not reach document.body. */
  readonly popoverTheme?: stylex.StyleXArray<stylex.CompiledStyles>
}

/** Checkbox menu entry for a multiple-selection menu: check, label, right-aligned tabular count. */
export function ShowMenuItem({ id, label, count }: ShowMenuEntry) {
  const narrow = React.useContext(NarrowLayoutContext)
  return <MenuItem id={id} textValue={label} aria-label={`${label}, ${count}`} {...stylex.props(styles.item, narrow && styles.touchRow)}>
    {({ isSelected }) => <>
      <span aria-hidden="true" data-show-check={isSelected ? 'checked' : 'unchecked'} {...stylex.props(styles.check, isSelected && styles.checked)}>{isSelected && <Icon name="check" size={12} />}</span>
      <span {...stylex.props(styles.label)}>{label}</span>
      <span aria-hidden="true" data-show-count="" {...stylex.props(styles.count)}>{count}</span>
    </>}
  </MenuItem>
}

/** Trigger plus multiple-selection menu; it stays open while entries toggle. */
export function ShowMenu({ groups, selectedKeys, onSelectionChange, label = 'Show', activeCount, actions = [], popoverTheme }: ShowMenuProps) {
  const active = activeCount ?? selectedKeys.size
  const narrow = React.useContext(NarrowLayoutContext)
  const [expanded, setExpanded] = React.useState<ReadonlySet<string>>(new Set())
  // A selected rare entry opens its group, which then stays open so unchecking it never moves focus out from under the reader.
  const forced = groups.filter(group => group.more?.some(entry => selectedKeys.has(entry.id)))
  if (forced.some(group => !expanded.has(group.id))) setExpanded(new Set([...expanded, ...forced.map(group => group.id)]))
  const select = (keys: Selection) =>
    onSelectionChange(keys === 'all' ? new Set(groups.flatMap(group => [...group.entries, ...group.more ?? []].map(entry => entry.id))) : new Set([...keys].map(String)))
  const toggle = (id: string) => setExpanded(current => { const next = new Set(current); if (!next.delete(id)) next.add(id); return next })
  return <MenuTrigger>
    <Button data-show-active={active > 0 ? active : undefined} {...stylex.props(styles.trigger, narrow && styles.touchRow, active > 0 && styles.triggerActive)}>
      <span>{label}</span>
      <Icon name="chevron-down" size={12} />
      {active > 0 && <><span aria-hidden="true" {...stylex.props(styles.badge)}>{active}</span><VisuallyHidden>, {active} active</VisuallyHidden></>}
    </Button>
    <Popover placement="bottom end" {...stylex.props(styles.popover, popoverTheme)}>
      <Menu aria-label={label} selectionMode="multiple" selectedKeys={selectedKeys} onSelectionChange={select} {...stylex.props(styles.menu)}>
        {groups.flatMap(group => {
          const open = expanded.has(group.id)
          const canToggle = group.more !== undefined && group.more.length > 0 && !forced.includes(group)
          return [
            <MenuSection key={group.id} id={group.id} {...stylex.props(styles.section)}>
              <Header {...stylex.props(styles.header)}>{group.title}</Header>
              {group.entries.map(entry => <ShowMenuItem key={entry.id} {...entry} />)}
              {open && group.more?.map(entry => <ShowMenuItem key={entry.id} {...entry} />)}
            </MenuSection>,
            ...(canToggle ? [<MenuSection key={`${group.id}:more`} id={`${group.id}:more`} aria-label={`More ${group.title}`} selectionMode="none" {...stylex.props(styles.section, styles.moreSection)}>
              <MenuItem id={`${group.id}:toggle`} textValue={open ? 'Fewer' : 'More'} shouldCloseOnSelect={false} onAction={() => toggle(group.id)} {...stylex.props(styles.item, styles.command, narrow && styles.touchRow)}>
                <span aria-hidden="true" {...stylex.props(styles.moreGlyph)}><Icon name={open ? 'chevron-down' : 'chevron-right'} size={12} /></span>
                <span {...stylex.props(styles.label)}>{open ? 'Fewer' : 'More'}</span>
              </MenuItem>
            </MenuSection>] : []),
          ]
        })}
        {actions.length > 0 && <MenuSection id="show-menu-actions" aria-label={`${label} actions`} selectionMode="none" {...stylex.props(styles.section)}>
          {actions.map(action => <MenuItem key={action.id} id={action.id} textValue={action.label} isDisabled={action.isDisabled} onAction={action.onAction} {...stylex.props(styles.item, styles.command, styles.action, narrow && styles.touchRow)}>{action.label}</MenuItem>)}
        </MenuSection>}
      </Menu>
    </Popover>
  </MenuTrigger>
}

const styles = stylex.create({
  trigger: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0, minHeight: g.controlMd, paddingInline: s.sm, boxSizing: 'border-box', borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control, backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover }, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap', cursor: 'pointer', outlineStyle: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  triggerActive: { color: ink.fg, backgroundColor: surface.rowActive },
  // Narrow (phone) hosts: the trigger and every entry are 44px touch targets.
  touchRow: { minHeight: g.touchTarget },
  badge: { minWidth: g.icon, height: g.icon, paddingInline: s.xs, boxSizing: 'border-box', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', borderRadius: r.full, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, backgroundColor: surface.controlFill, color: ink.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  popover: { minWidth: g.menuMin, maxWidth: 'calc(100vw - 16px)', overflowY: 'auto', boxSizing: 'border-box', padding: s.xs, backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, boxShadow: elevation.popover, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  menu: { outlineStyle: 'none' },
  // The More row reads as the end of the group above it, not as a group of its own.
  moreSection: { ':not(:first-child)': { marginTop: s.zero, paddingTop: s.zero, borderTopWidth: s.zero } },
  section: { display: 'flex', flexDirection: 'column', ':not(:first-child)': { marginTop: s.xs, paddingTop: s.xs, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border } },
  header: { minHeight: g.controlSm, display: 'flex', alignItems: 'center', paddingInline: s.md, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: t.weightMedium },
  item: { minHeight: g.menuRow, display: 'grid', gridTemplateColumns: `${g.icon} minmax(0, 1fr) auto`, alignItems: 'center', columnGap: s.md, paddingInline: s.md, borderRadius: r.sm, cursor: 'pointer', outlineStyle: 'none', ':is([data-focused])': { backgroundColor: surface.rowActive }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
  command: { color: ink.fgMuted, ':is([data-disabled])': { opacity: 0.64, cursor: 'default' } },
  action: { display: 'flex' },
  moreGlyph: { width: g.icon, display: 'inline-flex', justifyContent: 'center' },
  check: { width: g.icon, height: g.icon, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box', borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.controlFill },
  checked: { borderColor: accent.primary, backgroundColor: accent.primary, color: accent.onPrimary },
  label: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  count: { minWidth: '3ch', textAlign: 'right', color: ink.fgMuted, fontVariantNumeric: 'tabular-nums' },
})
