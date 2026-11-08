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

/** What a drag carries: the entity, and the tab it came from when it is one of ours. */
export interface EntityDragPayload<Kind extends string = string, Status = unknown> {
  readonly entity: EntityDescriptor<Kind, Status>
  /** The ref the tab opens. */
  readonly ref: string
  readonly tab?: { readonly space: string; readonly id: string } | undefined
}
