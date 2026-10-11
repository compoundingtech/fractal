import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Cell, Column, Row, Table, TableBody, TableHeader } from 'react-aria-components'
import { accentVars as accent, borderVars as border, radiusVars as radius, spaceVars as space, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex'
import type { UsageMeterCell, UsageMeterFact, UsageMeterHistory, UsageMeterRow, UsageMeterSortKey, UsageMeterTableProps } from './usage-meter-types'
import { usageMeterGrid } from './UsageMeterGrid.stylex.ts'

export type { UsageMeterCell, UsageMeterDetail, UsageMeterFact, UsageMeterHistory, UsageMeterHistoryPoint, UsageMeterRow, UsageMeterSort, UsageMeterSortKey, UsageMeterTableProps, UsageMeterTone, UsageMeterWindow } from './usage-meter-types'

export type UsageMeterTableComponentProps = UsageMeterTableProps & {
  readonly style?: stylex.StyleXStyles<{
    width?: string | number
    minWidth?: string | number
    maxWidth?: string | number
    blockSize?: string | number
    flexGrow?: number
    flexShrink?: number
  }>
  /** Reports whether the comparison column is shown (container inline size at least 15rem), so callers can rank the folded +N. Pass a stable callback. */
  readonly onComparisonVisibilityChange?: (visible: boolean) => void
}

/** Container width at which the comparison column is shown; mirrors the `@container (width < 15rem)` styles. */
const comparisonMinRem = 15

const columns: readonly { readonly id: UsageMeterSortKey; readonly label: string; readonly narrowLabel?: string }[] = [
  { id: 'account', label: 'Acct' },
  { id: 'governing', label: 'Window gov', narrowLabel: 'gov' },
  { id: 'reset', label: 'Reset', narrowLabel: 'rst' },
  { id: 'comparison', label: 'Comparison', narrowLabel: 'nxt' },
  { id: 'more', label: 'More', narrowLabel: '+N' },
]
const isSortKey = (key: string | number): key is UsageMeterSortKey => columns.some(column => column.id === key)

const Meter = ({ meter, stale, compact = false }: { readonly meter: UsageMeterCell; readonly stale: boolean; readonly compact?: boolean }) => <div data-usage-meter-tone={meter.tone} {...stylex.props(styles.meter, compact && styles.compactMeter)}>
  <span title={compact ? meter.label : undefined} {...stylex.props(styles.windowLabel, compact && styles.compactWindow)}>{meter.label}</span>
  <div {...stylex.props(styles.meterReading, compact && styles.compactReading)}>
    {meter.fraction !== undefined && meter.tone !== 'unknown' && <span aria-hidden="true" data-usage-meter-bar {...stylex.props(styles.track, stale && styles.staleBar)}>
      <span {...stylex.props(styles.fill, meter.tone === 'warning' && styles.warningFill, (meter.tone === 'critical' || meter.tone === 'exhausted') && styles.dangerFill, styles.fraction(meter.fraction))} />
    </span>}
    <span {...stylex.props(styles.reading, meter.tone === 'warning' && styles.warningText, (meter.tone === 'critical' || meter.tone === 'exhausted') && styles.dangerText, stale && styles.staleText)}>{meter.text}</span>
  </div>
</div>

const Facts = ({ facts }: { readonly facts: readonly UsageMeterFact[] }) => <dl {...stylex.props(styles.facts)}>
  {facts.map((fact, index) => <React.Fragment key={index}><dt {...stylex.props(styles.factLabel)}>{fact.label}</dt><dd {...stylex.props(styles.factValue)}>{fact.value}</dd></React.Fragment>)}
</dl>

/** Supplied period values as bars; unmeasured periods are gaps and open periods are marked. */
export const UsageMeterHistoryChart = ({ history }: { readonly history: UsageMeterHistory }): React.ReactElement => {
  if (history.kind !== 'ready') return <section aria-label={history.label} data-usage-meter-history={history.kind} {...stylex.props(styles.history)}>
    <h4 {...stylex.props(styles.title)}>{history.label}</h4>
    <p role={history.kind === 'unavailable' ? 'alert' : 'status'} {...stylex.props(styles.staleText)}>{history.text}</p>
  </section>
  const max = Math.max(0, ...history.points.flatMap(point => point.value === undefined ? [] : [point.value]))
  const first = history.points[0]
  const last = history.points.at(-1)
  return <section aria-label={history.label} data-usage-meter-history="ready" {...stylex.props(styles.history)}>
    <h4 {...stylex.props(styles.title)}>{history.label}</h4>
    <p {...stylex.props(styles.identity)}>{history.text}</p>
    {history.points.length === 0 ? <p {...stylex.props(styles.staleText)}>No periods supplied</p> : <>
      <ol aria-label={`${history.label} by period`} {...stylex.props(styles.bars)}>
        {history.points.map(point => <li key={point.id} aria-label={`${point.label}: ${point.text}${point.partial ? ', period still open' : ''}`} title={`${point.label}: ${point.text}${point.partial ? ' (open)' : ''}`} data-usage-meter-history-point={point.id} data-partial={point.partial || undefined} data-unmeasured={point.value === undefined || undefined} {...stylex.props(styles.slot)}>
          {point.value === undefined
            ? <span aria-hidden="true" {...stylex.props(styles.gap)} />
            : <span aria-hidden="true" {...stylex.props(styles.bar, point.partial && styles.partialBar, styles.barHeight(max === 0 ? 0 : point.value / max))} />}
        </li>)}
      </ol>
      <p aria-hidden="true" {...stylex.props(styles.axis)}><span>{first?.label}</span><span>{last?.label}</span></p>
    </>}
    <Facts facts={history.facts} />
  </section>
}

/** Complete supplied evidence, without reconstructing windows or interpreting source policy. */
export const UsageMeterRowDetail = ({ row, history, onClose }: { readonly row: UsageMeterRow; readonly history?: UsageMeterHistory; readonly onClose: () => void }): React.ReactElement => <section aria-label={`${row.label} usage details`} data-usage-meter-detail={row.id} {...stylex.props(styles.detail)}>
  <header {...stylex.props(styles.detailHeader)}><h3 {...stylex.props(styles.title)}>{row.label}</h3><Button aria-label="Close usage details" onPress={onClose} {...stylex.props(styles.button)}>Close</Button></header>
  <p {...stylex.props(styles.identity)}>{row.identity}</p>
  {row.stale && <p {...stylex.props(styles.staleText)}>Stale · last-known values</p>}
  {row.note !== undefined && <p {...stylex.props(styles.note)}>{row.note}</p>}
  <Facts facts={row.detail.facts} />
  <h4 {...stylex.props(styles.title)}>Windows</h4>
  {row.detail.windows.map(window => <section key={window.id} aria-label={window.label} {...stylex.props(styles.detailWindow)}>
    <h5 {...stylex.props(styles.title)}>{window.label}</h5>
    <Meter meter={window.meter} stale={row.stale} />
    <Facts facts={window.facts} />
  </section>)}
  <h4 {...stylex.props(styles.title)}>Resets</h4>
  <p {...stylex.props(styles.reset)}>{row.reset}{row.resetCount !== undefined && <span aria-label={`${row.resetCount} reported resets`}> ↺{row.resetCount}</span>}</p>
  <Facts facts={row.detail.resets} />
  {history !== undefined && <UsageMeterHistoryChart history={history} />}
</section>

/** Controlled, stable-id monitor. Keyboard shortcuts are scoped to this focus surface. */
export const UsageMeterTable = ({ rows, status: observation, sort, onSort, onCycleSort, selectedId, onSelect, onCloseDetail, history, onComparisonVisibilityChange, style }: UsageMeterTableComponentProps): React.ReactElement => {
  const selected = rows.find(row => row.id === selectedId)
  const observeComparison = React.useCallback((element: HTMLElement | null) => {
    if (element === null || onComparisonVisibilityChange === undefined) return
    let visible: boolean | undefined
    const observer = new ResizeObserver(entries => {
      const entry = entries.at(-1)
      if (entry === undefined) return
      const rem = Number.parseFloat(getComputedStyle(element.ownerDocument.documentElement).fontSize) || 16
      const next = entry.contentRect.width >= comparisonMinRem * rem
      if (next === visible) return
      visible = next
      onComparisonVisibilityChange(next)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [onComparisonVisibilityChange])
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.defaultPrevented || event.repeat || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.key !== 's') return
    if (!event.currentTarget.contains(event.currentTarget.ownerDocument.activeElement)) return
    const target = event.target
    if (!(target instanceof Element) || target.closest('input, textarea, [contenteditable]:not([contenteditable="false"]), [role="textbox"]') !== null) return
    event.preventDefault()
    event.stopPropagation()
    onCycleSort()
  }
  return <section ref={observeComparison} aria-label="Usage monitor" tabIndex={0} onKeyDownCapture={onKeyDown} data-usage-meter-table {...stylex.props(styles.root, style)}>
    <header {...stylex.props(styles.toolbar)}>
      <p role="status" data-usage-meter-status={observation.kind} {...stylex.props(styles.statusText, observation.kind !== 'current' && styles.staleText)}>{observation.text}</p>
      <Button onPress={onCycleSort} aria-label="Cycle usage sort" aria-keyshortcuts="s" {...stylex.props(styles.button)}>Sort <kbd {...stylex.props(styles.shortcut)}>s</kbd></Button>
    </header>
    <Table aria-label="Usage meters" selectionMode="single" selectionBehavior="replace" disallowEmptySelection selectedKeys={selected === undefined ? [] : [selected.id]} onSelectionChange={keys => {
      if (keys === 'all') return
      const id = keys.values().next().value
      if (typeof id === 'string') onSelect(id)
    }} sortDescriptor={{ column: sort.key, direction: sort.direction }} onSortChange={descriptor => {
      if (isSortKey(descriptor.column) && descriptor.direction !== undefined) onSort({ key: descriptor.column, direction: descriptor.direction })
    }} {...stylex.props(usageMeterGrid.table, styles.table)}>
      <TableHeader {...stylex.props(usageMeterGrid.header)}>
        {columns.map(column => <Column key={column.id} id={column.id} isRowHeader={column.id === 'account'} allowsSorting data-usage-meter-column={column.id} {...stylex.props(usageMeterGrid.column, styles.header, column.id === 'account' && styles.accountColumn, column.id === 'governing' && styles.governingColumn, column.id === 'reset' && styles.resetColumn, column.id === 'comparison' && styles.comparisonColumn, column.id === 'more' && styles.moreColumn)}>
          {({ sortDirection }) => <span {...stylex.props(styles.headerContent)}>{column.narrowLabel === undefined ? column.label : <><span {...stylex.props(styles.wideLabel)}>{column.label}</span><span aria-hidden="true" {...stylex.props(styles.narrowHeaderLabel)}>{column.narrowLabel}</span></>}<span aria-hidden="true" {...stylex.props(styles.sortIndicator)}>{sortDirection === 'ascending' ? '↑' : sortDirection === 'descending' ? '↓' : ''}</span></span>}
        </Column>)}
      </TableHeader>
      <TableBody {...stylex.props(usageMeterGrid.body)} renderEmptyState={() => <span {...stylex.props(styles.empty)}>No usage rows supplied</span>}>
        {rows.map(row => <Row key={row.id} id={row.id} textValue={row.label} data-usage-meter-row={row.id} data-stale={row.stale || undefined} {...stylex.props(usageMeterGrid.row, styles.row, selected?.id === row.id && styles.selectedRow)}>
          <Cell {...stylex.props(usageMeterGrid.cell, styles.cell)}><span data-usage-meter-account title={`${row.label} · ${row.identity}${row.stale ? ' · Stale' : ''}${row.note === undefined ? '' : ` · ${row.note}`}`} {...stylex.props(usageMeterGrid.truncate, styles.accountLabel)}>{row.identity}{row.stale && <span {...stylex.props(styles.staleText)}> · Stale</span>}{row.note !== undefined && <span aria-label={row.note}> !</span>}</span></Cell>
          <Cell {...stylex.props(usageMeterGrid.cell, styles.cell)}><Meter meter={row.governing} stale={row.stale} compact /></Cell>
          <Cell {...stylex.props(usageMeterGrid.cell, styles.cell)}><span {...stylex.props(styles.reset)}>{row.reset}{row.resetCount !== undefined && <span aria-label={`${row.resetCount} reported resets`}> ↺{row.resetCount}</span>}</span></Cell>
          <Cell {...stylex.props(usageMeterGrid.cell, styles.cell, styles.comparisonColumn)}>{row.comparison === undefined ? <span {...stylex.props(styles.staleText)}>—</span> : <Meter meter={row.comparison} stale={row.stale} compact />}</Cell>
          <Cell {...stylex.props(usageMeterGrid.cell, styles.cell)}><Button aria-label={`Show all windows and evidence for ${row.label}`} aria-expanded={selected?.id === row.id} onPress={() => onSelect(row.id)} {...stylex.props(styles.moreButton)}>
            <span {...stylex.props(styles.wideCount)}>+{row.omitted}</span><span {...stylex.props(styles.narrowCount)}>+{row.omitted + (row.comparison === undefined ? 0 : 1)}</span>
          </Button></Cell>
        </Row>)}
      </TableBody>
    </Table>
    {selected !== undefined && <UsageMeterRowDetail row={selected} history={history} onClose={onCloseDetail} />}
  </section>
}

const styles = stylex.create({
  root: { containerType: 'inline-size', minWidth: 0, width: '100%', color: ink.fg, backgroundColor: surface.canvas, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  toolbar: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.md, padding: space.md },
  statusText: { margin: 0, minWidth: 0, overflowWrap: 'anywhere', color: ink.fgSoft },
  button: { display: 'inline-flex', alignItems: 'center', gap: space.sm, flexShrink: 0, color: ink.fgSoft, backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover, ':active': surface.rowActive }, border: `1px solid ${border.borderStrong}`, borderRadius: radius.keyboard, padding: `${space.xs} ${space.sm}`, font: 'inherit', cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '2px' },
  shortcut: { color: ink.fgMuted, fontFamily: type.fontMono, fontSize: type.denseSize },
  table: { textAlign: 'start', fontFamily: type.fontMono, fontSize: type.denseSize },
  header: { zIndex: 1, color: ink.fgMuted, fontSize: type.denseSize, fontWeight: type.weightRegular, cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  headerContent: { display: 'flex', alignItems: 'center', gap: '2px', whiteSpace: 'nowrap', overflow: 'hidden' },
  sortIndicator: { display: { default: 'inline-block', ':empty': 'none' }, flexShrink: 0, color: ink.fgSoft },
  accountColumn: { width: { default: '22%', '@container (width < 15rem)': '41%' } },
  governingColumn: { width: { default: '25%', '@container (width < 15rem)': '30%' } },
  resetColumn: { width: { default: '19%', '@container (width < 15rem)': '20%' } },
  comparisonColumn: { width: '25%', display: { default: 'table-cell', '@container (width < 15rem)': 'none' } },
  moreColumn: { width: '9%' },
  row: { backgroundColor: { default: surface.transparent, ':hover': surface.rowHover }, cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  selectedRow: { backgroundColor: surface.rowActive },
  cell: { outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  accountLabel: { color: ink.fg },
  identity: { display: 'block', margin: 0, color: ink.fgMuted, fontSize: type.denseSize, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  note: { display: 'block', margin: 0, color: ink.fgMuted, fontSize: type.denseSize, overflowWrap: 'anywhere' },
  meter: { minWidth: 0 },
  compactMeter: { display: 'flex', alignItems: 'center', gap: '2px', whiteSpace: 'nowrap' },
  compactWindow: { flexShrink: 0, maxWidth: '5ch' },
  compactReading: { flex: 1, gap: '2px' },
  windowLabel: { display: 'block', color: ink.fgMuted, fontSize: type.denseSize, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  meterReading: { display: 'flex', alignItems: 'center', gap: space.xs, minWidth: 0 },
  reading: { flexShrink: 0, color: ink.fgSoft, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  track: { display: 'block', flexBasis: '56px', flexGrow: 1, flexShrink: 1, minWidth: 0, maxWidth: '72px', height: '4px', overflow: 'hidden', borderRadius: radius.full, backgroundColor: border.borderStrong },
  fill: { display: 'block', height: '100%', backgroundColor: status.done },
  fraction: (fraction: number) => ({ width: `${fraction * 100}%` }),
  warningFill: { backgroundColor: status.attention },
  dangerFill: { backgroundColor: status.danger },
  warningText: { color: status.attention },
  dangerText: { color: status.dangerFg },
  staleText: { color: ink.fgMuted },
  staleBar: { opacity: 0.4 },
  reset: { margin: 0, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  moreButton: { display: 'inline-flex', alignItems: 'center', height: '18px', font: 'inherit', fontVariantNumeric: 'tabular-nums', color: ink.fgSoft, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover }, border: 0, padding: 0, cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  wideCount: { display: { default: 'inline', '@container (width < 15rem)': 'none' } },
  narrowCount: { display: { default: 'none', '@container (width < 15rem)': 'inline' } },
  narrowHeaderLabel: { display: { default: 'none', '@container (width < 36rem)': 'inline' }, minWidth: 0, overflow: 'hidden' },
  /** Stays the accessible column name when the container shows the short label. */
  wideLabel: { position: { default: 'static', '@container (width < 36rem)': 'absolute' }, width: { default: 'auto', '@container (width < 36rem)': '1px' }, height: { default: 'auto', '@container (width < 36rem)': '1px' }, overflow: { default: 'visible', '@container (width < 36rem)': 'hidden' }, clipPath: { default: 'none', '@container (width < 36rem)': 'inset(50%)' } },
  empty: { display: 'block', padding: space.xl, color: ink.fgMuted },
  detail: { padding: space.lg, backgroundColor: surface.raised, borderTop: `1px solid ${border.borderStrong}` },
  detailHeader: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.md },
  title: { margin: `${space.md} 0`, fontSize: type.metaSize, fontWeight: type.weightMedium, color: ink.fg },
  detailWindow: { paddingBlock: space.sm, borderBottom: `1px solid ${border.border}` },
  facts: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 2fr)', gap: `${space.xs} ${space.md}`, marginBlock: space.md },
  factLabel: { color: ink.fgMuted, overflowWrap: 'anywhere' },
  factValue: { margin: 0, color: ink.fgSoft, overflowWrap: 'anywhere', whiteSpace: 'pre-wrap' },
  history: { paddingBlock: space.sm, borderTop: `1px solid ${border.border}`, marginBlockStart: space.md },
  bars: { display: 'flex', alignItems: 'flex-end', gap: '1px', blockSize: '48px', margin: 0, padding: 0, listStyle: 'none', borderBottom: `1px solid ${border.borderStrong}` },
  slot: { display: 'flex', alignItems: 'flex-end', flexBasis: 0, flexGrow: 1, flexShrink: 1, minWidth: 0, blockSize: '100%' },
  bar: { display: 'block', inlineSize: '100%', minBlockSize: '1px', backgroundColor: accent.primary },
  barHeight: (fraction: number) => ({ blockSize: `${fraction * 100}%` }),
  partialBar: { opacity: 0.45, outline: `1px dashed ${accent.primary}`, outlineOffset: '-1px' },
  gap: { display: 'block', inlineSize: '100%', blockSize: '100%', backgroundImage: `repeating-linear-gradient(135deg, ${border.border} 0 2px, transparent 2px 5px)` },
  axis: { display: 'flex', justifyContent: 'space-between', gap: space.md, margin: `${space.xs} 0 0`, color: ink.fgMuted, fontSize: type.denseSize, fontVariantNumeric: 'tabular-nums' },
})
