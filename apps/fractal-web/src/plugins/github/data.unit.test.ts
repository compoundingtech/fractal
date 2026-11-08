import { describe, expect, it } from 'vitest'

import { gatewayGithubReader, GithubReadFailure, GithubReadUnsupported, sharedGithubReader, type GithubReader } from './data.ts'
import files from '../../../../../docs/st3/client-v0/fixtures/github-plugin-files.json' with { type: 'json' }
import pullRequest from '../../../../../docs/st3/client-v0/fixtures/github-plugin-pull-request.json' with { type: 'json' }

const pr = { kind: 'pull-request', owner: 'acme', repo: 'garden', number: 7 } as const

describe('gateway GitHub reader', () => {
  it('reads typed paths and decodes the envelope value', async () => {
    const asked: string[] = []
    const reader = gatewayGithubReader({ baseUrl: 'https://fractal.test', fetchImpl: async input => {
      asked.push(String(input))
      return new Response(JSON.stringify({ api_version: 'v0', request_id: 'r', snapshot: null, value: String(input).endsWith('/files') ? files : pullRequest }), { status: 200 })
    } })
    const read = await reader.item(pr)
    expect(read).toMatchObject({ status: 'found', item: { number: 7 } })
    await reader.files(pr)
    expect(asked).toEqual(['https://fractal.test/v1/client/plugins/github/acme/garden/pulls/7', 'https://fractal.test/v1/client/plugins/github/acme/garden/pulls/7/files'])
  })

  it('tells a daemon without the route apart from a refusal', async () => {
    const reader = gatewayGithubReader({ baseUrl: 'https://fractal.test', fetchImpl: async () => new Response('', { status: 404 }) })
    await expect(reader.item(pr)).rejects.toBeInstanceOf(GithubReadUnsupported)
  })

  it('fails visibly when Fractal refuses', async () => {
    const reader = gatewayGithubReader({ baseUrl: 'https://fractal.test', fetchImpl: async () => new Response('{}', { status: 403 }) })
    await expect(reader.item(pr)).rejects.toBeInstanceOf(GithubReadFailure)
  })
})

describe('shared GitHub reader', () => {
  it('shares one request and asks again after a failure or once reuse ends', async () => {
    let calls = 0, fail = true, clock = 0
    const inner: GithubReader = {
      item: async () => { calls++; if (fail) throw new Error('down'); return { status: 'gone' } },
      conversation: async () => ({ status: 'gone' }), checks: async () => ({ status: 'gone' }), files: async () => ({ status: 'gone' }),
    }
    const shared = sharedGithubReader(inner, { reuseMs: 1000, now: () => clock })
    await Promise.allSettled([shared.item(pr), shared.item(pr)])
    expect(calls).toBe(1)
    fail = false
    await shared.item(pr)
    await shared.item(pr)
    expect(calls).toBe(2)
    clock = 2000
    await shared.item(pr)
    expect(calls).toBe(3)
  })
})
