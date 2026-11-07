/** A GitHub issue in the pull request's shell: header, linked things, and one Conversation subtab. */
import * as stylex from '@stylexjs/stylex'

import { faceOf } from '../model.ts'
import type { GithubRef } from '../refs.ts'
import { githubUrlOf, labelOf, refOf } from '../refs.ts'
import type { GithubComment, GithubIssue, GithubRead } from '../schema.ts'
import { CommentCard, OpeningPost } from './Conversation.tsx'
import { staleWord, tabLabel } from './detail-model.ts'
import { DetailFrame, DetailHeader, DetailLoading, DetailTabs, detailStyles as d, ExternalLink, LinkedSection, Person, ReadProblem, type LinkedThing } from './DetailShell.tsx'

export interface IssueDetailProps {
  readonly target: Extract<GithubRef, { kind: 'issue' }>
  /** `undefined` while loading. */
  readonly issue: GithubRead<GithubIssue> | undefined
  /** The issue's comments, oldest first; `undefined` when not read (the tab links out to them). */
  readonly comments?: readonly GithubComment[] | undefined
  readonly linked?: readonly LinkedThing[] | undefined
  readonly now: number
  readonly onOpen?: ((ref: string) => void) | undefined
}

export function IssueDetail({ target, issue, comments, linked = [], now, onOpen }: IssueDetailProps) {
  const label = labelOf(target)
  if (issue === undefined) return <DetailFrame label={`Issue ${label}`}><DetailLoading what="issue" target={label} url={githubUrlOf(target)} /></DetailFrame>
  if (issue.status !== 'found') return <DetailFrame label={`Issue ${label}`}><ReadProblem read={issue} target={target} onOpen={onOpen} /></DetailFrame>
  const item = issue.item
  const face = faceOf({ kind: 'issue', item })
  // GitHub's issues API also answers for PRs; offer the PR kind instead of a thinner issue view.
  const asPullRequest: readonly LinkedThing[] = item.is_pull_request ? [{ ref: refOf({ kind: 'pull-request', owner: item.owner, repo: item.repo, number: item.number }), label: `#${item.number} as pull request`, kind: 'pull-request' }] : []
  const facts = [<Person key="author" login={item.author?.login} />, ...face.facts]
  return <DetailFrame label={`Issue ${label}`}>
    <DetailHeader title={item.title} handle={`#${item.number}`} state={face.state} tone={face.tone} url={item.url} stale={staleWord(issue, now)} facts={facts} labels={item.labels} />
    <LinkedSection items={[...linked, ...asPullRequest]} onOpen={onOpen} />
    <DetailTabs label="Issue sections" selected="conversation" onSelect={() => {}} tabs={[{
      id: 'conversation',
      label: tabLabel('Conversation', comments?.length ?? item.comments),
      panel: <div {...stylex.props(d.list)}>
        <OpeningPost author={item.author} verb="opened" at={item.created_at} body={item.body} url={item.url} now={now} />
        {comments === undefined
          ? item.comments === 0 ? null : <p {...stylex.props(d.summaryLine)}>{item.comments} {item.comments === 1 ? 'comment' : 'comments'}. <ExternalLink href={item.url}>Read on GitHub</ExternalLink></p>
          : <ol aria-label="Comments" {...stylex.props(d.list)}>{comments.map(comment => <li key={comment.id}><CommentCard comment={comment} now={now} /></li>)}</ol>}
        {comments !== undefined && comments.length < item.comments ? <p {...stylex.props(d.summaryLine)}>GitHub has more comments. <ExternalLink href={item.url}>More on GitHub</ExternalLink></p> : null}
      </div>,
    }]} />
  </DetailFrame>
}
