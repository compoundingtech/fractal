import { useAtomValue } from '@effect/atom-react'
import type * as Native from '@fractal/client/schema'
import { SystemNotice } from '@fractal/ui/assistant-ui/shell'
import * as stylex from '@stylexjs/stylex'
import * as Option from 'effect/Option'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { ObjectivesSource } from '../data/objectivesSource.ts'
import { useDataSource, useNow } from '../data/react.tsx'
import type { Feed } from '../data/source.ts'
import { bars, chartSummary, creditsText, forecastOf, leftText, safeLink, secondLine, sloGroups, spanText, toneOf } from './usageModel.ts'

// The Usage screen: objectives and budgets from st's served rows, as the phone and the terminal UI's
// Usage tab show them, beside the declared accounts. A row opens to its words and a chart of the last day.

const CHART_STEPS = 48
type Unobserved = Exclude<Feed<unknown>, { readonly _tag: 'Observed' }>

const Unread = ({ feed, what, onRetry }: { readonly feed: Unobserved; readonly what: string; readonly onRetry: () => void }) =>
  feed._tag === 'Waiting' ? <SystemNotice kind="loading" title={`Loading ${what}…`} />
    : feed.reason === 'unsupported' ? <SystemNotice kind="error" title={`The Fractal on this computer needs an update to show ${what}`} />
      : feed.reason === 'ungranted' ? <SystemNotice kind="error" title={`No access to ${what}`} detail="Read access has not been granted" />
        : <SystemNotice kind="error" title={`Could not read ${what}`} onRetry={onRetry} />

const Stale = ({ feed }: { readonly feed: Feed<unknown> }) => feed._tag === 'Observed' && feed.error
  ? <p role="status" {...stylex.props(styles.stale)}>Last reading: could not read again ({feed.error.detail})</p> : null

/** `label` names this pane's landmark and prefixes its section landmarks, so several panes on one page stay distinct. */
export const UsagePane = ({ label = 'Usage' }: { readonly label?: string } = {}): React.ReactNode => {
  const objectives = useDataSource().objectives
  return (
    <section aria-label={label} data-testid="usage-pane" {...stylex.props(styles.pane)}>
      {objectives === undefined
        ? <SystemNotice kind="error" title="Objectives and budgets not supported" detail="This connection does not offer them" />
        : <Usage source={objectives} label={label} />}
    </section>
  )
}
export default UsagePane

