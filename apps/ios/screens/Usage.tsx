import { useFocusEffect, useNavigation, useRoute, type RouteProp } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import { useCallback, useMemo, useRef, useState } from 'react';
import { Linking, RefreshControl, ScrollView, View } from 'react-native';
import type { SloHistory, SloList, UsagePeriod } from '../../../clients/typescript/fractal-client';
import { plainError } from '../../../clients/typescript/fractal-client';
import { agentName } from '../agentsView';
import { Banners, StatusLine, useMissionsOnFocus } from '../chrome';
import type { StackParams } from '../navigation';
import { useStore } from '../store';
import { theme } from '../theme';
import { bars, landed, leftText, ownedBy, safeLink, secondLine, forecastOf, sloGroups, toneOf, type Owned, type SloTone } from '../slo';
import { accounts, byOf, cost, creditsLine, groups, label, limitLine, nextBy, PERIODS, periodName, tokens, totalOf, type By, type Names, type Tone } from '../usage';
import { Button, ListRow, Note, Screen, SectionHeader, T } from '../ui';

// A focused usage screen follows its selected period. Fleet's card and a detail screen share
// the same period window; older members read on focus and explicit pull without a timer.
const tone: Record<Tone, string> = { ok: theme.idle, warning: theme.waiting, fault: theme.fault, quiet: theme.overlay0 };
// The last read, so a detail screen shows what the list showed.
let last: { hours: number; period: UsagePeriod } | null = null;

export function useUsage(hours: typeof PERIODS[number]) {
  const { client, feed, inventoryGranted } = useStore();
  const [period, setPeriod] = useState<UsagePeriod | null>(last?.hours === hours ? last.period : null);
  const [issue, setIssue] = useState('');
  const [reading, setReading] = useState(false);
  const read = useCallback(async () => {
    if (inventoryGranted && feed) { setReading(true); feed.refreshInventories(['usage']); return; }
    if (!client || inventoryGranted !== false) return;
    setReading(true);
    try {
      const result = await client.usagePeriod({ since_ms: Date.now() - hours * 3_600_000 });
      last = { hours, period: result.value };
      setPeriod(result.value);
      setIssue('');
    } catch (error) {
      const text = plainError(error);
      // A daemon from before the read answers its path with a bare 404.
      setIssue(/404|not-found|it is gone/i.test(text) ? 'This Fractal version does not serve usage yet; it needs an update.' : `Could not read usage: ${text}`);
    } finally {
      setReading(false);
    }
  }, [client, hours, feed, inventoryGranted]);
  useFocusEffect(useCallback(() => {
    setPeriod(last?.hours === hours ? last.period : null);
    if (inventoryGranted && feed) return feed.watchInventory('usage', {
      onRows: rows => {
        const next = rows[0]?.value ?? null;
        if (next) last = { hours, period: next };
        setPeriod(next); setReading(false);
      },
      onIssue: message => { setIssue(message); if (message) setReading(false); },
    }, hours);
    if (inventoryGranted === false) void read();
  }, [read, feed, inventoryGranted, hours]));
  return { period, issue, reading, read };
}

const sloTone: Record<SloTone, string> = { red: theme.fault, green: theme.idle, yellow: theme.waiting, quiet: theme.overlay0 };
const CHART_HOURS = 24;
const noChart: { open: string | null; history: Record<string, SloHistory | string> } = { open: null, history: {} };

type SloRead = { list: SloList | null; issue: string };
const noSlo: SloRead = { list: null, issue: '' };

/** Objectives and budgets from st's `slo.list`, read on focus and on pull, as the rest of this screen is on older members. What was read belongs to the client that read it. */
export function useSlo() {
  const { client } = useStore();
  const current = useRef(client);
  current.current = client;
  const [read_, setRead] = useState<Owned<typeof client, SloRead>>({ owner: null, value: noSlo });
  const read = useCallback(async () => {
    if (!client) return;
    const keep = (value: SloRead) => setRead(state => landed(state, current.current, client, noSlo, () => value));
    try {
      keep({ list: (await client.sloList()).value, issue: '' });
    } catch (error) {
      const text = plainError(error);
      const issue = /404|not-found|it is gone/i.test(text) ? 'The Fractal on this computer needs an update to show objectives and budgets.' : `Could not read objectives and budgets: ${text}`;
      // The last read of this member stays under the issue; another member's never does.
      setRead(state => landed(state, current.current, client, noSlo, value => ({ ...value, issue })));
    }
  }, [client]);
  useFocusEffect(useCallback(() => { void read(); }, [read]));
  return { ...ownedBy(read_, client, noSlo), read };
}

