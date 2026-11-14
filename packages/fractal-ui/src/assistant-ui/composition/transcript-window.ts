import type { ScenarioMountedWindow } from '@fractal/scenarios/storybook/check'

/** Source rows (a prompt counts as one) the first commit mounts; the latest prompt and a reasoning run crossing the bound mount whole besides. */
export const transcriptInitialSourceRowCount = 6
/** Story-declared window tied to the same constant as the real initial mount. */
export const transcriptMountedWindow: Extract<ScenarioMountedWindow, { readonly _tag: 'sourceRows' }> = { _tag: 'sourceRows', rows: transcriptInitialSourceRowCount }
