import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { quotaGeometry } from './quota-geometry.stylex.ts'
import { Icon } from './assistant-ui/composition/Icons.tsx'
import { Button, Cell, Column, Label, ListBox, ListBoxItem, Popover, Row, Select, SelectValue, Table, TableBody, TableHeader, VisuallyHidden } from 'react-aria-components'
import { accentVars as accent, borderVars as border, radiusVars as radius, spaceVars as space, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex.ts'
import type { QuotaAccount, QuotaDrawerProps, QuotaSortKey, QuotaWindow } from './quota-design-types.ts'
import { filterQuotaAccounts, quotaAccountAmount, quotaGoverningResetAt, quotaOmittedWindowCount, quotaProviderSubtotal, quotaRoleWindow, quotaSortKeys, quotaVisibleWindows, quotaWindowFraction, sortQuotaAccounts } from './quota-design-sort.ts'
import { usageMeterGrid, usageMeterGridVars } from './UsageMeterGrid.stylex.ts'
import { quotaStateLabels } from './quota-design-copy.ts'
import { FullValueTooltip, ValueTooltipTrigger } from './FullValueTooltip.tsx'

export type { QuotaDrawerProps } from './quota-design-types.ts'

/** Swap point for the pending overflow decision; primary evidence never folds. */
const quotaOverflowPolicy = 'collapse' as 'collapse' | 'hide' | 'truncate'

const sortLabels: Record<QuotaSortKey, string> = { governing: 'Gov', comparison: 'Cmp', account: 'Acct', reset: 'Reset', more: '+N' }
const sortNames: Record<QuotaSortKey, string> = { governing: 'governing usage', comparison: 'comparison usage', account: 'account', reset: 'reset time', more: 'omitted windows' }
const tokenFormat = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })
const usdFormat = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 2 })
const amountText = (value: number | undefined, unit: 'usd' | 'tokens'): string => value === undefined
  ? '—'
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

