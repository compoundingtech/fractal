/**
 * One descriptor for every entity the app shows: agents, missions, resources (files, diffs, terminals, PRs, issues,
 * runs, documents) and folders. The hover card resolves its content from `kind` + `id`, and a drag carries the same
 * descriptor as its payload, so a sidebar row, a tab, a transcript mention and a resource row name one entity one way.
 * Pure data: no React, no styles, so drag-and-drop and hosts import it without loading any card.
 */
export const ENTITY_KINDS = ['agent', 'mission', 'resource', 'folder'] as const
export type EntityKind = (typeof ENTITY_KINDS)[number]

/** The identity of an entity. Two refs with the same kind and id show the identical card. */
export interface EntityRef {
  readonly kind: EntityKind
  /** The host's stable reference: the agent ref, the mission id, the resource uri, the folder id. */
  readonly id: string
}

/** Status as color-meaning: only `needs-you` (amber) and `failed` (red) draw color; the rest stay neutral marks. */
export const ENTITY_STATUS_TONES = ['neutral', 'working', 'needs-you', 'failed', 'done'] as const
export type EntityStatusTone = (typeof ENTITY_STATUS_TONES)[number]
export interface EntityStatus {
  readonly tone: EntityStatusTone
  /** The reported status word, e.g. "Working" or "Waiting for you". */
  readonly label: string
}

/** What a card header and a drag preview both need: kind + ref + title + status. */
export interface EntityDescriptor extends EntityRef {
  readonly title: string
  readonly status?: EntityStatus
}

export const entityKey = (ref: EntityRef): string => `${ref.kind}:${ref.id}`
export const sameEntity = (a: EntityRef, b: EntityRef): boolean => a.kind === b.kind && a.id === b.id

/** The sidebar row's hover intent, the canonical timing for every entity card: open after 150ms, close 300ms after leaving. */
export const hoverCardTiming = { openMs: 150, closeMs: 300 } as const

/** Drag-and-drop MIME type for an entity descriptor. */
export const ENTITY_DRAG_TYPE = 'application/x-fractal-entity'

/** The drag items for one entity: the descriptor as JSON, and its title for drop targets outside the app. */
export const entityDragPayload = (entity: EntityDescriptor): Record<string, string> => ({
  [ENTITY_DRAG_TYPE]: JSON.stringify(descriptorOf(entity)),
  'text/plain': entity.title,
})

const isKind = (value: unknown): value is EntityKind => typeof value === 'string' && (ENTITY_KINDS as readonly string[]).includes(value)
const isTone = (value: unknown): value is EntityStatusTone => typeof value === 'string' && (ENTITY_STATUS_TONES as readonly string[]).includes(value)
const descriptorOf = ({ kind, id, title, status }: EntityDescriptor): EntityDescriptor =>
  status === undefined ? { kind, id, title } : { kind, id, title, status: { tone: status.tone, label: status.label } }

/** A dropped payload is untrusted input: anything that is not a complete descriptor is not an entity. */
export const parseEntityDragPayload = (data: string | undefined): EntityDescriptor | undefined => {
  if (data === undefined) return undefined
  let value: unknown
  try { value = JSON.parse(data) } catch { return undefined }
  if (typeof value !== 'object' || value === null) return undefined
  const { kind, id, title, status } = value as Record<string, unknown>
  if (!isKind(kind) || typeof id !== 'string' || id === '' || typeof title !== 'string') return undefined
  const parsedStatus = typeof status === 'object' && status !== null && isTone((status as Record<string, unknown>).tone) && typeof (status as Record<string, unknown>).label === 'string'
    ? { tone: (status as { tone: EntityStatusTone }).tone, label: (status as { label: string }).label }
    : undefined
  return parsedStatus === undefined ? { kind, id, title } : { kind, id, title, status: parsedStatus }
}
