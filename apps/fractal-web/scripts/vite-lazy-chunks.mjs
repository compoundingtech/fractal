import path from 'node:path'

/** @import { Plugin } from 'vite' */
/** Embed a same-build manifest subset: no runtime filename guessing or mutable manifest URL.
 * @param {Record<string, string>} entries Absolute source paths, keyed by feature.
 * @returns {Plugin}
 */
export const fractalLazyChunks = (entries) => {
  let base = '/'
  return {
    name: 'fractal-lazy-chunks',
    apply: 'build',
    enforce: 'post',
    configResolved(config) { base = config.base },
    generateBundle(_options, bundle) {
      const chunks = Object.values(bundle).filter(output => output.type === 'chunk')
      /** @type {Record<string, string[]>} */
      const manifest = {}
      for (const [feature, source] of Object.entries(entries)) {
        const root = chunks.find(chunk => chunk.facadeModuleId === source || source in chunk.modules)
        if (!root) this.error(`No emitted lazy chunk for ${feature}`)
        const assets = new Set()
        /** @param {string} file */
        const visit = file => {
          if (assets.has(file)) return
          const chunk = bundle[file]
          if (chunk?.type !== 'chunk') return
          assets.add(file)
          for (const imported of chunk.imports) visit(imported)
          for (const css of chunk.viteMetadata?.importedCss ?? []) assets.add(css)
        }
        visit(root.fileName)
        manifest[feature] = [...assets]
      }
      for (const output of Object.values(bundle)) {
        if (output.type !== 'asset' || !output.fileName.endsWith('.html')) continue
        const resolved = Object.fromEntries(Object.entries(manifest).map(([feature, assets]) => [feature,
          assets.map(file => base === './' || base === ''
            ? `./${path.posix.relative(path.posix.dirname(output.fileName), file)}`
            : `${base}${file}`),
        ]))
        const json = JSON.stringify(resolved).replaceAll('<', '\\u003c')
        const html = typeof output.source === 'string' ? output.source : new TextDecoder().decode(output.source)
        output.source = html.replace('</head>', `<script type="application/json" id="fractal-lazy-chunk-assets">${json}</script></head>`)
      }
    },
  }
}
