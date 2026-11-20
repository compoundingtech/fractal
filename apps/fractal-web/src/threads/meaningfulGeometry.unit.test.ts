import { expect, it } from 'vitest'
import { meaningfulBodyRegion } from './meaningfulGeometry.ts'

// Adopted unread-implementation, “Deterministic visibility default for implementation/review”.
const clip = { left: 0, right: 200, top: 0, bottom: 100 }
it.each([
  ['one pixel overlap', { left: 10, right: 100, top: 99, bottom: 119 }, false],
  ['clipped one-line body', { left: 10, right: 100, top: 90, bottom: 110 }, false],
  ['full one-line body', { left: 10, right: 100, top: 80, bottom: 100 }, true],
  ['long body top only', { left: 10, right: 100, top: 20, bottom: 220 }, false],
  ['long body final two lines', { left: 10, right: 100, top: -100, bottom: 100 }, true],
  ['only final clipped line', { left: 10, right: 100, top: -100, bottom: 120 }, false],
  ['clipped readable width', { left: -1, right: 100, top: 20, bottom: 40 }, false],
] as const)('%s', (_name, body, eligible) => {
  expect(meaningfulBodyRegion(body, 20, [clip]) !== undefined).toBe(eligible)
})
it('intersects every ancestor, visual viewport and composer boundary', () => {
  const body = { left: 10, right: 100, top: 40, bottom: 100 }
  expect(meaningfulBodyRegion(body, 20, [clip])).toEqual({ ...body, top: 60 })
  for (const obstruction of [{ ...clip, bottom: 99 }, { ...clip, top: 61 }, { ...clip, left: 11 }, { ...clip, right: 99 }]) {
    expect(meaningfulBodyRegion(body, 20, [clip, obstruction])).toBeUndefined()
  }
})
