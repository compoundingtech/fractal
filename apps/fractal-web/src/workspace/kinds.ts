import type * as React from 'react'
import type { WorkLogCall } from '@fractal/ui/assistant-ui'
import type { UxTelemetry } from '../telemetry/ux.ts'
import { makeKindRegistry, type KindPolicy } from './registry.ts'
import type { Subject, SubjectKind } from './subject.ts'
import { AgentSurface, AlertSurface, DeclarationSurface, DocumentSurface, InboxSurface, MissionsSurface, ResourceSurface, UnsupportedSurface, UsageSurface } from './surfaces.tsx'

/** What a kind renders and how it hides. Its data requirements live beside it in `registry.ts`. */
export type HiddenStrategy =
  /** Keep layout and React subscribed; revealing is one composited frame. For surfaces that are costly to lay out. */
  | 'content-visibility'
  /** Tear down effects and subscriptions; React state and DOM stay. For surfaces that are cheap to lay out. */
  | 'activity'

export interface SurfaceProps {
  readonly subject: Subject
  readonly name: string | undefined
  readonly visible: boolean
  /** This tab shows its terminal face. */
  readonly terminal: boolean
  readonly onOpenTool: (call: WorkLogCall) => void
  readonly onOpen: (ref: string) => void
  /** This tab gives way to another subject, in place. */
  readonly onReplace: (ref: string) => void
  readonly onClose: () => void
  /** The list this kind belongs to, where it is a drawer. */
  readonly onBackToAlerts: () => void
  readonly ux?: UxTelemetry | undefined
}

export interface KindSurface {
  readonly Surface: (props: SurfaceProps) => React.ReactNode
  readonly hidden: HiddenStrategy
  readonly scrollMemory: boolean
  /** How a row naming one of this kind's refs presents it, where a kind has its own. */
  readonly Row?: (props: { readonly ref: string }) => React.ReactNode
  /** The hover card for one of this kind's refs. */
  readonly Card?: (props: { readonly ref: string }) => React.ReactNode
}

const agent: KindSurface = { Surface: AgentSurface, hidden: 'content-visibility', scrollMemory: false }
const missions: KindSurface = { Surface: MissionsSurface, hidden: 'activity', scrollMemory: true }

const views: Readonly<Record<SubjectKind, KindSurface>> = {
  Conversation: agent,
  Terminal: agent,
  Inbox: { Surface: InboxSurface, hidden: 'activity', scrollMemory: true },
  Alert: { Surface: AlertSurface, hidden: 'activity', scrollMemory: true },
  Missions: missions,
  Mission: missions,
  Usage: { Surface: UsageSurface, hidden: 'activity', scrollMemory: true },
  Document: { Surface: DocumentSurface, hidden: 'activity', scrollMemory: true },
  Declaration: { Surface: DeclarationSurface, hidden: 'activity', scrollMemory: true },
  Resource: { Surface: ResourceSurface, hidden: 'activity', scrollMemory: false },
  Unsupported: { Surface: UnsupportedSurface, hidden: 'activity', scrollMemory: false },
}

/** Contributed kinds join here; the core never imports a plugin's internals. */
export const workspaceKinds = makeKindRegistry<KindSurface>({ views })

export const policyOf = (subject: Subject): KindPolicy => workspaceKinds.kindOf(subject).policy
