/** Drivers whose daemon roster projection reports subagents, including an explicit empty list. */
export const SUBAGENT_OBSERVING_DRIVERS: Readonly<Partial<Record<string, true>>> = { claude: true, codex: true, omp: true }

/** Preserve capability and the actual wire driver, including unfamiliar or absent values. */
export type SubagentDriver =
  | { readonly _tag: 'Observing'; readonly name: string }
  | { readonly _tag: 'NonObserving'; readonly name: 'opencode' | 'pi' }
  | { readonly _tag: 'Unknown'; readonly name?: string }

export const classifySubagentDriver = (name: string | undefined): SubagentDriver => {
  if (name !== undefined && Object.prototype.hasOwnProperty.call(SUBAGENT_OBSERVING_DRIVERS, name)) return { _tag: 'Observing', name }
  if (name === 'opencode' || name === 'pi') return { _tag: 'NonObserving', name }
  return name === undefined ? { _tag: 'Unknown' } : { _tag: 'Unknown', name }
}
