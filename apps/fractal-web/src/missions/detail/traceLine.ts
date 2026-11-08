import type { DetailStep } from './detailModel.ts'

// A trace row's second line. Not yet implemented: the tests in linking.unit.test.ts pin it first.

export interface TraceLine {
  readonly parts: readonly string[]
  readonly you: string | undefined
}

export const traceLine = (_step: DetailStep, _agentLabel: string | undefined, _now: number): TraceLine => ({ parts: [], you: undefined })
