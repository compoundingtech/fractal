// The Usage pane's quota accounts: the selected account's every limit and daily history on top, then the accounts worst first to pick from.
import { useAtom, useAtomValue } from '@effect/atom-react'
import { SystemNotice } from '@fractal/ui/assistant-ui/shell'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource, useNow } from '../data/react.tsx'
import type { QuotaRow } from './model.ts'
import type { UsageSource } from './source.ts'
import { historyAtom, historyUnitAtom, selectionAtom } from './state.ts'
import { accountDetailView, historyView, selectedRow, type AccountDetailView, type HistoryView } from './usageAccountView.ts'
import { fills, levels, QUOTA_FAILED, useUsageSectionView } from './UsageSection.tsx'
import type { UsageSectionView } from './usageSectionView.ts'

const Note = ({ view }: { readonly view: Exclude<UsageSectionView, { _tag: 'ready' }> }) => {
  switch (view._tag) {
    case 'loading': return <SystemNotice kind="loading" title="Reading quota…" />
    case 'not-configured': return <SystemNotice kind="empty" title="No quota source configured" detail="Account limits show here once this Fractal has one" />
    case 'failed': return <SystemNotice kind="error" title="Could not read quota" detail={QUOTA_FAILED[view.kind]} />
    case 'empty': return <SystemNotice kind="empty" title="The quota source reports no accounts" {...(view.old === undefined ? {} : { detail: `Reading ${view.old}` })} />
  }
}

/** A reading that is not current keeps its numbers but drops its colour, as the sidebar does. */
const Detail = ({ detail, muted }: { readonly detail: AccountDetailView; readonly muted: boolean }) => {
  const headingId = React.useId()
  return <article aria-labelledby={headingId} data-usage-detail={detail.id} {...stylex.props(styles.detail)}>
    <h3 id={headingId} {...stylex.props(styles.name)}>{detail.id}{detail.plan === undefined ? null : <span {...stylex.props(styles.meta)}> · {detail.plan}</span>}</h3>
    {detail.unreadable === undefined ? null : <p {...stylex.props(styles.meta)}>Summary unreadable: {detail.unreadable}</p>}
    <ul aria-label={`${detail.id} limits`} {...stylex.props(styles.list)}>
      {detail.limits.map(limit => <li key={limit.id} data-usage-limit={limit.id} data-usage-level={limit.level} {...stylex.props(styles.limit)}>
        <span {...stylex.props(styles.label)}>{limit.label}</span>
        <span {...stylex.props(styles.percent, levels[muted ? 'unknown' : limit.level])}>{limit.percent}{limit.level === 'limit' ? '!' : ''}</span>
        <span aria-hidden="true" {...stylex.props(styles.meter)}>{limit.fill === undefined ? null : <span style={{ inlineSize: `${limit.fill}%` }} {...stylex.props(styles.fill, fills[muted ? 'old' : limit.level])} />}</span>
        <span {...stylex.props(styles.reset)}>{limit.note === undefined ? '' : `${limit.note} · `}{limit.reset}</span>
      </li>)}
    </ul>
    {detail.banked === undefined ? null : <p data-usage-banked="" {...stylex.props(styles.meta)}>{detail.banked}</p>}
    <p data-usage-reading="" {...stylex.props(styles.meta, detail.reading.held && styles.held)}>This account’s reading, {detail.reading.age}{detail.reading.held ? ', held by the producer' : ''}</p>
    {detail.attempt === undefined ? null : <p data-usage-attempt="" {...stylex.props(styles.meta)}>Last attempt: {detail.attempt}</p>}
  </article>
}

