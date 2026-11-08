import { foldSlice } from './fold.ts'
import type { ChangesEvent, Slice, TerminalRecord } from './slice.ts'

type RosterAgent = Slice<'roster'>['state']['agents'][number]

/**
 * The gateway lists an agent's terminal-capable runtimes first in `runtime_ids`
 * (crates/fractal-api-core/src/api/exec_only.rs), and clients reach the terminal through them.
 * Authored rosters name only the agent runtime, so this derives the rest from the terminal slice:
 * every roster instant lists the terminals the agent owns at that instant, and a terminal that
 * appears or ends without an agent change gets one at that moment, as the gateway would publish.
 */
export const withTerminalRuntimes = (roster: Slice<'roster'>, terminal: Slice<'terminal'>): Slice<'roster'> => {
  const terminalRuntimes = new Set([
    ...terminal.state.terminals,
    ...terminal.timeline.flatMap((event) => (event._tag === 'terminal-create' ? [event.record] : [])),
  ].map((record) => record.runtime.id))
  const terminalsAt = (at: number) => foldSlice(terminal, at).state.terminals
  const listing = (agent: RosterAgent, terminals: readonly TerminalRecord[]): RosterAgent => {
    // A desired or stopped agent has no runtime to list, so it lists no terminal either.
    if (agent.runtime_ids.length === 0) return agent
    const owned = terminals.filter((record) => record.owner === agent.id).map((record) => record.runtime.id)
    const runtime_ids = [...owned, ...agent.runtime_ids.filter((id) => !terminalRuntimes.has(id))]
    return runtime_ids.length === agent.runtime_ids.length && runtime_ids.every((id, index) => id === agent.runtime_ids[index])
      ? agent
      : { ...agent, runtime_ids }
  }
  const relisted = (event: ChangesEvent): ChangesEvent => {
    const terminals = terminalsAt(event.at_ms)
    return { ...event, upserts: event.upserts.map((resource) => (resource.kind === 'agent' ? listing(resource as RosterAgent, terminals) : resource)) }
  }
  const state = { ...roster.state, agents: roster.state.agents.map((agent) => listing(agent, terminal.state.terminals)) }
  let timeline = roster.timeline.map((event) => (event._tag === 'changes' ? relisted(event) : event))
  // A terminal that comes, goes or changes hands changes its owners' listings at that instant. Owners are
  // tracked in timeline order, so a removal names the owner it removes from, never a later one.
  const owners = new Map(terminal.state.terminals.map((record) => [record.terminal, record.owner]))
  for (const event of terminal.timeline) {
    const terminalId = event._tag === 'terminal-create' ? event.record.terminal : event._tag === 'terminal-remove' ? event.terminal : undefined
    if (terminalId === undefined) continue
    const affected = new Set<string>()
    const previous = owners.get(terminalId)
    if (previous !== undefined) affected.add(previous)
    if (event._tag === 'terminal-create') {
      affected.add(event.record.owner)
      owners.set(terminalId, event.record.owner)
    } else owners.delete(terminalId)
    for (const owner of affected) {
      const current = foldSlice({ ...roster, state, timeline }, event.at_ms).state.agents.find((agent) => agent.id === owner)
      if (current === undefined) continue
      const next = listing(current, terminalsAt(event.at_ms))
      if (next === current) continue
      const index = timeline.findIndex((other) => other.at_ms > event.at_ms)
      const change: ChangesEvent = { _tag: 'changes', at_ms: event.at_ms, store: 0, upserts: [next], removes: [] }
      timeline = index === -1 ? [...timeline, change] : [...timeline.slice(0, index), change, ...timeline.slice(index)]
    }
  }
  return { ...roster, state, timeline }
}
