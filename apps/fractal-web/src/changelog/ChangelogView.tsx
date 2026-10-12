import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, typeVars as type, geometryVars as geometry, radiusVars as radius, spaceVars as space, elevationVars as elevation } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { lightTheme as assistantLightTheme } from '../../../../packages/fractal-ui/src/assistant-ui/composition-theme.ts'
import { darkTheme as assistantDarkTheme } from '../../../../packages/fractal-ui/src/assistant-ui/embrace-theme.ts'
import type { ChangelogEntry, ChangelogManifest } from './contract.ts'
import { initialSeen, unseenEntries, type ChangelogSeen } from './seen.ts'

export type ChangelogLoad = { readonly _tag: 'Loading' } | { readonly _tag: 'Failed' } | { readonly _tag: 'Ready'; readonly manifest: ChangelogManifest }
export interface ChangelogViewProps {
  readonly state: ChangelogLoad
  readonly baseline?: ChangelogSeen
  readonly onRetry: () => void
  readonly buildRevision?: string
}
const kindLabels = { feature: 'Feature', improvement: 'Improvement', fix: 'Fix' } as const
const Entry = ({ entry }: { readonly entry: ChangelogEntry }) => <li {...stylex.props(styles.entry)}>
  <div {...stylex.props(styles.meta)}><time dateTime={entry.date}>{entry.date}</time>{entry.kind === undefined ? null : <span data-changelog-kind={entry.kind} {...stylex.props(styles.kind)}>{kindLabels[entry.kind]}</span>}{entry.visibility === 'private' ? <span {...stylex.props(styles.badge)}>private</span> : null}</div>
  <h3 {...stylex.props(styles.title)}>{entry.title}</h3>
  <p {...stylex.props(styles.summary)}>{entry.summary}</p>
  {entry.publicPrUrl === undefined ? null : <a href={entry.publicPrUrl} target="_blank" rel="noreferrer" {...stylex.props(styles.link)}>View pull request</a>}
</li>
const Group = ({ title, entries }: { readonly title: string; readonly entries: readonly ChangelogEntry[] }) => <section>
  <h2 {...stylex.props(styles.heading)}>{title}</h2>
  <ol {...stylex.props(styles.list)}>{entries.map((entry) => <Entry key={entry.id} entry={entry} />)}</ol>
