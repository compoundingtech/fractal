import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, statusVars as status, typeVars as type, spaceVars as space, radiusVars as radius, geometryVars as geometry, elevationVars as elevation } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { PaletteMode, PaletteRow, PaletteSection } from './model.ts'

export interface CommandPaletteProps {
  readonly isOpen: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly mode: PaletteMode
  readonly onModeChange: (mode: PaletteMode) => void
  readonly query: string
  readonly onQueryChange: (query: string) => void
  readonly sections: readonly PaletteSection[]
  readonly onSelect: (row: PaletteRow) => void
  /** Platform chord shown on the footer, e.g. `⌘K` or `Ctrl K`. */
  readonly shortcut: string
  /** Inventory caveat (waiting, partial, unavailable); a missing match is then not proof of absence. */
  readonly inventoryNote?: string
  /** Themed container inside the workspace; the default body portal would lose the theme variables. */
  readonly portalContainer?: Element | null
}

/** Highlighted UTF-16 positions as `<mark>` runs; plain text when nothing matched. */
const Highlight = ({ text, positions }: { readonly text: string; readonly positions: readonly number[] }) => {
  if (positions.length === 0) return <>{text}</>
  const marked = new Set(positions)
  const runs: { readonly text: string; readonly marked: boolean }[] = []
  for (let index = 0; index < text.length; index++) {
    const isMarked = marked.has(index)
    const last = runs.at(-1)
    if (last !== undefined && last.marked === isMarked) runs[runs.length - 1] = { text: last.text + text[index], marked: isMarked }
    else runs.push({ text: text[index]!, marked: isMarked })
  }
  return <>{runs.map((run, index) => run.marked ? <mark key={index} {...stylex.props(styles.mark)}>{run.text}</mark> : <React.Fragment key={index}>{run.text}</React.Fragment>)}</>
}

const RowContent = ({ row }: { readonly row: PaletteRow }) => <>
  <span {...stylex.props(styles.rowMain)}>
    <span slot="label" {...stylex.props(styles.rowTitle)}><Highlight text={row.title} positions={row.titleHighlights} /></span>
    {/* Detail stays in the visible accessible name, not also in aria-describedby. */}
    <span {...stylex.props(styles.rowDetail)}>
      {row.context.map((segment, index) => <React.Fragment key={index}>{index > 0 ? ' · ' : null}<span {...stylex.props(styles.segment)}><Highlight text={segment.text} positions={segment.highlights} /></span></React.Fragment>)}
      {row.state === '' ? null : <> · {row.state}</>}
      {row.disabledReason === undefined ? null : <> · {row.disabledReason}</>}
    </span>
  </span>
  {row.marks.length === 0 && row.shortcut === undefined ? null : <span {...stylex.props(styles.marks)}>
    {row.marks.map(mark => <span key={mark} {...stylex.props(styles.markLabel)}>{mark}</span>)}
    {row.shortcut === undefined ? null : <kbd {...stylex.props(styles.kbd)}>{row.shortcut}</kbd>}
  </span>}
</>

/**
 * Top-anchored switcher (P2) with semantic sections (G1). React Aria owns virtual focus, arrow keys, Enter and
 * focus containment; the input keeps DOM focus while `aria-activedescendant` follows the focused row.
 */
