import * as React from 'react'
import { createPortal } from 'react-dom'
import * as stylex from '@stylexjs/stylex'
import { quotaGeometry } from './quota-geometry.stylex.ts'
import { Icon } from './assistant-ui/composition/Icons.tsx'
import { Button, Cell, Column, Focusable, Row, Table, TableBody, TableHeader, VisuallyHidden } from 'react-aria-components'
import { accentVars as accent, borderVars as border, radiusVars as radius, spaceVars as space, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex'
import type { UsageMeterCell, UsageMeterFact, UsageMeterHistory, UsageMeterHistoryPoint, UsageMeterRow, UsageMeterSortKey, UsageMeterTableProps } from './usage-meter-types'
import { usageMeterGrid, usageMeterGridVars } from './UsageMeterGrid.stylex.ts'
import { FullValueTooltip, ValueTooltipTrigger } from './FullValueTooltip.tsx'

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
  /** Reports whether the comparison column is shown (container inline size at least 20rem), so callers can rank the folded +N. Pass a stable callback. */
  readonly onComparisonVisibilityChange?: (visible: boolean) => void
  /** Undefined keeps the inline toolbar; null waits for the host's shared header to mount. */
  readonly toolbarTarget?: HTMLElement | null
}

/** Container width at which the comparison column is shown; mirrors the `@container (width < 20rem)` styles. */
const comparisonMinRem = 20

const columns: readonly { readonly id: UsageMeterSortKey; readonly label: string; readonly narrowLabel?: string }[] = [
  { id: 'account', label: 'Account' },
  { id: 'governing', label: 'Window' },
  { id: 'reset', label: 'Reset' },
  { id: 'comparison', label: 'Next' },
  { id: 'more', label: 'More', narrowLabel: 'Δ' },
]
const isSortKey = (key: string | number): key is UsageMeterSortKey => columns.some(column => column.id === key)
const resetEvidence = (row: UsageMeterRow): string =>
  `Reset ${row.reset}${row.resetAt === undefined ? '' : ` · ${row.resetAt}`} · ${row.resetCount === undefined ? 'Reset count Unreported' : `${row.resetCount} reported resets`}`

/** Detail meters have room for label and reading; compact cells clip them, so one focusable disclosure carries both in full. */
const Meter = ({ meter, stale, compact = false }: { readonly meter: UsageMeterCell; readonly stale: boolean; readonly compact?: boolean }) => {
  const danger = meter.tone === 'critical' || meter.tone === 'exhausted'
  const label = <span data-usage-meter-window {...stylex.props(styles.windowLabel, compact && styles.compactWindow)}>{meter.label}</span>
  const reading = <span {...stylex.props(styles.meterReading, compact && styles.compactReading)}>
    {meter.fraction !== undefined && meter.tone !== 'unknown' && <span aria-hidden="true" data-usage-meter-bar {...stylex.props(styles.track, stale && styles.staleBar, compact && styles.compactTrack)}>
      <span {...stylex.props(styles.fill, meter.tone === 'warning' && styles.warningFill, danger && styles.dangerFill, styles.fraction(meter.fraction))} />
    </span>}
    <span data-usage-meter-reading {...stylex.props(styles.reading, compact && styles.compactReadingText, meter.tone === 'warning' && styles.warningText, danger && styles.dangerText, stale && styles.staleText)}>{meter.text}</span>
  </span>
  return compact
    ? <FullValueTooltip full={`${meter.label} · ${meter.text}`} data={{ 'data-usage-meter-tone': meter.tone }} style={styles.meter} visibleStyle={styles.compactMeter} valueData={{ 'data-usage-meter-full-value': true }}>{label}{reading}</FullValueTooltip>
    : <div data-usage-meter-tone={meter.tone} {...stylex.props(styles.meter)}>{label}{reading}</div>
}

