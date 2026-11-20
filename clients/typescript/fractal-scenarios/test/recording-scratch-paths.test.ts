import { describe, expect, it } from 'vitest'

import { scanRecordingContent } from '../scripts/scan.ts'

describe('recording scratch path prefixes', () => {
  for (const prefix of ['st3', 'fractal']) {
    it(`rejects ${prefix} recorder scratch paths outside the system temporary directory`, () => {
      const path = 'recordings/x.json'
      const text = JSON.stringify({ cwd: `build/${prefix}-scenario-fixture` })

      expect(scanRecordingContent(path, text)).toEqual([
        { path, line: 1, rule: 'scratch path' },
      ])
    })
  }
})