export function CommandPalette({ isOpen, onOpenChange, mode, onModeChange, query, onQueryChange, sections, onSelect, shortcut, inventoryNote, portalContainer }: CommandPaletteProps) {
  const rows = React.useMemo(() => new Map(sections.flatMap(section => section.rows.map(row => [row.id, row] as const))), [sections])
  const disabledKeys = React.useMemo(() => [...rows.values()].filter(row => row.disabledReason !== undefined).map(row => row.id), [rows])
  const needsYou = sections.find(section => section.id === 'needs-you')
  const urgent = needsYou?.total ?? needsYou?.rows.length ?? 0
  const boundary = React.useRef<HTMLDivElement | null>(null)
  React.useLayoutEffect(() => {
    const node = boundary.current
    if (!isOpen || node === null) return
    const dismiss = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.isComposing) return
      event.preventDefault()
      event.stopImmediatePropagation()
      onOpenChange(false)
    }
    // One Escape closes the palette from every descendant. Without this capture SearchField spends the first
    // Escape clearing the query, and the Results region would need its own handler.
    node.addEventListener('keydown', dismiss, true)
    return () => node.removeEventListener('keydown', dismiss, true)
  })
  const title = mode === 'Switch' ? 'Switch agent' : 'Commands'
  return <Aria.ModalOverlay isOpen={isOpen} onOpenChange={onOpenChange} isDismissable UNSTABLE_portalContainer={portalContainer ?? undefined} {...stylex.props(styles.overlay)}>
    <Aria.Modal {...stylex.props(styles.modal)}>
      <Aria.Dialog aria-label="Switch agents or run commands" {...stylex.props(styles.dialog)}>
        <div ref={boundary} data-command-palette {...stylex.props(styles.column)}>
          <div {...stylex.props(styles.heading)}>
            <strong {...stylex.props(styles.title)}>{title}</strong>
            <div role="group" aria-label="Palette mode" {...stylex.props(styles.modes)}>
              {(['Switch', 'Commands'] as const).map(value => <Aria.Button key={value} aria-pressed={mode === value} onPress={() => onModeChange(value)} {...stylex.props(styles.button, mode === value && styles.buttonPressed)}>{value}</Aria.Button>)}
            </div>
            <Aria.Button onPress={() => onOpenChange(false)} {...stylex.props(styles.button)}>Close <kbd {...stylex.props(styles.kbd)}>Esc</kbd></Aria.Button>
          </div>
          {/* disableAutoFocusFirst: Autocomplete otherwise delays aria-activedescendant for an unchanged first row,
              so the first ArrowDown after typing announces one row while another is focused. */}
          {/* A new mode owns a new RAC collection and virtual-focus state. */}
          <Aria.Autocomplete key={mode} disableAutoFocusFirst inputValue={query} onInputChange={onQueryChange}>
            <Aria.SearchField aria-label="Search agents, missions and commands" {...stylex.props(styles.search)}>
              <Aria.Input autoFocus placeholder={mode === 'Switch' ? 'Agent, folder, host or mission…' : 'Command…'} {...stylex.props(styles.input)} />
            </Aria.SearchField>
            {mode === 'Switch' && urgent > 0 ? <p {...stylex.props(styles.pinned)}>Needs you · {urgent} {urgent === 1 ? 'agent' : 'agents'} pinned first</p> : null}
            {inventoryNote === undefined ? null : <p role="status" {...stylex.props(styles.note)}>{inventoryNote}</p>}
            {/* Live rosters plus missions reach hundreds of rows; only the visible window mounts. */}
            {rows.size === 0 ? null : <div {...stylex.props(styles.results)}>
              <Aria.Virtualizer layout={Aria.ListLayout} layoutOptions={layoutOptions}>
              <Aria.Menu render={renderResultsMenu} aria-label={`${title} results`} disabledKeys={disabledKeys} onAction={key => { const row = rows.get(String(key)); if (row !== undefined && row.disabledReason === undefined) onSelect(row) }} {...stylex.props(styles.menu)}>
                {sections.map(section => <Aria.MenuSection key={section.id} id={section.id} {...stylex.props(styles.section)}>
                  <Aria.Header {...stylex.props(styles.sectionHeader, section.id === 'needs-you' && styles.urgent)}>{section.title} · {section.total ?? section.rows.length}</Aria.Header>
                  {/* Render functions defer row content until the virtualizer mounts the row. */}
                  {section.rows.map(row => <Aria.MenuItem key={row.id} id={row.id} textValue={row.title} {...rowProps}>{() => <RowContent row={row} />}</Aria.MenuItem>)}
                </Aria.MenuSection>)}
              </Aria.Menu>
              </Aria.Virtualizer>
            </div>}
          </Aria.Autocomplete>
          {rows.size === 0 ? <div role="status" {...stylex.props(styles.empty)}>
            <strong>{mode === 'Switch' ? 'No matching agents or missions' : 'No matching commands'}</strong>
            <p {...stylex.props(styles.emptyHint)}>{mode === 'Switch' ? 'Try a name, folder, host or mission.' : 'Try another command name.'}{inventoryNote === undefined ? '' : ' The inventory is incomplete, so a missing match does not mean the agent does not exist.'}</p>
            {query === '' ? null : <Aria.Button onPress={() => onQueryChange('')} {...stylex.props(styles.button)}>Clear search</Aria.Button>}
          </div> : null}
          <footer {...stylex.props(styles.footer)}>
            <span>↑ ↓ move · Enter {mode === 'Switch' ? 'opens' : 'runs'} · Esc closes</span>
            <span>{shortcut}</span>
          </footer>
        </div>
      </Aria.Dialog>
    </Aria.Modal>
  </Aria.ModalOverlay>
}

const layoutOptions = { estimatedRowHeight: 48, estimatedHeadingHeight: 30, padding: 6 }
// A tab stop within the virtualizer's scroller gives keyboard users direct scrolling access without
// changing Autocomplete's own menu tabIndex or transferring arrow-key focus away from its input.
const renderResultsMenu = (props: React.ComponentPropsWithRef<'div'>) => <div {...props}><div role="group" aria-label="Results" tabIndex={0}>{props.children}</div></div>

