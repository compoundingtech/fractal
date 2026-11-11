import { BuildIdentitySegment } from '@overeng/devbar/build-identity'
import { formatBuildIdentityDetails, formatRelativeTime, type BuildIdentity } from '@overeng/utils/build-identity'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { accentVars as accent, borderVars as border, geometryVars as geometry, spaceVars as space, textVars as ink, typeVars as type } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { lightTheme as assistantLightTheme } from '../../../../packages/fractal-ui/src/assistant-ui/composition-theme.ts'
import { darkTheme as assistantDarkTheme } from '../../../../packages/fractal-ui/src/assistant-ui/embrace-theme.ts'

/** Canonical identity plus the optional producer kind the app's virtual module adds. */
export type BuildChangesIdentity = BuildIdentity & { readonly sourceKind?: string | undefined }

export interface BuildChangesSegmentProps {
  readonly identity: BuildChangesIdentity
  readonly deploymentId: string | undefined
  /** Fixed Unix seconds for deterministic rendering; otherwise the kit's minute clock and the open time. */
  readonly now?: number | undefined
  readonly dark: boolean
  readonly hasUnseen: boolean
  /**
   * Runs at most once per opening, when the first readable changes item is wholly inside the
   * popover's visible area. The popover's height is bounded by the viewport, so opening alone never
   * proves the list was seen. Loading and failure states render no such item.
   */
  readonly onChangesSeen: () => void
  /** Intent (hover or focus) without opening; never acknowledges. */
  readonly onPrefetch: () => void
  /** Host-owned recent-changes content, mounted only while open; attach `firstEntryRef` to its first readable item. */
  readonly changes: (firstEntryRef: React.RefCallback<HTMLHeadingElement>) => React.ReactNode
}

export const buildChangesLabel = 'Build and changes'

const detailLabels = { displayVersion: 'Version', machineVersion: 'Machine version', deployed: 'Deployed' } as const
/** Wholly visible, tolerant of subpixel rounding at the scroll edge. */
const visibleRatio = 0.99

/** One dev-bar trigger (short identity + unseen dot) and one popover holding build identity and recent changes. */
export const BuildChangesSegment = ({ identity, deploymentId, now, dark, hasUnseen, onChangesSeen, onPrefetch, changes }: BuildChangesSegmentProps) => (
  <span onPointerEnter={onPrefetch} onFocus={onPrefetch} {...stylex.props(styles.host)}>
    <BuildIdentitySegment
      identity={identity}
      now={now}
      disclosure={{
        label: buildChangesLabel,
        hasUnseen,
        render: () => <BuildChangesPanel identity={identity} deploymentId={deploymentId} now={now} dark={dark} onChangesSeen={onChangesSeen} changes={changes} />,
      }}
    />
  </span>
)

/**
 * Calls `onSeen` once, the first time the referenced element is wholly visible. The observer uses
 * the viewport as root, so the popover's own scroll clipping counts: an item below the fold of a
 * short popover stays unseen until it is scrolled into view.
 */
const useSeenOnceVisible = (onSeen: () => void): React.RefCallback<HTMLElement> => {
  const callback = React.useRef(onSeen)
  callback.current = onSeen
  const seen = React.useRef(false)
  return React.useCallback((node: HTMLElement | null) => {
    if (node === null || seen.current) return
    const observer = new IntersectionObserver((records) => {
      if (seen.current || !records.some((record) => record.isIntersecting && record.intersectionRatio >= visibleRatio)) return
      seen.current = true
      observer.disconnect()
      callback.current()
    }, { threshold: visibleRatio })
    observer.observe(node)
    return () => observer.disconnect()
  }, [])
}

