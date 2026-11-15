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

/** How a person reads Fractal's own machinery as the one who asked or wrote. */
export const FRACTAL_NAME = 'Fractal'

/**
 * Whether an actor is Fractal's own machinery rather than an agent or a person: the daemon, or the st3 reconciler,
 * which runs as an agent seat. Mirrors `is_fractal` in crates/fractal-ui-model/src/attention.rs exactly; the served
 * `raised_by` labels these 'st'. Check it before any roster name, so the reconciler reads as Fractal even though the
 * roster names it.
 */
export const isFractalMachinery = (id: string): boolean => id.startsWith('daemon/') || id === 'agent/st3/reconciler'
