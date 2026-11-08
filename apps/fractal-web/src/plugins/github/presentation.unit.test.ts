import { describe, expect, it } from 'vitest'
import { issueFixture, pullRequestFixture, runFixture } from './fixtures.ts'
import { githubFace } from './presentation.ts'

describe('GitHub thing faces', () => {
  it('shows the spoken number and GitHub update time without inferring an ask', () => {
    const face = githubFace({ kind: 'issue', item: issueFixture })
    expect(face.title).toBe(issueFixture.title)
    expect(face.id).toBe(`#${issueFixture.number}`)
    expect(face.at).toEqual({ at: Date.parse(issueFixture.updated_at), verb: 'updated' })
    expect(face.act).toBeUndefined()
  })

  it('uses specific observed PR failure causes', () => {
    expect(githubFace({ kind: 'pull-request', item: { ...pullRequestFixture, state: 'open', merge_state: 'unstable' } }).act).toEqual({ tone: 'failed', label: 'checks failing' })
    expect(githubFace({ kind: 'pull-request', item: { ...pullRequestFixture, state: 'open', merge_state: 'clean', review_decision: 'changes_requested' } }).act).toEqual({ tone: 'failed', label: 'changes requested' })
    expect(githubFace({ kind: 'pull-request', item: { ...pullRequestFixture, state: 'merged', merge_state: 'unstable', review_decision: 'changes_requested' } }).act).toBeUndefined()
  })

  it('ages running CI from start and completed CI from finish', () => {
    const started_at = '2026-11-09T12:00:00Z'
    const completed_at = '2026-11-09T12:05:00Z'
    const running = githubFace({ kind: 'run', item: { ...runFixture, status: 'in_progress', conclusion: null, started_at, completed_at: null } })
    expect(running.at).toEqual({ at: Date.parse(started_at), verb: 'started' })
    expect(running.mark.shape).toBe('filled')
    const failed = githubFace({ kind: 'run', item: { ...runFixture, status: 'completed', conclusion: 'timed_out', started_at, completed_at } })
    expect(failed.at).toEqual({ at: Date.parse(completed_at), verb: 'finished' })
    expect(failed.act).toEqual({ tone: 'failed', label: 'timed out' })
  })
})
