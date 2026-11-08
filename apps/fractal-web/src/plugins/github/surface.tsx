/**
 * The tab body for any GitHub kind: reads through the host's `GithubReader` and hands the answers to the
 * presentational detail views. The PR tab reads its conversation, checks and files only once their
 * subtab is shown.
 */
import * as React from 'react'

import { useGithubRead, type GithubReader, type Loaded } from './data.ts'
import { ReadProblem } from './detail/DetailShell.tsx'
import { IssueDetail, PullRequestDetail, RunDetail, type PullRequestSection } from './detail/index.ts'
import { refOf, type GithubRef } from './refs.ts'
import type { GithubComment, GithubIssue, GithubPullRequest, GithubRead, GithubRun } from './schema.ts'

const asRead = <T,>(loaded: Loaded<T>): GithubRead<T> | undefined =>
  loaded._tag === 'Read' ? loaded.read
    : loaded._tag === 'Failed' ? { status: 'unavailable', reason: loaded.message, retry_after_s: null }
    : loaded._tag === 'Unsupported' ? { status: 'unavailable', reason: 'This Fractal daemon does not serve GitHub reads yet.', retry_after_s: null }
    : undefined

const useNow = () => {
  const [now, setNow] = React.useState(() => Date.now())
  React.useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000)
    return () => clearInterval(timer)
  }, [])
  return now
}

function PullRequestSurface({ target, onOpen }: { readonly target: Extract<GithubRef, { kind: 'pull-request' }>; readonly onOpen?: (ref: string) => void }) {
  const [section, setSection] = React.useState<PullRequestSection>('conversation')
  const key = refOf(target)
  const item = useGithubRead(key, (reader: GithubReader) => reader.item(target) as Promise<GithubRead<GithubPullRequest>>)
  const conversation = useGithubRead(`${key}#conversation`, reader => reader.conversation(target), section === 'conversation')
  const checks = useGithubRead(`${key}#checks`, reader => reader.checks(target), section === 'checks')
  const files = useGithubRead(`${key}#files`, reader => reader.files(target), section === 'files')
  const optional = <T,>(name: string, read: GithubRead<T> | undefined) => read === undefined ? {} : { [name]: read }
  return <PullRequestDetail target={target} pullRequest={asRead(item)} now={useNow()} section={section} onSection={setSection}
    {...optional('conversation', asRead(conversation))} {...optional('checks', asRead(checks))} {...optional('files', asRead(files))} {...(onOpen === undefined ? {} : { onOpen })} />
}

function IssueSurface({ target, onOpen }: { readonly target: Extract<GithubRef, { kind: 'issue' }>; readonly onOpen?: (ref: string) => void }) {
  const issue = useGithubRead(refOf(target), (reader: GithubReader) => reader.item(target) as Promise<GithubRead<GithubIssue>>)
  return <IssueDetail target={target} issue={asRead(issue)} now={useNow()} {...(onOpen === undefined ? {} : { onOpen })} />
}

function RunSurface({ target, onOpen }: { readonly target: Extract<GithubRef, { kind: 'run' }>; readonly onOpen?: (ref: string) => void }) {
  const run = useGithubRead(refOf(target), (reader: GithubReader) => reader.item(target) as Promise<GithubRead<GithubRun>>)
  return <RunDetail target={target} run={asRead(run)} now={useNow()} {...(onOpen === undefined ? {} : { onOpen })} />
}

/**
 * A comment opens the item it sits on. GitHub's issues read answers for PRs too and says which it is, so
 * a comment on a PR lands on the PR tab.
 */
function CommentSurface({ target, onOpen }: { readonly target: Extract<GithubRef, { kind: 'comment' }>; readonly onOpen?: (ref: string) => void }) {
  const comment = useGithubRead(refOf(target), (reader: GithubReader) => reader.item(target) as Promise<GithubRead<GithubComment>>)
  const read = asRead(comment)
  const number = target.number ?? (read?.status === 'found' ? read.item.number : undefined)
  const issueTarget = number === undefined ? undefined : { kind: 'issue' as const, owner: target.owner, repo: target.repo, number }
  const issue = useGithubRead(issueTarget === undefined ? undefined : refOf(issueTarget), issueTarget === undefined ? undefined : (reader: GithubReader) => reader.item(issueTarget) as Promise<GithubRead<GithubIssue>>)
  const open = onOpen === undefined ? {} : { onOpen }
  if (issueTarget === undefined) return read === undefined || read.status === 'found' ? null : <ReadProblem read={read} target={target} {...open} />
  const issueRead = asRead(issue)
  if (issueRead?.status === 'found' && issueRead.item.is_pull_request) return <PullRequestSurface target={{ kind: 'pull-request', owner: target.owner, repo: target.repo, number: issueTarget.number }} {...open} />
  return <IssueDetail target={issueTarget} issue={issueRead} now={Date.now()} {...open} />
}

export function GithubSurface({ target, onOpen }: { readonly target: GithubRef; readonly onOpen?: (ref: string) => void }) {
  const open = onOpen === undefined ? {} : { onOpen }
  switch (target.kind) {
    case 'pull-request': return <PullRequestSurface target={target} {...open} />
    case 'issue': return <IssueSurface target={target} {...open} />
    case 'run': return <RunSurface target={target} {...open} />
    case 'comment': return <CommentSurface target={target} {...open} />
  }
}