const styles = stylex.create({
  overlay: {
    position: 'fixed', insetInline: 0, insetBlockStart: 0, insetBlockEnd: 'var(--wf-devbar-space, 0px)', zIndex: 110,
    display: 'flex', justifyContent: 'center', alignItems: 'flex-start', backgroundColor: 'rgba(0, 0, 0, 0.4)',
    paddingInline: { default: space.md, '@media (min-width: 640px)': space.xl },
    paddingBlockStart: { default: `max(${space.md}, env(safe-area-inset-top))`, '@media (min-width: 640px)': 'min(4rem, 8dvh)' },
    paddingBlockEnd: `max(${space.md}, env(safe-area-inset-bottom))`,
  },
  modal: {
    width: '100%', maxWidth: 640, maxHeight: '100%', minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden',
    backgroundColor: surface.raised, color: ink.fg, borderWidth: 1, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.md,
    boxShadow: elevation.dialog, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading,
  },
  dialog: { display: 'flex', flexDirection: 'column', minHeight: 0, maxHeight: '100%', outline: 'none' },
  column: { display: 'flex', flexDirection: 'column', minHeight: 0, maxHeight: '100%' },
  heading: { flexShrink: 0, display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.md, padding: space.lg, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: border.border },
  title: { marginInlineEnd: 'auto', fontSize: type.uiSize, lineHeight: type.uiLeading, fontWeight: type.weightSemibold },
  modes: { display: 'flex', gap: space.xs },
  button: {
    display: 'inline-flex', alignItems: 'center', gap: space.sm, height: geometry.controlMd, paddingInline: space.md, borderRadius: radius.control,
    borderWidth: 1, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fgSoft,
    fontFamily: 'inherit', fontSize: type.metaSize, cursor: 'pointer',
    ':hover': { backgroundColor: surface.rowHover, color: ink.fg },
    ':focus-visible': { outline: `2px solid ${accent.primary}`, outlineOffset: 1 },
  },
  buttonPressed: { borderColor: accent.primary, backgroundColor: surface.rowActive, color: ink.fg },
  kbd: { fontFamily: type.fontSans, fontSize: type.denseSize, color: ink.fgSoft, borderWidth: 1, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.keyboard, paddingInline: space.xs },
  search: { flexShrink: 0, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: border.border },
  input: {
    width: '100%', boxSizing: 'border-box', padding: space.lg, backgroundColor: 'transparent', color: ink.fg, borderWidth: 0, outline: 'none',
    fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading,
    '::placeholder': { color: ink.fgMuted },
    '::-webkit-search-cancel-button': { display: 'none' },
  },
  pinned: { flexShrink: 0, margin: 0, paddingBlock: space.sm, paddingInline: space.lg, color: status.attention, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: border.border },
  note: { flexShrink: 0, margin: 0, paddingBlock: space.sm, paddingInline: space.lg, color: ink.fgMuted },
  results: { flexGrow: 1, flexShrink: 1, minHeight: 0, display: 'flex', flexDirection: 'column' },
  menu: {
    flexGrow: 1, flexShrink: 1, minHeight: 0, overflowY: 'auto', overscrollBehavior: 'contain', outline: 'none', paddingInline: space.sm,
    ':focus-visible': { boxShadow: `inset 0 0 0 2px ${accent.primary}` },
  },
  section: { paddingBlockEnd: space.xs },
  sectionHeader: { paddingInline: space.md, paddingBlockStart: space.md, paddingBlockEnd: space.xs, color: ink.fgMuted, fontSize: type.denseSize, fontWeight: type.weightMedium, letterSpacing: '0.04em', textTransform: 'uppercase' },
  urgent: { color: status.attention },
  row: {
    display: 'flex', alignItems: 'center', gap: space.md, minHeight: geometry.menuRow, paddingBlock: space.sm, paddingInline: space.md,
    borderRadius: radius.control, outline: 'none', cursor: 'pointer', color: ink.fgSoft,
    ':is([data-hovered])': { backgroundColor: surface.rowHover },
    ':is([data-focused])': { backgroundColor: surface.rowActive, color: ink.fg, boxShadow: `inset 2px 0 0 ${accent.primary}` },
    ':is([data-disabled])': { cursor: 'not-allowed', color: ink.fgMuted },
  },
  rowMain: { flexGrow: 1, minWidth: 0, display: 'flex', flexDirection: 'column' },
  rowTitle: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: type.uiSize, lineHeight: type.uiLeading, color: 'inherit' },
  rowDetail: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgSoft, fontSize: type.metaSize },
  segment: { overflowWrap: 'anywhere' },
  marks: { flexShrink: 0, display: 'flex', alignItems: 'center', gap: space.sm, fontSize: type.denseSize, color: ink.fgSoft },
  markLabel: { whiteSpace: 'nowrap' },
  mark: { backgroundColor: 'transparent', color: ink.fg, fontWeight: type.weightSemibold, textDecorationLine: 'underline', textDecorationColor: accent.primary, textDecorationThickness: 2, textUnderlineOffset: 3 },
  empty: { flexShrink: 0, padding: space.xl, display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: space.md },
  emptyHint: { margin: 0, color: ink.fgMuted },
  footer: { flexShrink: 0, display: 'flex', flexWrap: 'wrap', justifyContent: 'space-between', gap: space.md, paddingBlock: space.md, paddingInline: space.lg, borderTopWidth: 1, borderTopStyle: 'solid', borderTopColor: border.border, color: ink.fgMuted, fontSize: type.denseSize },
})
const rowProps = stylex.props(styles.row)
