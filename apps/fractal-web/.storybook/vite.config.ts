import { defineConfig } from 'vite'
import { createStylexVitePlugins } from '../scripts/stylex.mjs'
import { fractalBuildIdentity } from '../scripts/vite-build-identity.mjs'

// Reuse production styling and checkout identity without starting the app's
// live gateway, extension bundler or HTTP-host-only asset/CSP plugins.
export default defineConfig({
  plugins: [
    fractalBuildIdentity(),
    ...createStylexVitePlugins({
      externalPackages: ['@smalltalk/fractal-ui'],
      entries: [new URL('./preview.ts', import.meta.url).pathname],
    }),
  ],
  resolve: { dedupe: ['effect', 'react', 'react-dom'] },
})
