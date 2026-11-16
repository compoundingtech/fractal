// @genie-bootstrap
import {
  catalog,
  effectUtilsArtifactUrls,
  packageJson,
  privatePackageComposition,
  privatePackageDefaults,
  workspaceMember,
} from '../../../genie/repo.ts'

/*
 * Storybook over the Attention tray's real SwiftUI renders: TrayGallery writes PNGs and
 * `manifest.json` into `renders/`; the stories only frame them, they do not re-create the UI.
 */
const composition = privatePackageComposition({
  workspace: workspaceMember('flakes/attention-tray/design'),
  dependencies: {
    external: catalog.pick('react', 'react-dom'),
  },
  devDependencies: {
    external: {
      '@overeng/utils-storybook': effectUtilsArtifactUrls['@overeng/utils-storybook'],
      ...catalog.pick(
        '@storybook/react',
        '@storybook/react-vite',
        '@types/node',
        '@types/react',
        '@types/react-dom',
        '@vitejs/plugin-react',
        'storybook',
        'typescript',
        'vite',
      ),
    },
  },
})

export default packageJson(
  {
    name: '@schickling/attention-tray-design',
    ...privatePackageDefaults,
  },
  composition,
)
