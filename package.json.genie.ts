import { packageJson } from './repos/effect-utils/genie/external.ts'
import devbarPkg from './repos/effect-utils/packages/@overeng/devbar/package.json.genie.ts'
import fractalPkg from './packages/fractal/package.json.genie.ts'

// Imported package compositions own their complete foreign workspace closure: devbar's diagnostics
// packages and the authoring package's `@overeng/kdl`.
export const composedPackages = [devbarPkg, fractalPkg]

/**
 * The root pnpm workspace's members, listed explicitly: no `apps/**`, `packages/**` or
 * `clients/**` globs. App-independent SDK and UI packages live under `packages/`.
 */
export const workspaceMembers = [
  'clients/typescript/fractal-client',
  'clients/typescript/fractal-declarations',
  'clients/typescript/fractal-views',
  'clients/typescript/fractal-scenarios',
  'apps/ios',
  'packages/fractal-ui',
  'apps/fractal-web',
  'packages/fractal-sdk',
  'packages/fractal',
] as const

export default packageJson.aggregateFromPackages({
  packages: composedPackages,
  name: 'fractal-workspace',
  repoName: 'fractal',
  extraMembers: workspaceMembers,
})
