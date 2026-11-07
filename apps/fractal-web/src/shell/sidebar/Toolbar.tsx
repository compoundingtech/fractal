import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { Button as AriaButton } from 'react-aria-components'
import { ChromeControl } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl.tsx'
import { lightTheme as compositionLightTheme } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-theme.ts'
import { NarrowLayoutContext } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

import {
  Input,
  Menu,
  MenuItem,
  MenuPopover,
  MenuTrigger,
} from '../../ui-compat/components.tsx'

import { spaceVars as space } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

import { sortModes, type SortMode } from './filter.ts'
import { SidebarShowMenu } from './ShowMenu.tsx'
import type { ShowRow } from './show.ts'
import type { SidebarFilters } from './state.ts'
import { Icon } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'

const readLightScheme = () => document.documentElement.dataset.scheme !== 'dark'
const subscribeScheme = (notify: () => void) => {
  const observer = new MutationObserver(notify)
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-scheme'] })
  return () => observer.disconnect()
}

/** Inputs of the shared sidebar toolbar. */
export interface SidebarToolbarProps {
  readonly filters: SidebarFilters
  readonly update: (value: SidebarFilters) => void
  readonly hosts: readonly string[]
  readonly showRows: readonly ShowRow[]
  readonly onEnterResults?: () => void
  /** Host surface override for the search field, e.g. a fill that stands apart from its sidebar. */
  readonly searchStyle?: stylex.StyleXStyles
}
/** One line of sidebar controls: search, the Filter menu and view-only ordering. */
export const SidebarToolbar = ({
  filters,
  update,
  hosts,
  showRows,
  onEnterResults,
  searchStyle,
}: SidebarToolbarProps) => {
  const light = React.useSyncExternalStore(subscribeScheme, readLightScheme, () => true)
  const narrow = React.useContext(NarrowLayoutContext)
  const search = React.useRef<HTMLDivElement>(null)
  const shortcut = React.useCallback((node: HTMLDivElement | null) => {
    if (node === null) return
    const listener = (event: KeyboardEvent) => {
      const target = event.target
      if (
        event.key !== '/' ||
        event.metaKey ||
        event.ctrlKey ||
        event.altKey ||
        (target instanceof HTMLElement &&
          (target.isContentEditable || target.closest('input,textarea,select,[role="textbox"]')))
      )
        return
      event.preventDefault()
      search.current?.querySelector('input')?.focus()
    }
    document.addEventListener('keydown', listener)
    return () => document.removeEventListener('keydown', listener)
  }, [])
  return (
    <div ref={shortcut} {...stylex.props(styles.toolbar, narrow && styles.phoneInset)}>
      <div ref={search} {...stylex.props(styles.searchSlot)}>
        <Input
          aria-label="Search agents"
          aria-description="Free text matches agent names and IDs. Use is:asking for agents waiting for an answer, repo:, state:, or tool: to search other facts, and machine:build-1 or host:build-1,build-2 to filter host prefixes, ignoring case. Comma-separated values match any host; separate tokens must all match."
          placeholder="Find an agent"
          size="sm"
          value={filters.query}
          onChange={(query) => update({ ...filters, query })}
          onKeyDown={(event) => {
            if (event.key === 'Escape') update({ ...filters, query: '' })
            if (event.key === 'ArrowDown' && onEnterResults !== undefined) { event.preventDefault(); onEnterResults() }
          }}
          stylexStyle={[styles.search, searchStyle, !narrow && styles.searchWithHint, narrow && styles.touchSearch, narrow && filters.query !== '' && styles.touchSearchWithClear]}
        />
        {filters.query === ''
          ? !narrow && <kbd aria-hidden="true" {...stylex.props(styles.searchHint)}>/</kbd>
          // The same clear as Escape: the whole query, free text and tokens. Focus moves first, because this button then unmounts.
          : <AriaButton aria-label="Clear search" onPress={() => { search.current?.querySelector('input')?.focus(); update({ ...filters, query: '' }) }} {...stylex.props(styles.searchClear, narrow && styles.searchClearTouch)}><Icon name="x" size={12} /></AriaButton>}
      </div>
      <SidebarShowMenu
        filters={filters}
        update={update}
        rows={showRows}
        hosts={hosts}
        {...(light ? { popoverTheme: compositionLightTheme } : {})}
      />
      <MenuTrigger>
        <ChromeControl icon="sort" label={`Sort agents · ${sortModes[filters.sort]}`} active={filters.sort !== 'manual'} />
        <MenuPopover>
          <Menu
            aria-label="Sort agents within each folder"
            selectionMode="single"
            selectedKeys={[filters.sort]}
            onAction={(key) => update({ ...filters, sort: key as SortMode })}
          >
            {Object.entries(sortModes).map(([value, label]) => (
              <MenuItem
                key={value}
                id={value}
                textValue={label}
                stylexStyle={narrow && styles.touchRow}
                suffix={filters.sort === value ? <Icon name="check" /> : undefined}
              >
                {label}
              </MenuItem>
            ))}
          </Menu>
        </MenuPopover>
      </MenuTrigger>
    </div>
  )
}

const styles = stylex.create({
  toolbar: {
    display: 'flex',
    alignItems: 'center',
    gap: space.sm,
    paddingInline: space.md,
    paddingBottom: space.md,
  },
  searchSlot: { position: 'relative', flexGrow: 1, flexShrink: 1, flexBasis: 0, minWidth: 0 },
  search: {
    boxSizing: 'border-box',
    width: '100%',
    borderRadius: 6,
    ':focus-visible': { outline: `2px solid ${interaction.focusRing}`, outlineOffset: 2 },
  },
  // Desktop density: the field shares the controls' height and text size on the one line.
  searchWithHint: { height: g.controlMd, paddingBlock: 0, paddingInlineEnd: g.controlSm, fontSize: t.metaSize },
  searchHint: {
    position: 'absolute',
    insetInlineEnd: s.sm,
    top: '50%',
    transform: 'translateY(-50%)',
    minWidth: g.icon,
    boxSizing: 'border-box',
    textAlign: 'center',
    borderWidth: g.hairline,
    borderStyle: 'solid',
    borderColor: border.border,
    borderRadius: r.sm,
    color: ink.fgMuted,
    fontFamily: t.fontSans,
    fontSize: t.denseSize,
    pointerEvents: 'none',
  },
  searchClear: {
    position: 'absolute',
    insetInlineEnd: s.xs2,
    top: '50%',
    transform: 'translateY(-50%)',
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    boxSizing: 'border-box',
    width: g.controlSm,
    height: g.controlSm,
    padding: 0,
    borderWidth: 0,
    borderRadius: r.sm,
    backgroundColor: { default: 'transparent', ':hover': surface.rowHover },
    color: { default: ink.fgMuted, ':hover': ink.fg },
    cursor: 'pointer',
    outlineStyle: 'none',
    ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing },
  },
  searchClearTouch: { insetInlineEnd: 0, width: g.touchTarget, height: g.touchTarget },
  // Narrow (phone) hosts: every toolbar control is a 44px touch target; the search text is 16px so iOS does not zoom on focus.
  phoneInset: { paddingInline: s.md },
  touchRow: { minHeight: g.touchTarget },
  touchSearch: { minHeight: g.touchTarget, fontSize: t.touchInputSize },
  touchSearchWithClear: { paddingInlineEnd: g.touchTarget },
})