/** Budgets and objectives with each budget's projected use at its reset; tap a row for its words and a chart of the last day. */
function SloSection({ list, issue }: { list: SloList | null; issue: string }) {
  const { client } = useStore();
  const current = useRef(client);
  current.current = client;
  // The open row and the charts read for it belong to the member they were read from, and row ids repeat between members.
  const [shown, setShown] = useState<Owned<typeof client, typeof noChart>>({ owner: null, value: noChart });
  const { open, history } = ownedBy(shown, client, noChart);
  const now = Date.now();
  const link = safeLink(list?.otel_ui_url);
  const toggle = (id: string) => {
    if (!client) return;
    setShown(state => landed(state, current.current, client, noChart, value => ({ ...value, open: value.open === id ? null : id })));
    if (open === id) return;
    const keep = (chart: SloHistory | string) => setShown(state => landed(state, current.current, client, noChart, value => ({ ...value, history: { ...value.history, [id]: chart } })));
    client.sloHistory(id, { since_ms: Date.now() - CHART_HOURS * 3_600_000, steps: 48 })
      .then(result => keep(result.value))
      .catch(error => keep(plainError(error)));
  };
  if (issue && !list) return <Note tone="warning">{issue}</Note>;
  if (!list) return null;
  return <View>
    {issue ? <Note tone="warning">{issue}</Note> : null}
    {sloGroups(list.rows).map(group => <View key={group.kind}>
      <SectionHeader title={group.title} count={group.rows.length} />
      {group.rows.map(row => {
        const color = sloTone[toneOf(row)];
        const line = secondLine(row, now);
        const chart = history[row.id];
        const forecast = forecastOf(row, now);
        return <View key={row.id}>
          <ListRow title={<T bold numberOfLines={1}><T color={color}>● </T>{row.account ? `${row.id} · ${row.account}` : row.id}</T>} right={<T bold color={color}>{leftText(row)}</T>} onPress={() => toggle(row.id)}
            second={<T dim numberOfLines={1}>{line.text}{line.forecast ? <T color={line.forecast.runsOut ? theme.waiting : undefined} dim={!line.forecast.runsOut}>{line.forecast.short}</T> : null}</T>} />
          {open === row.id ? <View style={{ paddingHorizontal: 12, paddingBottom: 8 }}>
            <T dim>{row.explain}</T>
            {forecast ? <T color={forecast.runsOut ? theme.waiting : undefined}>forecast {forecast.long}{row.forecast_basis_minutes != null ? ` (from the last ${row.forecast_basis_minutes >= 1440 ? `${Math.floor(row.forecast_basis_minutes / 1440)}d` : `${Math.floor(row.forecast_basis_minutes / 60)}h`})` : ''}</T> : null}
            <T dim>left, last {CHART_HOURS} hours</T>
            {typeof chart === 'string' ? <Note tone="warning">{chart}</Note> : chart ? <T color={color}>{chart.samples ? bars(chart, 48) : 'No readings in this period yet.'}</T> : <T dim>Loading the chart…</T>}
          </View> : null}
        </View>;
      })}
    </View>)}
    {link ? <ListRow title={<T color={theme.accent}>History in OpenTelemetry ›</T>} onPress={() => void Linking.openURL(link)} /> : null}
  </View>;
}

export function useNames(): Names {
  const { data } = useStore();
  useMissionsOnFocus();
  return useMemo(() => ({
    agents: new Map(data.agents.map(agent => [agent.id, agentName(agent)])),
    missions: new Map(data.missions.map(mission => [mission.id, mission.title])),
  }), [data.agents, data.missions]);
}

