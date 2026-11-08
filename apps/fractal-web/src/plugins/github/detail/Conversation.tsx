/** The Conversation subtab: the opening body, then reviews, review threads and comments, oldest first. */
import { markdownBlock } from '@fractal/client/markdown'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { Markdown } from '@fractal/ui/assistant-ui'
import { reviewTone, reviewVerdict } from '../model.ts'
import type { GithubComment, GithubConversation, GithubReview, GithubReviewThread, GithubUser } from '../schema.ts'
import { agoWord, conversationTimeline } from './detail-model.ts'
import { detailStyles as d, ExternalLink, Person, toneStyle } from './DetailShell.tsx'

/** Images in bodies stay links: the kit defers them, and opening one leaves Fractal for the source. */
const openImage = (src: string) => { window.open(src, '_blank', 'noopener,noreferrer') }

/** A GitHub body: GitHub defines these fields as GitHub Flavored Markdown, so the text is Markdown by contract. */
export function GithubMarkdown({ text }: { readonly text: string }) {
  return <div {...stylex.props(d.body)}><Markdown text={markdownBlock(text)} onLoadImage={openImage} /></div>
}

function When({ at, url, now }: { readonly at: string | null; readonly url: string; readonly now: number }) {
  return <ExternalLink href={url} label={at === null ? 'Open on GitHub' : `${agoWord(at, now)}, open on GitHub`}>{at === null ? 'on GitHub' : agoWord(at, now)}</ExternalLink>
}

export function OpeningPost({ author, verb, at, body, url, now }: { readonly author: GithubUser | null; readonly verb: string; readonly at: string; readonly body: string; readonly url: string; readonly now: number }) {
  return <section aria-label="Description" {...stylex.props(d.card)}>
    <div {...stylex.props(d.cardHead)}><Person login={author?.login} /><span>{verb}</span><span {...stylex.props(d.spacer)} /><When at={at} url={url} now={now} /></div>
    {body.trim() === '' ? <p {...stylex.props(d.muted)}>No description provided.</p> : <GithubMarkdown text={body} />}
  </section>
}

export function CommentCard({ comment, now }: { readonly comment: GithubComment; readonly now: number }) {
  const reactions = comment.reactions.reduce((sum, reaction) => sum + reaction.count, 0)
  return <article aria-label={`Comment by ${comment.author?.login ?? 'ghost'}`} data-entry="comment" {...stylex.props(d.card)}>
    <div {...stylex.props(d.cardHead)}>
      <Person login={comment.author?.login} />
      {comment.author_association === 'NONE' ? null : <span>{comment.author_association.toLowerCase()}</span>}
      {reactions === 0 ? null : <span>{reactions} {reactions === 1 ? 'reaction' : 'reactions'}</span>}
      <span {...stylex.props(d.spacer)} /><When at={comment.created_at} url={comment.url} now={now} />
    </div>
    <GithubMarkdown text={comment.body} />
  </article>
}

function ReviewCard({ review, now }: { readonly review: GithubReview; readonly now: number }) {
  return <article aria-label={`Review by ${review.author?.login ?? 'ghost'}: ${reviewVerdict(review.state)}`} data-entry="review" data-review-state={review.state} {...stylex.props(d.card)}>
    <div {...stylex.props(d.cardHead)}>
      <Person login={review.author?.login} />
      <span {...stylex.props(d.word, toneStyle(reviewTone(review.state)))}>{reviewVerdict(review.state)}</span>
      <span {...stylex.props(d.spacer)} /><When at={review.submitted_at} url={review.url} now={now} />
    </div>
    {review.body.trim() === '' ? null : <GithubMarkdown text={review.body} />}
  </article>
}

function ThreadCard({ thread, now }: { readonly thread: GithubReviewThread; readonly now: number }) {
  const where = thread.line === null ? thread.path : `${thread.path}:${thread.line}`
  return <article aria-label={`Review thread on ${where}`} data-entry="thread" data-resolved={thread.resolved} {...stylex.props(d.card)}>
    <div {...stylex.props(d.cardHead)}>
      <span {...stylex.props(d.mono)}>{where}</span>
      {thread.resolved ? <span>Resolved</span> : null}
      {thread.outdated ? <span>Outdated</span> : null}
    </div>
    <ol {...stylex.props(d.nested)}>{thread.comments.map(comment => <li key={comment.id} {...stylex.props(d.stack)}>
      <div {...stylex.props(d.cardHead)}><Person login={comment.author?.login} /><span {...stylex.props(d.spacer)} /><When at={comment.created_at} url={comment.url} now={now} /></div>
      <GithubMarkdown text={comment.body} />
    </li>)}</ol>
  </article>
}

export function ConversationPanel({ opening, conversation, problem, moreUrl, now }: { readonly opening: React.ReactNode; readonly conversation: GithubConversation | undefined; readonly problem: React.ReactNode | undefined; readonly moreUrl: string; readonly now: number }) {
  const timeline = conversation === undefined ? [] : conversationTimeline(conversation)
  return <div {...stylex.props(d.list)}>
    {opening}
    {problem}
    {timeline.length === 0 ? null : <ol aria-label="Timeline" {...stylex.props(d.list)}>{timeline.map(entry => <li key={`${entry.kind}:${entry.kind === 'review' ? entry.review.id : entry.kind === 'thread' ? entry.thread.id : entry.comment.id}`}>
      {entry.kind === 'review' ? <ReviewCard review={entry.review} now={now} /> : entry.kind === 'thread' ? <ThreadCard thread={entry.thread} now={now} /> : <CommentCard comment={entry.comment} now={now} />}
    </li>)}</ol>}
    {conversation?.truncated === true ? <p {...stylex.props(d.summaryLine)}>GitHub has more of this conversation. <ExternalLink href={moreUrl}>More on GitHub</ExternalLink></p> : null}
  </div>
}