/** Mounted only while open: one compact build line (details on demand) keeps the first change near the top. */
const BuildChangesPanel = ({ identity, deploymentId, now, dark, onChangesSeen, changes }: {
  readonly identity: BuildChangesIdentity
  readonly deploymentId: string | undefined
  readonly now: number | undefined
  readonly dark: boolean
  readonly onChangesSeen: () => void
  readonly changes: (firstEntryRef: React.RefCallback<HTMLHeadingElement>) => React.ReactNode
}) => {
  // The panel mounts on open, so an unpinned clock reads the time the popover was opened.
  const [openedAt] = React.useState(() => now ?? Math.floor(Date.now() / 1000))
  const at = now ?? openedAt
  const changesHeading = React.useId()
  const firstEntryRef = useSeenOnceVisible(onChangesSeen)
  const revision = identity.rev === undefined || identity.rev === '' ? undefined : identity.rev
  const committed = identity.commitTs === undefined ? undefined : formatRelativeTime({ ts: identity.commitTs, now: at, format: 'long' })
  const unknown = <span {...stylex.props(styles.muted)}>unknown</span>
  const rows: readonly { readonly label: string; readonly value: React.ReactNode }[] = [
    {
      label: 'Revision',
      value: revision === undefined ? unknown : <>{revision}{identity.dirty ? <>* <span {...stylex.props(styles.muted)}>uncommitted changes</span></> : null}</>,
    },
    { label: 'Committed', value: committed ?? unknown },
    ...(identity.sourceKind === undefined ? [] : [{ label: 'Source', value: identity.sourceKind }]),
    ...formatBuildIdentityDetails({ identity, now: at }).map((line) => ({ label: detailLabels[line.label], value: line.value })),
    ...(deploymentId === undefined ? [] : [{ label: 'Deployment', value: deploymentId }]),
  ]
  return (
    <div data-devbar-build-changes="" {...stylex.props(dark ? assistantDarkTheme : assistantLightTheme, styles.panel)}>
      <details data-build-details="" {...stylex.props(styles.build)}>
        <summary data-build-summary="" {...stylex.props(styles.summary)}>
          <span {...stylex.props(styles.label)}>Build</span>{' '}
          {revision === undefined
            ? <span {...stylex.props(styles.muted)}>revision unknown</span>
            : <span {...stylex.props(styles.mono)}>{revision.slice(0, 7)}{identity.dirty ? '*' : ''}</span>}
          {committed === undefined ? null : <> · {committed}</>}
          {revision !== undefined && identity.dirty ? <> · <span data-build-dirty="" {...stylex.props(styles.muted)}>uncommitted changes</span></> : null}
        </summary>
        <dl {...stylex.props(styles.details)}>
          {rows.map((row) => (
            <React.Fragment key={row.label}>
              <dt {...stylex.props(styles.term)}>{row.label}</dt>
              <dd {...stylex.props(styles.value)}>{row.value}</dd>
            </React.Fragment>
          ))}
        </dl>
      </details>
      <section aria-labelledby={changesHeading} {...stylex.props(styles.changes)}>
        <h1 id={changesHeading} {...stylex.props(styles.heading)}>What's new</h1>
        {changes(firstEntryRef)}
      </section>
    </div>
  )
}

const styles = stylex.create({
  host: { display: 'inline-flex', alignItems: 'center', minWidth: 0 },
  panel: { width: `min(${geometry.changelogMax}, calc(100vw - 2.5rem))`, paddingInline: space.sm, color: ink.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading, whiteSpace: 'normal' },
  build: { marginBlock: space.xs, fontSize: type.metaSize, lineHeight: type.metaLeading },
  label: { fontWeight: type.weightSemibold },
  mono: { fontFamily: type.fontMono, userSelect: 'text' },
  heading: { fontSize: type.uiSize, fontWeight: type.weightSemibold, lineHeight: type.uiLeading, marginTop: space.md, marginBottom: 0, marginInline: 0 },
  details: { display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', columnGap: space.lg, rowGap: space.xs, marginTop: space.sm, marginBottom: 0, marginInline: 0 },
  summary: { cursor: 'pointer', overflowWrap: 'anywhere', outline: { default: 'none', ':focus-visible': `${geometry.focusRing} solid ${accent.primary}` }, outlineOffset: geometry.focusRing },
  term: { color: ink.fgMuted },
  value: { margin: 0, fontFamily: type.fontMono, overflowWrap: 'anywhere', userSelect: 'text' },
  muted: { color: ink.fgMuted, fontFamily: type.fontSans },
  changes: { marginTop: space.sm, borderTop: `${geometry.hairline} solid ${border.border}` },
})