export function UsageScreen() {
  const navigation = useNavigation<NativeStackNavigationProp<StackParams>>();
  const [hours, setHours] = useState<typeof PERIODS[number]>(PERIODS[0]);
  const [by, setBy] = useState<By>('agent');
  const { period, issue, reading, read } = useUsage(hours);
  const slo = useSlo();
  const names = useNames();
  const scroll = useRef<ScrollView>(null);
  const now = Date.now();
  const rows = period?.rows ?? [];
  const limits = period?.limits ?? [];
  const declared = period?.accounts ?? [];
  const total = totalOf(rows);
  const open = (id: string) => navigation.navigate('UsageDetail', { id, hours });
  const top = (grouping: By) => groups(rows, grouping).find(group => !group.id.startsWith('usage/'));
  return <Screen>
    <Banners />
    <ScrollView ref={scroll} contentInsetAdjustmentBehavior="automatic" refreshControl={<RefreshControl refreshing={reading} onRefresh={() => { void read(); void slo.read(); }} tintColor={theme.overlay1} />} contentContainerStyle={{ paddingBottom: 32 }}>
      <StatusLine />
      <View style={{ flexDirection: 'row', gap: 8, paddingHorizontal: 12, paddingTop: 8 }}>
        <Button label={periodName(hours)} onPress={() => setHours(PERIODS[(PERIODS.indexOf(hours as typeof PERIODS[number]) + 1) % PERIODS.length])} />
        <Button label={`by ${by}`} onPress={() => setBy(nextBy(by))} />
      </View>
      {issue ? <Note tone="warning">{issue}</Note> : null}
      <SloSection list={slo.list} issue={slo.issue} />
      {!period && !issue ? <Note>Loading usage…</Note> : null}
      {period && !rows.length && !limits.length && !declared.length ? <Note>No spend recorded in this period.</Note> : null}
      {rows.length || limits.length || declared.length ? <>
        <SectionHeader title="accounts" count={accounts(rows, limits, declared).length} />
        {accounts(rows, limits, declared).map(({ id, total: spent, limit, declared: account }) => {
          const line = limit ? limitLine(limit, now) : null;
          const credits = account?.credits ? creditsLine(account.credits) : null;
          return <ListRow key={id} title={label(id, names, rows, declared)} right={<T color={theme.accent}>{cost(spent)}</T>} onPress={() => open(id)}
            second={credits
              ? <T dim>credits <T bold color={tone[credits.tone]}>{credits.used}</T> of {credits.budget} this month</T>
              : line
                ? <T dim>weekly <T bold color={tone[line.weekly.tone]}>{line.weekly.text}</T>{line.resets ? ` ${line.resets}` : ''}{limit?.five_hour_percent != null ? <> · 5-hour <T bold color={tone[line.fiveHour.tone]}>{line.fiveHour.text}</T></> : null} · {line.measured}</T>
                : `${tokens(spent.tokens)} tokens · no limits reported`} />;
        })}
        <SectionHeader title="summary" />
        <T style={{ paddingHorizontal: 12, paddingVertical: 4 }}><T bold>{cost(total)}</T><T dim> · {tokens(total.tokens)} tokens · {periodName(hours)}</T></T>
        {(['agent', 'mission'] as const).map(grouping => {
          const best = top(grouping);
          return best ? <T key={grouping} style={{ paddingHorizontal: 12, paddingVertical: 2 }}><T dim>most by {grouping}  </T>{label(best.id, names, rows, declared)}<T color={theme.accent}>  {cost(best.total)}</T></T> : null;
        })}
        {total.unpriced > 0 ? <Note tone="warning">{tokens(total.unpriced)} tokens had no price, so the cost is at least this.</Note> : null}
        <SectionHeader title={`by ${by}`} count={groups(rows, by).length} />
        {groups(rows, by).map(({ id, total: spent }) => <ListRow key={id} title={label(id, names, rows, declared)} right={<T color={theme.accent}>{cost(spent)}</T>} onPress={() => open(id)}
          second={`${tokens(spent.tokens)} tokens · ${tokens(spent.output)} out · ${spent.tokens ? Math.floor(spent.cached * 100 / spent.tokens) : 0}% cached`} />)}
        <Note>Costs are API-equivalent list prices.</Note>
      </> : null}
    </ScrollView>
  </Screen>;
}

