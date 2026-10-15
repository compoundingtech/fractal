import type { StorybookConfig } from '@storybook/react-vite'

// This workspace does not depend on @overeng/utils-storybook. Keep the app index
// separate from the kit, using the app's production StyleX and identity plugins.
const config: StorybookConfig = {
  stories: ['../src/**/*.stories.tsx'],
  framework: {
    name: '@storybook/react-vite',
    options: { builder: { viteConfigPath: new URL('./vite.config.ts', import.meta.url).pathname } },
  },
  addons: ['@storybook/addon-docs', '@storybook/addon-a11y'],
  core: { disableTelemetry: true },
}
export default config
