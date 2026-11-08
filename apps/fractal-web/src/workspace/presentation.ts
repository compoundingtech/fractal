import type { Attention } from '@fractal/client/schema'
import type { IconName } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import type { EntityRef, EntityStatus } from '../../../../packages/fractal-ui/src/assistant-ui/entity/entity.ts'
import type { Agent, Feed } from '../data/source.ts'
import type { Subject } from './subject.ts'

/**
 * How every surface names one subject: its tab, the breadcrumb, a palette row, a drag, the document title
 * and the hover card all read one descriptor, so they cannot disagree and no surface invents its own label.
 * Every kind must supply `present` (see `KindSurface`), and a title is never a bare string: it is observed,
 * still loading, or gone. A kind word or a ref fragment therefore cannot pass as a name.
 */
export type Title =
  /** A name the person would recognise: what the entity is called in the data that describes it. */
  | { readonly _tag: 'Known'; readonly text: string }
  /** The describing data has not arrived. `kind` is the word the surface shows meanwhile ("Alert"). */
  | { readonly _tag: 'Pending'; readonly kind: string }
  /** The describing data arrived and no longer has this entity: answered, deleted, or never existed. */
  | { readonly _tag: 'Gone'; readonly kind: string }

/** A fixed list or screen: its name is its kind, by design, not a fallback. */
export type FixedTitle = { readonly _tag: 'Fixed'; readonly text: string }

export const Title = {
  /** Only from a row of observed data. */
  known: (row: { readonly title: string } | { readonly name: string }): Title => ({ _tag: 'Known', text: 'title' in row ? row.title : row.name }),
  pending: (kind: string): Title => ({ _tag: 'Pending', kind }),
  gone: (kind: string): Title => ({ _tag: 'Gone', kind }),
  /** Pending until `observed`, then the row or gone. */
  of: <A extends { readonly title: string } | { readonly name: string }>(kind: string, observed: boolean, row: A | undefined): Title =>
    row !== undefined ? Title.known(row) : observed ? Title.gone(kind) : Title.pending(kind),
} as const

/** The leading mark of a tab or row. */
export type Mark =
  | { readonly _tag: 'Icon'; readonly name: IconName }
  /** The agent's own sidebar ring: status, needs-you amber, failed red. */
  | { readonly _tag: 'Agent'; readonly agent: string }
  | { readonly _tag: 'Status'; readonly status: EntityStatus }

export interface KindDescriptor {
  readonly title: Title | FixedTitle
  /** What this is, in words: "Alert", "Mission", "Pull request". The card header and screen readers use it. */
  readonly kind: string
  readonly mark: Mark
  /** The hover card's identity. `null` states there is no card for this kind, by design. */
  readonly card: EntityRef | null
  /** Disambiguating context, drawn dimmed: the asking agent, a repository. */
  readonly detail?: string
}

/** What presenting reads: the same observations the sidebar and palette read, never a second fetch. */
export interface PresentationWorld {
  readonly agents: readonly Agent[]
  /** The roster has been observed, so an agent absent from it is gone rather than loading. */
  readonly rosterKnown: boolean
  readonly needsYou: ReadonlySet<string>
  readonly attention: Feed<readonly Attention[]>
  /** Titles by ref, from the subject index: missions, documents and observed resources. */
  readonly subjects: ReadonlyMap<string, { readonly title: string }>
  /** The missions feed has been observed, so a mission absent from `subjects` is gone rather than loading. */
  readonly missionsKnown: boolean
}

export type Present = (subject: Subject, world: PresentationWorld) => KindDescriptor

/** The one string a surface prints for a title. A pending or gone title prints its kind word; the surface styles it. */
export const titleText = (title: Title | FixedTitle): string =>
  title._tag === 'Known' || title._tag === 'Fixed' ? title.text : title.kind
