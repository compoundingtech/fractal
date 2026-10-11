import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Cell, Column, Row, Table, TableBody, TableHeader } from 'react-aria-components'
import { accentVars as accent, borderVars as border, radiusVars as radius, spaceVars as space, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex.ts'
import type { QuotaAccount, QuotaDrawerProps, QuotaSortKey, QuotaWindow } from './quota-design-types.ts'
import { filterQuotaAccounts, quotaAccountAmount, quotaGoverningResetAt, quotaOmittedWindowCount, quotaProviderSubtotal, quotaRoleWindow, quotaSortKeys, quotaVisibleWindows, quotaWindowFraction, sortQuotaAccounts } from './quota-design-sort.ts'

export type { QuotaDrawerProps } from './quota-design-types.ts'

const sortLabels: Record<QuotaSortKey, string> = { governing: 'Gov', comparison: 'Cmp', account: 'Acct', reset: 'Reset', more: '+N' }
const sortNames: Record<QuotaSortKey, string> = { governing: 'governing usage', comparison: 'comparison usage', account: 'account', reset: 'reset time', more: 'omitted windows' }
const tokenFormat = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })
const usdFormat = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 2 })
const amountText = (value: number | undefined, unit: 'usd' | 'tokens'): string => value === undefined
  ? 'Unknown tok'
  : unit === 'usd' ? usdFormat.format(value) : `${tokenFormat.format(value)} tok`

const resetText = (account: QuotaAccount, now: string): string => {
  const reset = quotaGoverningResetAt(account)
  const clock = Date.parse(now)
  if (reset === undefined || !Number.isFinite(clock)) return '—'
  const delta = reset - clock
  const seconds = Math.floor(Math.abs(delta) / 1_000)
  const sign = delta < 0 ? '−' : ''
  if (seconds < 60) return `${sign}${seconds}s`
  if (seconds < 3_600) return `${sign}${Math.floor(seconds / 60)}m`
  if (seconds < 86_400) return `${sign}${Math.floor(seconds / 3_600)}h`
  return `${sign}${Math.floor(seconds / 86_400)}d`
}

const WindowMeter = ({ window, label = false, chip = false, stale = false }: { readonly window: QuotaWindow; readonly label?: boolean; readonly chip?: boolean; readonly stale?: boolean }): React.ReactElement => {
  const fraction = quotaWindowFraction(window)
  const danger = window.exhausted || (fraction !== undefined && fraction >= 0.8)
  const warning = !danger && fraction !== undefined && fraction >= 0.6
  const reading = fraction === undefined ? '—' : `${Math.round(fraction * 100)}%`
  const explanation = `${window.label}: ${fraction === undefined ? window.reported ? 'usage unknown' : 'not reported' : `${reading} used`}${window.exhausted ? '; exhausted' : ''}${window.governing ? '; governing' : ''}${window.comparison ? '; comparison' : ''}${stale ? '; last-known reading' : ''}`
  return <div aria-label={explanation} title={explanation} data-quota-window={window.id} data-quota-exhausted={window.exhausted || undefined} data-quota-fraction={fraction} data-quota-stale={stale || undefined} {...stylex.props(styles.meter, chip && styles.chip)}>
    {label && <span {...stylex.props(styles.windowLabel)}>{window.label}</span>}
    <span {...stylex.props(styles.reading, warning && styles.warning, danger && styles.danger)}>{reading}{window.exhausted && <span {...stylex.props(styles.exhausted)}>Exh</span>}</span>
    {fraction !== undefined && <span aria-hidden="true" data-quota-bar {...stylex.props(styles.track, stale && styles.staleMeter)}><span {...stylex.props(styles.fill, warning && styles.warningFill, danger && styles.dangerFill, styles.fraction(Math.min(1, fraction)))} /></span>}
  </div>
}

interface DrawerRow {
  readonly id: string
  readonly label: string
  readonly account?: QuotaAccount
  readonly accounts?: readonly QuotaAccount[]
}