const Facts = ({ facts }: { readonly facts: readonly UsageMeterFact[] }) => <dl {...stylex.props(styles.facts)}>
  {facts.map((fact, index) => <React.Fragment key={index}><dt {...stylex.props(styles.factLabel)}>{fact.label}</dt><dd {...stylex.props(styles.factValue)}>{fact.value}</dd></React.Fragment>)}
</dl>

/** One tab stop: arrows, Home and End move between periods; hover or focus opens that period's tooltip. */
const HistoryBars = ({ label, points }: { readonly label: string; readonly points: readonly UsageMeterHistoryPoint[] }): React.ReactElement => {
  const [active, setActive] = React.useState(points.length - 1)
  const current = Math.min(active, points.length - 1)
  const max = Math.max(0, ...points.flatMap(point => point.value === undefined ? [] : [point.value]))
  return <ol aria-label={`${label} by period`} onKeyDown={event => {
    const next = event.key === 'ArrowLeft' ? current - 1 : event.key === 'ArrowRight' ? current + 1 : event.key === 'Home' ? 0 : event.key === 'End' ? points.length - 1 : -1
    if (next < 0 || next >= points.length) return
    event.preventDefault()
    setActive(next)
    event.currentTarget.querySelectorAll<HTMLElement>('[data-usage-meter-history-point]')[next]?.focus()
  }} {...stylex.props(styles.bars)}>
    {points.map((point, index) => <li key={point.id} {...stylex.props(styles.slotItem)}><ValueTooltipTrigger label={`${point.label}: ${point.text}${point.partial ? ' (open)' : ''}`}>
      <Focusable><span role="img" tabIndex={index === current ? 0 : -1} onFocus={() => setActive(index)} aria-label={`${point.label}: ${point.text}${point.partial ? ', period still open' : ''}`} data-usage-meter-history-point={point.id} data-partial={point.partial || undefined} data-unmeasured={point.value === undefined || undefined} {...stylex.props(styles.slot)}>
        {point.value === undefined
          ? <span aria-hidden="true" {...stylex.props(styles.gap)} />
          : <span aria-hidden="true" {...stylex.props(styles.bar, point.partial && styles.partialBar, styles.barHeight(max === 0 ? 0 : point.value / max))} />}
      </span></Focusable>
    </ValueTooltipTrigger></li>)}
  </ol>
}

/** Region name with the caller's page context appended, so repeated monitors keep unique landmarks. */
const landmark = (label: string, context: string | undefined): string => context === undefined ? label : `${label}, ${context}`

/** Supplied period values as bars; unmeasured periods are gaps and open periods are marked. */
export const UsageMeterHistoryChart = ({ history, landmarkContext }: { readonly history: UsageMeterHistory; readonly landmarkContext?: string }): React.ReactElement => {
  if (history.kind !== 'ready') return <section aria-label={landmark(history.label, landmarkContext)} data-usage-meter-history={history.kind} {...stylex.props(styles.history)}>
    <h4 {...stylex.props(styles.title)}>{history.label}</h4>
    <p role={history.kind === 'unavailable' ? 'alert' : 'status'} {...stylex.props(styles.staleText)}>{history.text}</p>
    {history.kind === 'loading' && <div aria-hidden="true" data-usage-meter-history-skeleton {...stylex.props(styles.bars, styles.historySkeleton)} />}
  </section>
  const first = history.points[0]
  const last = history.points.at(-1)
  return <section aria-label={landmark(history.label, landmarkContext)} data-usage-meter-history="ready" {...stylex.props(styles.history)}>
    <h4 {...stylex.props(styles.title)}>{history.label}</h4>
    <p {...stylex.props(styles.identity)}>{history.text}</p>
    {history.points.length === 0 ? <p {...stylex.props(styles.staleText)}>No periods supplied</p> : <>
      <HistoryBars label={history.label} points={history.points} />
      <p aria-hidden="true" data-usage-meter-history-axis {...stylex.props(styles.axis)}><span>{first?.label}</span><span>{last?.label}</span></p>
    </>}
    <div data-usage-meter-history-facts {...stylex.props(styles.historyFacts)}><Facts facts={history.facts} /></div>
  </section>
}

