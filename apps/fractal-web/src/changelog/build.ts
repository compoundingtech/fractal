import { buildIdentity } from 'virtual:build-identity'
import { changelogDeployment, changelogSources } from '../extensions/build.ts'
import { compileChangelog } from './compiler.ts'
import { changelogAudienceByDeployment } from './contract.ts'

/**
 * Development only: production builds replace this module with a precompiled literal before
 * bundling (scripts/extensions-build.mjs), so registries and private prose never reach a bundle
 * through it. Load lazily; the unseen dot reads ./metadata.ts.
 */
export const changelogManifest = compileChangelog({
  build: { machineVersion: buildIdentity.machineVersion },
  sources: changelogSources,
  audience: changelogAudienceByDeployment[changelogDeployment],
})
export { changelogDeployment }
