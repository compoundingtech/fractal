import { changelogDeployment, changelogSources } from '../../.changelog/mod.ts'
import { compileChangelog } from './compiler.ts'
import { changelogAudienceByDeployment } from './contract.ts'

/**
 * Development only: production builds replace this module with a precompiled literal before
 * bundling (scripts/extensions-build.mjs), so registries and private prose never reach a bundle
 * through it. Load lazily; the unseen dot reads ./metadata.ts.
 */
export const getChangelogManifest = (machineVersion: string) => compileChangelog({
  build: { machineVersion },
  sources: changelogSources,
  audience: changelogAudienceByDeployment[changelogDeployment],
})
export { changelogDeployment }
