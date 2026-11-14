import { describe, expect, it } from 'vitest'

import * as fixtures from './fixtures.ts'

describe('GitHub plugin wire fixtures', () => {
  it('decode through the plugin schemas, every read state included', () => {
    expect(fixtures.pullRequestRead.status).toBe('found')
    expect(fixtures.stalePullRequestRead).toMatchObject({ status: 'found', stale: true })
    expect(fixtures.movedRead).toEqual({ status: 'moved', to: { owner: 'acme-garden', repo: 'garden', number: 7 } })
    expect(fixtures.goneRead.status).toBe('gone')
    expect(fixtures.notFoundRead.status).toBe('not_found')
    expect(fixtures.unavailableRead).toMatchObject({ status: 'unavailable', retry_after_s: 120 })
    expect(fixtures.pullRequestFixture).toMatchObject({ number: 7, state: 'open', merge_state: 'blocked' })
    expect(fixtures.issueFixture.is_pull_request).toBe(false)
    expect(fixtures.conversationFixture.review_threads).toHaveLength(2)
    expect(fixtures.checksFixture.runs.map(run => run.status)).toContain('in_progress')
    expect(fixtures.filesFixture.files.some(file => file.patch === null)).toBe(true)
    expect(fixtures.runFixture.jobs).toHaveLength(3)
  })
})
