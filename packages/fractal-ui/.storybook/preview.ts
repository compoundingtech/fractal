import { scenarioGlobalTypes, scenarioLoaders, withScenario } from '@fractal/scenarios/storybook'
import type { Decorator, Preview } from '@storybook/react-vite'
import * as React from 'react'
import { I18nProvider } from 'react-aria/I18nProvider'
import '../src/tokens.css'

/**
 * An explicit Theme toolbar choice overrides a story's scheme args. Unset leaves the story and
 * its URL args untouched, which is how the play gate selects its scheme.
 */
const withThemeToolbar: Decorator = (Story, context) => {
  const theme: unknown = context.globals.theme
  if (theme !== 'dark' && theme !== 'light') return React.createElement(Story)
  const args = {
    ...context.args,
    ...('scheme' in context.args ? { scheme: theme } : {}),
    ...(typeof context.args.dark === 'boolean' ? { dark: theme === 'dark' } : {}),
  }
  return React.createElement(Story, { args })
}

const preview: Preview = {
  globalTypes: {
    ...scenarioGlobalTypes,
    theme: {
      name: 'Theme',
      description: 'Light or dark, overriding the story default',
      toolbar: {
        icon: 'mirror',
        dynamicTitle: true,
        items: [
          { type: 'reset', title: 'Story default' },
          { value: 'dark', title: 'Dark' },
          { value: 'light', title: 'Light' },
        ],
      },
    },
  },
  decorators: [(Story) => React.createElement(I18nProvider, { locale: 'en-US' }, React.createElement(Story)), withScenario, withThemeToolbar],
  loaders: scenarioLoaders,
  parameters: {
    layout: 'fullscreen',
    controls: { expanded: true },
    a11y: { test: 'error' },
    // Selected per review session; no default, so gate cases keep their real window size.
    viewport: {
      options: {
        phone: { name: 'Phone 390', type: 'mobile', styles: { width: '390px', height: '844px' } },
        narrow: { name: 'Narrow 440', type: 'mobile', styles: { width: '440px', height: '900px' } },
        tablet: { name: 'Tablet 899', type: 'tablet', styles: { width: '899px', height: '1024px' } },
        laptop: { name: 'Laptop 1100', type: 'desktop', styles: { width: '1100px', height: '800px' } },
        desktop: { name: 'Desktop 1440', type: 'desktop', styles: { width: '1440px', height: '900px' } },
      },
    },
    options: {
      storySort: { order: ['Start', 'Screens', 'Surfaces', 'Components', 'Foundations', '*'] },
    },
  },
}
export default preview
