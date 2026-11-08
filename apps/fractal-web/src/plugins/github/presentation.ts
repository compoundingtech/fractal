import type { ThingFace } from '../../../../../packages/fractal-ui/src/assistant-ui/things/face.ts'
import { checkCounts, faceOf, type GithubItem } from './model.ts'
import type { GithubChecks } from './schema.ts'

/** Passive observations never imply that the viewer has permission or a duty to review. */
export const githubFace = (input: GithubItem, checks?: GithubChecks): ThingFace => {
  const decoded = faceOf(input)
  const failed = input.kind === 'run' && decoded.tone === 'failure'
  const failingChecks = input.kind === 'pull-request' && input.item.state === 'open' && (input.item.merge_state === 'unstable' || (checks !== undefined && checkCounts(checks.runs).failure > 0))
  const changesRequested = input.kind === 'pull-request' && input.item.state === 'open' && input.item.review_decision === 'changes_requested'
  const at = decoded.updatedAt === undefined ? undefined : Date.parse(decoded.updatedAt)
  return {
    kind: input.kind === 'pull-request' ? 'Pull request' : input.kind === 'issue' ? 'Issue' : input.kind === 'run' ? 'CI run' : 'Comment',
    icon: input.kind === 'pull-request' ? 'pull-request' : input.kind === 'issue' ? 'dot' : input.kind === 'run' ? 'check' : 'message',
    title: decoded.title,
    ...(input.kind === 'pull-request' || input.kind === 'issue' ? { id: `#${input.item.number}` } : input.kind === 'run' ? { id: `#${input.item.id}` } : {}),
    mark: { shape: input.kind === 'run' ? input.item.status === 'completed' ? 'ended' : 'filled' : decoded.tone === 'closed' || decoded.tone === 'merged' ? 'ended' : 'hollow', word: decoded.state.toLowerCase() },
    ...(failed || failingChecks || changesRequested ? { act: { tone: 'failed', label: failed ? decoded.state.toLowerCase() : failingChecks ? 'checks failing' : 'changes requested' } as const } : {}),
    ...(at === undefined ? {} : { at: { at, verb: input.kind === 'run' ? input.item.completed_at === null ? 'started' : 'finished' : 'updated' } }),
    facts: decoded.facts.map(value => ({ label: '', value: { _tag: 'Known', value } })),
  }
}
