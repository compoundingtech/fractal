import type { MissionDetailModel } from './detailModel.ts'

// The trace's wall clock. Not yet implemented: the tests in traceScale.unit.test.ts pin it first.

export interface Projection { readonly start: number; readonly end: number }
export interface SpanBox { readonly x0: number; readonly x1: number; readonly label: number }
export type LabelPlace = 'after' | 'inside' | 'before' | 'none'
export interface ScrubBounds { readonly t0: number; readonly now: number; readonly step: number; readonly page: number }

export const projectedSpans = (_model: MissionDetailModel): ReadonlyMap<string, Projection> => new Map()
export const traceWindow = (model: MissionDetailModel, _projected: ReadonlyMap<string, Projection>): { readonly t0: number; readonly t1: number } => ({ t0: model.startedAt, t1: model.now })
export const tickEvery = (_pxPerMs: number): number => 0
export const ticksIn = (_t0: number, _t1: number, _every: number): readonly number[] => []
export const placeSpanLabels = (spans: readonly SpanBox[], _limit: number): readonly LabelPlace[] => spans.map(() => 'none')
export const scrubTo = (t: number, _bounds: ScrubBounds): number | undefined => t
export const scrubKey = (_key: string, cursor: number | undefined, _bounds: ScrubBounds): { readonly handled: boolean; readonly cursor: number | undefined } => ({ handled: false, cursor })
