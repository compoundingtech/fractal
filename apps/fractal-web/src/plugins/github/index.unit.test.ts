import * as React from 'react'
import { describe, expect, it } from 'vitest'

import { githubContributions, githubLinkPreview } from './index.ts'

describe('GitHub plugin entry', () => {
  it('previews only github.com item links', () => {
    const link = React.createElement('a', { href: 'x' })
    expect(githubLinkPreview('https://example.com/a/b/pull/1', link)).toBeUndefined()
    expect(githubLinkPreview('https://github.com/acme/garden', link)).toBeUndefined()
    expect(githubLinkPreview('https://github.com/acme/garden/pull/7', link)).toBeDefined()
  })

  it('contributes one kind per GitHub item, each owning only its example (registry overlap rule)', () => {
    const contributions = githubContributions(kind => kind)
    expect(contributions.map(contribution => contribution.id)).toEqual(['github.pull-request', 'github.issue', 'github.comment', 'github.run'])
    for (const contribution of contributions) {
      for (const ref of contribution.examples) {
        expect(contributions.filter(other => other.claims(ref)).map(other => other.id)).toEqual([contribution.id])
      }
    }
    expect(contributions[0]!.policy.resources({ _tag: 'Resource', ref: 'resource/github/acme/garden/pull/7' })).toEqual([{ _tag: 'Contributed', kind: 'github.pull-request', ref: 'resource/github/acme/garden/pull/7' }])
  })
})
