import { BuildIdentitySegment } from '@overeng/devbar/build-identity'
import { formatBuildIdentityDetails, formatRelativeTime, type BuildIdentity } from '@overeng/utils/build-identity'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { borderVars as border, geometryVars as geometry, spaceVars as space, textVars as ink, typeVars as type } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
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
  /** Runs when the popover opens; the popover always shows the changes list, so the host acknowledges here. */
  readonly onOpen: () => void
  /** Intent (hover or focus) without opening; never acknowledges. */
  readonly onPrefetch: () => void
  /** Host-owned recent-changes content, mounted only while open. */
  readonly changes: () => React.ReactNode
}

export const buildChangesLabel = 'Build and changes'

const detailLabels = { displayVersion: 'Version', machineVersion: 'Machine version', deployed: 'Deployed' } as const

/** One dev-bar trigger (short identity + unseen dot) and one popover holding build identity and recent changes. */
export const BuildChangesSegment = ({ identity, deploymentId, now, dark, hasUnseen, onOpen, onPrefetch, changes }: BuildChangesSegmentProps) => (
  <span onPointerEnter={onPrefetch} onFocus={onPrefetch} {...stylex.props(styles.host)}>
    <BuildIdentitySegment
      identity={identity}
      now={now}
      disclosure={{
        label: buildChangesLabel,
        hasUnseen,
        onOpenChange: (open) => { if (open) onOpen() },
        render: () => <BuildChangesPanel identity={identity} deploymentId={deploymentId} now={now} dark={dark}>{changes()}</BuildChangesPanel>,
      }}
    />
  </span>
)

const BuildChangesPanel = ({ identity, deploymentId, now, dark, children }: {
  readonly identity: BuildChangesIdentity
  readonly deploymentId: string | undefined
  readonly now: number | undefined
  readonly dark: boolean
  readonly children: React.ReactNode
}) => {
  // The panel mounts on open, so an unpinned clock reads the time the popover was opened.
  const [openedAt] = React.useState(() => now ?? Math.floor(Date.now() / 1000))
  const at = now ?? openedAt
  const buildHeading = React.useId()
  const changesHeading = React.useId()
  const revision = identity.rev === undefined || identity.rev === '' ? undefined : identity.rev
  const rows: readonly { readonly label: string; readonly value: React.ReactNode }[] = [
    {
      label: 'Revision',
      value: revision === undefined
        ? <span {...stylex.props(styles.muted)}>unknown</span>
        : <>{revision}{identity.dirty ? <>* <span data-build-dirty="" {...stylex.props(styles.muted)}>uncommitted changes</span></> : null}</>,
    },
    { label: 'Committed', value: identity.commitTs === undefined ? <span {...stylex.props(styles.muted)}>unknown</span> : formatRelativeTime({ ts: identity.commitTs, now: at, format: 'long' }) },
    ...(identity.sourceKind === undefined ? [] : [{ label: 'Source', value: identity.sourceKind }]),
    ...formatBuildIdentityDetails({ identity, now: at }).map((line) => ({ label: detailLabels[line.label], value: line.value })),
    ...(deploymentId === undefined ? [] : [{ label: 'Deployment', value: deploymentId }]),
  ]
  return (
    <div data-devbar-build-changes="" {...stylex.props(dark ? assistantDarkTheme : assistantLightTheme, styles.panel)}>
      <section aria-labelledby={buildHeading}>
        <h1 id={buildHeading} {...stylex.props(styles.heading)}>Build</h1>
        <dl {...stylex.props(styles.details)}>
          {rows.map((row) => (
            <React.Fragment key={row.label}>
              <dt {...stylex.props(styles.term)}>{row.label}</dt>
              <dd {...stylex.props(styles.value)}>{row.value}</dd>
            </React.Fragment>
          ))}
        </dl>
      </section>
      <section aria-labelledby={changesHeading} {...stylex.props(styles.changes)}>
        <h1 id={changesHeading} {...stylex.props(styles.heading)}>What's new</h1>
        {children}
      </section>
    </div>
  )
}

const styles = stylex.create({
  host: { display: 'inline-flex', alignItems: 'center', minWidth: 0 },
  panel: { width: `min(${geometry.changelogMax}, calc(100vw - 2.5rem))`, paddingInline: space.sm, color: ink.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading, whiteSpace: 'normal' },
  heading: { fontSize: type.uiSize, fontWeight: type.weightSemibold, lineHeight: type.uiLeading, marginTop: space.md, marginBottom: space.sm, marginInline: 0 },
  details: { display: 'grid', gridTemplateColumns: 'max-content minmax(0, 1fr)', columnGap: space.lg, rowGap: space.xs, margin: 0, fontSize: type.metaSize, lineHeight: type.metaLeading },
  term: { color: ink.fgMuted },
  value: { margin: 0, fontFamily: type.fontMono, overflowWrap: 'anywhere', userSelect: 'text' },
  muted: { color: ink.fgMuted, fontFamily: type.fontSans },
  changes: { marginTop: space.lg, borderTop: `${geometry.hairline} solid ${border.border}` },
})
