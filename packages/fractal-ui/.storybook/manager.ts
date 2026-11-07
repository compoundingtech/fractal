import { addons } from 'storybook/manager-api'
import { create } from 'storybook/theming'

// The chrome follows the app's default dark look; the canvas follows the Theme toolbar.
addons.setConfig({
  theme: create({
    base: 'dark',
    brandTitle: 'Fractal',
    brandUrl: './',
    brandTarget: '_self',
    colorPrimary: '#346bf1',
    colorSecondary: '#346bf1',
    appBg: '#000000',
    appContentBg: '#0a0a0a',
    appPreviewBg: '#0a0a0a',
    appBorderColor: 'rgba(255, 255, 255, 0.08)',
    appBorderRadius: 6,
    barBg: '#000000',
    barTextColor: '#a3a3a3',
    barSelectedColor: '#f5f5f5',
    barHoverColor: '#f5f5f5',
    textColor: '#f5f5f5',
    textMutedColor: '#a3a3a3',
    textInverseColor: '#0a0a0a',
    inputBg: '#111111',
    inputBorder: 'rgba(255, 255, 255, 0.08)',
    inputTextColor: '#f5f5f5',
    inputBorderRadius: 6,
    fontBase: `-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif`,
    fontCode: `ui-monospace, 'SF Mono', Menlo, Consolas, monospace`,
  }),
})
