import { defineCatalog, packageJson } from '../../repos/effect-utils/genie/external.ts'
import kdlPkg from '../../repos/effect-utils/packages/@overeng/kdl/package.json.genie.ts'

const catalog = defineCatalog({
  effect: '4.1.0',
  '@noble/hashes': '2.4.0',
  '@effect/platform-node': '4.1.0',
  '@effect/vitest': '4.1.0',
  '@types/node': '24.10.0',
  typescript: '6.0.3',
  vitest: '4.1.9',
  'oxc-parser': '0.127.0',
})
const declarations = {
  data: { name: '@fractal/declarations' },
  meta: { workspace: { repoName: 'fractal', memberPath: 'clients/typescript/fractal-declarations', deps: [] } },
}
const deps = catalog.compose({
  workspace: { repoName: 'fractal', memberPath: 'packages/fractal' },
  dependencies: { workspace: [kdlPkg], external: catalog.pick('effect', '@noble/hashes') },
  devDependencies: {
    // Parity tests compare this package's KDL with the generated declaration encoder.
    workspace: [declarations],
    external: catalog.pick('@effect/platform-node', '@effect/vitest', '@types/node', 'typescript', 'vitest', 'oxc-parser'),
  },
})

export default packageJson({
  name: 'fractal',
  version: '0.1.0',
  private: true,
  license: 'MIT',
  type: 'module',
  exports: {
    '.': './src/index.ts',
    './effect': './src/effect.ts',
    './tree': './src/tree.ts',
    './testing': './src/testing.ts',
    './source-map': './src/source-map.ts',
  },
  scripts: {
    test: 'vitest run --config vitest.config.ts',
    typecheck: 'tsc --noEmit',
  },
}, deps)
