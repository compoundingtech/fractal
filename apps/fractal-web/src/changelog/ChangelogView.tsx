import type * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, typeVars as type, geometryVars as geometry, radiusVars as radius, spaceVars as space } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { ChangelogEntry, ChangelogManifest } from './contract.ts'
import { initialSeen, unseenEntries, type ChangelogSeen } from './seen.ts'

export type ChangelogLoad = { readonly _tag: 'Loading' } | { readonly _tag: 'Failed' } | { readonly _tag: 'Ready'; readonly manifest: ChangelogManifest }
export interface ChangelogViewProps {
  readonly state: ChangelogLoad
  readonly baseline?: ChangelogSeen
  readonly onRetry: () => void
  /** Receives the first readable item: the first entry's title, or the empty notice's title. Loading and failure have none. */
  readonly firstEntryRef?: React.Ref<HTMLHeadingElement> | undefined
}
const kindLabels = { feature: 'Feature', improvement: 'Improvement', fix: 'Fix' } as const
const Entry = ({ entry, titleRef }: { readonly entry: ChangelogEntry; readonly titleRef: React.Ref<HTMLHeadingElement> | undefined }) => <li {...stylex.props(styles.entry)}>
  <div {...stylex.props(styles.meta)}><time dateTime={entry.date}>{entry.date}</time>{entry.kind === undefined ? null : <span data-changelog-kind={entry.kind} {...stylex.props(styles.kind)}>{kindLabels[entry.kind]}</span>}{entry.visibility === 'private' ? <span {...stylex.props(styles.badge)}>private</span> : null}</div>
  <h3 ref={titleRef} {...stylex.props(styles.title)}>{entry.title}</h3>
  <p {...stylex.props(styles.summary)}>{entry.summary}</p>
  {entry.publicPrUrl === undefined ? null : <a href={entry.publicPrUrl} target="_blank" rel="noreferrer" {...stylex.props(styles.link)}>View pull request</a>}
</li>
const Group = ({ title, entries, firstEntryRef }: { readonly title: string; readonly entries: readonly ChangelogEntry[]; readonly firstEntryRef: React.Ref<HTMLHeadingElement> | undefined }) => <section>
  <h2 {...stylex.props(styles.heading)}>{title}</h2>
  <ol {...stylex.props(styles.list)}>{entries.map((entry, index) => <Entry key={entry.id} entry={entry} titleRef={index === 0 ? firstEntryRef : undefined} />)}</ol>
</section>
/** Manifest membership, never version or timestamp comparisons, determines the new group; the host shows build identity. */
export const ChangelogView = ({ state, baseline = initialSeen, onRetry, firstEntryRef }: ChangelogViewProps) => {
  if (state._tag === 'Loading') return <div role="status" {...stylex.props(styles.notice)}>Loading recent changes…</div>
  // Retry replaces its own control; keep focus in the hosting dialog so Escape still reaches it.
  if (state._tag === 'Failed') return <div {...stylex.props(styles.notice)}><h2 {...stylex.props(styles.title)}>Changes couldn't be loaded</h2><p>Your workspace is unaffected. Try loading the changelog again.</p><Aria.Button onPress={(event) => { (event.target as HTMLElement).closest<HTMLElement>('[role="dialog"]')?.focus(); onRetry() }} {...stylex.props(styles.button)}>Retry</Aria.Button></div>
  const { entries } = state.manifest
  if (entries.length === 0) return <div {...stylex.props(styles.notice)}><h2 ref={firstEntryRef} {...stylex.props(styles.title)}>No changes yet</h2><p>This build has no changelog entries.</p></div>
  if (!baseline.visited) return <Group title="Recent changes" entries={entries} firstEntryRef={firstEntryRef} />
  const fresh = unseenEntries(entries, baseline)
  const newIds = new Set(fresh.map((entry) => entry.id))
  const recent = entries.filter((entry) => !newIds.has(entry.id))
  return <>{fresh.length === 0 ? null : <Group title="New since you last looked" entries={fresh} firstEntryRef={firstEntryRef} />}{recent.length === 0 ? null : <Group title="Recent" entries={recent} firstEntryRef={fresh.length === 0 ? firstEntryRef : undefined} />}</>
}
const styles = stylex.create({
  heading: { fontSize: type.metaSize, color: ink.fgMuted, fontWeight: type.weightMedium, marginTop: space.xxl, marginInline: 0, marginBottom: space.xs }, list: { listStyle: 'none', margin: 0, padding: 0 },
  entry: { paddingBlock: space.xl, paddingInline: 0, borderBottom: `${geometry.hairline} solid ${border.border}`, ':last-child': { borderBottomWidth: 0 } },
  meta: { display: 'flex', gap: space.md, alignItems: 'center', color: ink.fgMuted, fontSize: type.metaSize }, badge: { border: `${geometry.hairline} solid ${border.borderStrong}`, borderRadius: radius.keyboard, paddingBlock: space.xxxs, paddingInline: space.sm, fontSize: type.denseSize },
  kind: { display: 'inline-flex', alignItems: 'center', borderWidth: geometry.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.full, paddingBlock: space.xxxs, paddingInline: space.sm, backgroundColor: surface.controlFill, color: ink.fgSoft, fontSize: type.denseSize, lineHeight: type.metaLeading, fontWeight: type.weightMedium, whiteSpace: 'nowrap' },
  title: { fontSize: type.uiSize, fontWeight: type.weightSemibold, lineHeight: type.uiLeading, marginBlock: space.sm, marginInline: 0 }, summary: { fontSize: type.uiSize, color: ink.fgSoft, lineHeight: type.bodyLeading, marginTop: space.xs, marginInline: 0, marginBottom: 0 },
  notice: { paddingBlock: space.xxl, paddingInline: 0, fontSize: type.uiSize, lineHeight: type.bodyLeading }, button: { border: `${geometry.hairline} solid ${border.borderStrong}`, backgroundColor: surface.controlFill, color: ink.fg, borderRadius: radius.sm, fontSize: type.metaSize, paddingBlock: space.xs, paddingInline: space.md, cursor: 'pointer', outline: { default: 'none', ':focus-visible': `${geometry.focusRing} solid ${accent.primary}` }, outlineOffset: geometry.focusRing },
  link: { display: 'inline-block', marginTop: space.md, color: ink.fg, fontSize: type.metaSize, textDecoration: 'underline' },
})
