import { commonPnpmPolicySettings, pnpmWorkspaceYaml, projectPnpmSourceInputs } from './repos/effect-utils/genie/external.ts'
import { composedPackages, workspaceMembers } from './package.json.genie.ts'

export const composedSourceInputs = projectPnpmSourceInputs({
  packages: composedPackages,
  repoName: 'fractal',
})

// The shared native-build denylist names packages this lock does not contain; `ignoreScripts`
// already blocks every dependency lifecycle script.
const { allowBuilds: _sharedDenylist, ...pnpmPolicy } = commonPnpmPolicySettings

// One frozen root lock for every member. The hoisted linker keeps the flat `node_modules`
// layout Expo's Metro resolver already relies on; workspace packages are linked, not injected.
export default pnpmWorkspaceYaml.root({
  packages: composedPackages,
  repoName: 'fractal',
  extraMembers: workspaceMembers,
  ...pnpmPolicy,
  overrides: {
    ...pnpmPolicy.overrides,
    ...composedSourceInputs.overrides,
    // Keep platform-node's shared peer in the same Effect cohort as the consumers.
    '@effect/platform-node>@effect/platform-node-shared': '4.1.0',
    // One @pierre/highlights for the kit and @pierre/diffs (0.3.0 has 0.2.0's API plus languages). With
    // diffs' pinned 0.2.0 hoisted, app code and the kit resolved two copies of the highlighter module.
    '@pierre/diffs>@pierre/highlights': '0.3.0',
  },
  peerDependencyRules: {
    ...pnpmPolicy.peerDependencyRules,
    allowedVersions: {
      ...pnpmPolicy.peerDependencyRules?.allowedVersions,
      ...composedSourceInputs.peerDependencyRules.allowedVersions,
    },
  },
  minimumReleaseAgeExclude: ['@types/node'],
  nodeLinker: 'hoisted',
})
