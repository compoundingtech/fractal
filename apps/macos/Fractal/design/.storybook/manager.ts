import { addons } from 'storybook/manager-api'
import { themes } from 'storybook/theming'

// The manager follows macOS appearance. The theme is read once at startup, so a switch reloads.
const media = window.matchMedia('(prefers-color-scheme: dark)')
addons.setConfig({ theme: media.matches ? themes.dark : themes.light })
media.addEventListener('change', () => window.location.reload())
