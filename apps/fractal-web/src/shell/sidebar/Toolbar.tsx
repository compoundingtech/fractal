import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { DialogTrigger, Dialog, Popover } from 'react-aria-components'
import { ChromeControl, compositionLightTheme, NarrowLayoutContext } from '@fractal/ui/assistant-ui/shell'
import { geometryVars as g, typeVars as t, spaceVars as s } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

import {
  Button,
  Checkbox,
  Input,
  Menu,
  MenuItem,
  MenuPopover,
  MenuTrigger,
} from '../../ui-compat/components.tsx'
import { scale, tokens } from '../../ui-compat/tokens.stylex.ts'
import { spaceVars as space } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

import { parseSidebarQuery, sortModes, type SortMode } from './filter.ts'
import { SidebarShowMenu } from './ShowMenu.tsx'
import type { ShowRow } from './show.ts'
import { defaultFilters, type SidebarFilters } from './state.ts'
import { statuses, type AgentStatus } from './StatusIcon.tsx'
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
/** Sidebar search, Show, visibility, status and view-only ordering controls. */
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
  const parsedQuery = parseSidebarQuery(filters.query)
  const activeCount =
    Number(filters.hideEnded) +
    Number(filters.hideRetired) +
    Number(parsedQuery.asking) +
    parsedQuery.machines.length +
    parsedQuery.facets.length +
    filters.statuses.length
  return (
    <div ref={shortcut} {...stylex.props(styles.toolbar, narrow && styles.phoneInset)}>
      <div {...stylex.props(styles.controls)}>
        <SidebarShowMenu
          query={filters.query}
          onQueryChange={(query) => update({ ...filters, query })}
          rows={showRows}
          hosts={hosts}
          {...(light ? { popoverTheme: compositionLightTheme } : {})}
        />
        <ChromeControl icon="check" label="Show finished agents" pressed={!filters.hideEnded && !filters.hideRetired} onPress={() => { const show = filters.hideEnded || filters.hideRetired; update({ ...filters, hideEnded: !show, hideRetired: !show }) }} />
        <div {...stylex.props(styles.menus)}>
          <DialogTrigger>
            <ChromeControl icon="filter" label={`Filter agents${activeCount > 0 ? ` · ${activeCount} active` : ''}`} active={activeCount > 0} />
            <Popover placement="bottom end" {...stylex.props(styles.popover)}>
              <Dialog aria-label="Agent filters" {...stylex.props(styles.dialog)}>
                <Checkbox
                  isSelected={filters.hideEnded}
                  stylexStyle={narrow && styles.touchRow}
                  onChange={(hideEnded) => update({ ...filters, hideEnded })}
                >
                  Hide ended
                </Checkbox>
                <Checkbox
                  isSelected={filters.hideRetired}
                  stylexStyle={narrow && styles.touchRow}
                  onChange={(hideRetired) => update({ ...filters, hideRetired })}
                >
                  Hide retired
                </Checkbox>
                <fieldset {...stylex.props(styles.statuses)}>
                  <legend>Status</legend>
                  {(Object.keys(statuses) as AgentStatus[]).map((status) => (
                    <Checkbox
                      key={status}
                      isSelected={filters.statuses.includes(status)}
                      stylexStyle={narrow && styles.touchRow}
                      onChange={(selected) =>
                        update({
                          ...filters,
                          statuses: selected
                            ? [...filters.statuses, status]
                            : filters.statuses.filter((value) => value !== status),
                        })
                      }
                    >
                      {statuses[status].label}
                    </Checkbox>
                  ))}
                </fieldset>
                <Button
                  size="sm"
                  variant="secondary"
                  stylexStyle={narrow && styles.touchRow}
                  onPress={() =>
                    update({ ...defaultFilters, query: parsedQuery.text, sort: filters.sort })
                  }
                >
                  Reset filters
                </Button>
              </Dialog>
            </Popover>
          </DialogTrigger>
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
      </div>
      <div ref={search}>
        <Input
          aria-label="Search agents"
          aria-description="Free text matches agent names and IDs. Use is:asking for agents waiting for an answer, repo:, state:, or tool: to search other facts, and machine:build-1 or host:build-1,build-2 to filter host prefixes, ignoring case. Comma-separated values match any host; separate tokens must all match."
          placeholder="Search… repo: state: tool:  /"
          size="sm"
          value={filters.query}
          onChange={(query) => update({ ...filters, query })}
          onKeyDown={(event) => {
            if (event.key === 'Escape') update({ ...filters, query: '' })
            if (event.key === 'ArrowDown' && onEnterResults !== undefined) { event.preventDefault(); onEnterResults() }
          }}
          stylexStyle={[styles.search, searchStyle, narrow && styles.touchSearch]}
        />
      </div>
    </div>
  )
}

const styles = stylex.create({
  toolbar: {
    display: 'flex',
    flexDirection: 'column',
    gap: space.md,
    paddingInline: space.md,
    paddingBottom: space.md,
  },
  search: {
    boxSizing: 'border-box',
    borderRadius: 6,
    ':focus-visible': { outline: `2px solid ${tokens['--ds-focus-color']}`, outlineOffset: 2 },
  },
  controls: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: scale.space2,
  },
  popover: {
    backgroundColor: tokens['--ds-background-100'],
    borderRadius: scale.radiusDefault,
    borderWidth: 1,
    borderStyle: 'solid',
    borderColor: tokens['--ds-gray-alpha-400'],
    padding: scale.space3,
    boxShadow: '0 8px 24px rgb(0 0 0 / .15)',
    width: '240px',
    maxHeight: '80vh',
    overflowY: 'auto',
  },
  dialog: { display: 'flex', flexDirection: 'column', gap: scale.space3, outline: 'none' },
  statuses: {
    display: 'flex',
    flexDirection: 'column',
    gap: scale.space2,
    border: 0,
    padding: 0,
    margin: 0,
  },
  menus: { display: 'flex', alignItems: 'center', gap: scale.space1 },
  // Narrow (phone) hosts: every toolbar control is a 44px touch target; the search text is 16px so iOS does not zoom on focus.
  phoneInset: { paddingInline: s.md },
  touchRow: { minHeight: g.touchTarget },
  touchSearch: { minHeight: g.touchTarget, fontSize: t.touchInputSize },
})
