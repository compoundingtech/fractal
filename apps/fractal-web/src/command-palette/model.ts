import type { Agent } from '../data/source.ts'
import type { SidebarNode } from '../folders/sidebarContract.ts'
import { matchFields } from './match.ts'

/** Palette mode: Switch lists destinations, Commands lists shell actions. */
export type PaletteMode = 'Switch' | 'Commands'

export type CommandId = 'attention.open' | 'terminal.toggle' | 'details.toggle' | 'sidebar.toggle' | 'theme.toggle' | 'link.copy'
export type CommandCategory = 'Navigate' | 'View' | 'Share'

/** A shell command with its observed availability; handlers live with the shell, not here. */
export interface PaletteCommand {
  readonly id: CommandId
  readonly title: string
  readonly detail: string
  readonly category: CommandCategory
  readonly keywords?: readonly string[]
  /** Visible shortcut chip, e.g. `⌘B`; absent when the command has no global binding. */
  readonly shortcut?: string
  /** Known unavailability; the row stays visible and disabled with this reason. */
  readonly disabledReason?: string
}

/** Agent identity and mission identity stay distinct; a mission opens an agent that works on it. */
export type PaletteTarget =
  | { readonly _tag: 'Agent'; readonly ref: string }
  | { readonly _tag: 'Mission'; readonly id: string; readonly agentRef: string | undefined }
  | { readonly _tag: 'Command'; readonly id: CommandId }

export interface PaletteRow {
  /** Unique menu key: the agent ref, mission id or command id. */
  readonly id: string
  readonly target: PaletteTarget
  readonly title: string
  readonly titleHighlights: readonly number[]
  /** Context segments shown after the title, each with its own highlights. */
  readonly context: readonly { readonly text: string; readonly highlights: readonly number[] }[]
  readonly state: string
  readonly marks: readonly ('Current' | 'Recent')[]
  readonly shortcut?: string
  readonly disabledReason?: string
}

export interface PaletteSection {
  readonly id: 'needs-you' | 'recent' | 'matches' | 'agents' | 'missions' | CommandCategory
  readonly title: string
  readonly rows: readonly PaletteRow[]
  /** Row count before `leadingRows` cut the tail; headers always show the complete count. */
  readonly total?: number
}

/** Mission rows read only these fields; live rows are generated `St3.Mission` values. */
export interface PaletteMission {
  readonly id: string
  readonly title: string
  readonly state: string | { readonly raw: string }
}

export const recentLimit = 20

/** Most recent first, unique, bounded; only refs are kept, never names or transcript content. */
export const recordVisit = (recents: readonly string[], ref: string): readonly string[] =>
  ref === '' || recents[0] === ref ? recents : [ref, ...recents.filter(value => value !== ref)].slice(0, recentLimit)

/** Folder path per agent subject from the canonical sidebar arrangement (`Design / Icons`). */
export const agentFolders = (tree: readonly SidebarNode[]): ReadonlyMap<string, string> => {
  const folders = new Map<string, string>()
  const visit = (nodes: readonly SidebarNode[], path: readonly string[]) => {
    for (const node of nodes) {
      if (node._tag === 'Agent') {
        if (path.length > 0 && !folders.has(node.subject)) folders.set(node.subject, path.join(' / '))
      } else visit(node.children, node._tag === 'Folder' ? [...path, node.label] : path)
    }
  }
  visit(tree, [])
  return folders
}

const byLabel = (left: { readonly title: string; readonly id: string }, right: { readonly title: string; readonly id: string }) =>
  left.title.localeCompare(right.title) || left.id.localeCompare(right.id)

/**
 * Switch results. Empty query: Needs you pinned, then recent agents newest first, then the remaining agents by
 * name, then missions. Query: every token must match the agent name, folder, host or ref (word prefix > initials >
 * substring); matching urgent agents stay pinned, the rest rank by score, then recency, then name.
 */
