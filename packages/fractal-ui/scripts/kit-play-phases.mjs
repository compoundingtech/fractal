// Split one case's wait for storyFinished into Storybook's own render phases.
// `marks` are [name, milliseconds since the page's navigation start], in arrival order, with the
// strict `storyFinished` event last. The first interval is everything before the first phase
// the page reported; each later interval belongs to the phase that began it, so the a11y check
// (after the last render phase, before storyFinished) shows up under that last phase.
export function phaseTimings(marks) {
  const timings = {}
  if (!Array.isArray(marks) || !marks.length) return timings
  const valid = marks.filter(mark => Array.isArray(mark) && typeof mark[0] === 'string' && Number.isFinite(mark[1]))
  if (!valid.length) return timings
  timings.storyBootMilliseconds = valid[0][1]
  for (let index = 0; index < valid.length - 1; index++) {
    const key = `storyPhase_${valid[index][0]}_Milliseconds`
    timings[key] = (timings[key] ?? 0) + Math.max(0, valid[index + 1][1] - valid[index][1])
  }
  return timings
}