const WindowMeter = ({ window, chip = false, stale = false }: { readonly window: QuotaWindow; readonly chip?: boolean; readonly stale?: boolean }): React.ReactElement => {
  const fraction = quotaWindowFraction(window)
  const danger = window.exhausted || (fraction !== undefined && fraction >= 0.8)
  const warning = !danger && fraction !== undefined && fraction >= 0.6
  const reading = fraction === undefined ? '—' : `${Math.round(fraction * 100)}%`
  const explanation = `${window.label}: ${fraction === undefined ? 'Unreported' : `${reading} used`}${window.exhausted ? '; exhausted' : ''}${window.governing ? '; governing' : ''}${window.comparison ? '; comparison' : ''}${stale ? '; last-known reading' : ''}`
  return <FullValueTooltip full={explanation} data={{ 'data-quota-window': window.id, 'data-quota-exhausted': window.exhausted || undefined, 'data-quota-fraction': fraction, 'data-quota-stale': stale || undefined }} style={[styles.meter, chip && styles.chip]} visibleStyle={styles.meterLine}>
    <span {...stylex.props(styles.reading, warning && styles.warning, danger && styles.danger)}>{reading}{window.exhausted && <Icon name="alert" size={10} />}</span>
    {fraction !== undefined && <span data-quota-bar {...stylex.props(styles.track, stale && styles.staleMeter)}><span {...stylex.props(styles.fill, warning && styles.warningFill, danger && styles.dangerFill, styles.fraction(Math.min(1, fraction)))} /></span>}
  </FullValueTooltip>
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

/**
 * Accounts whose ` · provider` suffix does not fit whole beside the name. The suffix folds into the account tooltip instead
 * of being cut mid-word, so the name keeps the slot whenever secondary columns show. Re-measured on every render, on
 * resize and once web fonts settle; the suffix keeps its intrinsic width while folded, so the decision is stable.
 */
const useFoldedProviders = (scope: React.RefObject<HTMLElement | null>): ReadonlySet<string> => {
  const [folded, setFolded] = React.useState<ReadonlySet<string>>(() => new Set())
  const measure = React.useCallback(() => {
    const root = scope.current
    if (root === null) return
    const next = new Set<string>()
    for (const button of root.querySelectorAll<HTMLElement>('[data-quota-account-button]')) {
      const label = button.querySelector<HTMLElement>('[data-quota-account-label]')
      const provider = button.querySelector<HTMLElement>('[data-quota-account-provider]')
      if (label === null || provider === null || provider.getClientRects().length === 0) continue
      if (Math.ceil(label.getBoundingClientRect().width) + provider.scrollWidth > button.clientWidth) next.add(button.dataset.quotaAccountButton ?? '')
    }
    setFolded(previous => previous.size === next.size && [...next].every(id => previous.has(id)) ? previous : next)
  }, [scope])
  React.useLayoutEffect(measure)
  React.useEffect(() => {
    const root = scope.current
    if (root === null || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(root)
    const fonts = root.ownerDocument.fonts as FontFaceSet | undefined
    fonts?.addEventListener('loadingdone', measure)
    return () => {
      observer.disconnect()
      fonts?.removeEventListener('loadingdone', measure)
    }
  }, [measure, scope])
  return folded
}

/** Controlled filter keys preserve missing retained choices and only report the changed field. */
const QuotaFilter = ({ label, value, options, valueLabels, onChange }: { readonly label: string; readonly value: string; readonly options: readonly string[]; readonly valueLabels?: Readonly<Partial<Record<string, string>>>; readonly onChange: (value: string) => void }) => {
  const choices = value !== '' && !options.includes(value) ? [value, ...options] : options
  return <Select aria-label={`Filter ${label.toLowerCase()}`} selectedKey={value === '' ? 'all' : `value:${value}`} onSelectionChange={key => {
    if (key === 'all') onChange('')
    else if (typeof key === 'string' && key.startsWith('value:')) onChange(key.slice(6))
  }} {...stylex.props(styles.filterLabel)}>
    <Label>{label}</Label>
    <Button {...stylex.props(styles.select)}><SelectValue {...stylex.props(usageMeterGrid.truncate)} /><Icon name="chevron-down" size={10} /></Button>
    <Popover {...stylex.props(styles.filterPopover)}>
      <ListBox {...stylex.props(styles.filterOptions)}>
        <ListBoxItem id="all" textValue="All" {...stylex.props(styles.filterOption)}>All</ListBoxItem>
        {choices.map(choice => <ListBoxItem key={choice} id={`value:${choice}`} textValue={valueLabels?.[choice] ?? choice} {...stylex.props(styles.filterOption)}>{valueLabels?.[choice] ?? choice}</ListBoxItem>)}
      </ListBox>
    </Popover>
  </Select>
}

/** Presentation-only: callers retain last-good accounts and own all persistent UI state. */
export const QuotaDrawer = (props: QuotaDrawerProps): React.ReactElement => {
  const accounts = sortQuotaAccounts(filterQuotaAccounts(props.accounts, props), props.sort, props)
  const rows = providerRows(accounts, props.grouped)
  const providers = [...new Set(props.accounts.map(account => account.provider))].sort()
  const plans = [...new Set(props.accounts.map(account => account.plan))].sort()
  const states = [...new Set(props.accounts.map(account => account.state))].sort()
  const drawerRef = React.useRef<HTMLElement>(null)
  const foldedProviders = useFoldedProviders(drawerRef)
  // A supplied column holding only governing windows (already under Usage) would stay empty in every row.
  const displayColumns = props.variant === 'columns'
    ? props.columns.filter(column => props.accounts.some(account => account.windows.some(window => window.column === column.id && window.id !== quotaRoleWindow(account, 'governing')?.id)))
    : [{ id: 'windows', label: 'Windows' }]
  const filtersActive = props.provider !== '' || props.plan !== '' || props.state !== ''
  const retained = props.accounts.length > 0 && (props.status.kind === 'loading' || props.status.kind === 'stale' || props.status.kind === 'failed')
  const statusNote = retained ? 'Last-good accounts retained' : props.status.kind === 'current' ? 'Reported usage · no forecasts' : 'Source status · no inferred usage'
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
    if (!(target instanceof Element) || target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [aria-haspopup="listbox"], [role="listbox"], [role="option"]') !== null) return
    if (event.key === 's' && !event.shiftKey) cycleSort()
    else if (event.key === 'S' || (event.key === 's' && event.shiftKey)) props.onSort({ key: props.sort.key, direction: props.sort.direction === 'ascending' ? 'descending' : 'ascending' })
    else if (event.key === 'u' && !event.shiftKey) toggleUnit()
    else if (event.key === 'm' && !event.shiftKey) props.onCollapsed(!props.collapsed)
    else if (event.key === 'Escape') props.onEscape()
    else return
    event.preventDefault()
    event.stopPropagation()
  }
  return <section ref={drawerRef} aria-label={props.landmarkContext === undefined ? 'Quota drawer' : `${props.landmarkContext} quota drawer`} tabIndex={0} onKeyDownCapture={onKeyDown} data-quota-drawer data-quota-variant={props.variant} data-quota-unit={props.unit} data-quota-collapsed={props.collapsed} data-quota-line-tolerance={quotaGeometry.lineCentreTolerance} {...stylex.props(styles.root)}>
    <header {...stylex.props(styles.toolbar)}>
      <h2 {...stylex.props(styles.title)}>Quota <span {...stylex.props(styles.count)}>{accounts.length}/{props.accounts.length}</span></h2>
      <Button aria-label={`Toggle unit: ${props.unit === 'usd' ? '$ / tok' : 'tok / $'}`} aria-keyshortcuts="u" onPress={toggleUnit} {...stylex.props(styles.button)}>{props.unit === 'usd' ? '$ / tok' : 'tok / $'}</Button>
      <Button aria-label={props.collapsed ? 'Expand quota' : 'Collapse quota'} aria-expanded={!props.collapsed} aria-keyshortcuts="m" onPress={() => props.onCollapsed(!props.collapsed)} {...stylex.props(styles.button)}><Icon name={props.collapsed ? 'chevron-right' : 'chevron-down'} size={12} /></Button>
    </header>
    <div role="status" aria-live="polite" data-quota-status={props.status.kind} {...stylex.props(styles.statusSlot, (props.status.kind === 'failed' || props.status.kind === 'incompatible') && styles.danger)}>
      <FullValueTooltip full={`${props.status.text} · ${statusNote}`} style={styles.statusTrigger}>
        <span {...stylex.props(styles.statusLine)}>{props.status.text}</span>
        <span {...stylex.props(styles.statusNote)}>{statusNote}</span>
      </FullValueTooltip>
    </div>
    {!props.collapsed && <>
      <div {...stylex.props(styles.filters)}>
        <QuotaFilter label="Provider" value={props.provider} options={providers} onChange={provider => props.onFilter({ provider })} />
        <QuotaFilter label="Plan" value={props.plan} options={plans} onChange={plan => props.onFilter({ plan })} />
        <QuotaFilter label="State" value={props.state} options={states} valueLabels={quotaStateLabels} onChange={state => props.onFilter({ state })} />
      </div>
      <div {...stylex.props(styles.viewControls)}>
        <Button aria-label="Group by provider" aria-pressed={props.grouped} onPress={() => props.onGrouped(!props.grouped)} {...stylex.props(styles.button, props.grouped && styles.pressed)}>By provider</Button>
        {filtersActive && <Button aria-label="Clear quota filters" onPress={() => props.onFilter({ provider: '', plan: '', state: '' })} {...stylex.props(styles.button)}>Clear filters</Button>}
      </div>
      <div data-quota-scroll {...stylex.props(styles.scroll)}>
        <div role="toolbar" aria-label="Sort quota" {...stylex.props(styles.sortHeader)}>
          {quotaSortKeys.map(key => <ValueTooltipTrigger key={key} label={`Sort by ${sortNames[key]}`}>
            <Button data-quota-sort={key} aria-label={`Sort by ${sortLabels[key]}: ${sortNames[key]}`} aria-pressed={props.sort.key === key} aria-keyshortcuts="s S" onPress={() => props.onSort({ key, direction: key === props.sort.key ? props.sort.direction === 'descending' ? 'ascending' : 'descending' : key === 'account' ? 'ascending' : 'descending' })} {...stylex.props(styles.sortButton, props.sort.key === key && styles.activeSort)}>{sortLabels[key]}<span aria-hidden="true" data-quota-sort-indicator {...stylex.props(styles.sortArrow, props.sort.key !== key && styles.inactiveSortArrow)}><Icon name={props.sort.key === key ? props.sort.direction === 'descending' ? 'chevron-down' : 'send' : 'swap'} size={10} /></span></Button>
          </ValueTooltipTrigger>)}
        </div>
        <Table aria-label="Account quotas" selectionMode="none" aria-busy={props.status.kind === 'loading'} onRowAction={key => {
          const row = rows.find(value => value.id === key)
          if (row?.account !== undefined) props.onOpenUsage(row.account.id)
        }} {...stylex.props(usageMeterGrid.table, styles.table)}>
          <TableHeader {...stylex.props(usageMeterGrid.header)}>
            <Column id="account" data-quota-column="account" textValue="Account" isRowHeader {...stylex.props(usageMeterGrid.column, styles.column, styles.accountColumn)}>Account</Column>
            <Column id="primary" data-quota-column="primary" textValue="Governing usage" {...stylex.props(usageMeterGrid.column, styles.column, styles.primaryColumn)}><FullValueTooltip full="Governing usage" valueData={{ 'data-quota-full-value': true }}>Usage</FullValueTooltip></Column>
            <Column id="reset" data-quota-column="reset" textValue="Reset" {...stylex.props(usageMeterGrid.column, styles.column, styles.resetColumn, styles.numeric)}>Reset</Column>
            {displayColumns.map(column => <Column key={column.id} id={`window:${column.id}`} data-quota-column={`window:${column.id}`} textValue={column.label} {...stylex.props(usageMeterGrid.column, styles.column, styles.secondaryColumn)}>{column.label}</Column>)}
            <Column id="amount" data-quota-column="amount" textValue={props.unit === 'usd' ? 'Cost in USD or tokens' : 'Tokens'} {...stylex.props(usageMeterGrid.column, styles.column, styles.amountColumn, styles.numeric)}>{props.unit === 'usd' ? <FullValueTooltip full="Cost in USD or tokens" valueData={{ 'data-quota-full-value': true }}>$ / tok</FullValueTooltip> : 'Tokens'}</Column>
            <Column id="more" data-quota-column="more" textValue="All window evidence" {...stylex.props(usageMeterGrid.column, styles.column, styles.moreColumn, styles.numeric)}><FullValueTooltip full="All window evidence" valueData={{ 'data-quota-full-value': true }}>+N</FullValueTooltip></Column>
          </TableHeader>
          <TableBody {...stylex.props(usageMeterGrid.body)} renderEmptyState={() => <div data-quota-empty {...stylex.props(styles.empty)}>{props.accounts.length === 0 ? 'No quota accounts published' : 'No accounts match these filters'}<span {...stylex.props(styles.emptyNote)}>{props.accounts.length === 0 ? 'Accounts will appear when the source publishes inventory.' : 'Change provider, plan or state to see retained accounts.'}</span></div>}>
            {rows.map(row => {
              const account = row.account
              if (account === undefined) {
                const members = row.accounts ?? []
                const subtotal = quotaProviderSubtotal(members, props.unit)
                const parts = [subtotal.usdAccounts > 0 ? `${usdFormat.format(subtotal.usd)} (${subtotal.usdAccounts} accts)` : undefined, subtotal.tokenAccounts > 0 ? `${tokenFormat.format(subtotal.tokens)} tok (${subtotal.tokenAccounts} accts)` : undefined, subtotal.unknownAccounts > 0 ? `${subtotal.unknownAccounts} Unreported` : undefined].filter(part => part !== undefined)
                const subtotalEvidence = `${subtotal.usd} USD across ${subtotal.usdAccounts} accounts; ${subtotal.tokens} tokens across ${subtotal.tokenAccounts} accounts; ${subtotal.unknownAccounts} Unreported accounts`
                return <Row key={row.id} id={row.id} textValue={row.label} isDisabled data-quota-provider={row.label} {...stylex.props(usageMeterGrid.row, styles.groupRow)}>
                  <Cell {...stylex.props(usageMeterGrid.cell, styles.cell)}><FullValueTooltip full={`${row.label} · ${members.length} accounts · reported subtotal ${parts.join(' + ')}`} style={[usageMeterGrid.truncate, styles.groupLabel]}>{row.label} · {members.length}</FullValueTooltip></Cell>
                  <Cell {...stylex.props(usageMeterGrid.cell, styles.cell)} />
                  <Cell {...stylex.props(usageMeterGrid.cell, styles.cell)} />
                  {displayColumns.map(column => <Cell key={column.id} {...stylex.props(usageMeterGrid.cell, styles.cell, styles.secondaryColumn)} />)}
                  <Cell textValue={subtotalEvidence} {...stylex.props(usageMeterGrid.cell, styles.cell, styles.amountColumn, styles.numeric)}><FullValueTooltip full={subtotalEvidence} style={[usageMeterGrid.truncate, styles.amount]} valueData={{ 'data-quota-full-value': true }}>{parts.join(' + ')}</FullValueTooltip></Cell>
                  <Cell {...stylex.props(usageMeterGrid.cell, styles.cell)} />
                </Row>
              }
              const amount = quotaAccountAmount(account, props.unit)
              const visible = quotaVisibleWindows(account, props)
              const governing = quotaRoleWindow(account, 'governing')
              const secondary = visible.filter(window => window.id !== governing?.id)
              const omitted = quotaOmittedWindowCount(account, props, visible)
              const foldedCount = omitted + (quotaOverflowPolicy === 'collapse' ? secondary.length : 0)
              const stale = retained || account.state === 'stale' || account.state === 'failed'
              const reset = resetText(account, props.now)
              const accountEvidence = `${account.label} · ${account.provider} · ${account.plan} · ${quotaStateLabels[account.state]}`
              const amountEvidence = amount.value === undefined ? 'Tokens Unreported' : `${amount.value} ${amount.unit === 'usd' ? 'USD' : 'tokens'}`
              const resetEvidence = `${reset === '—' ? 'Governing reset Unreported' : `Governing reset ${reset}`}${governing?.resetAt === undefined ? '' : ` · ${governing.resetAt}`} · ${account.resetCount === undefined ? 'Reset count Unreported' : `${account.resetCount} reported resets`}`
              const moreEvidence = `Open all ${account.windows.length} windows, amount and reset evidence for ${account.label}`
              return <Row key={row.id} id={row.id} textValue={account.label} data-quota-row={account.id} data-quota-account-label={account.label} data-quota-state={account.state} {...stylex.props(usageMeterGrid.row, styles.row)}>
                <Cell data-quota-cell="account" {...stylex.props(usageMeterGrid.cell, styles.cell)}>
                  <div {...stylex.props(styles.accountLine)}>
                    <ValueTooltipTrigger label={accountEvidence}><Button aria-label={`Open usage for ${accountEvidence}`} onPress={() => props.onOpenUsage(account.id)} data-quota-account-button={account.id} {...stylex.props(usageMeterGrid.truncate, styles.accountButton)}><span data-quota-account-label>{account.label}</span><span data-quota-account-provider {...stylex.props(styles.accountProvider, foldedProviders.has(account.id) && styles.foldedProvider)}> · {account.provider}</span></Button></ValueTooltipTrigger>
                  </div>
                </Cell>
                <Cell data-quota-cell="primary" {...stylex.props(usageMeterGrid.cell, styles.cell)}>
                  {governing === undefined ? <FullValueTooltip full="Governing usage Unreported">—</FullValueTooltip> : <WindowMeter window={governing} stale={stale} />}
                </Cell>
                <Cell data-quota-cell="reset" textValue={resetEvidence} {...stylex.props(usageMeterGrid.cell, styles.cell, styles.resetCell, styles.numeric)}><FullValueTooltip full={resetEvidence} style={styles.resetToken} valueData={{ 'data-quota-full-value': true }}><span data-quota-reset-reading>{reset}</span></FullValueTooltip></Cell>
                {displayColumns.map(column => <Cell key={column.id} data-quota-cell={`window:${column.id}`} {...stylex.props(usageMeterGrid.cell, styles.cell, styles.secondaryColumn)}>
                  <div {...stylex.props(styles.windowList, props.variant === 'stacked' && styles.stackedList, props.variant === 'chips' && styles.chipList)}>
                    {(props.variant === 'columns' ? secondary.filter(window => window.column === column.id) : secondary).map(window => <WindowMeter key={window.id} window={window} stale={stale} chip={props.variant === 'chips'} />)}
                  </div>
                </Cell>)}
                <Cell data-quota-cell="amount" textValue={amountEvidence} {...stylex.props(usageMeterGrid.cell, styles.cell, styles.amountColumn, styles.numeric)}><FullValueTooltip full={amountEvidence} style={[usageMeterGrid.truncate, styles.amount]} valueData={{ 'data-quota-full-value': true }}>{amountText(amount.value, amount.unit)}</FullValueTooltip></Cell>
                <Cell data-quota-cell="more" {...stylex.props(usageMeterGrid.cell, styles.cell, styles.numeric)}>
                  <ValueTooltipTrigger label={moreEvidence}><Button onPress={() => props.onOpenUsage(account.id)} data-quota-more={omitted} {...stylex.props(styles.moreButton)}>
                    <span data-quota-more-count {...stylex.props(styles.fullCount)}>+{omitted}</span>
                    <span data-quota-more-count {...stylex.props(styles.foldedSecondaryCount)}>+{foldedCount}</span>
                    <VisuallyHidden><span data-quota-more-description>{` ${moreEvidence}`}</span></VisuallyHidden>
                  </Button></ValueTooltipTrigger>
                </Cell>
              </Row>
            })}
          </TableBody>
        </Table>
      </div>
      <footer data-quota-shortcuts {...stylex.props(styles.shortcuts)}><FullValueTooltip full="With focus inside quota: s cycles sort, Shift+s reverses direction, u toggles units, m collapses, Escape closes. Shortcuts do not intercept form fields."><span {...stylex.props(styles.wideShortcuts)}>s sort · S reverse · u unit · m collapse · Esc close</span><span {...stylex.props(styles.narrowShortcuts)}>s/S sort · u unit · m · Esc</span></FullValueTooltip></footer>
    </>}
  </section>
}

const styles = stylex.create({
  root: { containerType: 'inline-size', display: 'flex', flexDirection: 'column', width: '100%', minWidth: 0, maxWidth: '100%', height: '100%', minHeight: 0, boxSizing: 'border-box', color: ink.fg, backgroundColor: surface.canvas, fontFamily: type.fontSans, fontSize: type.denseSize, lineHeight: type.metaLeading, outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  toolbar: { display: 'flex', alignItems: 'center', gap: space.sm, padding: space.md, flexShrink: 0 },
  title: { margin: 0, fontSize: type.metaSize, fontWeight: type.weightSemibold, flexGrow: 1, minWidth: 0 },
  count: { fontSize: type.denseSize, fontWeight: type.weightRegular, color: ink.fgMuted, fontVariantNumeric: 'tabular-nums' },
  button: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', minHeight: quotaGeometry.compactControlHeight, minWidth: 0, padding: `${space.xs2} ${space.sm}`, color: ink.fgSoft, backgroundColor: { default: surface.controlFill, ':hover': surface.rowHover, ':active': surface.rowActive }, border: quotaGeometry.strongHair, borderRadius: radius.keyboard, font: 'inherit', cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.focusOffset },
  pressed: { color: ink.fg, backgroundColor: surface.rowActive },
  statusSlot: { height: quotaGeometry.drawerStatusHeight, minHeight: quotaGeometry.drawerStatusHeight, maxHeight: quotaGeometry.drawerStatusHeight, boxSizing: 'border-box', padding: `0 ${space.md}`, flexShrink: 0, color: ink.fgSoft, overflow: 'hidden' },
  statusTrigger: { display: 'block', minWidth: 0 },
  statusLine: { display: 'block', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  statusNote: { display: 'block', color: ink.fgMuted, fontSize: type.microSize, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  filters: { display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', gap: space.xs, padding: `0 ${space.md} ${space.sm}`, flexShrink: 0 },
  filterLabel: { minWidth: 0, color: ink.fgMuted, fontSize: type.microSize },
  select: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.xs2, width: '100%', minWidth: 0, maxWidth: '100%', boxSizing: 'border-box', font: 'inherit', fontSize: type.denseSize, color: ink.fgSoft, backgroundColor: surface.raised, border: quotaGeometry.strongHair, borderRadius: radius.keyboard, height: quotaGeometry.compactControlHeight, padding: space.xs2, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.focusOffset },
  filterPopover: { minWidth: 'var(--trigger-width)', maxWidth: '100%', backgroundColor: surface.raised, border: quotaGeometry.strongHair, borderRadius: radius.keyboard, color: ink.fg, fontFamily: type.fontSans, fontSize: type.denseSize },
  filterOptions: { padding: space.xs2, outline: 'none', maxHeight: quotaGeometry.filterMenuMaximumHeight, overflowY: 'auto' },
  filterOption: { padding: `${space.xs2} ${space.sm}`, cursor: 'pointer', borderRadius: radius.keyboard, outline: 'none', backgroundColor: { default: surface.transparent, ':is([data-focused])': surface.rowHover, ':is([data-selected])': surface.rowActive } },
  viewControls: { display: 'flex', flexWrap: 'wrap', gap: space.xs, padding: `0 ${space.md} ${space.md}`, flexShrink: 0 },
  scroll: { minHeight: 0, minWidth: 0, flexGrow: 1, overflowY: 'auto', overflowX: 'hidden', scrollbarWidth: 'thin', scrollbarColor: `${status.scrollThumb} ${surface.transparent}`, scrollbarGutter: 'stable' },
  // Content-sized controls keep a full 8px between neighbours; leftover width is shared so hover and pressed fills stay even.
  sortHeader: { position: 'sticky', top: 0, zIndex: 3, display: 'flex', gap: space.md, padding: `${space.xs} ${space.xs}`, backgroundColor: surface.raised, borderBottom: quotaGeometry.strongHair },
  sortButton: { display: 'inline-flex', flex: '1 0 auto', alignItems: 'center', justifyContent: 'center', gap: space.xs2, minHeight: quotaGeometry.compactControlHeight, padding: `${space.xs2} 0`, color: ink.fgMuted, backgroundColor: { default: surface.transparent, ':hover': surface.rowHover, ':active': surface.rowActive }, border: 0, font: 'inherit', fontSize: type.microSize, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  activeSort: { color: ink.fg, backgroundColor: surface.rowActive },
  sortArrow: { display: 'inline-flex', fontFamily: type.fontMono, fontSize: type.microSize },
  // Below 18rem the labels alone fit with 8px gaps: inactive "sortable" glyphs fold, the active direction glyph stays.
  inactiveSortArrow: { display: { default: 'inline-flex', '@container (max-width: 18rem)': 'none' } },
  table: { fontSize: type.denseSize, outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  column: { top: quotaGeometry.stickyColumnOffset, zIndex: 2, color: ink.fgMuted, textAlign: 'left', fontSize: type.microSize, fontWeight: type.weightMedium },
  accountColumn: { width: 'auto' },
  primaryColumn: { width: `calc(${quotaGeometry.primaryUsageWidth} + ${usageMeterGridVars.cellPaddingInline} * 2)`, fontFamily: type.fontMono },
  resetColumn: { width: `calc(${quotaGeometry.primaryResetWidth} + ${usageMeterGridVars.cellPaddingInline} * 2)`, minWidth: quotaGeometry.primaryResetMinimum, fontFamily: type.fontMono },
  secondaryColumn: { width: `calc(${quotaGeometry.secondaryWindowWidth} + ${usageMeterGridVars.cellPaddingInline} * 2)`, fontFamily: type.fontMono, display: { default: 'table-cell', '@container (max-width: 26rem)': quotaOverflowPolicy === 'truncate' ? 'table-cell' : 'none' } },
  amountColumn: { width: `calc(${quotaGeometry.amountWidth} + ${usageMeterGridVars.cellPaddingInline} * 2)`, fontFamily: type.fontMono, display: { default: 'table-cell', '@container (max-width: 18rem)': quotaOverflowPolicy === 'truncate' ? 'table-cell' : 'none' } },
  moreColumn: { width: `calc(${quotaGeometry.moreWidth} + ${usageMeterGridVars.cellPaddingInline} * 2)`, fontFamily: type.fontMono },
  fullCount: { display: { default: 'inline', '@container (max-width: 26rem)': 'none' } },
  foldedSecondaryCount: { display: { default: 'none', '@container (max-width: 26rem)': 'inline' } },
  // Atomic box (pre keeps its leading space) so the suffix shows whole or folds whole into the account tooltip.
  accountProvider: { display: { default: 'inline-block', '@container (max-width: 18rem)': 'none' }, whiteSpace: 'pre', verticalAlign: 'top' },
  foldedProvider: { width: 0, overflow: 'hidden', visibility: 'hidden' },
  resetCell: { overflow: 'visible', textOverflow: 'clip' },
  numeric: { textAlign: 'right' },
  cell: { minWidth: 0, outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  row: { backgroundColor: { default: surface.transparent, ':hover': surface.rowHover }, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.insetFocusOffset },
  accountLine: { display: 'flex', alignItems: 'center', minWidth: 0, height: usageMeterGridVars.lineHeight },
  accountButton: { flex: '1 1 0', padding: 0, textAlign: 'left', border: 0, backgroundColor: surface.transparent, color: ink.fg, font: 'inherit', lineHeight: usageMeterGridVars.lineHeight, fontWeight: type.weightMedium, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.inlineFocusOffset },
  amount: { color: ink.fgSoft, fontFamily: type.fontMono, fontSize: type.microSize, fontVariantNumeric: 'tabular-nums' },
  // Block/top-aligned so a smaller mono token never shifts the cell's 18px line box off its baseline (23px rows at 2x).
  resetToken: { display: 'block', color: ink.fgMuted, fontFamily: type.fontMono, fontSize: type.microSize, fontVariantNumeric: 'tabular-nums', lineHeight: usageMeterGridVars.lineHeight, whiteSpace: 'nowrap', overflow: 'visible', textOverflow: 'clip' },
  moreButton: { flexShrink: 0, verticalAlign: 'top', padding: 0, border: 0, backgroundColor: surface.transparent, color: ink.fgSoft, font: 'inherit', fontSize: type.microSize, lineHeight: usageMeterGridVars.lineHeight, cursor: 'pointer', outline: { default: 'none', ':focus-visible': quotaGeometry.focusOutline }, outlineOffset: quotaGeometry.inlineFocusOffset },
  meter: { display: 'flex', flex: '1 1 0', minWidth: 0, height: usageMeterGridVars.lineHeight, overflow: 'hidden' },
  meterLine: { display: 'flex', flex: '1 1 0', flexDirection: 'row', alignItems: 'center', minWidth: 0, height: '100%', gap: space.xs2 },
  staleMeter: { opacity: quotaGeometry.retainedMeterOpacity },
  reading: { display: 'inline-flex', alignItems: 'center', justifyContent: 'flex-end', gap: space.xs2, flexShrink: 0, minWidth: quotaGeometry.readingWidth, color: ink.fgSoft, fontFamily: type.fontMono, fontSize: type.microSize, fontVariantNumeric: 'tabular-nums', lineHeight: usageMeterGridVars.lineHeight, whiteSpace: 'nowrap' },
  track: { display: 'block', flex: '1 1 0', minWidth: space.xs2, height: quotaGeometry.drawerMeterHeight, backgroundColor: surface.rowActive, overflow: 'hidden' },
  fill: { display: 'block', height: '100%', backgroundColor: ink.fgMuted },
  fraction: (fraction: number) => ({ width: `${fraction * 100}%` }),
  warning: { color: status.attention },
  danger: { color: status.dangerFg },
  warningFill: { backgroundColor: status.attention },
  dangerFill: { backgroundColor: status.danger },
  windowList: { display: 'flex', flexDirection: 'row', alignItems: 'center', gap: space.xs2, minWidth: 0, height: usageMeterGridVars.lineHeight, overflow: 'hidden' },
  stackedList: { gap: quotaGeometry.windowListGap },
  chipList: { gap: quotaGeometry.windowListGap },
  chip: { paddingInline: space.xs2, boxSizing: 'border-box', backgroundColor: surface.raised, borderRadius: radius.keyboard },
  groupRow: { backgroundColor: surface.raised },
  groupLabel: { color: ink.fgSoft, fontWeight: type.weightSemibold },
  empty: { padding: `${space.xl} ${space.md}`, textAlign: 'center', color: ink.fgSoft, overflowWrap: 'anywhere' },
  emptyNote: { display: 'block', marginTop: space.sm, color: ink.fgMuted, fontSize: type.microSize },
  // One line at every width: below 18rem the short legend swaps in; the full sentence stays in the tooltip.
  shortcuts: { padding: `${space.xs} ${space.md}`, color: ink.fgMuted, fontSize: type.microSize, lineHeight: quotaGeometry.shortcutLeading, flexShrink: 0, whiteSpace: 'nowrap', borderTop: quotaGeometry.hair },
  wideShortcuts: { display: { default: 'inline', '@container (max-width: 18rem)': 'none' } },
  narrowShortcuts: { display: { default: 'none', '@container (max-width: 18rem)': 'inline' } },
})
