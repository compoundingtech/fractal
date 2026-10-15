import type { SubjectNote } from '@smalltalk/st3-client/schema'
import { DateTime } from 'effect'

/** Newest first by creation, then by id descending so equal instants order stably. */
export const compareNewestFirst = (left: SubjectNote, right: SubjectNote): number =>
  DateTime.toEpochMillis(right.created_at) - DateTime.toEpochMillis(left.created_at) || right.id.localeCompare(left.id)

export const newestFirst = (notes: readonly SubjectNote[]): readonly SubjectNote[] => [...notes].sort(compareNewestFirst)

export const newestNote = (notes: readonly SubjectNote[]): SubjectNote | undefined => {
  let newest: SubjectNote | undefined
  for (const note of notes) if (newest === undefined || compareNewestFirst(note, newest) < 0) newest = note
  return newest
}
