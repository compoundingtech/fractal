import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { createStylexVitePlugins } from './scripts/stylex.mjs'
import { pierreLanguagesPlugin } from '../../packages/fractal-ui/scripts/pierre-languages.mjs'

export default defineConfig({
  base: './',
  plugins: [
    pierreLanguagesPlugin(),
    ...createStylexVitePlugins({
      externalPackages: ['@fractal/ui', '@overeng/devbar', '@overeng/meters', '@overeng/stylex-tokens'],
      entries: [new URL('./src/embed/main.tsx', import.meta.url).pathname],
    }),
    react(),
  ],
  resolve: {
    alias: [{ find: /^\.\/storageAdapter\.ts$/, replacement: new URL('./src/embed/memoryStorageAdapter.ts', import.meta.url).pathname }],
    dedupe: ['effect', 'react', 'react-dom', '@stylexjs/stylex'],
  },
  build: { outDir: 'dist-embed', emptyOutDir: true, rolldownOptions: { input: 'embed.html' } },
})
