import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, DialogTrigger, Disclosure, DisclosurePanel, Heading, Popover, VisuallyHidden } from 'react-aria-components'
import { Icon } from './assistant-ui/composition/Icons.tsx'
import { borderVars as border, elevationVars as elevation, geometryNumbers, geometryVars as geometry, radiusVars as radius, spaceVars as space, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex.ts'
import { quotaGeometry } from './quota-geometry.stylex.ts'
import { usageMeterGridVars } from './UsageMeterGrid.stylex.ts'
import { FullValueTooltip, ValueTooltipTrigger } from './FullValueTooltip.tsx'
import { comparisonMinRem, compactMeterReading } from './UsageMeterTable.tsx'
import type { UsageMeterCell, UsageMeterRow, UsageMeterTableProps } from './usage-meter-types.ts'

export interface QuotaSidebarFooterProps {
  /** Most pressing first: ranking is the caller's policy. The first row's primary reading also names the collapsed footer. */
  readonly rows: readonly UsageMeterRow[]
  readonly status: UsageMeterTableProps['status']
  /** Opens the caller's usage details for one account. */
  readonly onOpenAccount: (id: string) => void
}

/** The expanded footer may take this share of its sidebar's block size; beyond it the rows scroll. */
const sidebarShare = 0.5

/** Starts expanded only when every row fits in the footer's share of the sidebar without scrolling; the header is the border and trigger above the rows. */
export const quotaFooterStartsExpanded = ({ panelHeight, headerHeight, sidebarHeight }: { readonly panelHeight: number; readonly headerHeight: number; readonly sidebarHeight: number }): boolean =>
  headerHeight + panelHeight <= sidebarHeight * sidebarShare

/**
 * Windows behind `+N`: the comparison once it no longer has its own column, then every other supplied window. Matching
 * is by window label, so the count and the popover list are one value.
 */
export const quotaFooterFoldedWindows = (row: UsageMeterRow, wide: boolean): readonly UsageMeterCell[] => [
  ...(!wide && row.comparison !== undefined ? [row.comparison] : []),
  ...row.detail.windows.map(window => window.meter).filter(meter => meter.label !== row.governing.label && meter.label !== row.comparison?.label),
]

/** One line: window label, the bar where there is room, and a reading that is never cut; the full text is announced. */
const Reading = ({ meter, stale, bar, full = false }: { readonly meter: UsageMeterCell; readonly stale: boolean; readonly bar: boolean; readonly full?: boolean }) => {
  const danger = meter.tone === 'critical' || meter.tone === 'exhausted'
  return <span data-quota-footer-reading data-usage-meter-tone={meter.tone} {...stylex.props(styles.reading)}>
    <span aria-hidden="true" {...stylex.props(styles.window)}>{meter.label}</span>
    {bar && meter.fraction !== undefined && meter.tone !== 'unknown' && <span aria-hidden="true" data-quota-footer-bar {...stylex.props(styles.track, stale && styles.staleBar)}>
      <span {...stylex.props(styles.fill, meter.tone === 'warning' && styles.warningFill, danger && styles.dangerFill, styles.fraction(Math.min(1, meter.fraction)))} />
    </span>}
    <span aria-hidden="true" data-quota-footer-value {...stylex.props(styles.value, meter.tone === 'warning' && styles.warningText, danger && styles.dangerText, stale && styles.staleText)}>
      {meter.tone === 'exhausted' && <Icon name="alert" size={10} />}{full ? meter.text : compactMeterReading(meter)}
    </span>
    <VisuallyHidden elementType="span">{`${meter.label} ${meter.text}${stale ? ', stale' : ''}`}</VisuallyHidden>
  </span>
}

const MoreWindows = ({ row, windows }: { readonly row: UsageMeterRow; readonly windows: readonly UsageMeterCell[] }) => <DialogTrigger>
  <Button data-quota-footer-more={windows.length} {...stylex.props(styles.textButton, styles.focusRing)}>
    +{windows.length}<VisuallyHidden elementType="span">{` more windows for ${row.label}`}</VisuallyHidden>
  </Button>
  <Popover placement="top end" offset={geometryNumbers.tooltipOffset} {...stylex.props(styles.popover)}>
    <Dialog data-quota-footer-windows={row.id} {...stylex.props(styles.dialog)}>{({ close }) => <>
      <Heading slot="title" level={3} {...stylex.props(styles.dialogTitle)}>{`${row.label} · more windows`}</Heading>
      <ul {...stylex.props(styles.windowList)}>
        {windows.map(meter => <li key={meter.label} {...stylex.props(styles.windowItem)}><Reading meter={meter} stale={row.stale} bar full /></li>)}
      </ul>
      <Button onPress={close} {...stylex.props(styles.closeButton)}>Close</Button>
    </>}</Dialog>
  </Popover>
</DialogTrigger>

const FooterRow = ({ row, wide, onOpenAccount }: { readonly row: UsageMeterRow; readonly wide: boolean; readonly onOpenAccount: (id: string) => void }) => {
  const descriptionId = React.useId()
  const folded = quotaFooterFoldedWindows(row, wide)
  const evidence = `${row.label}${row.stale ? ' · Stale' : ''}${row.note === undefined ? '' : ` · ${row.note}`} · Reset ${row.reset}`
  return <div role="row" data-quota-footer-row={row.id} data-stale={row.stale || undefined} {...stylex.props(styles.row)}>
    <div role="cell" data-quota-footer-cell="account" {...stylex.props(styles.cell)}>
      <ValueTooltipTrigger label={`${row.identity} · ${evidence}`}>
        <Button aria-describedby={descriptionId} onPress={() => onOpenAccount(row.id)} data-quota-footer-account={row.id} {...stylex.props(styles.account, styles.focusRing)}>{row.identity}</Button>
      </ValueTooltipTrigger>
      <VisuallyHidden elementType="span" id={descriptionId}>{`${evidence} · Opens usage details`}</VisuallyHidden>
    </div>
    <div role="cell" data-quota-footer-cell="primary" {...stylex.props(styles.cell)}><Reading meter={row.governing} stale={row.stale} bar={wide} /></div>
    <div role="cell" data-quota-footer-cell="reset" {...stylex.props(styles.cell, styles.reset)}>
      <span aria-hidden="true" data-quota-footer-reset>{row.compactReset ?? row.reset}</span><VisuallyHidden elementType="span">{`Reset ${row.reset}`}</VisuallyHidden>
    </div>
    {wide && <div role="cell" data-quota-footer-cell="secondary" {...stylex.props(styles.cell)}>
      {row.comparison === undefined
        ? <><span aria-hidden="true" {...stylex.props(styles.staleText)}>—</span><VisuallyHidden elementType="span">No secondary window</VisuallyHidden></>
        : <Reading meter={row.comparison} stale={row.stale} bar />}
    </div>}
    <div role="cell" data-quota-footer-cell="more" {...stylex.props(styles.cell, styles.more)}>{folded.length > 0 && <MoreWindows row={row} windows={folded} />}</div>
  </div>
}

/**
 * Quota in the sidebar footer, one 22px line per account in the caller's priority order: account, primary reading,
 * reset, then the secondary window from 20rem, and `+N` for the rest. Below 20rem bars drop before the account is cut.
 * Place it as a direct child of the sidebar column: while the user has not toggled it, it is expanded only when every row
 * fits in half of that column's height, and collapsed otherwise; the user's toggle then holds.
 */
export const QuotaSidebarFooter = ({ rows, status: observation, onOpenAccount }: QuotaSidebarFooterProps): React.ReactElement => {
  const root = React.useRef<HTMLDivElement>(null)
  const trigger = React.useRef<HTMLButtonElement>(null)
  const scroller = React.useRef<HTMLDivElement>(null)
  const chosen = React.useRef(false)
  const panelHeight = React.useRef(0)
  const [expanded, setExpanded] = React.useState(true)
  const [choice, setChoice] = React.useState<'space' | 'user'>('space')
  const [room, setRoom] = React.useState<{ readonly wide: boolean; readonly budget?: number }>({ wide: false })
  React.useLayoutEffect(() => {
    const footer = root.current
    const sidebar = footer?.parentElement
    if (footer === null || sidebar === null || sidebar === undefined) return
    const measure = () => {
      const rem = Number.parseFloat(getComputedStyle(footer.ownerDocument.documentElement).fontSize) || 16
      const wide = footer.getBoundingClientRect().width >= comparisonMinRem * rem
      const headerHeight = trigger.current === null ? 0 : trigger.current.getBoundingClientRect().bottom - footer.getBoundingClientRect().top
      // Rows are measurable only while the panel is open; a collapsed footer keeps its last open measurement.
      const panel = scroller.current
      if (panel !== null && panel.closest('[hidden]') === null) panelHeight.current = panel.scrollHeight
      const budget = Math.max(0, Math.floor(sidebar.clientHeight * sidebarShare - headerHeight))
      setRoom(current => current.wide === wide && current.budget === budget ? current : { wide, budget })
      if (!chosen.current) setExpanded(quotaFooterStartsExpanded({ panelHeight: panelHeight.current, headerHeight, sidebarHeight: sidebar.clientHeight }))
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(footer)
    observer.observe(sidebar)
    return () => observer.disconnect()
  }, [])
  const pressing = rows[0]
  return <Disclosure ref={root} isExpanded={expanded} onExpandedChange={next => { chosen.current = true; setChoice('user'); setExpanded(next) }} data-quota-footer data-quota-footer-choice={choice} data-quota-footer-wide={room.wide || undefined} {...stylex.props(styles.root)}>
    <Heading level={2} {...stylex.props(styles.heading)}>
      <Button ref={trigger} slot="trigger" data-quota-footer-trigger {...stylex.props(styles.trigger, styles.focusRing)}>
        <Icon name={expanded ? 'chevron-down' : 'chevron-right'} size={12} />
        <span {...stylex.props(styles.title)}>Usage</span>
        {pressing !== undefined && <span data-quota-footer-pressing {...stylex.props(styles.pressing)}>
          <VisuallyHidden elementType="span">, most pressing </VisuallyHidden>
          <Reading meter={pressing.governing} stale={pressing.stale} bar={false} />
          <VisuallyHidden elementType="span">{` · ${pressing.identity}`}</VisuallyHidden>
        </span>}
      </Button>
    </Heading>
    <DisclosurePanel data-quota-footer-panel>
      <div ref={scroller} data-quota-footer-scroller {...stylex.props(styles.scroller, room.budget !== undefined && styles.budget(room.budget))}>
        <div role="table" aria-label="Usage by account" data-quota-footer-table {...stylex.props(styles.table, room.wide ? styles.wideTracks : styles.narrowTracks)}>
          <VisuallyHidden elementType="div" role="row">
            <span role="columnheader">Account</span><span role="columnheader">Primary window</span><span role="columnheader">Reset</span>
            {room.wide && <span role="columnheader">Secondary window</span>}<span role="columnheader">More windows</span>
          </VisuallyHidden>
          {rows.map(row => <FooterRow key={row.id} row={row} wide={room.wide} onOpenAccount={onOpenAccount} />)}
        </div>
        <p role="status" data-quota-footer-status={observation.kind} {...stylex.props(styles.status)}>
          <FullValueTooltip full={observation.text} style={styles.statusValue}>{observation.text}</FullValueTooltip>
        </p>
      </div>
    </DisclosurePanel>
  </Disclosure>
}

const styles = stylex.create({
  root: { flexShrink: 0, minWidth: 0, borderTop: quotaGeometry.hair, color: ink.fg, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading },
  heading: { margin: 0, fontSize: 'inherit', fontWeight: 'inherit' },
  trigger: { display: 'flex', alignItems: 'center', gap: space.sm, boxSizing: 'border-box', width: '100%', minHeight: geometry.controlMd, paddingInline: space.lg, border: 0, color: ink.fgSoft, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover }, font: 'inherit', textAlign: 'start', cursor: 'pointer' },
  // Inset ring: cells and the scroller clip their overflow, so an outset ring would be cut off.
  focusRing: { outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  title: { flexShrink: 0, color: ink.fg, fontWeight: type.weightMedium },
  pressing: { display: 'inline-flex', minWidth: 0, marginInlineStart: 'auto', fontFamily: type.fontMono, fontSize: type.denseSize, fontVariantNumeric: 'tabular-nums' },
  scroller: { overflowY: 'auto', overscrollBehavior: 'contain' },
  budget: (px: number) => ({ maxBlockSize: `${px}px` }),
  table: { position: 'relative', display: 'grid', columnGap: space.md, paddingInline: space.lg, fontFamily: type.fontMono, fontSize: type.denseSize, lineHeight: usageMeterGridVars.lineHeight, fontVariantNumeric: 'tabular-nums' },
  wideTracks: { gridTemplateColumns: quotaGeometry.footerWideTracks },
  narrowTracks: { gridTemplateColumns: quotaGeometry.footerNarrowTracks },
  row: { display: 'grid', gridTemplateColumns: 'subgrid', gridColumn: '1 / -1', alignItems: 'center', height: usageMeterGridVars.rowHeight },
  cell: { minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap' },
  reset: { color: ink.fgMuted, textAlign: 'end' },
  more: { textAlign: 'end' },
  account: { display: 'block', boxSizing: 'border-box', width: '100%', padding: 0, border: 0, color: ink.fg, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover }, font: 'inherit', textAlign: 'start', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', cursor: 'pointer' },
  textButton: { display: 'inline-flex', alignItems: 'center', height: quotaGeometry.meterMoreButtonHeight, padding: 0, border: 0, color: ink.fgSoft, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover }, font: 'inherit', fontVariantNumeric: 'tabular-nums', cursor: 'pointer' },
  reading: { display: 'inline-flex', alignItems: 'center', gap: space.xs, whiteSpace: 'nowrap' },
  window: { color: ink.fgMuted },
  value: { display: 'inline-flex', alignItems: 'center', gap: space.xs2, color: ink.fgSoft },
  track: { display: 'block', flexShrink: 0, width: quotaGeometry.footerBarWidth, height: quotaGeometry.meterTrackHeight, overflow: 'hidden', borderRadius: radius.full, backgroundColor: border.borderStrong },
  fill: { display: 'block', height: '100%', backgroundColor: status.done },
  fraction: (fraction: number) => ({ width: `${fraction * 100}%` }),
  warningFill: { backgroundColor: status.attention },
  dangerFill: { backgroundColor: status.danger },
  warningText: { color: status.attention },
  dangerText: { color: status.dangerFg },
  staleText: { color: ink.fgMuted },
  staleBar: { opacity: quotaGeometry.staleMeterOpacity },
  status: { display: 'flex', minWidth: 0, margin: 0, paddingInline: space.lg, height: usageMeterGridVars.rowHeight, alignItems: 'center', color: ink.fgMuted, fontSize: type.denseSize },
  statusValue: { display: 'block', minWidth: 0, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  popover: { boxSizing: 'border-box', maxWidth: `min(${geometry.tooltipMax}, calc(100vw - ${space.lg}))`, backgroundColor: surface.raised, color: ink.fg, border: quotaGeometry.strongHair, borderRadius: radius.md, boxShadow: elevation.popover },
  dialog: { display: 'flex', flexDirection: 'column', gap: space.sm, padding: space.lg, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, outline: 'none' },
  dialogTitle: { margin: 0, fontSize: type.metaSize, fontWeight: type.weightMedium, color: ink.fg },
  windowList: { display: 'flex', flexDirection: 'column', gap: space.xs, margin: 0, padding: 0, listStyle: 'none', fontFamily: type.fontMono, fontSize: type.denseSize, fontVariantNumeric: 'tabular-nums' },
  windowItem: { display: 'flex' },
  closeButton: { alignSelf: 'flex-end', color: ink.fgSoft, backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover }, border: quotaGeometry.strongHair, borderRadius: radius.keyboard, paddingBlock: space.xs2, paddingInline: space.sm, font: 'inherit', cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.focusRingOffset },
})
