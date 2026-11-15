/**
 * The one way the web names an agent. Ids are addresses, not names: the full id belongs in a title or tooltip,
 * never in the visible text, and no part of it stands in for a name.
 */

/** How a person reads Fractal's own machinery, as the terminal app does. */
export const FRACTAL_NAME = 'Fractal'

/** How a person reads an agent that has no name, where the copy stands alone or starts a sentence. */
export const UNNAMED_AGENT = 'An agent'

/**
 * Whether an actor is Fractal's own machinery rather than an agent or a person: the daemon, or the st3 reconciler,
 * which runs as an agent seat. Mirrors `is_fractal` in crates/fractal-ui-model/src/attention.rs exactly; the served
 * `raised_by` labels these 'st'.
 */
export const isFractalMachinery = (id: string): boolean => id.startsWith('daemon/') || id === 'agent/st3/reconciler'

/**
 * An agent's visible name: Fractal for Fractal's own machinery, whatever the roster calls it; else the name Fractal
 * serves, as declared; else An agent. The daemon labels a seat that declares no name by its id, or its id without
 * `agent/`, so a label in either form is no name. A seat that declares exactly that form reads as unnamed too: the
 * served name cannot tell the two apart.
 */
export const agentDisplayName = ({ id, name }: { readonly id: string; readonly name?: string | undefined }): string => {
  if (isFractalMachinery(id)) return FRACTAL_NAME
  const label = name?.trim() ?? ''
  return label === '' || label === id || `agent/${label}` === id ? UNNAMED_AGENT : label
}

/** A shown agent name inside a sentence ("To an agent"): only the unnamed form changes case. */
export const nameInSentence = (shown: string): string => shown === UNNAMED_AGENT ? 'an agent' : shown
