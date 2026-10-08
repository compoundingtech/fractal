import { defineConfig } from 'vite'
import { execFileSync } from 'node:child_process'
import react from '@vitejs/plugin-react'
import { createStylexVitePlugins } from './scripts/stylex.mjs'
import { webfractalGateway } from './scripts/vite-gateway.mjs'
import { extensionBuild } from './scripts/extensions-build.mjs'

/**
 * `--mode scenario` serves and builds the scenario entry (`scenario.html`): the same app and
 * data layer, but the page owns no early roster socket and the runtime binds the scenario kit's
 * replay seams. Every other mode keeps the live `index.html` entry.
 */
export default defineConfig(({ mode }) => ({
  root: 'src/web',
  plugins: [
    {
      name: 'fractal-build-identity',
      resolveId(id) {
        if (id === 'virtual:build-identity') return '\0fractal-build-identity'
      },
      load(id) {
        if (id !== '\0fractal-build-identity') return
        const rev = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
        return (
          'export const buildIdentity=' +
          JSON.stringify({ baseVersion: '0.1.0', machineVersion: '0.1.0+' + rev.slice(0, 12), gitRev: rev }) +
          ';export const deploymentId=undefined;'
        )
      },
    },
    extensionBuild({ entry: '../src/extensions/app-public.ts', privateBuild: false }),
    ...createStylexVitePlugins({
      externalPackages: ['@smalltalk/fractal-ui'],
      entries: [
        new URL('./src/web/main.tsx', import.meta.url).pathname,
        new URL('./src/web/scenarioMain.tsx', import.meta.url).pathname,
      ],
    }),
    react(),
    webfractalGateway(),
  ],
  server: {
    strictPort: true,
    host: '127.0.0.1',
    allowedHosts: (process.env.WF_ALLOWED_HOSTS ?? '127.0.0.1,localhost').split(','),
    ...(mode === 'scenario' ? { open: '/scenario.html' } : {}),
  },
  resolve: { dedupe: ['effect', 'react', 'react-dom'] },
  ...(mode === 'scenario'
    ? {
        build: {
          outDir: '../../dist/web-scenario',
          emptyOutDir: true,
          rollupOptions: { input: { scenario: new URL('./src/web/scenario.html', import.meta.url).pathname } },
        },
      }
    : { build: { outDir: '../../dist/web', emptyOutDir: true } }),
}))
