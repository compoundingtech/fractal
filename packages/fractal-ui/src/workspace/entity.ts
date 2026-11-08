import { Schema } from 'effect'

/**
 * One description of an entity, shared by its tab, its drag chip, its palette row and its hover card.
 * `kind` and `status` are the host's; the primitives only carry them into render props.
 */
export interface EntityDescriptor<Kind extends string = string, Status = unknown> {
  readonly kind: Kind
  readonly id: string
  readonly title: string
  readonly status?: Status | undefined
}

export const defaultEntityDragType = 'application/x-overeng-workspace-entity'

/** What a drag carries: the entity, and the tab it came from when it is one of ours. */
export interface EntityDragPayload<Kind extends string = string, Status = unknown> {
  readonly entity: EntityDescriptor<Kind, Status>
  /** The ref the tab opens. */
  readonly ref: string
  readonly tab?: { readonly space: string; readonly id: string } | undefined
}

const PayloadSchema = Schema.Struct({
  entity: Schema.Struct({ kind: Schema.String, id: Schema.String, title: Schema.String, status: Schema.optional(Schema.Unknown) }),
  ref: Schema.String,
  tab: Schema.optional(Schema.Struct({ space: Schema.String, id: Schema.String })),
})

const decodePayload = Schema.decodeUnknownOption(Schema.fromJsonString(PayloadSchema))

export const encodeEntityDrag = (payload: EntityDragPayload): string => JSON.stringify(payload)

/** Untrusted input: anything that is not a well-formed payload is `undefined`. */
export const decodeEntityDrag = (text: string): EntityDragPayload | undefined => {
  const decoded = decodePayload(text)
  return decoded._tag === 'Some' ? decoded.value : undefined
}
