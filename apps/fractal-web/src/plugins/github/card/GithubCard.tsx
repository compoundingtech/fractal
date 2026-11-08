/**
 * The GitHub hover card: a read-only face of a PR, issue, comment or run, drawn as the kit's resource card, and the
 * trigger that shows it. The card is presentational over one `Loaded` read. `GithubHoverLink` opens it through the
 * kit `HoverCard` (150ms after the pointer or keyboard focus arrives, 300ms grace on leaving) and only then reads;
 * a daemon without the plugin reads keeps the card shut, so the link stays a plain link.
 * People are logins with an initials disc; no avatar or body image is ever loaded.
 */
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { spaceVars as space, textVars as text } from '../../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { HoverCard } from '../../../../../../packages/fractal-ui/src/assistant-ui/entity/HoverCard.tsx'
import { ResourceEntityCard } from '../../../../../../packages/fractal-ui/src/assistant-ui/entity/ResourceCard.tsx'
import type { EntityStatusTone } from '../../../../../../packages/fractal-ui/src/assistant-ui/entity/entity.ts'
import type { IconName } from '../../../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { useGithubRead, type GithubReader, type Loaded } from '../data.ts'
import { Labels, Person } from '../detail/DetailShell.tsx'
import { agoWord, staleWord } from '../detail/detail-model.ts'
import { faceOf, plainExcerpt, readProblem, type GithubItem, type Tone } from '../model.ts'
import { githubUrlOf, labelOf, refOf, type GithubRef } from '../refs.ts'
import type { GithubComment, GithubIssue, GithubPullRequest, GithubRead, GithubRun } from '../schema.ts'

type AnyItem = GithubPullRequest | GithubIssue | GithubComment | GithubRun

const itemOf = (target: GithubRef, item: AnyItem): GithubItem => ({ kind: target.kind, item } as GithubItem)

const kindWord: Record<GithubRef['kind'], string> = { 'pull-request': 'Pull request', issue: 'Issue', comment: 'Comment', run: 'Run' }
const kindIcon: Record<GithubRef['kind'], IconName> = { 'pull-request': 'pull-request', issue: 'dot', comment: 'message', run: 'check' }
/** GitHub's state colours fold into the kit's: only a failure draws colour (red); open work reads as working. */
const toneOf: Record<Tone, EntityStatusTone> = { open: 'working', draft: 'neutral', pending: 'working', merged: 'done', closed: 'done', success: 'done', failure: 'failed', neutral: 'neutral' }

export interface GithubCardProps {
  readonly target: GithubRef
  readonly loaded: Loaded<AnyItem>
  readonly now: number
}

/** The card body for one read state, in the kit's resource card grammar. */
export function GithubCard({ target, loaded, now }: GithubCardProps) {
  const link = githubUrlOf(target)
  const common = { id: refOf(target), kind: kindWord[target.kind], icon: kindIcon[target.kind] }
  if (loaded._tag === 'Unsupported') return null
  if (loaded._tag === 'Loading') {
    return <div data-github-card="loading" aria-busy="true"><ResourceEntityCard {...common} title={labelOf(target)} text="Loading from GitHub…" /></div>
  }
  if (loaded._tag === 'Failed') {
    return <div data-github-card="error"><ResourceEntityCard {...common} title="Preview unavailable" summary={[labelOf(target)]} text={loaded.message} note={link} /></div>
  }
  const read: GithubRead<AnyItem> = loaded.read
  const problem = readProblem(read, target)
  if (read.status !== 'found' || problem !== undefined) {
    return <div data-github-card={read.status}><ResourceEntityCard {...common} title={problem?.title ?? labelOf(target)} summary={[labelOf(target)]} {...(problem?.detail === undefined ? {} : { text: problem.detail })} /></div>
  }
  const item = read.item
  const face = faceOf(itemOf(target, item))
  const stale = staleWord(read, now)
  const excerpt = target.kind === 'comment' ? (item as GithubComment).body : target.kind === 'pull-request' || target.kind === 'issue' ? (item as GithubPullRequest | GithubIssue).body : ''
  const labels = 'labels' in item ? item.labels : []
  return <div data-github-card="found">
    <ResourceEntityCard {...common} title={face.title} status={{ tone: toneOf[face.tone], label: face.state }} summary={[face.label]}
      {...(excerpt.trim() === '' ? {} : { text: plainExcerpt(excerpt) })} {...(stale === undefined ? {} : { note: <span data-stale>{stale}</span> })}>
      {face.author === undefined && face.updatedAt === undefined ? null : <span {...stylex.props(styles.byline)}>{face.author === undefined ? null : <Person login={face.author} />}{face.updatedAt === undefined ? null : <span {...stylex.props(styles.muted)}>updated {agoWord(face.updatedAt, now)}</span>}</span>}
      {face.facts.length === 0 ? null : <span {...stylex.props(styles.muted)}>{face.facts.join(' · ')}</span>}
      <Labels labels={labels} />
    </ResourceEntityCard>
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
  // Without the daemon's plugin reads (before rollout, or no reader here) the card stays shut: just the link.
  return <HoverCard placement="bottom start" onOpenChange={setOpen} content={() => loaded._tag === 'Unsupported' ? null : <GithubCard target={target} loaded={loaded} now={now ?? Date.now()} />}>{children}</HoverCard>
}

/** The card for a GitHub subject ref, for an `EntityCardSource`: mounted only while a card shows, so it reads then. */
export function GithubEntityCard({ subjectRef, target, now }: { readonly subjectRef: string; readonly target: GithubRef; readonly now?: number }) {
  const loaded = useGithubRead(subjectRef, readItem(target), true)
  return <GithubCard target={target} loaded={loaded} now={now ?? Date.now()} />
}

const styles = stylex.create({
  byline: { display: 'flex', alignItems: 'center', gap: space.md, flexWrap: 'wrap' },
  muted: { color: text.fgMuted, overflowWrap: 'anywhere' },
})
