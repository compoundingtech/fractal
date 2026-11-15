import type { Feed, Fleet } from './source.ts'

/**
 * The one way the web names an agent. Ids are addresses, not names: the full id belongs in a title or tooltip,
 * never in the visible text.
 */

/** The last segment of an id (`agent/example/atlas` → `atlas`): the name shown when no real name exists. */
export const shortAgentName = (id: string): string => id.split('/').filter((part) => part !== '').at(-1) ?? id

/**
 * An agent's visible name: its roster name when that is a real name, else its short name. The daemon labels a
 * seat that declares no name by its path, so a label equal to the id, or to the id without `agent/`, is not a name.
 */
export const agentDisplayName = ({ id, name }: { readonly id: string; readonly name?: string | undefined }): string => {
  const label = name?.trim() ?? ''
  return label === '' || label === id || `agent/${label}` === id ? shortAgentName(id) : label
}

/** A roster lookup by agent id; roster names are already display names (see `fleetFromAgents`). */
export type AgentNames = (id: string) => string | undefined

/** Roster display names by agent ref, for surfaces that name agents they do not hold a row for. */
export const selectRosterNames = (feed: Feed<Fleet>): ReadonlyMap<string, string> =>
  new Map(feed._tag === 'Observed' ? feed.value.agents.map((agent) => [agent.ref, agent.name] as const) : [])
export const sameRosterNames = (left: ReadonlyMap<string, string>, right: ReadonlyMap<string, string>): boolean =>
  left.size === right.size && [...left].every(([ref, name]) => right.get(ref) === name)
