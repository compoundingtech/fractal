import { scenarioGlobalTypes, scenarioLoaders, withScenario } from '@fractal/scenarios/storybook'
import type { Preview } from '@storybook/react-vite'
import * as React from 'react'
import { I18nProvider } from 'react-aria/I18nProvider'
import '../src/tokens.css'

const preview: Preview = {
  globalTypes: scenarioGlobalTypes,
  decorators: [(Story) => React.createElement(I18nProvider, { locale: 'en-US' }, React.createElement(Story)), withScenario],
  loaders: scenarioLoaders,
  parameters: {
    layout: 'fullscreen',
    controls: { expanded: true },
    a11y: { test: 'error' },
  },
}
export default preview
