// @genie-bootstrap
import { defineCatalog, packageJson, privatePackageDefaults } from '../../../../repos/effect-utils/genie/external.ts'
import storybookPkg from '../../../../repos/effect-utils/packages/@overeng/utils-storybook/package.json.genie.ts'

// Match @fractal/ui's browser toolchain, with Vite satisfying utils-storybook's peer.
const catalog = defineCatalog({
  react: '19.2.3',
  'react-dom': '19.2.3',
  '@storybook/react': '10.6.1',
  '@storybook/react-vite': '10.6.1',
  '@types/node': '26.6.4',
  '@types/react': '19.2.18',
  '@types/react-dom': '19.2.7',
  storybook: '10.6.1',
  typescript: '6.0.3',
  vite: '^8.3.8',
})

/* FractalMacGallery writes native PNGs and manifests; stories only frame them. */
const composition = catalog.compose({
  workspace: { repoName: 'fractal', memberPath: 'apps/macos/Fractal/design' },
  dependencies: {
    external: catalog.pick('react', 'react-dom'),
  },
  devDependencies: {
    workspace: [storybookPkg],
    external: catalog.pick(
      '@storybook/react',
      '@storybook/react-vite',
      '@types/node',
      '@types/react',
      '@types/react-dom',
      'storybook',
      'typescript',
      'vite',
    ),
  },
})

export default packageJson(
  {
    name: '@compoundingtech/fractal-mac-design',
    ...privatePackageDefaults,
    scripts: {
      storybook: 'storybook dev -p 6023 --host 127.0.0.1 --no-open --disable-telemetry',
      'build-storybook': 'storybook build --disable-telemetry',
    },
  },
  composition,
)
