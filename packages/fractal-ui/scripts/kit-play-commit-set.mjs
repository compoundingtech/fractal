import { createHash } from 'node:crypto'

// The per-commit play set (KIT_PLAY_SET=commit). Pull requests run it; merge groups and manual runs
// run the full strict case set with axe (KIT_PLAY_SET=all), including the explicit overrides below.
//
// It starts from the stories tagged `critical` and narrows them in two ways, which is what makes it
// fit the per-commit budget (about 60 s on the 4-vCPU runner; the critical set took about 185 s):
//
//  1. Each story runs in one case, not four. The case is one of two diagonals of the
//     scheme × width matrix, chosen by a stable hash of the story id, so about half the stories run
//     dark at 440 and half light at 1440: every story's play runs, and both schemes and both widths
//     stay covered across the set. Stories in BOTH_WIDTHS are about layout at a width, so they run
//     dark at 440 and at 1440.
//  2. Stories in MERGE_ONLY do not run per commit. They are the slowest stories whose coverage is
//     behavioural depth or render cost rather than a kit contract, and the merge gate still runs them.
//
// Every id named here must be a critical story, so a rename cannot silently drop coverage.
export const MERGE_ONLY = [
  // 15 s of case time: every state of the transcript's no-green rule in one story.
  'fractal-ui-transcript--no-green-all-states',
  // 10 s of case time: renders a hundred agents; a render-cost check rather than a contract.
  'fractal-ui-sidebar-roster--hundred-agent-render',
]
export const BOTH_WIDTHS = [
  'fractal-ui-composer--footer-width-budget',
  'fractal-ui-composer--kit-label-resize',
  'fractal-ui-composer--layout-shapes',
  'fractal-ui-sidebar-agent-row--densities-and-children',
  'fractal-ui-transcript-composer--host-width',
  'fractal-ui-transcript-composer--reading-column',
  'fractal-ui-workbench--all-states',
  'fractal-narrow-layout--decode-stale-sidebar-hosts',
]

const FULL = [['dark', 440], ['dark', 1440], ['light', 440], ['light', 1440]]

// Scheme-only aliases remain available in Storybook; the gate already overrides their scheme.
// The filter benchmark itself renders 240/440 × dark/light probes, so repeat its measurements
// in only one outer browser case. FilterBehavior retains functional coverage in all four cases.
export const FULL_CASE_OVERRIDES = new Map([
  ['fractal-ui-transcript--no-green-all-states-light', []],
  ['fractal-ui-composer--footer-width-budget-light', []],
  ['fractal-ui-sidebar-tree--filter-keystroke-to-paint', [['dark', 1440]]],
  // Collapse removes the production sidebar separator while a drag is held at this measured viewport.
  ['fractal-narrow-layout--sidebar-drag-unmount-cleanup', [['dark', 1100], ['light', 1100]]],
])

// Validate against the complete index before selecting tags or shards, so stale policy cannot
// silently remove coverage. An omitted scheme alias must share its kept base story's source.
export function validateCaseOverrides(entries, overrides = FULL_CASE_OVERRIDES) {
  const stories = new Map(Object.values(entries).filter(entry => entry.type === 'story').map(entry => [entry.id, entry]))
  for (const [id, cases] of overrides) {
    if (!stories.has(id)) throw new Error(`The strict case policy names ${id}, which is not a known story`)
    if (cases.length !== 0) continue
    const baseId = id.endsWith('-light') ? id.slice(0, -6) : undefined
    if (!baseId || !stories.has(baseId) || overrides.get(baseId)?.length === 0 || stories.get(baseId).importPath !== stories.get(id).importPath) {
      throw new Error(`The strict case policy omits ${id}, which is not a scheme alias of a kept story`)
    }
  }
}

// A stable pick of one corner pair, independent of the other stories in the set.
export const diagonalCase = id => FULL[createHash('sha256').update(id).digest()[0] % 2 === 0 ? 0 : 3]

// The cases of the per-commit set for the critical story ids, plus what the set leaves out.
export function commitSetCases(criticalIds, { mergeOnlyIds = MERGE_ONLY, bothWidthsIds = BOTH_WIDTHS, overrides = FULL_CASE_OVERRIDES } = {}) {
  const critical = new Set(criticalIds)
  for (const id of [...mergeOnlyIds, ...bothWidthsIds]) {
    if (!critical.has(id)) throw new Error(`The per-commit set names ${id}, which is not a critical story`)
  }
  const mergeOnly = new Set(mergeOnlyIds), bothWidths = new Set(bothWidthsIds)
  const ids = criticalIds.filter(id => !mergeOnly.has(id) && overrides.get(id)?.length !== 0)
  const cases = ids.flatMap(id => (overrides.get(id) ?? (bothWidths.has(id) ? [FULL[0], FULL[1]] : [diagonalCase(id)])).map(([scheme, width]) => ({ id, scheme, width })))
  return { ids, cases, omittedStories: criticalIds.filter(id => mergeOnly.has(id) || overrides.get(id)?.length === 0), omittedCases: criticalIds.length * 4 - cases.length }
}

export const fullCases = (ids, overrides = FULL_CASE_OVERRIDES) => ids.flatMap(id => (overrides.get(id) ?? FULL).map(([scheme, width]) => ({ id, scheme, width })))