/** Complete supplied evidence, without reconstructing windows or interpreting source policy. */
export const UsageMeterRowDetail = ({ row, history, onClose, landmarkContext }: { readonly row: UsageMeterRow; readonly history?: UsageMeterHistory; readonly onClose: () => void; readonly landmarkContext?: string }): React.ReactElement => <section aria-label={landmark(`${row.label} usage details`, landmarkContext)} data-usage-meter-detail={row.id} {...stylex.props(styles.detail)}>
  <header {...stylex.props(styles.detailHeader)}><h3 {...stylex.props(styles.title)}>{row.label}</h3><Button aria-label="Close usage details" onPress={onClose} {...stylex.props(styles.button)}>Close</Button></header>
  <p {...stylex.props(styles.identity)}>{row.identity}</p>
  {row.stale && <p {...stylex.props(styles.staleText)}>Stale · last-known values</p>}
  {row.note !== undefined && <p {...stylex.props(styles.note)}>{row.note}</p>}
  <Facts facts={row.detail.facts} />
  <h4 {...stylex.props(styles.title)}>Windows</h4>
  {row.detail.windows.map(window => <section key={window.id} aria-label={landmark(window.label, landmarkContext)} {...stylex.props(styles.detailWindow)}>
    <h5 {...stylex.props(styles.title)}>{window.label}</h5>
    <Meter meter={window.meter} stale={row.stale} />
    <Facts facts={window.facts} />
  </section>)}
  <h4 {...stylex.props(styles.title)}>Resets</h4>
  <p {...stylex.props(styles.reset)}><FullValueTooltip full={resetEvidence(row)} valueData={{ 'data-usage-meter-full-value': true }}>{row.reset}{row.resetCount !== undefined && <span aria-label={`${row.resetCount} reported resets`} {...stylex.props(styles.inlineGlyph)}><Icon name="swap" size={10} />{row.resetCount}</span>}</FullValueTooltip></p>
  <Facts facts={row.detail.resets} />
  {history !== undefined && <UsageMeterHistoryChart history={history} landmarkContext={landmarkContext} />}
</section>

