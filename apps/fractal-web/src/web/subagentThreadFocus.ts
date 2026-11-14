import * as Atom from 'effect/reactivity/Atom'

/** A count press requests focus without replacing the selected child thread. */
export const subagentThreadFocus = Atom.family((_parent: string) => Atom.make(0))
