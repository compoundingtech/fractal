import { useAtom } from '@effect/atom-react'
import { Schema } from 'effect'

import { persistedAtom } from '../../state/persistence.ts'
import {
  askingToken,
  matchesSidebarHost,
  parseSidebarQuery,
  printSidebarQuery,
  sidebarHostValue,
  type SidebarMatchRow,
  type SidebarQuery,
} from './filter.ts'
import type { SidebarFilters } from './state.ts'

/** Show menu keys are the search tokens themselves, so menu and search share one grammar. */
export const machineKey = (machine: string): string => `machine:${machine}`
/** Every machine value in the query's machine tokens, first occurrence first. */
export const selectedMachines = (parsedQuery: SidebarQuery): readonly string[] => [...new Set(parsedQuery.machines.flat())]
const sameMachines = (left: readonly string[], right: readonly string[]): boolean =>
  left.every((machine) => right.includes(machine)) && right.every((machine) => left.includes(machine))
/** Checked Show entries derived from the query. */
export const selectedShowKeys = (parsedQuery: SidebarQuery): ReadonlySet<string> =>
  new Set([...(parsedQuery.asking ? [askingToken] : []), ...selectedMachines(parsedQuery).map(machineKey)])
/** Menu selection back to the query: text stays; typed machine groups stay unless the machine choice changed,
 * in which case the checked machines become one OR-ed machine token. */
export const withShowKeys = ({
  parsedQuery,
  keys,
}: {
  readonly parsedQuery: SidebarQuery
  readonly keys: ReadonlySet<string>
}): SidebarQuery => {
  const machines = [...keys].flatMap((key) => (key.startsWith('machine:') ? [key.slice('machine:'.length)] : []))
  const unchanged = sameMachines(machines, selectedMachines(parsedQuery))
  return {
    ...parsedQuery,
    asking: keys.has(askingToken),
    machines: unchanged ? parsedQuery.machines : machines.length === 0 ? [] : [machines],
  }
}

/** Menu keys for state that is not in the query: selected statuses, and finished agents left showing. */
export const statusKey = (status: string): string => `status:${status}`
export const showEndedKey = 'show:ended'
export const showRetiredKey = 'show:retired'
/** Every checked Filter entry: the query's own, the statuses, and finished agents that are not hidden. */
export const selectedFilterKeys = (filters: Pick<SidebarFilters, 'hideEnded' | 'hideRetired' | 'statuses'>, parsedQuery: SidebarQuery): ReadonlySet<string> =>
  new Set([
    ...selectedShowKeys(parsedQuery),
    ...filters.statuses.map(statusKey),
    ...(filters.hideEnded ? [] : [showEndedKey]),
    ...(filters.hideRetired ? [] : [showRetiredKey]),
  ])
/** A Filter menu selection back to the settings; query choices go through the shared grammar. */
export const withFilterKeys = ({ filters, keys }: { readonly filters: SidebarFilters; readonly keys: ReadonlySet<string> }): SidebarFilters => ({
  ...filters,
  query: printSidebarQuery(withShowKeys({ parsedQuery: parseSidebarQuery(filters.query), keys })),
  hideEnded: !keys.has(showEndedKey),
  hideRetired: !keys.has(showRetiredKey),
  statuses: [...keys].flatMap((key) => (key.startsWith('status:') ? [key.slice('status:'.length)] : [])),
})
/** The trigger's count: everything narrowing the list except free text, typed facets included, so Reset always has something to clear. */
export const activeFilterCount = (filters: Pick<SidebarFilters, 'hideEnded' | 'hideRetired' | 'statuses'>, parsedQuery: SidebarQuery): number =>
  Number(filters.hideEnded) + Number(filters.hideRetired) + Number(parsedQuery.asking) +
  selectedMachines(parsedQuery).length + parsedQuery.facets.length + filters.statuses.length

/** Machine entries: every observed host, plus selected machines no host reports, so they stay clearable. */
export const showMachines = ({
  hosts,
  parsedQuery,
}: {
  readonly hosts: readonly string[]
  readonly parsedQuery: SidebarQuery
}): readonly { readonly value: string; readonly label: string }[] => {
  const entries = new Map<string, string>()
  for (const host of hosts.toSorted()) if (!entries.has(sidebarHostValue(host))) entries.set(sidebarHostValue(host), host)
  for (const machine of selectedMachines(parsedQuery)) if (!entries.has(machine)) entries.set(machine, machine)
  return [...entries].map(([value, label]) => ({ value, label }))
}