// One group's spend: its total, how its tokens split, and where they went by step, agent, model
// and host; an account's (or the whole period's) limits too.
export function UsageDetailScreen() {
  const route = useRoute<RouteProp<StackParams, 'UsageDetail'>>();
  const { id, hours } = route.params;
  const { period, issue } = useUsage(PERIODS.find(period => period === hours) ?? PERIODS[0]);
  const names = useNames();
  const navigation = useNavigation<NativeStackNavigationProp<StackParams>>();
  const now = Date.now();
  const rows = period?.rows ?? [];
  const own = byOf(id);
  const mine = own ? rows.filter(row => groups([row], own)[0]?.id === id) : rows;
  const total = totalOf(mine);
  const limits = (period?.limits ?? []).filter(limit => id === `account/${limit.account}`);
  const declared = period?.accounts ?? [];
  const credits = declared.find(account => account.credits && id === `account/${account.account ?? account.name}`)?.credits;
  return <Screen>
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ padding: 12, paddingBottom: 32 }}>
      <T bold style={{ fontSize: 17 }}>{label(id, names, rows, declared)}</T>
      {issue ? <Note tone="warning">{issue}</Note> : null}
      {!mine.length ? <Note>No spend recorded in {periodName(hours)}.</Note> : <>
        <T style={{ paddingVertical: 4 }}><T bold color={theme.accent}>{cost(total)}</T><T dim> API-equivalent · {tokens(total.tokens)} tokens · {periodName(hours)}</T></T>
        {total.unpriced > 0 ? <Note tone="warning">{tokens(total.unpriced)} tokens had no price, so the true cost is higher.</Note> : null}
        <T dim>input {tokens(total.input)} · output {tokens(total.output)} · to cache {tokens(total.cacheWrite)} · cached {tokens(total.cached)}</T>
        {(['step', 'agent', 'model', 'host'] as const).filter(grouping => grouping !== own).map(grouping => {
          const found = groups(mine, grouping);
          if (found.length < 2 && grouping !== 'step') return null;
          return <View key={grouping}>
            <SectionHeader title={`by ${grouping}`} count={found.length} />
            {found.slice(0, 8).map(group => <ListRow key={group.id} title={own === 'mission' && grouping === 'step' ? group.id.split('/').pop() : label(group.id, names, rows, declared)}
              right={<T color={theme.accent}>{cost(group.total)}  <T dim>{tokens(group.total.tokens)}</T></T>}
              onPress={() => navigation.push('UsageDetail', { id: group.id, hours })} />)}
            {found.length > 8 ? <Note>and {found.length - 8} more</Note> : null}
          </View>;
        })}
      </>}
      {credits ? (() => {
        const line = creditsLine(credits);
        return <View>
          <SectionHeader title="credits" />
          <T>used <T bold color={tone[line.tone]}>{line.used}</T> of {line.budget} this month</T>
        </View>;
      })() : null}
      {limits.map(limit => {
        const line = limitLine(limit, now);
        return <View key={limit.account}>
          <SectionHeader title="limits" />
          <T>weekly <T bold color={tone[line.weekly.tone]}>{line.weekly.text}</T>{line.resets ? ` ${line.resets}` : ''}{limit.five_hour_percent != null ? <> · 5-hour <T bold color={tone[line.fiveHour.tone]}>{line.fiveHour.text}</T></> : null}</T>
          <T dim>{line.measured} by {label(limit.measured_by, names, rows, declared)} on {limit.host} · {limit.seats.length} seat{limit.seats.length === 1 ? '' : 's'}{limit.plan ? ` · plan ${limit.plan}` : ''}</T>
        </View>;
      })}
    </ScrollView>
  </Screen>;
}

/** Fleet's usage card: each account's spend today and its weekly share; it opens Usage. */
export function UsageCard() {
  const navigation = useNavigation<NativeStackNavigationProp<StackParams>>();
  const { period, issue } = useUsage(PERIODS[0]);
  const names = useNames();
  const now = Date.now();
  const rows = period?.rows ?? [];
  const declared = period?.accounts ?? [];
  const found = accounts(rows, period?.limits ?? [], declared);
  return <View>
    <SectionHeader title="usage · today" />
    {issue ? <Note tone="warning">{issue}</Note> : null}
    {!period && !issue ? <Note>Loading usage…</Note> : null}
    {found.map(({ id, total: spent, limit, declared: account }) => {
      const weekly = limit ? limitLine(limit, now).weekly : null;
      const credits = account?.credits ? creditsLine(account.credits) : null;
      return <ListRow key={id} title={label(id, names, rows, declared)} onPress={() => navigation.navigate('Usage')}
        right={<T color={theme.accent}>{cost(spent)}</T>}
        second={credits
          ? <T dim>credits <T bold color={tone[credits.tone]}>{credits.used}</T> of {credits.budget}</T>
          : weekly ? <T dim>weekly <T bold color={tone[weekly.tone]}>{weekly.text}</T></T> : `${tokens(spent.tokens)} tokens`} />;
    })}
    {period ? <ListRow title={<T color={theme.accent}>All usage ›</T>} onPress={() => navigation.navigate('Usage')} /> : null}
  </View>;
}
