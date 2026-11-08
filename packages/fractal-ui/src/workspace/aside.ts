import type { Band } from './model.ts'

export interface AsideExplicit {
  readonly open: boolean
  readonly band: Band
}

export interface AsideResolution {
  readonly open: boolean
  readonly band: Band
  /** The explicit toggle still in force; `undefined` once the pane left the band it was made in. */
  readonly explicit: AsideExplicit | undefined
}

/**
 * Whether a pane's aside shows. By default it shows when the pane is at least `threshold` wide. A
 * person's explicit open or close wins while the pane stays in the band it was made in; once the pane
 * crosses into the other band by more than `hysteresis`, the default applies again.
 */
export const resolveAside = ({ width, threshold, hysteresis, explicit }: {
  readonly width: number
  readonly threshold: number
  readonly hysteresis?: number | undefined
  readonly explicit?: AsideExplicit | undefined
}): AsideResolution => {
  const band: Band = width >= threshold ? 'wide' : 'narrow'
  if (explicit === undefined) return { open: band === 'wide', band, explicit: undefined }
  const margin = hysteresis ?? 24
  const left = explicit.band === 'wide' ? width < threshold - margin : width >= threshold + margin
  return left ? { open: band === 'wide', band, explicit: undefined } : { open: explicit.open, band, explicit }
}

/** The band an explicit toggle at this width is recorded with. */
export const bandOf = (width: number, threshold: number): Band => width >= threshold ? 'wide' : 'narrow'

export const clampSize = (size: number, { min, max, available }: { readonly min: number; readonly max: number; readonly available?: number | undefined }): number => {
  const upper = available === undefined ? max : Math.max(min, Math.min(max, available))
  return Math.max(min, Math.min(upper, size))
}