/** One roster row as the Filter menu counts it; `others` = passes free text and typed facets, everything outside the menu. */
export interface ShowRow {
  readonly host: string
  readonly asking: boolean
  readonly others: boolean
  /** Omitted facts count as no status and not finished. */
  readonly status?: string
  readonly ended?: boolean
  readonly retired?: boolean
}
const noMenuFilters = { hideEnded: false, hideRetired: false, statuses: [] }
/** A row's menu facts. `ended` also covers state done, as `hideEnded` does; both matchers share this reading. */
export const showRowFacts = (row: Pick<SidebarMatchRow, 'status' | 'agent'>): Pick<ShowRow, 'status' | 'ended' | 'retired'> => ({
  status: row.status, ended: row.status === 'ended' || row.agent?.state === 'done', retired: row.status === 'retired',
})
/** Settings with everything the menu owns removed, so `others` measures only free text and typed facets. */
export const outsideFilterMenu = <T extends SidebarFilters>(filters: T): T => ({ ...filters, ...noMenuFilters })
/** Faceted counts: each entry counts rows that pass every other filter and that entry. */
export const showCounts = ({
  rows,
  parsedQuery,
  machines,
  filters = noMenuFilters,
  statuses = [],
}: {
  readonly rows: readonly ShowRow[]
  readonly parsedQuery: SidebarQuery
  readonly machines: readonly string[]
  readonly filters?: Pick<SidebarFilters, 'hideEnded' | 'hideRetired' | 'statuses'>
  readonly statuses?: readonly string[]
}) => {
  let asking = 0, ended = 0, retired = 0
  const machineCounts = new Map(machines.map((machine) => [machine, 0]))
  const statusCounts = new Map(statuses.map((status) => [status, 0]))
  for (const row of rows) {
    if (!row.others) continue
    const passesAsking = !parsedQuery.asking || row.asking
    const passesMachines = matchesSidebarHost(row.host, parsedQuery.machines)
    const passesStatus = filters.statuses.length === 0 || (row.status !== undefined && filters.statuses.includes(row.status))
    const passesEnded = !(filters.hideEnded && row.ended === true)
    const passesRetired = !(filters.hideRetired && row.retired === true)
    const finished = passesEnded && passesRetired
    if (row.asking && passesMachines && passesStatus && finished) asking++
    if (passesAsking && passesStatus && finished) for (const machine of machines) if (matchesSidebarHost(row.host, [[machine]])) machineCounts.set(machine, machineCounts.get(machine)! + 1)
    if (passesAsking && passesMachines && finished && row.status !== undefined && statusCounts.has(row.status)) statusCounts.set(row.status, statusCounts.get(row.status)! + 1)
    if (passesAsking && passesMachines && passesStatus) {
      if (row.ended === true && passesRetired) ended++
      if (row.retired === true && passesEnded) retired++
    }
  }
  return { asking, machines: machineCounts, statuses: statusCounts, ended, retired }
}

/** Browser-local machine groups: `null` = no readable choice; `[]` = explicitly cleared.
 * Version 2 preserves AND between tokens and OR within each token. */
export const machineMemory = {
  key: 'sidebar.show.machines@2',
  version: 2,
  schema: Schema.NullOr(Schema.Array(Schema.Array(Schema.String.check(Schema.isPattern(/^[^\s,A-Z]+$/u))).check(Schema.isMinLength(1)))),
  defaultValue: null,
}
const rememberedMachines = persistedAtom(machineMemory)
const sameMachineGroups = (left: SidebarQuery['machines'], right: SidebarQuery['machines']): boolean =>
  left.length === right.length && left.every((group, index) => sameMachines(group, right[index]!))
/** Replace only machine tokens, preserving the remembered AND/OR structure. */
export const withRememberedMachines = ({
  query,
  remembered,
}: {
  readonly query: string
  readonly remembered: SidebarQuery['machines'] | null
}): string => {
  const parsedQuery = parseSidebarQuery(query)
  if (remembered === null || sameMachineGroups(remembered, parsedQuery.machines)) return query
  return printSidebarQuery({ ...parsedQuery, machines: remembered })
}
/** Partial or unknown typed hosts still filter live, but never overwrite the last complete choice. */
export const machinesToRemember = (query: string, hosts: readonly string[]): SidebarQuery['machines'] | null => {
  const machines = parseSidebarQuery(query).machines
  const known = new Set(hosts.map(sidebarHostValue))
  return machines.every((group) => group.every((machine) => known.has(machine))) ? machines : null
}
/** The query stays the only live machine state; memory mirrors complete machine tokens. */
export const useMachineMemory = () => {
  const [remembered, setRemembered] = useAtom(rememberedMachines)
  const remember = (query: string, hosts: readonly string[]) => {
    const next = machinesToRemember(query, hosts)
    if (next !== null && (remembered === null || !sameMachineGroups(next, remembered))) setRemembered(next)
  }
  return { remembered, remember }
}