const History = ({ view }: { readonly view: HistoryView }) => {
  const captionId = React.useId()
  switch (view._tag) {
    case 'not-configured': return <SystemNotice kind="empty" title="No account history here" detail="Daily usage shows here once this Fractal relays usage history for the account" />
    case 'loading': return <SystemNotice kind="loading" title="Reading history…" />
    case 'unavailable': return <SystemNotice kind="error" title="Could not read history" detail={view.reason} />
    case 'incompatible': return <SystemNotice kind="error" title="History is in a shape this app cannot read" detail={view.reason} />
  }
  return <figure aria-labelledby={captionId} data-usage-history="" {...stylex.props(styles.figure)}>
    <figcaption id={captionId} {...stylex.props(styles.name)}>{view.caption}</figcaption>
    <div role="img" aria-label={view.summary} {...stylex.props(styles.bars)}>
      {view.bars.map(bar => <span key={bar.day} title={bar.words} data-usage-day={bar.day} data-state={bar.state} data-open={bar.open || undefined}
        style={{ blockSize: bar.state === 'recorded' ? `${Math.max(bar.fill ?? 0, 1)}%` : undefined }}
        {...stylex.props(styles.bar, styles[bar.state], bar.open && styles.open)} />)}
    </div>
    {/* Each bar's day as a table for screen readers, which never get a hover title; the bars stay out of the tab order. */}
    <Aria.VisuallyHidden elementType="div">
      <table data-usage-history-table="">
        <caption>Each day in the chart</caption>
        <thead><tr><th scope="col">Date</th><th scope="col">Value</th><th scope="col">Unit</th><th scope="col">State</th></tr></thead>
        <tbody>{view.bars.map(bar => <tr key={bar.day}><th scope="row">{bar.day}</th><td>{bar.value ?? 'none'}</td><td>{view.unit}</td><td>{bar.status}</td></tr>)}</tbody>
      </table>
    </Aria.VisuallyHidden>
    <p aria-hidden="true" {...stylex.props(styles.axis)}><span>{view.bars[0]?.day}</span><span>today, so far</span></p>
    <p {...stylex.props(styles.meta)}>{view.summary}</p>
    {view.notes.map(note => <p key={note} {...stylex.props(styles.meta, styles.held)}>{note}</p>)}
  </figure>
}

const LiveHistory = ({ source, ledgerAccountId }: { readonly source: UsageSource; readonly ledgerAccountId: string }) =>
  <History view={historyView({ ledgerAccountId, history: useAtomValue(historyAtom({ source, ledgerAccountId })), preferred: useAtomValue(historyUnitAtom) })} />

/** Only a declared source with a published ledger account has history to read. */
const AccountHistory = ({ row }: { readonly row: QuotaRow }) => {
  const monitor = useDataSource().usage
  const ledgerAccountId = row.account.ledgerAccountId
  return monitor._tag === 'declared' && ledgerAccountId !== undefined
    ? <LiveHistory source={monitor.source} ledgerAccountId={ledgerAccountId} />
    : <History view={{ _tag: 'not-configured' }} />
}

/** `label` prefixes the region's landmark, as the pane's other sections do. */
export const UsageAccounts = ({ label }: { readonly label: string }): React.ReactNode => {
  const { state, view } = useUsageSectionView()
  const [selected, select] = useAtom(selectionAtom(useDataSource().usage))
  const now = useNow()
  const headingId = React.useId()
  const row = view._tag === 'ready' ? selectedRow({ state, view, selected }) : undefined
  return <section aria-label={`${label}: quota`} data-usage-accounts={view._tag} {...stylex.props(styles.region)}>
    {view._tag !== 'ready' ? <Note view={view} /> : <>
      {view.old === undefined ? null : <p role="status" {...stylex.props(styles.meta, styles.inset, styles.held)}>Quota snapshot {view.old}: account readings are muted</p>}
      {row === undefined ? null : <>
        <Detail detail={accountDetailView({ row, now })} muted={view.old !== undefined || row.retained} />
        <AccountHistory row={row} />
      </>}
      <h3 id={headingId} {...stylex.props(styles.section)}>quota accounts <span {...stylex.props(styles.meta)}>{view.rows.length}</span></h3>
      <Aria.GridList aria-labelledby={headingId} selectionMode="single" selectionBehavior="replace" disallowEmptySelection selectedKeys={row === undefined ? [] : [row.accountId]}
        onSelectionChange={keys => { const [key] = keys === 'all' ? [] : keys; if (key !== undefined) select(String(key)) }} {...stylex.props(styles.list)}>
        {view.rows.map(account => <Aria.GridListItem key={account.id} id={account.id} textValue={account.name} data-usage-account={account.id} data-usage-level={account.level}
          {...stylex.props(styles.row, account.id === row?.accountId && styles.selected)}>
          <span {...stylex.props(styles.label, account.id === row?.accountId && styles.selectedName)}>{account.name}</span>
          <span {...stylex.props(styles.percent, levels[view.old === undefined && account.old === undefined ? account.level : 'unknown'])}>{account.percent}{account.level === 'limit' ? '!' : ''}</span>
          <span {...stylex.props(styles.reset, styles.wide)}>{account.window} · {account.old ?? account.reset}{account.more === undefined ? '' : ` · +${account.more.count}${account.more.atLimit ? '!' : ''}`}</span>
        </Aria.GridListItem>)}
      </Aria.GridList>
      {view.unreported === 0 ? null : <p {...stylex.props(styles.meta, styles.inset)}>{view.unreported} {view.unreported === 1 ? 'account' : 'accounts'} not reporting</p>}
    </>}
  </section>
}

