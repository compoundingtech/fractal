// Workshop-only composition: not exported from the package. Rows are the shipped SR-2 SidebarAgentRow.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Disclosure, DisclosurePanel, Input, Label, SearchField } from 'react-aria-components'
import { SidebarAgentRow } from '../../assistant-ui/sidebar/SidebarAgentRow'
import type { SidebarAgentRow as Row } from '../../assistant-ui/sidebar/model'
import type { DiscRefinement } from '../../assistant-ui/sidebar/SidebarStatus'
import { Icon } from '../../assistant-ui/composition/Icons'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../assistant-ui/composition-tokens.stylex'
import type { RosterFolder } from './milestone-fixtures'

export type RosterState = 'ready' | 'loading' | 'empty' | 'error' | 'stale'
export interface MilestoneSidebarProps {
  readonly state: RosterState
  readonly folders: readonly RosterFolder[]
  readonly width: number
  readonly now: number
  readonly activeRef?: string
  readonly query: string
  readonly onQueryChange: (query: string) => void
  readonly searchRef: React.RefObject<HTMLInputElement | null>
  readonly rowAdaptive: 'off' | '300' | '260'
  readonly rosterLoading: 'skeleton' | 'line'
  readonly errorPlacement: 'sidebar' | 'banner'
  readonly glyphRefinement: DiscRefinement
  readonly sectionNav: 'footer' | 'header'
  readonly inboxCount: number
  readonly onOpen: (row: Row) => void
  readonly onRetry: () => void
  readonly onOpenPalette: () => void
  readonly onCollapse: () => void
  readonly modKey: string
}

/** Roving focus across folder headers and row buttons; arrow keys never leave the list. */
const ROSTER_STOPS = '[data-roster-stop], [data-testid="taste-agent-row"] > button'