/** Controlled, stable-id monitor. Keyboard shortcuts are scoped to this focus surface. */
export const UsageMeterTable = ({ rows, status: observation, sort, onSort, onCycleSort, selectedId, onSelect, onCloseDetail, history, landmarkContext, onComparisonVisibilityChange, toolbarTarget, style }: UsageMeterTableComponentProps): React.ReactElement => {
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
    const active = event.currentTarget.ownerDocument.activeElement
    if (!event.currentTarget.contains(active) && !toolbarTarget?.contains(active)) return
    const target = event.target
    if (!(target instanceof Element) || target.closest('input, textarea, [contenteditable]:not([contenteditable="false"]), [role="textbox"]') !== null) return
    event.preventDefault()
    event.stopPropagation()
    onCycleSort()
  }
  const toolbar = <header onKeyDownCapture={onKeyDown} {...stylex.props(styles.toolbar, toolbarTarget !== undefined && styles.hostedToolbar)}>
    <p role="status" data-usage-meter-status={observation.kind} {...stylex.props(styles.statusText, toolbarTarget !== undefined && styles.hostedStatus, observation.kind !== 'current' && styles.staleText)}>{toolbarTarget === undefined ? observation.text : <FullValueTooltip full={observation.text}>{observation.text}</FullValueTooltip>}</p>
    <Button onPress={onCycleSort} aria-keyshortcuts="s" data-usage-meter-cycle-sort {...stylex.props(styles.button)}><VisuallyHidden elementType="span">Cycle usage </VisuallyHidden>Sort <kbd aria-hidden="true" {...stylex.props(styles.shortcut)}>s</kbd></Button>
  </header>
  return <section ref={observeComparison} aria-label={landmark('Usage monitor', landmarkContext)} tabIndex={0} onKeyDownCapture={onKeyDown} data-usage-meter-table {...stylex.props(styles.root, style)}>
    {toolbarTarget === undefined ? toolbar : toolbarTarget === null ? null : createPortal(toolbar, toolbarTarget)}
    <Table aria-label="Usage meters" selectionMode="single" selectionBehavior="replace" disallowEmptySelection selectedKeys={selected === undefined ? [] : [selected.id]} onSelectionChange={keys => {
      if (keys === 'all') return
      const id = keys.values().next().value
      if (typeof id === 'string') onSelect(id)
    }} sortDescriptor={{ column: sort.key, direction: sort.direction }} onSortChange={descriptor => {
      if (isSortKey(descriptor.column) && descriptor.direction !== undefined) onSort({ key: descriptor.column, direction: descriptor.direction })
    }} {...stylex.props(usageMeterGrid.table, styles.table)}>
      <TableHeader {...stylex.props(usageMeterGrid.header)}>
        {columns.map(column => <Column key={column.id} id={column.id} isRowHeader={column.id === 'account'} allowsSorting textValue={column.label} data-usage-meter-column={column.id} {...stylex.props(usageMeterGrid.column, styles.header, column.id === 'account' && styles.accountColumn, column.id === 'governing' && styles.governingColumn, column.id === 'reset' && styles.resetColumn, column.id === 'comparison' && styles.comparisonColumn, column.id === 'more' && styles.moreColumn)}>
          {({ sortDirection }) => <span {...stylex.props(styles.headerContent, (column.id === 'reset' || column.id === 'more') && styles.numericHeader)}>{column.narrowLabel === undefined ? column.label : <><span {...stylex.props(styles.wideLabel)}>{column.label}</span><span aria-hidden="true" {...stylex.props(styles.narrowHeaderLabel)}>{column.narrowLabel}</span></>}<span aria-hidden="true" {...stylex.props(styles.sortIndicator)}>{sortDirection === undefined ? null : <Icon name={sortDirection === 'ascending' ? 'send' : 'chevron-down'} size={10} />}</span></span>}
        </Column>)}
      </TableHeader>
      <TableBody {...stylex.props(usageMeterGrid.body)} renderEmptyState={() => <span {...stylex.props(styles.empty)}>No usage rows supplied</span>}>
        {rows.map(row => {
          const accountEvidence = `${row.label} · ${row.identity}${row.stale ? ' · Stale' : ''}${row.note === undefined ? '' : ` · ${row.note}`}`
          const resetDescription = resetEvidence(row)
          const isSelected = selected?.id === row.id
          return <Row key={row.id} id={row.id} textValue={row.label} data-usage-meter-row={row.id} data-stale={row.stale || undefined} {...stylex.props(usageMeterGrid.row, styles.row, isSelected && styles.selectedRow)}>
          <Cell textValue={accountEvidence} {...stylex.props(usageMeterGrid.cell, styles.cell, isSelected && styles.selectedCell)}><FullValueTooltip full={accountEvidence} data={{ 'data-usage-meter-account': true }} style={[usageMeterGrid.truncate, styles.accountLabel]} valueData={{ 'data-usage-meter-full-value': true }}>{row.compactIdentity === undefined ? row.identity : <><span data-usage-meter-identity="full" {...stylex.props(styles.fullIdentity)}>{row.identity}</span><span data-usage-meter-identity="compact" {...stylex.props(styles.compactIdentity)}><span data-usage-meter-account-prefix {...stylex.props(styles.compactPrefix)}><span dir="ltr">{row.compactIdentity.prefix}</span></span><span data-usage-meter-account-name {...stylex.props(styles.compactAccount)}>{row.compactIdentity.account}</span></span></>}{row.stale && <span {...stylex.props(styles.staleText)}> · Stale</span>}{row.note !== undefined && <span aria-label={row.note} {...stylex.props(styles.inlineGlyph)}><Icon name="alert" size={10} /></span>}</FullValueTooltip></Cell>
          <Cell {...stylex.props(usageMeterGrid.cell, styles.cell, isSelected && styles.selectedCell)}><Meter meter={row.governing} stale={row.stale} compact /></Cell>
          <Cell textValue={resetDescription} {...stylex.props(usageMeterGrid.cell, styles.cell, styles.numeric, isSelected && styles.selectedCell)}><FullValueTooltip full={resetDescription} style={styles.reset} valueData={{ 'data-usage-meter-full-value': true }}>{row.reset}{row.resetCount !== undefined && <span aria-label={`${row.resetCount} reported resets`} {...stylex.props(styles.inlineGlyph)}><Icon name="swap" size={10} />{row.resetCount}</span>}</FullValueTooltip></Cell>
          <Cell {...stylex.props(usageMeterGrid.cell, styles.cell, styles.comparisonColumn, isSelected && styles.selectedCell)}>{row.comparison === undefined ? <span {...stylex.props(styles.staleText)}>—</span> : <Meter meter={row.comparison} stale={row.stale} compact />}</Cell>
          <Cell {...stylex.props(usageMeterGrid.cell, styles.cell, styles.numeric, isSelected && styles.selectedCell)}><Button aria-expanded={isSelected} onPress={() => onSelect(row.id)} data-usage-meter-more={row.id} {...stylex.props(styles.moreButton)}>
            <span data-usage-meter-more-count {...stylex.props(styles.wideCount)}>+{row.omitted}</span><span data-usage-meter-more-count {...stylex.props(styles.narrowCount)}>+{row.omitted + (row.comparison === undefined ? 0 : 1)}</span><VisuallyHidden elementType="span">{` more: show all windows and evidence for ${row.label}`}</VisuallyHidden>
          </Button></Cell>
        </Row>})}
      </TableBody>
    </Table>
    {selected !== undefined && <UsageMeterRowDetail row={selected} history={history} onClose={onCloseDetail} landmarkContext={landmarkContext} />}
  </section>
}

