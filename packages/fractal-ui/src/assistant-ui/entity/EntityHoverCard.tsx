/**
 * Identity: one entity, one card. Hosts provide an `EntityCardSource` that resolves an `EntityRef` (kind + id) to its
 * card, built from the same feeds the sidebar row reads. Every surface then asks for the card by ref:
 *   sidebar row      <HoverCardSurface> around <EntityHoverCard entity={ref} />
 *   workspace tab    the tab's hover slot holds <EntityHoverCard entity={ref} />
 *   transcript link  <EntityHoverCard entity={ref}><Link /></EntityHoverCard>
 *   resource row     <EntityHoverCard entity={ref} placement="left"><Button /></EntityHoverCard>
 * Sources nest: an inner source answers the kinds it knows (a plugin's resources) and defers the rest outward.
 */
import * as React from 'react'
import { HoverCard, type HoverCardPlacement } from './HoverCard'
import type { EntityRef } from './entity.ts'

/** Returns the card for an entity, or `undefined` when this source does not know it. */
export type EntityCardResolver = (entity: EntityRef) => React.ReactNode | undefined
const EntityCardSourceContext = React.createContext<EntityCardResolver | undefined>(undefined)

export function EntityCardSource({ resolve, children }: { readonly resolve: EntityCardResolver; readonly children: React.ReactNode }) {
  const outer = React.useContext(EntityCardSourceContext)
  const composed = React.useCallback<EntityCardResolver>(entity => resolve(entity) ?? outer?.(entity), [resolve, outer])
  return <EntityCardSourceContext.Provider value={composed}>{children}</EntityCardSourceContext.Provider>
}

/** The resolved card, for a host that owns the surface (a tab, a row); `null` when no source knows the entity. */
export function useEntityCard(entity: EntityRef): React.ReactNode | null {
  const resolve = React.useContext(EntityCardSourceContext)
  return resolve?.(entity) ?? null
}

export interface EntityHoverCardProps {
  readonly entity: EntityRef
  /** A focusable trigger to wrap. Omitted: renders the card itself, for a host that owns the surface. */
  readonly children?: React.ReactElement
  readonly placement?: HoverCardPlacement
}

export function EntityHoverCard({ entity, children, placement }: EntityHoverCardProps) {
  const resolve = React.useContext(EntityCardSourceContext)
  if (children === undefined) return <>{resolve?.(entity) ?? null}</>
  return <HoverCard placement={placement} content={() => resolve?.(entity) ?? null}>{children}</HoverCard>
}
