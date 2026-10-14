/** @import { Plugin, Rollup } from 'vite' */

const fontFile = /\.(?:woff2?|ttf|otf)$/i
const comments = /\/\*[\s\S]*?\*\//g
const fontFaces = /@font-face\s*\{([^}]*)\}/gi
// Quoted values keep parentheses and the other quote; unquoted ones need them escaped.
const urls = /url\(\s*(?:"([^"]*)"|'([^']*)'|((?:\\.|[^'"()\s\\])+))\s*\)/gi

/** @param {string} url */
const fileNameOf = (url) => url.replace(/\\(.)/g, '$1').split(/[?#]/)[0]?.split('/').pop() ?? url

/**
 * Problems in a finished bundle's stylesheets, one human-readable line each.
 *
 * - Every emitted CSS asset has a rule. A stylesheet whose rules were lost
 *   minifies to a newline, and Vite still emits and links it, so the build
 *   exits zero while the chunk ships nothing.
 * - Every font file a chunk loads is declared by an `@font-face` in CSS loaded
 *   with that chunk: its own stylesheets, those of its static imports, and the
 *   entries' stylesheets. A preloaded file without a declared face never
 *   registers in `document.fonts`, so code waiting for that family fails.
 *
 * @param {Rollup.OutputBundle} bundle
 * @returns {string[]}
 */
const cssAssetProblems = (bundle) => {
  const problems = []
  const chunks = Object.values(bundle).filter((output) => output.type === 'chunk')
  /** @type {Map<string, string>} */
  const stylesheets = new Map()
  for (const output of Object.values(bundle)) {
    if (output.type !== 'asset' || !output.fileName.endsWith('.css')) continue
    const css = typeof output.source === 'string' ? output.source : new TextDecoder().decode(output.source)
    stylesheets.set(output.fileName, css)
    if (css.replace(comments, '').trim() !== '') continue
    const owners = chunks.filter((chunk) => chunk.viteMetadata?.importedCss.has(output.fileName))
    problems.push(
      `${output.fileName} has no CSS rules (${Buffer.byteLength(css)} bytes), yet ${owners.map((chunk) => chunk.fileName).join(', ') || 'the bundle'} loads it`,
    )
  }

  /** Stylesheets a chunk and its static imports load, added to `css`. */
  const collectCss = (/** @type {{ fileName: string, css: Set<string>, seen?: Set<string> }} */ { fileName, css, seen = new Set() }) => {
    const chunk = bundle[fileName]
    if (seen.has(fileName) || chunk?.type !== 'chunk') return css
    seen.add(fileName)
    for (const stylesheet of chunk.viteMetadata?.importedCss ?? []) css.add(stylesheet)
    for (const imported of chunk.imports) collectCss({ fileName: imported, css, seen })
    return css
  }
  /** @type {Set<string>} */
  const entryCss = new Set()
  for (const chunk of chunks) if (chunk.isEntry) collectCss({ fileName: chunk.fileName, css: entryCss })

  for (const chunk of chunks) {
    const fonts = [...(chunk.viteMetadata?.importedAssets ?? [])].filter((asset) => fontFile.test(asset))
    if (fonts.length === 0) continue
    const loaded = collectCss({ fileName: chunk.fileName, css: new Set(entryCss) })
    const declared = new Set()
    for (const stylesheet of loaded) {
      for (const [, face = ''] of (stylesheets.get(stylesheet) ?? '').matchAll(fontFaces)) {
        for (const [, double, single, bare] of face.matchAll(urls)) declared.add(fileNameOf(double ?? single ?? bare ?? ''))
      }
    }
    for (const font of fonts) {
      if (declared.has(fileNameOf(font))) continue
      problems.push(
        `${chunk.fileName} loads ${font}, but no @font-face in its stylesheets (${[...loaded].join(', ') || 'none'}) declares it`,
      )
    }
  }
  return problems
}

/**
 * Fails the production build when an emitted stylesheet is empty or a loaded
 * font has no face. Runs after Vite's CSS plugin has emitted every stylesheet.
 *
 * @returns {Plugin}
 */
export const fractalCssAssets = () => ({
  name: 'fractal-css-assets',
  apply: 'build',
  enforce: 'post',
  generateBundle(_options, bundle) {
    const problems = cssAssetProblems(bundle)
    if (problems.length > 0) this.error(`[fractal:css-assets]\n${problems.join('\n')}`)
  },
})
