import { ShowMenu, type ShowMenuProps } from '../../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarShowMenu.tsx'

import { askingToken, parseSidebarQuery } from './filter.ts'
import { activeFilterCount, machineKey, selectedFilterKeys, showCounts, showEndedKey, showMachines, showRetiredKey, statusKey, withFilterKeys, type ShowRow } from './show.ts'
import { defaultFilters, type SidebarFilters } from './state.ts'
import { statuses, type AgentStatus } from './StatusIcon.tsx'

/** Statuses an operator filters by every day; the rest fold behind More. */
const commonStatuses: readonly AgentStatus[] = ['working', 'waiting', 'idle', 'failed']
const rareStatuses = (Object.keys(statuses) as AgentStatus[]).filter((status) => !commonStatuses.includes(status))

/** The sidebar's one Filter menu. Query choices are read from and written to the search query, never held separately. */
export const SidebarShowMenu = ({
  filters,
  update,
  rows,
  hosts,
  popoverTheme,
}: {
  readonly filters: SidebarFilters
  readonly update: (value: SidebarFilters) => void
  readonly rows: readonly ShowRow[]
  readonly hosts: readonly string[]
  readonly popoverTheme?: ShowMenuProps['popoverTheme']
}) => {
  const parsedQuery = parseSidebarQuery(filters.query)
  const machines = showMachines({ hosts, parsedQuery })
  const counts = showCounts({ rows, parsedQuery, machines: machines.map((machine) => machine.value), filters, statuses: Object.keys(statuses) })
  const statusEntry = (status: AgentStatus) => ({ id: statusKey(status), label: statuses[status].label, count: counts.statuses.get(status) ?? 0 })
  const active = activeFilterCount(filters, parsedQuery)
  return (
    <ShowMenu
      label="Filter"
      activeCount={active}
      groups={[
        { id: 'attention', title: 'Attention', entries: [{ id: askingToken, label: 'Waiting for answer', count: counts.asking }] },
        ...(machines.length === 0
          ? []
          : [{
              id: 'machines',
              title: 'Machine',
              entries: machines.map((machine) => ({
                id: machineKey(machine.value),
                label: machine.label,
                count: counts.machines.get(machine.value) ?? 0,
              })),
            }]),
        { id: 'status', title: 'Status', entries: commonStatuses.map(statusEntry), more: rareStatuses.map(statusEntry) },
        {
          id: 'finished',
          title: 'Finished',
          entries: [
            { id: showEndedKey, label: 'Show ended', count: counts.ended },
            { id: showRetiredKey, label: 'Show retired', count: counts.retired },
          ],
        },
      ]}
      actions={[{
        id: 'reset',
        label: 'Reset filters',
        isDisabled: active === 0,
        onAction: () => update({ ...defaultFilters, query: parsedQuery.text, sort: filters.sort }),
      }]}
      selectedKeys={selectedFilterKeys(filters, parsedQuery)}
      onSelectionChange={(keys) => update(withFilterKeys({ filters, keys }))}
      {...(popoverTheme === undefined ? {} : { popoverTheme })}
    />
  )
}