const Usage = ({ source, label }: { readonly source: ObjectivesSource; readonly label: string }): React.ReactNode => {
  const list = useAtomValue(source.list)
  const accounts = useAtomValue(source.accounts)
  const now = useNow()
  const [open, setOpen] = React.useState<string>()
  const rows = list._tag === 'Observed' ? list.value.rows : []
  const link = list._tag === 'Observed' ? safeLink(list.value.otel_ui_url) : null
  return <>
    <header {...stylex.props(styles.header)}>
      <h2 {...stylex.props(styles.heading)}>Usage</h2>
      {list._tag === 'Observed' ? <span {...stylex.props(styles.meta)}>as of {spanText(now - list.value.as_of_unix_ms)} ago{list.freshness === 'stale' && !list.error ? ' · reading again…' : ''}</span> : null}
      <Aria.Button onPress={source.refresh} {...stylex.props(styles.button)}>Refresh</Aria.Button>
    </header>
    <div {...stylex.props(styles.body)}>
      <Stale feed={list} />
      {list._tag !== 'Observed' ? <Unread feed={list} what="objectives and budgets" onRetry={source.refresh} />
        : rows.length === 0 ? <SystemNotice kind="empty" title="No objectives or budgets" detail="st declares none on this computer" />
          : sloGroups(rows).map(group => (
            <section key={group.kind} aria-label={`${label}: ${group.title}`}>
              <h3 {...stylex.props(styles.section)}>{group.title} <span {...stylex.props(styles.meta)}>{group.rows.length}</span></h3>
              <ul {...stylex.props(styles.list)}>
                {group.rows.map(row => {
                  const tone = toneOf(row)
                  const line = secondLine(row, now)
                  const expanded = open === row.id
                  return (
                    <li key={row.id} data-slo={row.id} data-tone={tone}>
                      <Aria.Button aria-expanded={expanded} onPress={() => setOpen(expanded ? undefined : row.id)} {...stylex.props(styles.row)}>
                        <span aria-hidden="true" {...stylex.props(styles.dot, tones[tone])}>●</span>
                        <span {...stylex.props(styles.name)}>{row.account === undefined ? row.id : `${row.id} · ${row.account}`}</span>
                        <span {...stylex.props(styles.left, tones[tone])}>{leftText(row)}</span>
                        <span {...stylex.props(styles.second)}>{line.text}{line.forecast ? <span {...stylex.props(line.forecast.runsOut && tones.yellow)}>{line.forecast.short}</span> : null}</span>
                      </Aria.Button>
                      {expanded ? <Detail row={row} source={source} now={now} /> : null}
                    </li>
                  )
                })}
              </ul>
            </section>
          ))}
      {link ? <a href={link} target="_blank" rel="noopener noreferrer" {...stylex.props(styles.link)}>History in OpenTelemetry ›</a> : null}
      <section aria-label={`${label}: accounts`}>
        <Stale feed={accounts} />
        {accounts._tag !== 'Observed' ? <Unread feed={accounts} what="usage" onRetry={source.refresh} />
          : (accounts.value.accounts ?? []).length === 0 ? <SystemNotice kind="empty" title="No declared accounts" />
            : <>
              <h3 {...stylex.props(styles.section)}>accounts <span {...stylex.props(styles.meta)}>{accounts.value.accounts!.length}</span></h3>
              <ul {...stylex.props(styles.list)}>
                {accounts.value.accounts!.map(account => (
                  <li key={account.name} {...stylex.props(styles.account)}>
                    <span {...stylex.props(styles.name)}>{account.name}</span>
                    <span {...stylex.props(styles.meta)}>{[account.provider, account.plan, account.kind].filter(Boolean).join(' · ')}</span>
                    <span {...stylex.props(styles.second, styles.wide)}>
                      {account.credits ? <span {...stylex.props(styles.budget)}>{creditsText(account.credits)}</span> : null}
                      {rows.filter(row => row.kind === 'budget' && row.account === (account.account ?? account.name)).map(row => <span key={row.id} {...stylex.props(styles.budget)}><span aria-hidden="true" {...stylex.props(tones[toneOf(row)])}>● </span>{row.id} {leftText(row)}</span>)}
                    </span>
                  </li>
                ))}
              </ul>
            </>}
      </section>
    </div>
  </>
}

const Detail = ({ row, source, now }: { readonly row: Native.SloRow; readonly source: ObjectivesSource; readonly now: number }): React.ReactNode => {
  const forecast = forecastOf(row, now)
  const resets = Option.getOrUndefined(row.resets_at_unix_ms)
  const fields: ReadonlyArray<readonly [string, string | undefined]> = [
    ['status', row.status], ['about', row.about], ['target', row.target], ['current', row.current], ['window', row.window], ['account', row.account],
    ['resets', resets === undefined ? undefined : resets > now ? `in ${spanText(resets - now)}` : 'now'],
    // The phone's basis wording: whole days from a day, else whole hours.
    ['forecast', forecast ? `${forecast.long}${row.forecast_basis_minutes === undefined ? '' : ` (from the last ${row.forecast_basis_minutes >= 1440 ? `${Math.floor(row.forecast_basis_minutes / 1440)}d` : `${Math.floor(row.forecast_basis_minutes / 60)}h`})`}` : undefined],
    ['measured', row.measured_at_unix_ms === undefined ? undefined : `${spanText(now - row.measured_at_unix_ms)} ago`],
  ]
  return (
    <div aria-label={`${row.id} detail`} {...stylex.props(styles.detail)}>
      <p {...stylex.props(styles.explain)}>{row.explain}</p>
      <dl {...stylex.props(styles.fields)}>
        {fields.filter(([, value]) => value !== undefined).map(([name, value]) => <React.Fragment key={name}><dt {...stylex.props(styles.meta)}>{name}</dt><dd {...stylex.props(styles.value, name === 'forecast' && forecast?.runsOut && tones.yellow)}>{value}</dd></React.Fragment>)}
      </dl>
      <Chart id={row.id} kind={row.kind} tone={toneOf(row)} source={source} />
    </div>
  )
}

