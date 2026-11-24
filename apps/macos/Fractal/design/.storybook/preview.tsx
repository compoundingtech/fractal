import type { Preview } from '@storybook/react'

import { AppearanceProvider, type AppearanceChoice } from '../src/appearance.tsx'
import '../src/panel.css'

const preview: Preview = {
  globalTypes: {
    appearance: {
      description: 'Appearance of the renders: follow macOS, or force Light or Dark',
      toolbar: {
        title: 'Appearance',
        icon: 'mirror',
        items: [
          { value: 'system', title: 'System', icon: 'browser' },
          { value: 'light', title: 'Light', icon: 'sun' },
          { value: 'dark', title: 'Dark', icon: 'moon' },
        ],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: { appearance: 'system' },
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    // oxlint-disable-next-line overeng/named-args -- Storybook calls decorators positionally (Story, context)
    (Story, context) => (
      <AppearanceProvider choice={context.globals['appearance'] as AppearanceChoice | undefined}>
        <Story />
      </AppearanceProvider>
    ),
  ],
}

export default preview
