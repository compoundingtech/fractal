import optimizeLocales from '@react-aria/optimize-locales-plugin'

// English-only until app i18n exists; expand this list and every root I18nProvider together.
export const englishAriaLocales = () => ({ ...optimizeLocales.vite({ locales: ['en-US'] }), enforce: 'pre' })
