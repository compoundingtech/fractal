import { execFileSync } from 'node:child_process'
import type { StorybookConfig } from '@storybook/react-vite'
import type { Plugin } from 'vite'
import tailwindcss from '@tailwindcss/vite'
import stylex from '@stylexjs/unplugin'

/** Hosted static builds expose the exact source revision at /build-identity.json. */
const buildIdentity = (): Plugin => ({
  name: 'fractal-ui-build-identity',
  apply: 'build',
  generateBundle() {
    const git = (...args: string[]) => execFileSync('git', args, { encoding: 'utf8' }).trim()
    const identity = { sourceKind: 'git', rev: git('rev-parse', 'HEAD'), dirty: git('status', '--porcelain', '--untracked-files=no') !== '' }
    this.emitFile({ type: 'asset', fileName: 'build-identity.json', source: `${JSON.stringify(identity)}\n` })
  },
})

const config: StorybookConfig = {
  stories: ['../src/**/*.stories.tsx'],
  framework: '@storybook/react-vite',
  addons: ['@storybook/addon-docs', '@storybook/addon-a11y'],
  core: { disableTelemetry: true },
  viteFinal: async (config) => ({
    ...config,
    // unplugin 0.19 appends the complete StyleX rule set to one emitted CSS asset.
    // Keep that asset in the preview head, not behind an arbitrary story import.
    build: { ...config.build, cssCodeSplit: false },
    plugins: [...(config.plugins ?? []), tailwindcss(), stylex.vite({ useCSSLayers: false }), buildIdentity()],
  }),
}
export default config