const styles = stylex.create({
  region: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0 },
  detail: { display: 'flex', flexDirection: 'column', gap: s.xs, padding: s.md, borderWidth: 1, borderStyle: 'solid', borderColor: c.border, borderRadius: r.sm },
  figure: { margin: 0, display: 'flex', flexDirection: 'column', gap: s.xs, paddingInline: s.md },
  // One column per day. The open day is an outline, since its total is still growing; a day the producer
  // did not report, or reported without a rate, is an empty dotted slot, unlike a recorded zero's flat tick.
  bars: { display: 'flex', alignItems: 'flex-end', gap: 2, blockSize: 72, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border },
  bar: { flex: '1 1 0', minWidth: 0, borderStartStartRadius: 2, borderStartEndRadius: 2, backgroundColor: c.primary },
  recorded: {},
  zero: { blockSize: 2, backgroundColor: c.fgMuted },
  unreported: { blockSize: '100%', backgroundColor: 'transparent', borderWidth: 1, borderBottomWidth: 0, borderStyle: 'dotted', borderColor: c.borderStrong },
  unrated: { blockSize: '100%', backgroundColor: 'transparent', borderWidth: 1, borderBottomWidth: 0, borderStyle: 'dotted', borderColor: c.attention },
  open: { backgroundColor: 'transparent', borderWidth: 1, borderBottomWidth: 0, borderStyle: 'dashed', borderColor: c.primary },
  axis: { margin: 0, display: 'flex', justifyContent: 'space-between', color: c.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  name: { margin: 0, fontSize: t.metaSize, fontWeight: t.weightSemibold, overflowWrap: 'anywhere' },
  section: { margin: 0, paddingInline: s.md, paddingBlock: s.xs, fontSize: t.metaSize, fontWeight: t.weightSemibold, color: c.fgMuted },
  inset: { paddingInline: s.md },
  list: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: 2 },
  // Two lines: what and how much, then the meter and when it resets; one grid keeps every meter on one column.
  limit: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) auto', columnGap: s.md, rowGap: 2, alignItems: 'center', paddingBlock: s.xs, fontSize: t.metaSize },
  // Selection is the accent (bar, tint, weight); focus is a neutral ring, as in the sidebar lists, so the two never read alike.
  row: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) auto', columnGap: s.md, rowGap: 2, padding: s.md, borderRadius: r.sm, fontSize: t.metaSize, cursor: 'pointer', outline: 'none', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `${g.focusRing} solid ${c.fgMuted}`, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  // The tint is the hover surface: the stronger rowActive drops the muted and amber inks below 4.5:1.
  selected: { backgroundColor: c.rowHover, boxShadow: `inset 3px 0 0 ${c.primary}` },
  selectedName: { fontWeight: t.weightSemibold },
  label: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  percent: { fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', textAlign: 'end' },
  meter: { display: 'block', blockSize: g.progressSm, minWidth: 0, borderRadius: r.full, overflow: 'hidden', backgroundColor: c.borderStrong },
  fill: { display: 'block', blockSize: '100%', borderRadius: r.full },
  reset: { color: c.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', textAlign: 'end', overflow: 'hidden', textOverflow: 'ellipsis' },
  wide: { gridColumn: '1 / -1', textAlign: 'start' },
  meta: { margin: 0, color: c.fgMuted, fontSize: t.denseSize },
  held: { color: c.attention },
})