export function MilestoneSidebar(props: MilestoneSidebarProps) {
  const { state, folders, width, query, rowAdaptive, glyphRefinement } = props
  const [closedFolders, setClosedFolders] = React.useState<ReadonlySet<string>>(() => new Set())
  const [openParents, setOpenParents] = React.useState<ReadonlySet<string>>(() => new Set())
  const needle = query.trim().toLowerCase()
  const visible = needle === '' ? folders : folders.map(folder => ({ ...folder, rows: folder.rows.filter(row => `${row.title} ${row.host} ${row.ref}`.toLowerCase().includes(needle)) })).filter(folder => folder.rows.length > 0)
  const variant = rowAdaptive !== 'off' && width < Number(rowAdaptive) ? 'SR-1' : 'SR-2'
  const showRows = state === 'ready' || state === 'stale'
  const listRef = React.useRef<HTMLDivElement>(null)
  const onListKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    const stops = [...event.currentTarget.querySelectorAll<HTMLElement>(ROSTER_STOPS)]
    const index = stops.findIndex(stop => stop === document.activeElement)
    if (index === -1) return
    event.preventDefault()
    // ArrowUp from the first stop returns to the filter, so the list and its filter are one keyboard surface.
    if (index === 0 && event.key === 'ArrowUp') props.searchRef.current?.focus()
    else stops[Math.min(stops.length - 1, Math.max(0, index + (event.key === 'ArrowDown' ? 1 : -1)))]?.focus()
  }
  const sections = <nav aria-label="Workspace sections" data-section-nav={props.sectionNav} {...stylex.props(styles.sections, props.sectionNav === 'footer' ? styles.sectionsFooter : styles.sectionsHeader)}>
    <Button {...stylex.props(styles.sectionButton)}>Inbox<span aria-label={`${props.inboxCount} unread`} {...stylex.props(styles.count)}>{props.inboxCount}</span></Button>
    <Button {...stylex.props(styles.sectionButton)}>Missions</Button>
    <Button {...stylex.props(styles.sectionButton)}>Resources</Button>
  </nav>
  return <aside aria-label="Agents" data-roster-state={state} data-row-variant={variant} {...stylex.props(styles.sidebar)}>
    <header {...stylex.props(styles.header)}>
      <strong {...stylex.props(styles.brand)}>Fractal</strong>
      <Button aria-label={`Open command palette ${props.modKey}K`} aria-keyshortcuts="Meta+K Control+K" onPress={props.onOpenPalette} {...stylex.props(styles.kbdButton)}><Icon name="search" size={12} /><kbd {...stylex.props(styles.kbd)}>{props.modKey}K</kbd></Button>
      <Button aria-label="Collapse sidebar" aria-keyshortcuts="Meta+B Control+B" onPress={props.onCollapse} {...stylex.props(styles.iconButton)}><Icon name="chevrons-left" size={14} /></Button>
    </header>
    <SearchField value={query} onChange={props.onQueryChange} isDisabled={!showRows} {...stylex.props(styles.search)}>
      <Label {...stylex.props(styles.visuallyHidden)}>Filter agents</Label>
      <Icon name="search" size={12} />
      <Input ref={props.searchRef} placeholder="Filter agents" aria-keyshortcuts="/" onKeyDown={event => {
        const first = event.key === 'ArrowDown' ? listRef.current?.querySelector<HTMLElement>(ROSTER_STOPS) : undefined
        if (first) { event.preventDefault(); first.focus() }
      }} {...stylex.props(styles.searchInput)} />
      {query === '' ? <kbd aria-hidden="true" {...stylex.props(styles.kbd)}>/</kbd> : <Button aria-label="Clear filter" {...stylex.props(styles.clear)}><Icon name="x" size={12} /></Button>}
    </SearchField>
    {props.sectionNav === 'header' && sections}
    <div ref={listRef} data-roster-list onKeyDown={onListKeyDown} {...stylex.props(styles.list)}>
      {state === 'stale' && <p role="status" data-roster-notice="stale" {...stylex.props(styles.notice)}><Icon name="clock" size={12} />Last observed list · reconnecting</p>}
      {state === 'loading' && <>
        <p role="status" data-roster-notice="loading" {...stylex.props(styles.notice, props.rosterLoading === 'skeleton' && styles.visuallyHidden)}><Icon name="spinner" size={12} spinning />Loading agents…</p>
        {props.rosterLoading === 'skeleton' && <div aria-hidden="true" data-roster-skeleton>{[0.72, 0.54, 0.64, 0.46, 0.6, 0.5].map((share, index) => <div key={index} {...stylex.props(styles.skeletonRow, variant === 'SR-1' && styles.skeletonRowSingle)}><span {...stylex.props(styles.skeletonDisc)} /><span {...stylex.props(styles.skeletonLines)}><span {...stylex.props(styles.skeletonLine)} style={{ width: `${share * 100}%` }} />{variant === 'SR-2' && <span {...stylex.props(styles.skeletonLine, styles.skeletonMeta)} style={{ width: `${(share - 0.18) * 100}%` }} />}</span></div>)}</div>}
      </>}
      {state === 'empty' && <div data-roster-empty {...stylex.props(styles.message)}><p {...stylex.props(styles.messageTitle)}>No agents yet</p><p {...stylex.props(styles.messageBody)}>Agents appear here as soon as one starts.</p></div>}
      {state === 'error' && props.errorPlacement === 'sidebar' && <div role="alert" data-roster-error {...stylex.props(styles.message, styles.messageStart)}>
        <p {...stylex.props(styles.messageTitle, styles.errorTitle)}><Icon name="alert" size={14} />Couldn’t load agents</p>
        <p {...stylex.props(styles.messageBody)}>The gateway did not return the agent list. Open conversations and drafts are unchanged.</p>
        <div {...stylex.props(styles.actions)}><Button onPress={props.onRetry} {...stylex.props(styles.actionButton)}>Retry</Button>
          <Disclosure {...stylex.props(styles.disclosure)}><Button slot="trigger" {...stylex.props(styles.linkButton)}>Details</Button><DisclosurePanel><p data-roster-error-detail {...stylex.props(styles.detail)}>roster read failed · HTTP 503 · attempt 3 of 3</p></DisclosurePanel></Disclosure>
        </div>
      </div>}
      {state === 'error' && props.errorPlacement === 'banner' && <p data-roster-notice="unavailable" {...stylex.props(styles.notice)}>Agent list unavailable</p>}
      {showRows && visible.length === 0 && <div data-roster-no-match {...stylex.props(styles.message)}><p {...stylex.props(styles.messageTitle)}>No agents match “{query.trim()}”</p><Button onPress={() => props.onQueryChange('')} {...stylex.props(styles.actionButton)}>Clear filter</Button></div>}
      {showRows && visible.map(folder => {
        const open = needle !== '' || !closedFolders.has(folder.id)
        return <section key={folder.id} aria-label={folder.label} data-folder={folder.id}>
          <Button data-roster-stop aria-expanded={open} onPress={() => setClosedFolders(current => { const next = new Set(current); if (next.has(folder.id)) next.delete(folder.id); else next.add(folder.id); return next })} {...stylex.props(styles.folder)}>
            <Icon name={open ? 'chevron-down' : 'chevron-right'} size={12} /><span {...stylex.props(styles.folderLabel)}>{folder.label}</span><span {...stylex.props(styles.count)}>{folder.rows.length}</span>
          </Button>
          {open && <div {...stylex.props(styles.rows)}>{folder.rows.map(row => <SidebarAgentRow key={row.ref} item={row} variant={variant} layout="SR2-A" glyph="SG-1" discRefinement={glyphRefinement} now={props.now} query={query} active={row.ref === props.activeRef} treeExpanded={openParents.has(row.ref)} onOpen={props.onOpen} onToggleChildren={ref => setOpenParents(current => { const next = new Set(current); if (next.has(ref)) next.delete(ref); else next.add(ref); return next })} />)}</div>}
        </section>
      })}
    </div>
    {props.sectionNav === 'footer' && sections}
  </aside>
}

