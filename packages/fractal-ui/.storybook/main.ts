/// <reference path="../../../apps/fractal-web/src/vite-env.d.ts" />
import type { StorybookConfig } from '@storybook/react-vite'
import type { Plugin } from 'vite'
import type { CliBuildIdentity } from 'virtual:build-identity'
import tailwindcss from '@tailwindcss/vite'
import stylex from '@stylexjs/unplugin'

// App stories import virtual:build-identity, which only the app Vite config defines; Storybook gets a synthetic one.
const storybookIdentity: CliBuildIdentity = { machineVersion: '0.0.0+storybook', sourceKind: 'storybook', dirty: false }
const buildIdentity: Plugin = {
  name: 'storybook-build-identity',
  resolveId: (id) => (id === 'virtual:build-identity' ? '\0storybook-build-identity' : undefined),
  load: (id) =>
    id === '\0storybook-build-identity'
      ? `export const buildIdentity=${JSON.stringify(storybookIdentity)};export const deploymentId=undefined;`
      : undefined,
}

const config: StorybookConfig = {
  // App stories are discovered by glob so adding one never edits this kit file.
  stories: ['../src/**/*.stories.tsx', '../../../apps/fractal-web/src/**/*.stories.tsx'],
  framework: '@storybook/react-vite',
  addons: ['@storybook/addon-docs', '@storybook/addon-a11y'],
  core: { disableTelemetry: true },
  viteFinal: async (config) => ({ ...config, plugins: [...(config.plugins ?? []), buildIdentity, tailwindcss(), stylex.vite({ useCSSLayers: false })] }),
}
export default config
