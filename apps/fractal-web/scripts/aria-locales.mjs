import optimizeLocales from '@react-aria/optimize-locales-plugin'

// Match optimize-locales-plugin 2.1.0's package scope and locale extensions
// (LocalesPlugin.js); it does not strip @internationalized packages.
const localeModule = /[/\\](?:@react-stately|@react-aria|@react-spectrum|@adobe[/\\]react-spectrum|react-stately|react-aria|react-aria-components)[/\\].*[/\\][a-z]{2}-[A-Z]{2}\.(?:json|mjs|js|cjs)$/
const englishModule = /[/\\]en-US\.(?:json|mjs|js|cjs)$/

// English-only until app i18n exists; expand this list and every root I18nProvider together.
export const englishAriaLocales = () => [
  { ...optimizeLocales.vite({ locales: ['en-US'] }), enforce: 'pre' },
  {
    name: 'fractal-english-aria-graph',
    generateBundle(_options, bundle) {
      const locales = Object.values(bundle).flatMap(output => output.type === 'chunk'
        ? Object.entries(output.modules).filter(([id, module]) => module.renderedLength > 0 &&
          localeModule.test(id)).map(([id]) => id)
        : [])
      if (!locales.some(id => englishModule.test(id)))
        this.error('React Aria locale graph has no English witness')
      const unsupported = locales.filter(id => !englishModule.test(id))
      if (unsupported.length) this.error(`Unsupported React Aria locales retained: ${unsupported.join(', ')}`)
    },
  },
]
