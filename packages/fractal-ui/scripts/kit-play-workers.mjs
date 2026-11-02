// How many browser workers the kit play gate runs at once. CI's runner is a machine of its own, so the gate fills it (at most 6, which
// the timing receipts were measured with). On a shared dev box scripts/ci-local-web sets KIT_PLAY_WORKERS to the play stage's share of
// the host (its cores over the build host's heavy slots), so two kit plays at once cannot take every core. An empty value is unset.
export const MAX_WORKERS = 6

export function playWorkers(env, parallelism) {
  const fill = Math.min(MAX_WORKERS, parallelism)
  const value = env.KIT_PLAY_WORKERS
  if (value === undefined || value === '') return fill
  if (!/^[1-9][0-9]*$/.test(value)) throw new Error('KIT_PLAY_WORKERS must be a positive whole number')
  return Math.min(fill, Number(value))
}