</section>
/** Manifest membership, never version or timestamp comparisons, determines the new group. */
export const ChangelogView = ({ state, baseline = initialSeen, onRetry, buildRevision }: ChangelogViewProps) => {
  if (state._tag === 'Loading') return <p role="status" {...stylex.props(styles.notice)}>Loading recent changes…</p>
  if (state._tag === 'Failed') return <div {...stylex.props(styles.notice)}><h2 {...stylex.props(styles.title)}>Changes couldn't be loaded</h2><p>Your workspace is unaffected. Try loading the changelog again.</p><Aria.Button onPress={onRetry} {...stylex.props(styles.button)}>Retry</Aria.Button></div>
  const { entries } = state.manifest
  const build = <p data-changelog-build="" {...stylex.props(styles.build)}>{buildRevision?.slice(0, 7) ?? state.manifest.build.machineVersion}</p>
  if (entries.length === 0) return <>{build}<div {...stylex.props(styles.notice)}><h2 {...stylex.props(styles.title)}>No changes yet</h2><p>This build has no changelog entries.</p></div></>
  if (!baseline.visited) return <>{build}<Group title="Recent changes" entries={entries} /></>
  const fresh = unseenEntries(entries, baseline)
  const newIds = new Set(fresh.map((entry) => entry.id))
  const recent = entries.filter((entry) => !newIds.has(entry.id))
  return <>{build}{fresh.length === 0 ? null : <Group title="New since you last looked" entries={fresh} />}{recent.length === 0 ? null : <Group title="Recent" entries={recent} />}</>
}
export interface ChangelogPopoverProps {
  readonly dark: boolean
  readonly isOpen: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly hasUnseen: boolean
  readonly children: React.ReactNode
}
/** Anchored to the visible status bar; the manifest remains independent of its host. */
export const ChangelogPopover = ({ dark, isOpen, onOpenChange, hasUnseen, children }: ChangelogPopoverProps) => {
  const anchorRef = React.useRef<HTMLElement | null>(null)
  const detachOverflow = React.useRef<(() => void) | undefined>(undefined)
  const triggerRef = React.useCallback((node: HTMLButtonElement | null) => {
    // Use the visible aside, not its zero-height theme holder; native overflow stays nearer.
    anchorRef.current = node?.closest<HTMLElement>('[popover], [data-testid="wf-devbar"] aside, [data-changelog-anchor]') ?? node
    detachOverflow.current?.()
    detachOverflow.current = undefined
    if (node === null) return
    // The native +N popover removes this trigger when light-dismiss closes both disclosures.
    // React Aria cannot restore focus to a removed trigger; restore the surviving outer control.
    const overflow = node.closest<HTMLElement>('[popover]')
    if (overflow === null || overflow.id === '') return
    const document = node.ownerDocument
    const fallback = document.querySelector<HTMLButtonElement>(`button[aria-controls="${CSS.escape(overflow.id)}"]`)
    if (fallback === null) return
    const restore = (event: Event) => {
      if (!('newState' in event) || event.newState !== 'closed') return
      requestAnimationFrame(() => {
        const active = document.activeElement
        // An intentional click into another control keeps that control's focus.
        if (fallback.isConnected && (active === document.body || active === null || !active.isConnected || overflow.contains(active))) fallback.focus()
      })
    }
    // Capture runs before the kit's toggle handler unmounts the trigger and its ref.
    overflow.addEventListener('toggle', restore, true)
    detachOverflow.current = () => overflow.removeEventListener('toggle', restore, true)
    return detachOverflow.current
  }, [])
  return <Aria.DialogTrigger isOpen={isOpen} onOpenChange={onOpenChange}>
    <Aria.Button ref={triggerRef} aria-label="Recent changes" {...stylex.props(styles.trigger)}><span>Changes</span>{hasUnseen ? <span data-testid="changelog-unseen" aria-label="Unread changes" {...stylex.props(styles.dot)} /> : null}</Aria.Button>
    <Aria.Popover data-changelog-popover="" triggerRef={anchorRef} placement="top end" offset={8} {...stylex.props(styles.popover, !dark && styles.popoverLight, dark ? assistantDarkTheme : assistantLightTheme)}>
      <Aria.Dialog aria-label="Recent changes" {...stylex.props(styles.dialog)}>
        <header {...stylex.props(styles.header)}><h1 {...stylex.props(styles.title)}>What's new</h1><Aria.Button slot="close" aria-label="Close recent changes" {...stylex.props(styles.button)}>Close</Aria.Button></header>
        <div {...stylex.props(styles.content)}>{isOpen ? children : null}</div>
      </Aria.Dialog>
    </Aria.Popover>
  </Aria.DialogTrigger>
}
const styles = stylex.create({
  trigger: { display: 'inline-flex', alignItems: 'center', gap: 6, borderWidth: 0, padding: '3px 5px', backgroundColor: 'transparent', color: 'inherit', cursor: 'pointer', fontFamily: 'inherit', fontSize: 'inherit', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: 2 },
  dot: { width: 6, height: 6, borderRadius: '50%', backgroundColor: accent.primary, flexShrink: 0 },
  popover: { width: 'min(420px, calc(100vw - 24px))', backgroundColor: `color-mix(in srgb, ${surface.raised} 80%, transparent)`, color: ink.fg, borderWidth: geometry.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.base, backdropFilter: `blur(${geometry.blur})`, boxShadow: elevation.popover, zIndex: 1100, fontFamily: type.fontSans },
  popoverLight: { borderColor: border.border },
  dialog: { outline: 'none' }, header: { display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', padding: 16, borderBottom: `1px solid ${border.border}` },
  content: { padding: '0 16px', maxHeight: 'min(560px, 65vh)', overflowY: 'auto', overscrollBehavior: 'contain' },
  heading: { fontSize: type.metaSize, color: ink.fgMuted, fontWeight: 500, margin: '18px 0 4px' }, list: { listStyle: 'none', margin: 0, padding: 0 },
  entry: { padding: '14px 0', borderBottom: `1px solid ${border.border}`, ':last-child': { borderBottomWidth: 0 } },
  meta: { display: 'flex', gap: 8, alignItems: 'center', color: ink.fgMuted, fontSize: type.metaSize }, badge: { border: `1px solid ${border.borderStrong}`, borderRadius: 4, padding: '1px 5px', fontSize: type.denseSize },
  kind: { display: 'inline-flex', alignItems: 'center', borderWidth: geometry.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.full, paddingBlock: space.xxxs, paddingInline: space.sm, backgroundColor: surface.controlFill, color: ink.fgSoft, fontSize: type.denseSize, lineHeight: type.metaLeading, fontWeight: type.weightMedium, whiteSpace: 'nowrap' },
  title: { fontSize: type.uiSize, fontWeight: 600, lineHeight: 1.5, margin: '5px 0' }, summary: { fontSize: type.uiSize, color: ink.fgSoft, lineHeight: 1.6, margin: '4px 0 0' },
  build: { fontSize: type.metaSize, color: ink.fgMuted, margin: '4px 0 0', overflowWrap: 'anywhere' },
  notice: { padding: '20px 0', fontSize: type.uiSize, lineHeight: 1.6 }, button: { border: `1px solid ${border.borderStrong}`, backgroundColor: surface.controlFill, color: ink.fg, borderRadius: 5, fontSize: type.metaSize, padding: '4px 8px', cursor: 'pointer', outline: { default: 'none', ':focus-visible': `2px solid ${accent.primary}` }, outlineOffset: 2 },
  link: { display: 'inline-block', marginTop: 8, color: accent.primary, fontSize: type.metaSize },
})
