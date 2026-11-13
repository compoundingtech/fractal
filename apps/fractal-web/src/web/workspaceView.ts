import { isMissionRef } from '../missions/missionTitle.ts'

/**
 * What the workspace body shows for the selected pane. Supported subjects and app screens have web surfaces;
 * an unknown pane states its own unavailability instead of repeating the transcript under a different tab.
 */
export type WorkspaceView =
  | { readonly _tag: 'Thread' }
  | { readonly _tag: 'Terminal'; readonly ref: TerminalRef }
  | { readonly _tag: 'Missions'; readonly mission?: string }
  | { readonly _tag: 'Usage' }
  | { readonly _tag: 'Alert'; readonly id: string }
  | { readonly _tag: 'TopicThread'; readonly ref: string }
  | { readonly _tag: 'Document'; readonly ref: `doc/${string}` }
  | { readonly _tag: 'ViewUnavailable'; readonly ref: string; readonly reason: string }

/** A terminal subject address, as `terminalSubjectForAgent` and the fleet projection produce it. */
export type TerminalRef = `terminal/${string}`
export const isTerminalRef = (ref: string): ref is TerminalRef => ref.startsWith('terminal/')
/** The Missions screen's tab, opened from the sidebar. */
export const MISSIONS_REF = 'missions'
const isDocumentRef = (ref: string): ref is `doc/${string}` => ref.startsWith('doc/')

/**
 * An alert's details view is reached by its URL alone (`?open=alert/<id>:detail`): a passing view over a list that
 * empties as it is worked through, so it never accumulates in the workspace's tabs.
 */
const ALERT_PREFIX = 'alert/'
export const isAlertRef = (ref: string): boolean => ref.startsWith(ALERT_PREFIX)
export const alertPaneId = (id: string): string => `${ALERT_PREFIX}${id}:detail`
/** The alert a selected pane id shows, if it shows one. */
export const alertIdOfPane = (pane: string): string | undefined =>
  isAlertRef(pane) && pane.endsWith(':detail') ? pane.slice(ALERT_PREFIX.length, -':detail'.length) : undefined

export const workspaceView = (pane: { readonly ref: string } | undefined): WorkspaceView =>
  pane === undefined
    ? { _tag: 'Thread' }
    : isTerminalRef(pane.ref)
      ? { _tag: 'Terminal', ref: pane.ref }
      : pane.ref === MISSIONS_REF
        ? { _tag: 'Missions' }
      : isMissionRef(pane.ref)
        ? { _tag: 'Missions', mission: pane.ref }
      : isAlertRef(pane.ref)
        ? { _tag: 'Alert', id: pane.ref.slice(ALERT_PREFIX.length) }
      : pane.ref === 'monitor/quota'
        ? { _tag: 'Usage' }
        : /^thread\/[a-zA-Z0-9_-]+$/.test(pane.ref)
          ? { _tag: 'TopicThread', ref: pane.ref }
        : isDocumentRef(pane.ref)
          ? { _tag: 'Document', ref: pane.ref }
        : { _tag: 'ViewUnavailable', ref: pane.ref, reason: 'This web client does not render this view yet.' }

// App screens opened from the sidebar footer: their tabs read as the buttons that opened them.
const SCREEN_LABELS: Readonly<Record<string, string | undefined>> = { [MISSIONS_REF]: 'Missions', 'monitor/quota': 'Usage' }

/**
 * A workspace tab's label: the subject's own title verbatim (agent names keep their case), else the
 * screen's name, else the ref's last segment in sentence case like "Thread".
 */
export const paneTabLabel = (ref: string, title: string | undefined): string => {
  if (isMissionRef(ref)) return 'Missions'
  if (title !== undefined) return title
  if (ref.startsWith('thread/')) return 'Thread'
  const label = SCREEN_LABELS[ref] ?? ref.split('/').at(-1) ?? ref
  return label.charAt(0).toUpperCase() + label.slice(1)
}

/** A conversation pane kept mounted after it was opened; `used` is its logical recency. */
export interface RetainedPane {
  readonly ref: string
  readonly name: string
  readonly used: number
}

/** Memory-only LRU over opened panes. Array order is insertion order and never changes for a
 * retained pane: moving a mounted subtree in the DOM discards its rendering state. */
export const retainPane = ({ panes, ref, name, limit }: {
  readonly panes: readonly RetainedPane[]
  readonly ref: string
  readonly name: string
  readonly limit: number
}): readonly RetainedPane[] => {
  const used = panes.reduce((latest, pane) => Math.max(latest, pane.used), 0) + 1
  if (panes.some(pane => pane.ref === ref)) return panes.map(pane => pane.ref === ref ? { ref, name, used } : pane)
  const next = [...panes, { ref, name, used }]
  if (next.length <= limit) return next
  const evicted = next.reduce((oldest, pane) => pane.used < oldest.used ? pane : oldest)
  return next.filter(pane => pane !== evicted)
}

/** All in-app workspace navigation keeps the dev replay signal visible for reload/early connect. */
export const workspaceHref = (ref: string, pane: string, search: string): string => {
  const path = `/w/${ref.split('/').map(encodeURIComponent).join('/')}?open=${encodeURIComponent(pane)}`
  if (import.meta.env.DEV) {
    const params = new URLSearchParams(search)
    const scenario = params.get('scenario')
    const attention = params.get('attention')
    if (scenario !== null) return `${path}&scenario=${encodeURIComponent(scenario)}${attention === null ? '' : `&attention=${encodeURIComponent(attention)}`}`
  }
  return path
}
