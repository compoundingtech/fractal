import { packageJson } from './repos/effect-utils/genie/external.ts'
import devbarPkg from './repos/effect-utils/packages/@overeng/devbar/package.json.genie.ts'

// Imported package composition owns the complete diagnostics workspace closure.
export const diagnosticsPackages = [devbarPkg]

/**
 * The root pnpm workspace's members, listed explicitly: no `apps/**`, `packages/**` or
 * `clients/**` globs. `packages/fractal-ui`, `apps/fractal-web` and its nested
 * `packages/fractal-sdk` join this list in the change that adds each package.
 */
export const workspaceMembers = [
  'clients/typescript/fractal-client',
  'clients/typescript/fractal-views',
  'clients/typescript/fractal-scenarios',
  'apps/ios',
  'packages/fractal-ui',
  'apps/fractal-web',
  'apps/fractal-web/packages/fractal-sdk',
] as const

export default packageJson.aggregateFromPackages({
  packages: diagnosticsPackages,
  name: 'smalltalk-workspace',
  repoName: 'smalltalk',
  extraMembers: workspaceMembers,
})
