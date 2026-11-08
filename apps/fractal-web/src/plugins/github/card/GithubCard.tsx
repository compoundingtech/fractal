/**
 * The GitHub hover card: a read-only face of a PR, issue, comment or run, and the trigger that shows it.
 * The card is presentational over one `Loaded` read; `GithubHoverLink` opens it 150ms after the pointer or
 * keyboard focus arrives and closes it 300ms after it leaves (the sidebar's delays), and only then reads.
 * People are logins with an initials disc; no avatar or body image is ever loaded.
 */
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { Focusable, Tooltip, TooltipTrigger } from 'react-aria-components'

import { borderVars as border, elevationVars as elevation, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from '../../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useGithubRead, type GithubReader, type Loaded } from '../data.ts'
import { Labels, Person, ToneChip } from '../detail/DetailShell.tsx'
import { agoWord, staleWord } from '../detail/detail-model.ts'
import { faceOf, plainExcerpt, readProblem, type GithubItem } from '../model.ts'
import { githubUrlOf, labelOf, refOf, type GithubRef } from '../refs.ts'
import type { GithubComment, GithubIssue, GithubPullRequest, GithubRead, GithubRun } from '../schema.ts'

type AnyItem = GithubPullRequest | GithubIssue | GithubComment | GithubRun

const itemOf = (target: GithubRef, item: AnyItem): GithubItem => ({ kind: target.kind, item } as GithubItem)

export interface GithubCardProps {
  readonly target: GithubRef
  readonly loaded: Loaded<AnyItem>
  readonly now: number
}

/** The card body for one read state. */
export function GithubCard({ target, loaded, now }: GithubCardProps) {
  const link = githubUrlOf(target)
  if (loaded._tag === 'Loading') {
    return <div data-github-card="loading" aria-busy="true" {...stylex.props(styles.card)}><span {...stylex.props(styles.handle)}>{labelOf(target)}</span><span {...stylex.props(styles.muted)}>Loading from GitHub…</span></div>
  }
  if (loaded._tag === 'Unsupported') return null
  if (loaded._tag === 'Failed') {
    return <div data-github-card="error" {...stylex.props(styles.card)}><span {...stylex.props(styles.handle)}>{labelOf(target)}</span><span {...stylex.props(styles.title)}>Preview unavailable</span><span {...stylex.props(styles.muted)}>{loaded.message}</span><span {...stylex.props(styles.muted)}>{link}</span></div>
  }
  const read: GithubRead<AnyItem> = loaded.read
  const problem = readProblem(read, target)
  if (read.status !== 'found' || problem !== undefined) {
    return <div data-github-card={read.status} {...stylex.props(styles.card)}><span {...stylex.props(styles.handle)}>{labelOf(target)}</span><span {...stylex.props(styles.title)}>{problem?.title}</span><span {...stylex.props(styles.muted)}>{problem?.detail}</span></div>
  }
  const item = read.item
  const face = faceOf(itemOf(target, item))
  const stale = staleWord(read, now)
  const excerpt = target.kind === 'comment' ? (item as GithubComment).body : target.kind === 'pull-request' || target.kind === 'issue' ? (item as GithubPullRequest | GithubIssue).body : ''
  const labels = 'labels' in item ? item.labels : []
  return <div data-github-card="found" {...stylex.props(styles.card)}>
    <div {...stylex.props(styles.top)}><span {...stylex.props(styles.handle)}>{face.label}</span><ToneChip tone={face.tone}>{face.state}</ToneChip></div>
    <span {...stylex.props(styles.title)}>{face.title}</span>
    {face.author === undefined && face.updatedAt === undefined ? null : <span {...stylex.props(styles.byline)}>{face.author === undefined ? null : <Person login={face.author} />}{face.updatedAt === undefined ? null : <span {...stylex.props(styles.muted)}>updated {agoWord(face.updatedAt, now)}</span>}</span>}
    {excerpt.trim() === '' ? null : <p {...stylex.props(styles.excerpt)}>{plainExcerpt(excerpt)}</p>}
    {face.facts.length === 0 ? null : <span {...stylex.props(styles.muted)}>{face.facts.join(' · ')}</span>}
    <Labels labels={labels} />
    {stale === undefined ? null : <span data-stale {...stylex.props(styles.muted)}>{stale}</span>}
  </div>
}

const readItem = (target: GithubRef) => (reader: GithubReader) => reader.item(target)

/**
 * Wraps one link (or any focusable trigger) with the card. The read starts when the card opens, never on
 * render, so a transcript full of links costs nothing until someone hovers one.
 */
export function GithubHoverLink({ target, now, children }: { readonly target: GithubRef; readonly now?: number; readonly children: React.ReactElement }) {
  const [open, setOpen] = React.useState(false)
  const loaded = useGithubRead(open ? refOf(target) : undefined, readItem(target), open)
  // Without the daemon's plugin reads (before rollout, or no reader here) the link stays a plain link.
  const shown = open && loaded._tag !== 'Unsupported'
  return <TooltipTrigger delay={150} closeDelay={300} isOpen={shown} onOpenChange={setOpen}>
    <Focusable>{children as React.ReactElement<React.DOMAttributes<Element>, string>}</Focusable>
    <Tooltip placement="bottom start" offset={6} {...stylex.props(styles.popover)}>
      <GithubCard target={target} loaded={loaded} now={now ?? Date.now()} />
    </Tooltip>
  </TooltipTrigger>
}

const styles = stylex.create({
  popover: { backgroundColor: surface.raised, color: text.fg, borderWidth: space.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: radius.md, boxShadow: elevation.popover, width: '22rem', maxWidth: 'calc(100vw - 32px)', boxSizing: 'border-box', pointerEvents: 'none', zIndex: 30 },
  card: { display: 'flex', flexDirection: 'column', gap: space.sm, padding: space.lg, fontFamily: type.fontSans, fontSize: type.metaSize, lineHeight: type.metaLeading, color: text.fg, minWidth: 0 },
  top: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.md },
  handle: { color: text.fgMuted, fontFamily: type.fontMono, fontSize: type.denseSize, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  title: { fontSize: type.uiSize, lineHeight: type.uiLeading, fontWeight: type.weightSemibold, overflowWrap: 'anywhere' },
  byline: { display: 'flex', alignItems: 'center', gap: space.md, flexWrap: 'wrap' },
  excerpt: { margin: 0, color: text.fgSoft, display: '-webkit-box', WebkitLineClamp: 3, WebkitBoxOrient: 'vertical', overflow: 'hidden', overflowWrap: 'anywhere' },
  muted: { color: text.fgMuted, overflowWrap: 'anywhere' },
})
