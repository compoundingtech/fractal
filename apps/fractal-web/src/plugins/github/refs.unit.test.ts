import { describe, expect, it } from 'vitest'

import { claimsKind, githubSubjectRef, githubUrlOf, parseGithubUrl, parseSubjectRef, readPathOf, refOf } from './refs.ts'

describe('GitHub subject refs', () => {
  it('reads both PR spellings the graph uses and writes the canonical one', () => {
    for (const ref of ['resource/github/acme/demo/pull/1', 'resource/github/acme/demo/pull-request/1']) {
      expect(parseSubjectRef(ref)).toEqual({ kind: 'pull-request', owner: 'acme', repo: 'demo', number: 1 })
      expect(githubSubjectRef(ref)).toBe('resource/github/acme/demo/pull-request/1')
    }
  })

  it('claims each kind exclusively and nothing else', () => {
    const refs = ['resource/github/acme/garden/pull-request/7', 'resource/github/acme/garden/issue/9', 'resource/github/acme/garden/comment/5', 'resource/github/acme/garden/run/9001']
    const kinds = ['pull-request', 'issue', 'comment', 'run'] as const
    for (const [i, ref] of refs.entries()) {
      expect(kinds.filter(kind => claimsKind(kind)(ref))).toEqual([kinds[i]])
    }
    for (const ref of ['resource/github/acme/garden', 'resource/github/acme/garden/commit/abc', 'resource/github/acme/app/ci/pull-request/42', 'resource/github/acme/garden/pull-request/0', 'resource/github/acme/garden/pull-request/7/x', 'resource/github/../x/pull/1', 'agent/x', 'mission/m']) {
      expect(kinds.some(kind => claimsKind(kind)(ref)), ref).toBe(false)
    }
  })
})

describe('GitHub links', () => {
  it('parses PRs, issues, comments and runs', () => {
    expect(parseGithubUrl('https://github.com/acme/garden/pull/7')).toEqual({ kind: 'pull-request', owner: 'acme', repo: 'garden', number: 7 })
    expect(parseGithubUrl('https://github.com/acme/garden/pull/7/files')).toMatchObject({ kind: 'pull-request', number: 7 })
    expect(parseGithubUrl('https://github.com/acme/garden/pull/7#discussion_r1001')).toMatchObject({ kind: 'pull-request', number: 7 })
    expect(parseGithubUrl('https://github.com/acme/garden/issues/9')).toEqual({ kind: 'issue', owner: 'acme', repo: 'garden', number: 9 })
    expect(parseGithubUrl('https://github.com/acme/garden/issues/9#issuecomment-55')).toEqual({ kind: 'comment', owner: 'acme', repo: 'garden', comment_id: 55, number: 9 })
    expect(parseGithubUrl('https://github.com/acme/garden/pull/7#issuecomment-55')).toEqual({ kind: 'comment', owner: 'acme', repo: 'garden', comment_id: 55, number: 7 })
    expect(parseGithubUrl('https://github.com/acme/garden/actions/runs/9001/job/3')).toEqual({ kind: 'run', owner: 'acme', repo: 'garden', run_id: 9001 })
  })

  it('refuses other hosts, credentials, ports and non-item pages', () => {
    const credentialedUrl = new URL('https://github.com/acme/garden/pull/7')
    credentialedUrl.username = 'fixture-user'
    credentialedUrl.password = 'pw'
    for (const href of ['https://gitlab.com/acme/garden/pull/7', 'https://evil.com/github.com/acme/garden/pull/7', credentialedUrl.href, 'https://github.com:8443/acme/garden/pull/7', 'https://github.com/acme/garden', 'https://github.com/acme/garden/tree/main', 'https://github.com/acme/garden/pull/abc', 'javascript:alert(1)', 'not a url', 'https://github.com/acme/garden/pull/7/merge']) {
      expect(parseGithubUrl(href), href).toBeUndefined()
    }
  })

  it('round-trips to github.com and to the daemon path without passing a URL', () => {
    const ref = parseGithubUrl('https://github.com/acme/garden/issues/9#issuecomment-55')!
    expect(githubUrlOf(ref)).toBe('https://github.com/acme/garden/issues/9#issuecomment-55')
    expect(refOf(ref)).toBe('resource/github/acme/garden/comment/55')
    expect(readPathOf(ref)).toBe('/v1/client/plugins/github/acme/garden/comments/55')
  })
})