const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '-2px' } as const
const styles = stylex.create({
  sidebar: { display: 'flex', flexDirection: 'column', height: '100%', minWidth: 0, minHeight: 0, backgroundColor: surface.sidebar, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  header: { display: 'flex', alignItems: 'center', gap: s.xs, height: g.controlLg, flexShrink: 0, paddingInline: s.md, paddingBlockStart: s.xs },
  brand: { flex: '1 1 0', minWidth: 0, paddingInlineStart: s.xs, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightSemibold },
  iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, height: g.controlSm, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.sidebarFgMuted, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': focusRing },
  kbdButton: { display: 'inline-flex', alignItems: 'center', gap: s.xs, height: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.sidebarFgMuted, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': focusRing },
  kbd: { fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading, color: ink.fgMuted, paddingInline: s.xs, borderRadius: r.keyboard, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong },
  search: { display: 'flex', alignItems: 'center', gap: s.sm, height: g.controlMd, flexShrink: 0, marginInline: s.md, marginBlock: s.xs, paddingInline: s.sm, boxSizing: 'border-box', borderRadius: r.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, backgroundColor: surface.controlFill, color: ink.fgMuted, ':focus-within': { borderColor: accent.primary } },
  searchInput: { flex: '1 1 0', minWidth: 0, padding: 0, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, outline: 'none', '::placeholder': { color: ink.fgMuted }, '::-webkit-search-cancel-button': { display: 'none' } },
  clear: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.icon, height: g.icon, padding: 0, borderWidth: 0, borderRadius: r.keyboard, backgroundColor: surface.transparent, color: ink.fgMuted, cursor: 'pointer', ':focus-visible': focusRing },
  list: { flex: '1 1 0', minHeight: 0, overflowY: 'auto', scrollbarGutter: 'stable', scrollbarWidth: 'thin', scrollbarColor: `${tone.scrollThumb} transparent`, paddingInline: s.xs, paddingBlockEnd: s.md, display: 'flex', flexDirection: 'column', gap: s.xs2 },
  rows: { display: 'flex', flexDirection: 'column', gap: s.xs2 },
  folder: { display: 'flex', alignItems: 'center', gap: s.xs, width: '100%', height: g.controlSm, marginBlockStart: s.sm, paddingInline: s.sm, boxSizing: 'border-box', borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.sidebarFgMuted, fontFamily: t.fontSans, fontSize: t.denseSize, fontWeight: t.weightMedium, textAlign: 'start', cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': focusRing },
  folderLabel: { flex: '1 1 0', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  count: { color: ink.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  notice: { display: 'flex', alignItems: 'center', gap: s.sm, minHeight: g.controlSm, margin: 0, paddingInline: s.sm, color: ink.fgMuted, fontSize: t.denseSize },
  message: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: s.xs, margin: 0, paddingInline: s.sm, paddingBlock: s.lg },
  messageStart: { paddingBlockStart: s.md },
  messageTitle: { display: 'flex', alignItems: 'center', gap: s.sm, margin: 0, color: ink.fg, fontSize: t.metaSize, fontWeight: t.weightMedium },
  errorTitle: { color: tone.dangerFg },
  messageBody: { margin: 0, color: ink.fgMuted, fontSize: t.denseSize, lineHeight: t.metaLeading },
  actions: { display: 'flex', alignItems: 'flex-start', gap: s.md, marginBlockStart: s.xs },
  disclosure: { display: 'flex', flexDirection: 'column', gap: s.xs },
  actionButton: { height: g.controlSm, paddingInline: s.md, borderRadius: r.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.denseSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  linkButton: { height: g.controlSm, padding: 0, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.denseSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': focusRing },
  detail: { margin: 0, color: ink.fgMuted, fontFamily: t.fontMono, fontSize: t.denseSize, overflowWrap: 'anywhere' },
  skeletonRow: { display: 'grid', gridTemplateColumns: `${g.icon} minmax(0, 1fr)`, alignItems: 'center', columnGap: s.xs2, height: g.sidebarRow, paddingInline: s.sm },
  skeletonRowSingle: { height: g.controlLg },
  skeletonDisc: { justifySelf: 'center', width: s.md, height: s.md, borderRadius: r.full, backgroundColor: surface.rowActive },
  skeletonLines: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0 },
  skeletonLine: { height: s.md, borderRadius: r.keyboard, backgroundColor: surface.rowActive },
  skeletonMeta: { height: s.sm, backgroundColor: surface.rowHover },
  sections: { display: 'flex', alignItems: 'center', gap: s.xs2, flexShrink: 0, height: g.controlLg, paddingInline: s.xs, boxSizing: 'border-box' },
  sectionsFooter: { borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  sectionsHeader: { borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  sectionButton: { display: 'inline-flex', alignItems: 'center', gap: s.xs, height: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.sidebarFgMuted, fontFamily: t.fontSans, fontSize: t.denseSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': focusRing },
  visuallyHidden: { position: 'absolute', width: '1px', height: '1px', margin: '-1px', padding: 0, overflow: 'hidden', clip: 'rect(0 0 0 0)', whiteSpace: 'nowrap', borderWidth: 0 },
})
