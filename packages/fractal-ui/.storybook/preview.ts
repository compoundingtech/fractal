import { scenarioGlobalTypes, scenarioLoaders, withScenario } from '@fractal/scenarios/storybook'
import type { Preview } from '@storybook/react-vite'
import '../src/tokens.css'

const preview: Preview = {
  globalTypes: scenarioGlobalTypes,
  decorators: [withScenario],
  loaders: scenarioLoaders,
  parameters: {
    layout: 'fullscreen',
    controls: { expanded: true },
    a11y: { test: 'error' },
  },
}
export default preview
