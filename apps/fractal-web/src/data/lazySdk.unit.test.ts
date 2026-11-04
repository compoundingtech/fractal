import { Effect } from 'effect'
import { describe, expect, it, vi } from 'vitest'
import { retryableImport } from './lazySdk.ts'

describe('SDK chunk acquisition', () => {
  it('does not load until used and shares successful concurrent acquisitions', async () => {
    const service = { ready: true }
    const load = vi.fn(async () => service)
    const acquire = retryableImport(load, 'terminal services')
    expect(load).not.toHaveBeenCalled()
    const values = await Promise.all([Effect.runPromise(acquire), Effect.runPromise(acquire)])
    expect(values).toEqual([service, service])
    expect(await Effect.runPromise(acquire)).toBe(service)
    expect(load).toHaveBeenCalledTimes(1)
  })

  it('contains a failed chunk and retries on the next explicit acquisition', async () => {
    const service = { ready: true }
    const load = vi.fn<() => Promise<typeof service>>()
      .mockRejectedValueOnce(new Error('chunk unavailable'))
      .mockResolvedValue(service)
    const acquire = retryableImport(load, 'editing services')
    const result = await Effect.runPromise(Effect.result(acquire))
    expect(result._tag).toBe('Failure')
    if (result._tag === 'Failure') {
      expect(result.failure._tag).toBe('SdkChunkFailure')
      expect(result.failure.message).toContain('Try again')
    }
    expect(await Effect.runPromise(acquire)).toBe(service)
    expect(load).toHaveBeenCalledTimes(2)
  })
})