const Chart = ({ id, kind, tone, source }: { readonly id: string; readonly kind: Native.SloRow['kind']; readonly tone: keyof typeof tones; readonly source: ObjectivesSource }): React.ReactNode => {
  const history = useAtomValue(source.history(id))
  const title = `${kind === 'budget' ? 'allowance' : 'error budget'} left, last 24 hours`
  if (history._tag !== 'Observed') return <Unread feed={history} what="the chart" onRetry={source.refresh} />
  return (
    <figure {...stylex.props(styles.chart)}>
      <figcaption {...stylex.props(styles.meta)}>{title}</figcaption>
      <Stale feed={history} />
      {history.value.samples === 0 ? <p {...stylex.props(styles.meta)}>No readings in this period yet.</p> : <>
        <pre aria-hidden="true" data-chart="" {...stylex.props(styles.bars, tones[tone])}>{bars(history.value, CHART_STEPS)}</pre>
        <p {...stylex.props(styles.meta)}>{chartSummary(history.value)}{history.value.steps.includes(null) ? ' · a dot had no reading' : ''}</p>
      </>}
    </figure>
  )
}

const tones = stylex.create({
  red: { color: c.dangerFg },
  yellow: { color: c.attention },
  // The live theme has no green token (its green is the neutral `done`); served green mixes evenly with the scheme's ink:
  // #247644 on the light canvas (5.5:1, 5.1:1 on row hover), #8cddaa on dark, so 12px text clears AA in both.
  green: { color: `color-mix(in srgb, #22c55e 50%, ${c.fg})` },
  quiet: { color: c.fgMuted },
})

const styles = stylex.create({
  pane: { position: 'absolute', inset: 0, zIndex: 2, display: 'flex', flexDirection: 'column', minHeight: 0, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  header: { display: 'flex', alignItems: 'baseline', gap: s.md, paddingInline: s.lg, paddingBlock: s.md, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border, flexShrink: 0 },
  heading: { margin: 0, fontSize: t.uiSize, fontWeight: t.weightSemibold },
  body: { flexGrow: 1, minHeight: 0, overflowY: 'auto', scrollbarGutter: 'stable', padding: s.md, display: 'flex', flexDirection: 'column', gap: s.md },
  button: { marginInlineStart: 'auto', padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` } },
  stale: { margin: 0, color: c.attention, fontSize: t.metaSize },
  section: { margin: 0, paddingInline: s.md, paddingBlock: s.sm, fontSize: t.metaSize, fontWeight: t.weightSemibold, color: c.fgMuted },
  list: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: 2 },
  row: { width: '100%', display: 'grid', gridTemplateColumns: '1rem minmax(0, 1fr) auto', alignItems: 'baseline', columnGap: s.md, rowGap: 2, padding: s.md, borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fg, textAlign: 'start', fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` } },
  dot: { textAlign: 'center' },
  name: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontWeight: t.weightMedium },
  left: { fontWeight: t.weightSemibold, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  second: { gridColumn: '2 / -1', minWidth: 0, color: c.fgMuted, fontSize: t.denseSize, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontVariantNumeric: 'tabular-nums' },
  detail: { display: 'flex', flexDirection: 'column', gap: s.sm, paddingBlock: s.sm, paddingInlineStart: 'calc(1rem + 16px)', paddingInlineEnd: s.md },
  explain: { margin: 0, fontSize: t.metaSize, overflowWrap: 'anywhere' },
  fields: { margin: 0, display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', columnGap: s.lg, rowGap: 2, fontSize: t.metaSize },
  value: { margin: 0, minWidth: 0, overflowWrap: 'anywhere' },
  chart: { margin: 0, display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  bars: { margin: 0, fontFamily: t.fontMono, fontSize: t.codeSize, lineHeight: 1.2, overflowX: 'auto', whiteSpace: 'pre' },
  meta: { margin: 0, color: c.fgMuted, fontSize: t.denseSize, fontWeight: t.weightRegular },
  wide: { gridColumn: '1 / -1' },
  account: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) auto', columnGap: s.md, rowGap: 2, padding: s.md, fontSize: t.metaSize },
  budget: { marginInlineEnd: s.md },
  // Brand blue alone is 4.3:1 on the dark canvas; mixing a fifth of the scheme's ink gives #5b87f2 on dark (5.8:1) and #1d46b5 on light (7.9:1).
  link: { alignSelf: 'flex-start', paddingInline: s.md, color: `color-mix(in srgb, ${c.primary} 80%, ${c.fg})`, fontSize: t.metaSize, textDecoration: 'none', ':hover': { textDecoration: 'underline' }, ':focus-visible': { outline: `2px solid ${c.primary}` } },
})
