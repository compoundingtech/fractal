import type { SubjectNote } from '@smalltalk/st3-client/schema'
import { DateTime } from 'effect'

/** Index the independently followed notes once per publication, newest first. */
export const notesByTarget = (notes: readonly SubjectNote[]): ReadonlyMap<string, readonly SubjectNote[]> => {
  const targets = new Map<string, SubjectNote[]>()
  for (const note of notes) {
    const rows = targets.get(note.target)
    if (rows === undefined) targets.set(note.target, [note])
    else rows.push(note)
  }
  for (const rows of targets.values()) rows.sort((left, right) =>
    DateTime.toEpochMillis(right.created_at) - DateTime.toEpochMillis(left.created_at) || right.id.localeCompare(left.id))
  return targets
}
