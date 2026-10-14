import type { StorybookConfig } from '@storybook/react-vite'
import tailwindcss from '@tailwindcss/vite'
import stylex from '@stylexjs/unplugin'

const config: StorybookConfig = {
  stories: ['../src/**/*.stories.tsx'],
  staticDirs: ['./fixtures'],
  framework: '@storybook/react-vite',
  addons: ['@storybook/addon-docs', '@storybook/addon-a11y'],
  core: { disableTelemetry: true },
  viteFinal: async (config) => ({
    ...config,
    // unplugin 0.19 appends the complete StyleX rule set to one emitted CSS asset.
    // Keep that asset in the preview head, not behind an arbitrary story import.
    build: { ...config.build, cssCodeSplit: false },
    plugins: [...(config.plugins ?? []), tailwindcss(), stylex.vite({ useCSSLayers: false })],
  }),
}
export default config
