export { matchPositions } from './fuzzy.ts'
/** `tokens` is always reported with Known usage; `usd` only when the source reported a USD cost, never a substituted $0. */
export type SidebarUsage = { readonly _tag: 'Known'; readonly scope: SidebarUsageScope; readonly tokens: number; readonly usd?: number } | { readonly _tag: 'Unknown' }
/** `reported-agent` is the agent's own reported aggregate; it claims neither a 24h window nor subagent inclusion. */
export type SidebarUsageScope = '24h-root-and-subagents' | 'lifetime-incl-subagents' | 'reported-agent'
export const usageScopeLabel = (scope: SidebarUsageScope): string => scope === '24h-root-and-subagents' ? '24h root and subagents' : scope === 'lifetime-incl-subagents' ? 'Lifetime including subagents' : 'Reported by agent'
export type SidebarDuration = { readonly _tag: 'Known'; readonly scope: '24h-activity-span' | 'lifetime'; readonly ms: number } | { readonly _tag: 'Unknown' }
export type SidebarLastTurn = { readonly _tag: 'Known'; readonly kind: 'turn-completed' | 'activity'; readonly at: number } | { readonly _tag: 'Unknown' }
/** Reported main-context occupancy; this is not cumulative usage or descendant accounting. */
export type SidebarContext = { readonly _tag: 'Known'; readonly usedTokens?: number; readonly windowTokens?: number; readonly usedPercent: number; readonly observedAt: number } | { readonly _tag: 'Unknown' }
/** `done`/`total` exactly as the run reports them (cancelled steps count toward `total`); Unknown when the run summary is not hydrated, never `0/0`. */
export type SidebarMissionProgress = { readonly _tag: 'Known'; readonly done: number; readonly total: number } | { readonly _tag: 'Unknown' }
/** The agent's current step: its name, a question waiting on you (with its decision handle when the bridge reports one), or Unknown. */
export type SidebarMissionStep = { readonly _tag: 'Step'; readonly name: string } | { readonly _tag: 'Waiting'; readonly question: string; readonly handle?: string } | { readonly _tag: 'Unknown' }
/** The mission the agent currently works on, plus missions queued behind it; None when the agent has no current mission. */
export type SidebarMissionContext =
  | { readonly _tag: 'Mission'; readonly id: string; readonly slug: string; readonly progress: SidebarMissionProgress; readonly step: SidebarMissionStep; readonly queued: readonly { readonly id: string; readonly slug: string }[] }
  | { readonly _tag: 'Unavailable' }
  | { readonly _tag: 'None' }
/**
 * Freshness of `missionContext`. Live: the current observation. Stale: the last observation, retained while the source
 * catches up. Failed: the last refresh failed and the retained value is shown. `updatedAt` is the real time of the retained
 * observation when the source reports one, never an invented time. Absent means Live.
 */
export type SidebarMissionInfo =
  | { readonly _tag: 'Live' }
  | { readonly _tag: 'Stale'; readonly updatedAt?: number }
  | { readonly _tag: 'Failed'; readonly updatedAt?: number }
export interface SidebarReported {
  readonly model?: string
  readonly context?: SidebarContext
  /** Producers may provide both scoped observations; the row prefers an explicit lifetime fact. */
  readonly usage?: SidebarUsage | readonly SidebarUsage[]
  readonly duration?: SidebarDuration | readonly SidebarDuration[]
  /** Turn completion takes precedence over generic activity, which remains explicitly labelled. */
  readonly lastTurn?: SidebarLastTurn | readonly SidebarLastTurn[]
  readonly worktree?: string
  readonly branch?: string
  readonly pullRequest?: { readonly ref: string; readonly title: string; readonly number: number; readonly url?: string; readonly state: string }
}
export interface SidebarAgentRow extends SidebarReported {
  readonly ref: string
  /** Host workspace identity; canonical refs remain the projection/map keys. */
  readonly id: string
  readonly parentRef?: string
  readonly title: string
  readonly description?: string
  readonly subagent?: SidebarSubagent
  readonly harness?: string
  readonly terminal?: string
  /** Mission ref for navigation and filtering; `missionContext` carries what line 2 draws. */
  readonly mission?: string
  /** Absent or None: line 2 reads `status · host`. */
  readonly missionContext?: SidebarMissionContext
  /** Absent: Live. Stale or Failed mutes the retained mission and keeps its popover reachable to say why. */
  readonly missionInfo?: SidebarMissionInfo
  readonly status: AgentStatus
  readonly statusLabel: string
  readonly host: string
  /** Exact metadata copied from the authoritative Agent projection, never duration/turn aliases. */
  readonly statusSince?: number
  readonly lastActivityAt?: number
  readonly needsMe?: boolean
  readonly freshness: 'live' | 'stale' | 'unobserved'
  readonly unread?: number
  readonly usage: SidebarUsage
  readonly duration: SidebarDuration
  readonly lastTurn: SidebarLastTurn
  readonly childrenKnown?: boolean
  readonly children: readonly SidebarAgentRow[]
}


export interface SidebarActions {
  readonly prefetch?: (ref: string) => void
  readonly details?: (ref: string) => void
  /** Terminal actions retain the owning host workspace, including nonselected rows. */
  readonly terminal?: (ref: string, workspaceId?: string) => void
  /** Owner uses the host workspace ID, without the canonical agent/ prefix. */
  readonly resource?: (ref: string, workspaceId?: string) => void
  readonly select?: (id: string, parentId?: string) => void
  /** Host-owned split placement; the row offers it only when the host supports it. */
  readonly split?: (ref: string, placement: 'right' | 'below') => void
  /** Host-owned canonical declaration view for an agent or mission ref; the row offers it only when set. */
  readonly declaration?: (ref: string) => void
}

export interface SidebarSubagent { readonly id: string; readonly subagent_type?: string; readonly work_id?: string; readonly session_id?: string; readonly started_at?: string; readonly lease_expires_at: string }
export const statuses = {
  working: { label: 'Working', tone: 'green' },
  waiting: { label: 'Needs you', tone: 'amber' },
  idle: { label: 'Idle', tone: 'gray' },
  pending: { label: 'Pending', tone: 'gray' },
  stale: { label: 'Stale observation', tone: 'gray' },
  offline: { label: 'Offline', tone: 'gray' },
  ended: { label: 'Ended', tone: 'gray' },
  suspended: { label: 'Suspended', tone: 'gray' },
  retired: { label: 'Retired', tone: 'gray' },
  unobserved: { label: 'Not observed', tone: 'gray' },
} as const
export type AgentStatus = keyof typeof statuses
export const compactTime = ({ at, now }: { readonly at: number; readonly now: number }): string => {
  const seconds = Math.max(0, Math.floor((now - at) / 1000))
  return seconds < 60
    ? `${seconds}s`
    : seconds < 3600
      ? `${Math.floor(seconds / 60)}m`
      : seconds < 86400
        ? `${Math.floor(seconds / 3600)}h`
        : `${Math.floor(seconds / 86400)}d`
}