const styles = stylex.create({
  root: { containerType: 'inline-size', minWidth: 0, width: '100%', color: ink.fg, backgroundColor: surface.canvas, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  toolbar: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.md, padding: space.md },
  hostedToolbar: { flex: 1, minWidth: 0, padding: 0, gap: space.xs, fontSize: type.denseSize },
  hostedStatus: { whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  statusText: { margin: 0, minWidth: 0, overflowWrap: 'anywhere', color: ink.fgSoft },
  button: { display: 'inline-flex', alignItems: 'center', gap: space.sm, flexShrink: 0, color: ink.fgSoft, backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover, ':active': surface.rowActive }, border: quotaGeometry.strongHair, borderRadius: radius.keyboard, padding: `${space.xs} ${space.sm}`, font: 'inherit', cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.focusRingOffset },
  shortcut: { color: ink.fgMuted, fontFamily: type.fontMono, fontSize: type.denseSize },
  table: { textAlign: 'start', fontFamily: type.fontMono, fontSize: type.denseSize },
  header: { zIndex: 1, color: ink.fgMuted, fontSize: type.microSize, fontWeight: type.weightRegular, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  headerContent: { display: 'flex', alignItems: 'center', gap: space.xs2, whiteSpace: 'nowrap', overflow: 'hidden' },
  sortIndicator: { display: { default: 'inline-block', ':empty': 'none' }, flexShrink: 0, color: ink.fgSoft },
  // Protect evidence columns first; the account retains its full accessible name and detail.
  accountColumn: { width: 'auto' },
  // Wide window/reset tracks from 40rem; More widens with its full header label at 36rem.
  governingColumn: { width: { default: quotaGeometry.meterWideColumnWidth, '@container (width < 40rem)': quotaGeometry.governingColumnWidth } },
  resetColumn: { width: { default: quotaGeometry.meterWideResetColumnWidth, '@container (width < 40rem)': quotaGeometry.meterResetColumnWidth } },
  comparisonColumn: { width: { default: quotaGeometry.meterWideColumnWidth, '@container (width < 40rem)': quotaGeometry.comparisonColumnWidth }, display: { default: 'table-cell', '@container (width < 20rem)': 'none' } },
  moreColumn: { width: { default: quotaGeometry.meterWideMoreColumnWidth, '@container (width < 36rem)': quotaGeometry.meterMoreColumnWidth } },
  /** Reset and +N read as numbers: right edge, tabular. */
  numeric: { textAlign: 'right' },
  numericHeader: { justifyContent: 'flex-end' },
  row: { backgroundColor: { default: surface.transparent, ':hover': surface.rowHover }, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  // The hover wash keeps muted and warning ink at AA; the accent underline marks the open row.
  selectedRow: { backgroundColor: surface.rowHover },
  selectedCell: { boxShadow: quotaGeometry.selectedUnderline },
  cell: { outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  accountLabel: { color: ink.fg },
  fullIdentity: { display: { default: 'inline', '@container (width < 36rem)': 'none' } },
  compactIdentity: { display: { default: 'none', '@container (width < 36rem)': 'flex' }, maxWidth: '100%', whiteSpace: 'nowrap', overflow: 'hidden' },
  compactPrefix: { flexBasis: 0, flexGrow: 1, flexShrink: 1, minWidth: 0, maxWidth: 'max-content', overflow: 'hidden', textOverflow: 'ellipsis', direction: 'rtl', textAlign: 'left' },
  compactAccount: { flexShrink: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', direction: 'ltr' },
  identity: { display: 'block', margin: 0, color: ink.fgMuted, fontSize: type.denseSize, overflowWrap: 'anywhere' },
  note: { display: 'block', margin: 0, color: ink.fgMuted, fontSize: type.denseSize, overflowWrap: 'anywhere' },
  meter: { display: 'block', minWidth: 0 },
  compactMeter: { display: 'flex', alignItems: 'center', gap: space.xs2, whiteSpace: 'nowrap' },
  compactWindow: { flexShrink: 0, minWidth: 0, maxWidth: { default: '7ch', '@container (width < 40rem)': '5ch' } },
  compactReading: { flex: 1, gap: space.xs2, minWidth: 0 },
  // Shared SVG glyphs are block-level; this keeps them on the text line inside the 18px line box.
  inlineGlyph: { display: 'inline-flex', alignItems: 'center', gap: space.xs2, height: usageMeterGridVars.lineHeight, marginInlineStart: space.xs2, verticalAlign: 'top' },
  windowLabel: { display: 'block', color: ink.fgMuted, fontSize: type.denseSize, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  meterReading: { display: 'flex', alignItems: 'center', gap: space.xs, minWidth: 0 },
  reading: { flexShrink: 0, color: ink.fgSoft, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  // The bar yields first (down to its minimum); only then does a reading ellipsize, with its full text in the meter's tooltip.
  compactReadingText: { minWidth: 0, maxWidth: `calc(100% - ${quotaGeometry.compactMeterMinimumWidth} - ${space.xs2})`, marginInlineStart: 'auto', overflow: 'hidden', textAlign: 'right', textOverflow: 'ellipsis' },
  track: { display: 'block', flexBasis: quotaGeometry.meterTrackBasis, flexGrow: 1, flexShrink: 1, minWidth: 0, maxWidth: quotaGeometry.meterTrackMaximumWidth, height: quotaGeometry.meterTrackHeight, overflow: 'hidden', borderRadius: radius.full, backgroundColor: border.borderStrong },
  compactTrack: { minWidth: quotaGeometry.compactMeterMinimumWidth },
  fill: { display: 'block', height: '100%', backgroundColor: status.done },
  fraction: (fraction: number) => ({ width: `${fraction * 100}%` }),
  warningFill: { backgroundColor: status.attention },
  dangerFill: { backgroundColor: status.danger },
  warningText: { color: status.attention },
  dangerText: { color: status.dangerFg },
  staleText: { color: ink.fgMuted },
  staleBar: { opacity: 0.4 },
  reset: { margin: 0, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  moreButton: { display: 'inline-flex', alignItems: 'center', height: quotaGeometry.meterMoreButtonHeight, font: 'inherit', fontVariantNumeric: 'tabular-nums', color: ink.fgSoft, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover }, border: 0, padding: 0, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  wideCount: { display: { default: 'inline', '@container (width < 20rem)': 'none' } },
  narrowCount: { display: { default: 'none', '@container (width < 20rem)': 'inline' } },
  narrowHeaderLabel: { display: { default: 'none', '@container (width < 36rem)': 'inline' }, minWidth: 0, overflow: 'hidden' },
  /** Stays the accessible column name when the container shows the short label. */
  wideLabel: { position: { default: 'static', '@container (width < 36rem)': 'absolute' }, width: { default: 'auto', '@container (width < 36rem)': quotaGeometry.visuallyHiddenSize }, height: { default: 'auto', '@container (width < 36rem)': quotaGeometry.visuallyHiddenSize }, overflow: { default: 'visible', '@container (width < 36rem)': 'hidden' }, clipPath: { default: 'none', '@container (width < 36rem)': 'inset(50%)' } },
  empty: { display: 'block', padding: space.xl, color: ink.fgMuted },
  detail: { padding: space.lg, backgroundColor: surface.raised, borderTop: quotaGeometry.strongHair },
  detailHeader: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.md },
  title: { margin: `${space.md} 0`, fontSize: type.metaSize, fontWeight: type.weightMedium, color: ink.fg },
  detailWindow: { paddingBlock: space.sm, borderBottom: quotaGeometry.hair },
  facts: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 2fr)', gap: `${space.xs} ${space.md}`, marginBlock: space.md },
  factLabel: { color: ink.fgMuted, overflowWrap: 'anywhere' },
  factValue: { margin: 0, color: ink.fgSoft, overflowWrap: 'anywhere', whiteSpace: 'pre-wrap' },
  history: { paddingBlock: space.sm, paddingInline: space.md, borderTop: quotaGeometry.hair, marginBlockStart: space.md },
  historySkeleton: { backgroundColor: border.borderStrong, borderRadius: radius.keyboard },
  historyFacts: { minWidth: 0 },
  bars: { display: 'flex', alignItems: 'flex-end', gap: space.hairline, blockSize: quotaGeometry.historyChartHeight, margin: 0, padding: 0, listStyle: 'none', borderBottom: quotaGeometry.strongHair },
  slotItem: { display: 'flex', flexBasis: 0, flexGrow: 1, flexShrink: 1, minWidth: 0, blockSize: '100%' },
  slot: { display: 'flex', alignItems: 'flex-end', inlineSize: '100%', blockSize: '100%', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.focusRingOffset },
  bar: { display: 'block', inlineSize: '100%', minBlockSize: quotaGeometry.historyMinimumBarHeight, backgroundColor: accent.primary },
  barHeight: (fraction: number) => ({ blockSize: `${fraction * 100}%` }),
  partialBar: { opacity: 0.45, outline: quotaGeometry.dashedAccentHair, outlineOffset: quotaGeometry.historyOutlineInset },
  gap: { display: 'block', inlineSize: '100%', blockSize: '100%', backgroundImage: quotaGeometry.unreportedHistoryPattern },
  axis: { display: 'flex', justifyContent: 'space-between', gap: space.md, margin: `${space.xs} 0 0`, color: ink.fgMuted, fontSize: type.denseSize, fontVariantNumeric: 'tabular-nums' },
})
