import optimizeLocales from '@react-aria/optimize-locales-plugin'

// English-only until app i18n exists; expand this list and every root I18nProvider together.
export const englishAriaLocales = () => [
  { ...optimizeLocales.vite({ locales: ['en-US'] }), enforce: 'pre' },
  {
    name: 'fractal-english-aria-graph',
    generateBundle(_options, bundle) {
      const locales = Object.values(bundle).flatMap(output => output.type === 'chunk'
        ? Object.entries(output.modules).filter(([id, module]) => module.renderedLength > 0 &&
          /\/(?:@react-aria\/[^/]+|react-aria)\/.*\/(?:intl|i18n)\/.*[a-z]{2}-[A-Z]{2}\.m?js$/.test(id)).map(([id]) => id)
        : [])
      if (!locales.some(id => id.endsWith('/en-US.mjs') || id.endsWith('/en-US.js')))
        this.error('React Aria locale graph has no English witness')
      const unsupported = locales.filter(id => !/\/en-US\.m?js$/.test(id))
      if (unsupported.length) this.error(`Unsupported React Aria locales retained: ${unsupported.join(', ')}`)
    },
  },
]
