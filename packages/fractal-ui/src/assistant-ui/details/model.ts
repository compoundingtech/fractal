import type { SidebarAgentRow } from '../sidebar/model'
import type { ResourceMetadata, ResourceData as TasteResource, ResourceFile as TasteFile } from '../resources/resource-model'

export const detailsStates = ['busy', 'partial', 'unknown', 'idle', 'new', 'absent', 'loading', 'error'] as const
export type DetailsState = typeof detailsStates[number]
export type DetailsValue<A> = ResourceMetadata<A>

/** Host-labelled observations; omitted facts are unreported, never inferred from status or age. */
export interface DetailsDisplayObservation {
  readonly label: string
  readonly value: string
  /** Host-supplied provenance or guard reason; never a guessed value. */
  readonly detail?: string
  /** This field's first observation is acquiring independently of loaded sibling fields. */
  readonly pending?: boolean
}

export interface DetailsOverview {
  readonly title: string | undefined
  readonly status: string | undefined
  /** Reported activity only; observation completeness never implies work. Omitted means no glyph. */
  readonly activity?: 'working' | 'waiting' | 'idle' | 'errored'
  readonly machine: string | undefined
  readonly harness: string | undefined
  readonly model: string | undefined
  /** Mission identity is separate from the agent/session title. */
  readonly mission?: string
  /** Explicit labels distinguish managed elapsed, root observed span and active estimates. */
  readonly durations?: readonly DetailsDisplayObservation[]
  /** Rendered status-since and incarnation-age strings, not session duration aliases. */
  readonly since: string | undefined
  readonly age: string | undefined
}

export const detailsSectionOrder = ['Workspace', 'Pull requests', 'CI runs', 'Reviews', 'Files touched', 'Terminals', 'Subagents', 'Missions', 'Attention', 'Declared bindings', 'History'] as const
export type DetailsSectionName = typeof detailsSectionOrder[number]
export interface DetailsResource {
  readonly uri: string
  readonly card: TasteResource
  readonly glyph: string
  readonly agent?: SidebarAgentRow
  readonly file?: TasteFile
  readonly terminal?: readonly string[]
}
export interface DetailsSection {
  readonly title: DetailsSectionName
  readonly items: DetailsValue<readonly DetailsResource[]>
  /** Safe host copy for unreported, unavailable or ungranted items; known [] means observed empty. */
  readonly unavailableReason?: string
  /** First acquisition is pending, distinct from an unavailable or unreported result. */
  readonly pending?: boolean
  /** This section's own read failed and can be tried again; other sections keep their content. */
  readonly failure?: DetailsReadFailure
}
/** A failed host read with its own recovery; refused reads are not failures and offer no retry. */
export interface DetailsReadFailure {
  /** One plain sentence, e.g. `Couldn't load spend.` */
  readonly message: string
  /** Re-runs only the read that failed. */
  readonly onRetry: () => void
}
/**
 * Spend is read when Details opens and on request, never on a timer: the pane shows when it was
 * last read so an old figure looks old, plus a Refresh action.
 */
export interface DetailsSpendRead {
  /** When the shown spend was read; absent until a read settles. */
  readonly updatedAt?: number
  /** A requested read is in flight; the shown figures are the previous read. */
  readonly refreshing: boolean
  readonly onRefresh: () => void
  readonly failure?: DetailsReadFailure
}