const providerRows = (accounts: readonly QuotaAccount[], grouped: boolean): readonly DrawerRow[] => {
  if (!grouped) return accounts.map(account => ({ id: `account:${account.id}`, label: account.label, account }))
  const providers = [...new Set(accounts.map(account => account.provider))].sort()
  return providers.flatMap(provider => {
    const members = accounts.filter(account => account.provider === provider)
    return [{ id: `provider:${provider}`, label: provider, accounts: members }, ...members.map(account => ({ id: `account:${account.id}`, label: account.label, account }))]
  })
}

/** Presentation-only: callers retain last-good accounts and own all persistent UI state. */
export const QuotaDrawer = (props: QuotaDrawerProps): React.ReactElement => {
  const accounts = sortQuotaAccounts(filterQuotaAccounts(props.accounts, props), props.sort, props)
  const rows = providerRows(accounts, props.grouped)
  const providers = [...new Set(props.accounts.map(account => account.provider))].sort()
  const plans = [...new Set(props.accounts.map(account => account.plan))].sort()
  const states = [...new Set(props.accounts.map(account => account.state))].sort()
  const displayColumns = props.variant === 'columns' ? props.columns : [{ id: 'windows', label: props.variant === 'stacked' ? 'Windows' : 'Governing + windows' }]
  const filtersActive = props.provider !== '' || props.plan !== '' || props.state !== ''
  const retained = props.accounts.length > 0 && (props.status.kind === 'loading' || props.status.kind === 'stale' || props.status.kind === 'failed')
  const cycleSort = () => {
    const index = quotaSortKeys.indexOf(props.sort.key)
    const key = quotaSortKeys[(index + 1) % quotaSortKeys.length] ?? 'governing'
    props.onSort({ key, direction: key === 'account' ? 'ascending' : 'descending' })
  }
  const toggleUnit = () => props.onUnit(props.unit === 'usd' ? 'tokens' : 'usd')
  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.defaultPrevented || event.repeat || event.altKey || event.ctrlKey || event.metaKey) return
    if (!event.currentTarget.contains(event.currentTarget.ownerDocument.activeElement)) return
    const target = event.target
    if (!(target instanceof Element) || target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"]') !== null) return
    if (event.key === 's' && !event.shiftKey) cycleSort()
    else if (event.key === 'S' || (event.key === 's' && event.shiftKey)) props.onSort({ key: props.sort.key, direction: props.sort.direction === 'ascending' ? 'descending' : 'ascending' })
    else if (event.key === 'u' && !event.shiftKey) toggleUnit()
    else if (event.key === 'm' && !event.shiftKey) props.onCollapsed(!props.collapsed)
    else if (event.key === 'Escape') props.onEscape()
    else return
    event.preventDefault()
    event.stopPropagation()
  }
  return <section aria-label={props.landmarkContext === undefined ? 'Quota drawer' : `${props.landmarkContext} quota drawer`} tabIndex={0} onKeyDownCapture={onKeyDown} data-quota-drawer data-quota-variant={props.variant} data-quota-unit={props.unit} data-quota-collapsed={props.collapsed} {...stylex.props(styles.root)}>
    <header {...stylex.props(styles.toolbar)}>
      <h2 {...stylex.props(styles.title)}>Quota <span {...stylex.props(styles.count)}>{accounts.length}/{props.accounts.length}</span></h2>
      <Button aria-label={`Toggle unit: ${props.unit === 'usd' ? '$ / tok' : 'tok / $'}`} aria-keyshortcuts="u" onPress={toggleUnit} {...stylex.props(styles.button)}>{props.unit === 'usd' ? '$ / tok' : 'tok / $'}</Button>
      <Button aria-label={props.collapsed ? 'Expand quota' : 'Collapse quota'} aria-expanded={!props.collapsed} aria-keyshortcuts="m" onPress={() => props.onCollapsed(!props.collapsed)} {...stylex.props(styles.button)}>{props.collapsed ? '+' : '−'}</Button>
    </header>
    <div role="status" aria-live="polite" data-quota-status={props.status.kind} title={`${props.status.text}${retained ? ' · Last-good accounts retained' : ''}`} {...stylex.props(styles.statusSlot, (props.status.kind === 'failed' || props.status.kind === 'incompatible') && styles.danger)}>
      <span {...stylex.props(styles.statusLine)}>{props.status.text}</span>
      <span {...stylex.props(styles.statusNote)}>{retained ? 'Last-good accounts retained' : props.status.kind === 'current' ? 'Reported usage · no forecasts' : 'Source status · no inferred usage'}</span>
    </div>
    {!props.collapsed && <>
      <div {...stylex.props(styles.filters)}>
        <label {...stylex.props(styles.filterLabel)}>Provider<select aria-label="Filter provider" value={props.provider} onChange={event => props.onFilter({ provider: event.currentTarget.value })} {...stylex.props(styles.select)}><option value="">All</option>{props.provider !== '' && !providers.includes(props.provider) && <option value={props.provider}>{props.provider}</option>}{providers.map(provider => <option key={provider} value={provider}>{provider}</option>)}</select></label>
        <label {...stylex.props(styles.filterLabel)}>Plan<select aria-label="Filter plan" value={props.plan} onChange={event => props.onFilter({ plan: event.currentTarget.value })} {...stylex.props(styles.select)}><option value="">All</option>{props.plan !== '' && !plans.includes(props.plan) && <option value={props.plan}>{props.plan}</option>}{plans.map(plan => <option key={plan} value={plan}>{plan}</option>)}</select></label>
        <label {...stylex.props(styles.filterLabel)}>State<select aria-label="Filter state" value={props.state} onChange={event => props.onFilter({ state: event.currentTarget.value })} {...stylex.props(styles.select)}><option value="">All</option>{props.state !== '' && !states.some(state => state === props.state) && <option value={props.state}>{props.state}</option>}{states.map(state => <option key={state} value={state}>{state}</option>)}</select></label>
      </div>
      <div {...stylex.props(styles.viewControls)}>
        <Button aria-label="Group by provider" aria-pressed={props.grouped} onPress={() => props.onGrouped(!props.grouped)} {...stylex.props(styles.button, props.grouped && styles.pressed)}>By provider</Button>
        {filtersActive && <Button aria-label="Clear quota filters" onPress={() => props.onFilter({ provider: '', plan: '', state: '' })} {...stylex.props(styles.button)}>Clear filters</Button>}
      </div>
      <div {...stylex.props(styles.scroll)}>
        <div role="toolbar" aria-label="Sort quota" {...stylex.props(styles.sortHeader)}>
          {quotaSortKeys.map(key => <Button key={key} data-quota-sort={key} aria-label={`Sort by ${sortLabels[key]}: ${sortNames[key]}`} aria-pressed={props.sort.key === key} aria-keyshortcuts="s S" onPress={() => props.onSort({ key, direction: key === props.sort.key ? props.sort.direction === 'descending' ? 'ascending' : 'descending' : key === 'account' ? 'ascending' : 'descending' })} {...stylex.props(styles.sortButton, props.sort.key === key && styles.activeSort)}>{sortLabels[key]}<span aria-hidden="true" {...stylex.props(styles.sortArrow)}>{props.sort.key === key ? props.sort.direction === 'descending' ? '↓' : '↑' : '↕'}</span></Button>)}
        </div>
        <Table aria-label="Account quotas" selectionMode="none" aria-busy={props.status.kind === 'loading'} onRowAction={key => {
          const row = rows.find(value => value.id === key)
          if (row?.account !== undefined) props.onOpenUsage(row.account.id)
        }} {...stylex.props(styles.table)}>
          <TableHeader>
            <Column id="account" isRowHeader {...stylex.props(styles.column, styles.accountColumn)}>Account</Column>
            {displayColumns.map(column => <Column key={column.id} id={`window:${column.id}`} {...stylex.props(styles.column)}>{column.label}</Column>)}
          </TableHeader>
          <TableBody renderEmptyState={() => <div data-quota-empty {...stylex.props(styles.empty)}>{props.accounts.length === 0 ? 'No quota accounts published' : 'No accounts match these filters'}<span {...stylex.props(styles.emptyNote)}>{props.accounts.length === 0 ? 'Accounts will appear when the source publishes inventory.' : 'Change provider, plan or state to see retained accounts.'}</span></div>}>
            {rows.map(row => {
              const account = row.account
              if (account === undefined) {
                const members = row.accounts ?? []
                const subtotal = quotaProviderSubtotal(members, props.unit)
                const parts = [subtotal.usdAccounts > 0 ? `${usdFormat.format(subtotal.usd)} (${subtotal.usdAccounts} accts)` : undefined, subtotal.tokenAccounts > 0 ? `${tokenFormat.format(subtotal.tokens)} tok (${subtotal.tokenAccounts} accts)` : undefined, subtotal.unknownAccounts > 0 ? `${subtotal.unknownAccounts} unknown` : undefined].filter(part => part !== undefined)
                return <Row key={row.id} id={row.id} textValue={row.label} isDisabled data-quota-provider={row.label} {...stylex.props(styles.groupRow)}>
                  <Cell {...stylex.props(styles.cell)}><span {...stylex.props(styles.groupLabel)}>{row.label} · {members.length}</span><span aria-label={`${row.label} reported subtotal: ${parts.join('; ')}`} {...stylex.props(styles.groupSubtotal)}>Subtotal {parts.join(' + ')}</span></Cell>
                  {displayColumns.map(column => <Cell key={column.id} {...stylex.props(styles.cell)} />)}
                </Row>
              }
              const amount = quotaAccountAmount(account, props.unit)
              const visible = quotaVisibleWindows(account, props)
              const omitted = quotaOmittedWindowCount(account, props)
              const governing = quotaRoleWindow(account, 'governing')
              const stale = retained || account.state === 'stale' || account.state === 'failed'
              const reset = resetText(account, props.now)
              return <Row key={row.id} id={row.id} textValue={account.label} data-quota-row={account.id} data-quota-state={account.state} {...stylex.props(styles.row)}>
                <Cell {...stylex.props(styles.cell)}>
                  <div title={`${account.label} · ${account.provider} · ${account.plan} · ${account.state}`}><Button aria-label={`Open usage for ${account.label}${account.state !== 'current' ? ` · ${account.state}` : ''}`} onPress={() => props.onOpenUsage(account.id)} {...stylex.props(styles.accountButton)}>{account.label}{account.state !== 'current' ? ` · ${account.state}` : ''}</Button></div>
                  <div title={`${account.provider} · ${account.plan} · ${amountText(amount.value, amount.unit)}`} {...stylex.props(styles.inlineMetadata)}>
                    <span {...stylex.props(styles.providerLabel)}>{account.provider} · {account.plan}</span>
                    <span {...stylex.props(styles.amount)}>{amountText(amount.value, amount.unit)}</span>
                  </div>
                  <div {...stylex.props(styles.inlineMetadata)}>
                    <span aria-label={reset === '—' ? 'Governing reset unknown' : `Governing reset ${reset}`} title={reset === '—' ? 'Governing reset unknown' : `Governing reset ${reset}`} {...stylex.props(styles.resetToken)}>{reset}</span>
                    <span aria-label={account.resetCount === undefined ? 'Reset count unknown' : `${account.resetCount} reported resets`} title={account.resetCount === undefined ? 'Reset count unknown' : `${account.resetCount} reported resets`} {...stylex.props(styles.countToken)}>↺{account.resetCount ?? '—'}</span>
                    {omitted > 0 ? <Button aria-label={`Open all windows and evidence for ${account.label}, ${omitted} omitted windows`} onPress={() => props.onOpenUsage(account.id)} data-quota-more={omitted} {...stylex.props(styles.moreButton)}>+{omitted}</Button> : <span data-quota-more={0} />}
                  </div>
                </Cell>
                {displayColumns.map(column => <Cell key={column.id} {...stylex.props(styles.cell)}>
                  {props.variant === 'columns'
                    ? account.windows.filter(window => window.column === column.id).map(window => <WindowMeter key={window.id} window={window} stale={stale} label={account.windows.filter(value => value.column === column.id).length > 1} />)
                    : <div {...stylex.props(styles.windowList, props.variant === 'chips' && styles.chipList)}>{props.variant === 'chips' && governing === undefined && <span {...stylex.props(styles.metadata)}>Governing unknown</span>}{visible.map(window => <WindowMeter key={window.id} window={window} stale={stale} label chip={props.variant === 'chips' && window.id !== governing?.id} />)}{props.variant === 'stacked' && visible.length === 0 && <span {...stylex.props(styles.metadata)}>No windows reported</span>}</div>}
                </Cell>)}
              </Row>
            })}
          </TableBody>
        </Table>
      </div>
      <footer {...stylex.props(styles.shortcuts)} title="With focus inside quota: s cycles sort, Shift+s reverses direction, u toggles units, m collapses, Escape closes. Shortcuts do not intercept form fields.">s sort · S reverse · u unit · m collapse · Esc close</footer>
    </>}
  </section>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', width: '100%', minWidth: 0, maxWidth: '100%', height: '100%', minHeight: 0, boxSizing: 'border-box', color: ink.fg, backgroundColor: surface.canvas, fontFamily: type.fontSans, fontSize: type.denseSize, lineHeight: type.metaLeading, outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  toolbar: { display: 'flex', alignItems: 'center', gap: space.sm, padding: space.md, flexShrink: 0 },
  title: { margin: 0, fontSize: type.metaSize, fontWeight: type.weightSemibold, flexGrow: 1, minWidth: 0 },
  count: { fontSize: type.denseSize, fontWeight: type.weightRegular, color: ink.fgMuted, fontVariantNumeric: 'tabular-nums' },
  button: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minHeight: '24px', minWidth: 0, padding: `${space.xs2} ${space.sm}`, color: ink.fgSoft, backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover, ':active': surface.rowActive }, border: `1px solid ${border.borderStrong}`, borderRadius: radius.keyboard, font: 'inherit', cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '1px' },
  pressed: { color: ink.fg, backgroundColor: surface.rowActive },
  statusSlot: { height: '40px', minHeight: '40px', maxHeight: '40px', boxSizing: 'border-box', padding: `0 ${space.md}`, flexShrink: 0, color: ink.fgSoft, overflow: 'hidden' },
  statusLine: { display: 'block', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  statusNote: { display: 'block', color: ink.fgMuted, fontSize: type.microSize, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  filters: { display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', gap: space.xs, padding: `0 ${space.md} ${space.sm}`, flexShrink: 0 },
  filterLabel: { minWidth: 0, color: ink.fgMuted, fontSize: type.microSize },
  select: { display: 'block', width: '100%', minWidth: 0, maxWidth: '100%', boxSizing: 'border-box', font: 'inherit', fontSize: type.denseSize, color: ink.fgSoft, backgroundColor: surface.raised, border: `1px solid ${border.borderStrong}`, borderRadius: radius.keyboard, height: '24px', padding: space.xs2, outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '1px' },
  viewControls: { display: 'flex', flexWrap: 'wrap', gap: space.xs, padding: `0 ${space.md} ${space.md}`, flexShrink: 0 },
  scroll: { minHeight: 0, minWidth: 0, flexGrow: 1, overflowY: 'auto', overflowX: 'hidden', scrollbarWidth: 'thin', scrollbarColor: `${status.scrollThumb} ${surface.transparent}`, scrollbarGutter: 'stable' },
  sortHeader: { position: 'sticky', top: 0, zIndex: 3, display: 'grid', gridTemplateColumns: 'repeat(5, minmax(0, 1fr))', gap: space.xs2, padding: `${space.xs} ${space.xs}`, backgroundColor: surface.raised, borderBottom: `1px solid ${border.borderStrong}` },
  sortButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', gap: space.xs2, minWidth: 0, minHeight: '24px', padding: `${space.xs2} 0`, color: ink.fgMuted, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover, ':active': surface.rowActive }, border: 0, font: 'inherit', fontSize: type.microSize, cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  activeSort: { color: ink.fg, backgroundColor: surface.rowActive },
  sortArrow: { fontFamily: type.fontMono, fontSize: type.microSize },
  table: { width: '100%', tableLayout: 'fixed', borderCollapse: 'collapse', borderSpacing: 0, fontSize: type.denseSize, outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  column: { position: 'sticky', top: '33px', zIndex: 2, padding: `${space.xs} ${space.xs}`, backgroundColor: surface.canvas, color: ink.fgMuted, textAlign: 'left', fontSize: type.microSize, fontWeight: type.weightMedium, overflowWrap: 'anywhere', verticalAlign: 'bottom' },
  accountColumn: { width: '34%' },
  cell: { padding: `${space.sm} ${space.xs}`, minWidth: 0, verticalAlign: 'top', borderBottom: `1px solid ${border.border}`, overflowWrap: 'anywhere', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  row: { backgroundColor: { default: surface.transparent, ':hover': surface.rowHover }, cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '-2px' },
  accountButton: { display: 'block', width: '100%', minWidth: 0, padding: 0, textAlign: 'left', border: 0, backgroundColor: surface.transparent, color: ink.fg, font: 'inherit', fontWeight: type.weightMedium, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '1px' },
  metadata: { display: 'block', color: ink.fgMuted, fontSize: type.microSize, lineHeight: '14px', overflowWrap: 'anywhere' },
  amount: { minWidth: 0, maxWidth: '100%', color: ink.fgSoft, fontFamily: type.fontMono, fontSize: type.microSize, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  inlineMetadata: { display: 'flex', alignItems: 'center', gap: space.xs2, height: '14px', minWidth: 0, maxWidth: '100%', color: ink.fgMuted, fontSize: type.microSize, lineHeight: '14px' },
  providerLabel: { flex: '1 1 0', minWidth: 0, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  resetToken: { flex: '1 1 auto', minWidth: 0, fontFamily: type.fontMono, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  countToken: { flexShrink: 0, fontFamily: type.fontMono, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  moreButton: { flexShrink: 0, padding: 0, border: 0, backgroundColor: surface.transparent, color: ink.fgSoft, font: 'inherit', fontSize: type.microSize, lineHeight: '14px', cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: '1px' },
  meter: { display: 'flex', flexDirection: 'column', minWidth: 0, gap: space.xs2, marginBottom: space.xs },
  staleMeter: { opacity: 0.65 },
  windowLabel: { color: ink.fgMuted, fontSize: type.microSize, lineHeight: '13px', overflowWrap: 'anywhere' },
  reading: { display: 'flex', flexWrap: 'wrap', gap: space.xs2, color: ink.fgSoft, fontFamily: type.fontMono, fontSize: type.microSize, fontVariantNumeric: 'tabular-nums', lineHeight: '14px', overflowWrap: 'anywhere' },
  exhausted: { fontFamily: type.fontSans, fontWeight: type.weightSemibold, fontSize: type.microSize },
  track: { display: 'block', width: '100%', height: '3px', backgroundColor: surface.rowActive, overflow: 'hidden' },
  fill: { display: 'block', height: '100%', backgroundColor: ink.fgFaint },
  fraction: (fraction: number) => ({ width: `${fraction * 100}%` }),
  warning: { color: status.attention },
  danger: { color: status.dangerFg },
  warningFill: { backgroundColor: status.attention },
  dangerFill: { backgroundColor: status.danger },
  windowList: { display: 'flex', flexDirection: 'column', gap: space.xs, minWidth: 0 },
  chipList: { flexDirection: 'row', flexWrap: 'wrap', gap: space.xs },
  chip: { flexGrow: 1, flexBasis: '40%', padding: space.xs, boxSizing: 'border-box', backgroundColor: surface.raised, borderRadius: radius.keyboard },
  groupRow: { backgroundColor: surface.raised },
  groupLabel: { display: 'block', color: ink.fgSoft, fontWeight: type.weightSemibold, overflowWrap: 'anywhere' },
  groupSubtotal: { display: 'block', color: ink.fgMuted, fontSize: type.microSize, fontFamily: type.fontMono, fontVariantNumeric: 'tabular-nums', overflowWrap: 'anywhere' },
  empty: { padding: `${space.xl} ${space.md}`, textAlign: 'center', color: ink.fgSoft, overflowWrap: 'anywhere' },
  emptyNote: { display: 'block', marginTop: space.sm, color: ink.fgMuted, fontSize: type.microSize },
  shortcuts: { padding: `${space.xs} ${space.md}`, color: ink.fgMuted, fontSize: type.microSize, lineHeight: '14px', flexShrink: 0, overflowWrap: 'anywhere', borderTop: `1px solid ${border.border}` },
})
