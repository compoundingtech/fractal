import { describe, expect, it } from 'vitest'

import { plainExcerpt } from './model.ts'

describe('card excerpt', () => {
  it('keeps words, drops markup and never yields an image', () => {
    expect(plainExcerpt('## Title\n\nSee [docs](https://x.y) and ![chart](https://img/x.png) **now**.')).toBe('Title See docs and [image: chart] now.')
    expect(plainExcerpt('a'.repeat(300))).toHaveLength(220)
  })
})
