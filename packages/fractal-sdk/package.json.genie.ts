import { defineCatalog, packageJson } from '../../repos/effect-utils/genie/external.ts'

const catalog = defineCatalog({
  effect: '4.0.0',
  '@effect/vitest': '4.0.0',
  '@types/node': '24.10.0',
  typescript: '6.0.3',
  vitest: '4.1.9',
})
const client = {
  data: { name: '@fractal/client' },
  meta: { workspace: { repoName: 'smalltalk', memberPath: 'clients/typescript/fractal-client', deps: [] } },
}
const deps = catalog.compose({
  workspace: { repoName: 'smalltalk', memberPath: 'packages/fractal-sdk' },
  dependencies: { workspace: [client], external: catalog.pick('effect') },
  devDependencies: { external: catalog.pick('@effect/vitest', '@types/node', 'typescript', 'vitest') },
})

export default packageJson({
  name: '@fractal/sdk',
  version: '0.1.0',
  private: true,
  license: 'MIT',
  type: 'module',
  exports: {
    './effect': './src/effect/mod.ts',
    './admission': './src/effect/admission.ts',
  },
  scripts: {
    test: 'vitest run --config vitest.config.ts',
    typecheck: 'tsc --noEmit',
  },
}, deps)
