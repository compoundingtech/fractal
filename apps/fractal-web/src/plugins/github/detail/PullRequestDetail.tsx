/**
 * A GitHub pull request as a Fractal tab: header, linked things, and Conversation | Checks | Files like GitHub.
 * Read-only and fixture-friendly: every read arrives as a prop (`undefined` while loading); nothing here fetches.
 */
import * as stylex from '@stylexjs/stylex'

import { mergeStateWord, pullRequestStateWord, pullRequestTone, reviewDecisionWord } from '../model.ts'
import type { GithubRef } from '../refs.ts'
import { githubUrlOf, labelOf, refOf } from '../refs.ts'
import type { GithubChecks, GithubConversation, GithubFiles, GithubPullRequest, GithubRead } from '../schema.ts'
import { CheckList, checkCountsLine } from './Checks.tsx'
import { ConversationPanel, OpeningPost } from './Conversation.tsx'
import { countOf, pullRequestChecksUrl, pullRequestFilesUrl, staleWord, tabLabel } from './detail-model.ts'
import { DetailFrame, DetailHeader, DetailLoading, DetailTabs, detailStyles as d, ExternalLink, LinkedSection, panelProblem, Person, ReadProblem, useSection, type LinkedThing } from './DetailShell.tsx'
import { FilesPanel } from './Files.tsx'

export type PullRequestSection = 'conversation' | 'checks' | 'files'
const sections: readonly PullRequestSection[] = ['conversation', 'checks', 'files']

export interface PullRequestDetailProps {
  /** The PR this tab shows; names it while loading and links problem states to GitHub. */
  readonly target: Extract<GithubRef, { kind: 'pull-request' }>
  /** `undefined` while loading. */
  readonly pullRequest: GithubRead<GithubPullRequest> | undefined
  readonly conversation?: GithubRead<GithubConversation> | undefined
  readonly checks?: GithubRead<GithubChecks> | undefined
  readonly files?: GithubRead<GithubFiles> | undefined
  /** Fractal things tied to this PR (agents and missions from `opened_by` facts), shown first. */
  readonly linked?: readonly LinkedThing[] | undefined
  readonly now: number
  readonly section?: PullRequestSection | undefined
  readonly onSection?: ((section: PullRequestSection) => void) | undefined
  readonly onOpen?: ((ref: string) => void) | undefined
}

export function PullRequestDetail({ target, pullRequest, conversation, checks, files, linked = [], now, section, onSection, onOpen }: PullRequestDetailProps) {
  const [selected, select] = useSection<PullRequestSection>('conversation', section, onSection, sections)
  const label = labelOf(target)
  const url = githubUrlOf(target)
  if (pullRequest === undefined) return <DetailFrame label={`Pull request ${label}`}><DetailLoading what="pull request" target={label} url={url} /></DetailFrame>
  if (pullRequest.status !== 'found') return <DetailFrame label={`Pull request ${label}`}><ReadProblem read={pullRequest} target={target} onOpen={onOpen} /></DetailFrame>
  const pr = pullRequest.item
  const closing: readonly LinkedThing[] = pr.closing_issues.map(issue => ({
    ref: refOf({ kind: 'issue', owner: issue.owner, repo: issue.repo, number: issue.number }),
    label: `${issue.owner === pr.owner && issue.repo === pr.repo ? '' : `${issue.owner}/${issue.repo}`}#${issue.number} ${issue.title}`,
    kind: 'issue',
  }))
  const facts = [
    <Person key="author" login={pr.author?.login} />,
    <span key="branches"><span {...stylex.props(d.mono)}>{pr.head_ref}</span> → <span {...stylex.props(d.mono)}>{pr.base_ref}</span></span>,
    `+${pr.additions} −${pr.deletions}`,
    pr.state === 'open' && !pr.draft ? mergeStateWord(pr.merge_state) : undefined,
    pr.state === 'open' ? reviewDecisionWord(pr.review_decision) : undefined,
  ].filter(fact => fact !== undefined)
  const conversationCount = countOf(conversation, item => item.reviews.length + item.review_threads.length + item.comments.length, pr.comments)
  const conversationProblem = panelProblem(conversation, 'conversation', target)
  const checksProblem = panelProblem(checks, 'checks', target)
  const filesProblem = panelProblem(files, 'files', target)
  const filesUrl = pullRequestFilesUrl(pr)
  return <DetailFrame label={`Pull request ${label}`}>
    <DetailHeader title={pr.title} handle={`#${pr.number}`} state={pullRequestStateWord(pr)} tone={pullRequestTone(pr)} url={pr.url} stale={staleWord(pullRequest, now)} facts={facts} labels={pr.labels} />
    <LinkedSection items={[...linked, ...closing]} onOpen={onOpen} />
    <DetailTabs label="Pull request sections" selected={selected} onSelect={select} tabs={[
      {
        id: 'conversation',
        label: tabLabel('Conversation', conversationCount),
        panel: <ConversationPanel
          opening={<OpeningPost author={pr.author} verb="opened" at={pr.created_at} body={pr.body} url={pr.url} now={now} />}
          conversation={conversation?.status === 'found' ? conversation.item : undefined}
          problem={conversationProblem}
          moreUrl={pr.url}
          now={now}
        />,
      },
      {
        id: 'checks',
        label: tabLabel('Checks', countOf(checks, item => item.runs.length)),
        panel: checksProblem ?? (checks?.status === 'found' ? <div {...stylex.props(d.stack)}>
          <p data-check-counts {...stylex.props(d.summaryLine)}><span>{checkCountsLine(checks.item.runs)}</span><span>head <span {...stylex.props(d.mono)}>{checks.item.head_sha.slice(0, 7)}</span></span>{staleWord(checks, now) === undefined ? null : <span>{staleWord(checks, now)}</span>}</p>
          <CheckList label="Checks" runs={checks.item.runs} now={now} />
          {checks.item.truncated ? <p {...stylex.props(d.summaryLine)}>GitHub has more checks. <ExternalLink href={pullRequestChecksUrl(pr)}>All checks on GitHub</ExternalLink></p> : null}
        </div> : null),
      },
      {
        id: 'files',
        label: tabLabel('Files', countOf(files, item => item.total, pr.changed_files)),
        fill: filesProblem === undefined,
        panel: filesProblem ?? (files?.status === 'found' ? <FilesPanel files={files.item} filesUrl={filesUrl} landmark={label} /> : null),
      },
    ]} />
  </DetailFrame>
}