export const switchSections = ({ query, agents, folders, needsYou, recents, current, statusLabel, missions }: {
  readonly query: string
  readonly agents: readonly Agent[]
  readonly folders: ReadonlyMap<string, string>
  readonly needsYou: ReadonlySet<string>
  readonly recents: readonly string[]
  readonly current: string
  readonly statusLabel: (agent: Agent) => string
  readonly missions: readonly PaletteMission[] | undefined
}): readonly PaletteSection[] => {
  const querying = query.trim() !== ''
  const recentRank = new Map(recents.map((ref, index) => [ref, index]))
  const rank = (ref: string) => recentRank.get(ref) ?? Number.MAX_SAFE_INTEGER
  const agentRows = agents.flatMap(agent => {
    const folder = folders.get(agent.ref) ?? ''
    // Every ref starts with `agent/`; searching that prefix would match every agent for "agent".
    const id = agent.ref.replace(/^agent\//, '')
    const matched = matchFields(query, [agent.name, folder, agent.host, id])
    if (matched === undefined) return []
    const [name = [], folderMarks = [], hostMarks = [], refMarks = []] = matched.highlights
    const urgent = needsYou.has(agent.ref)
    const row: PaletteRow = {
      id: agent.ref,
      target: { _tag: 'Agent', ref: agent.ref },
      title: agent.name,
      titleHighlights: name,
      context: [
        ...(folder === '' ? [] : [{ text: folder, highlights: folderMarks }]),
        { text: agent.host, highlights: hostMarks },
        // A ref is shown only when it is the reason for the match.
        ...(matched.primaryFields.includes(3) ? [{ text: id, highlights: refMarks }] : []),
      ],
      state: statusLabel(agent),
      // Urgency is named by the Needs you section header, so rows do not repeat it.
      marks: [...(agent.ref === current ? ['Current' as const] : []), ...(recentRank.has(agent.ref) ? ['Recent' as const] : [])],
    }
    return [{ row, score: matched.score, urgent, recent: rank(agent.ref) }]
  })
  type Ranked = (typeof agentRows)[number]
  const order = (left: Ranked, right: Ranked) =>
    (querying ? right.score - left.score : 0) || left.recent - right.recent || byLabel(left.row, right.row)
  const urgent = agentRows.filter(entry => entry.urgent).toSorted(order)
  const rest = agentRows.filter(entry => !entry.urgent).toSorted(order)
  const agentsByMission = new Map<string, Agent[]>()
  for (const agent of agents) if (agent.mission !== undefined) agentsByMission.set(agent.mission, [...agentsByMission.get(agent.mission) ?? [], agent])
  const missionRows = (missions ?? []).flatMap(mission => {
    const state = typeof mission.state === 'string' ? mission.state : mission.state.raw
    const matched = matchFields(query, [mission.title, mission.id])
    if (matched === undefined) return []
    // Prefer the agent the person last visited, then the current one, then a stable name order.
    const workers = (agentsByMission.get(mission.id) ?? []).toSorted((left, right) => rank(left.ref) - rank(right.ref) || Number(right.ref === current) - Number(left.ref === current) || left.name.localeCompare(right.name))
    const agent = workers[0]
    const [title = [], idMarks = []] = matched.highlights
    const row: PaletteRow = {
      id: mission.id,
      target: { _tag: 'Mission', id: mission.id, agentRef: agent?.ref },
      title: mission.title,
      titleHighlights: title,
      context: [
        { text: agent === undefined ? 'mission' : `mission · ${agent.name}${workers.length > 1 ? ` +${workers.length - 1}` : ''}`, highlights: [] },
        ...(matched.primaryFields.includes(1) ? [{ text: mission.id, highlights: idMarks }] : []),
      ],
      state,
      marks: [],
      ...(agent === undefined ? { disabledReason: 'No agent in the roster works on this mission, and this web client has no mission view yet.' } : {}),
    }
    return [{ row, score: matched.score, working: agent !== undefined }]
  }).toSorted((left, right) => (querying ? right.score - left.score : 0) || Number(right.working) - Number(left.working) || byLabel(left.row, right.row))
  const sections: PaletteSection[] = []
  if (urgent.length > 0) sections.push({ id: 'needs-you', title: 'Needs you', rows: urgent.map(entry => entry.row) })
  if (querying) {
    if (rest.length > 0) sections.push({ id: 'matches', title: 'Agents', rows: rest.map(entry => entry.row) })
  } else {
    const recent = rest.filter(entry => entry.recent !== Number.MAX_SAFE_INTEGER)
    const others = rest.filter(entry => entry.recent === Number.MAX_SAFE_INTEGER)
    if (recent.length > 0) sections.push({ id: 'recent', title: 'Recent', rows: recent.map(entry => entry.row) })
    if (others.length > 0) sections.push({ id: 'agents', title: 'Agents', rows: others.map(entry => entry.row) })
  }
  if (missionRows.length > 0) sections.push({ id: 'missions', title: 'Missions', rows: missionRows.map(entry => entry.row) })
  return sections
}

/**
 * The first `limit` rows in display order. Sections keep their order and only the tail is cut, so every row key
 * shown first stays valid when the complete list replaces it.
 */
export const leadingRows = (sections: readonly PaletteSection[], limit: number): readonly PaletteSection[] => {
  let remaining = limit
  return sections.flatMap(section => {
    if (remaining <= 0) return []
    const rows = section.rows.slice(0, remaining)
    remaining -= rows.length
    return [rows.length === section.rows.length ? section : { ...section, rows, total: section.rows.length }]
  })
}

const categories: readonly CommandCategory[] = ['Navigate', 'View', 'Share']

/** Commands grouped by category in a fixed order; a query ranks rows inside each category. */
export const commandSections = ({ query, commands }: { readonly query: string; readonly commands: readonly PaletteCommand[] }): readonly PaletteSection[] =>
  categories.flatMap(category => {
    const rows = commands.filter(command => command.category === category).flatMap((command, index) => {
      const matched = matchFields(query, [command.title, command.detail, ...(command.keywords ?? [])])
      if (matched === undefined) return []
      const [title = [], detail = []] = matched.highlights
      const row: PaletteRow = {
        id: command.id,
        target: { _tag: 'Command', id: command.id },
        title: command.title,
        titleHighlights: title,
        context: [{ text: command.detail, highlights: detail }],
        state: '',
        marks: [],
        ...(command.shortcut === undefined ? {} : { shortcut: command.shortcut }),
        ...(command.disabledReason === undefined ? {} : { disabledReason: command.disabledReason }),
      }
      return [{ row, score: matched.score, index }]
    }).toSorted((left, right) => right.score - left.score || left.index - right.index)
    return rows.length === 0 ? [] : [{ id: category, title: category, rows: rows.map(entry => entry.row) }]
  })
