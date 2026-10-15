import type { Preview } from '@storybook/react-vite'
import '../src/web/app.css'
import { setTheme } from '../src/ui-compat/foundations.ts'

setTheme('system')

const preview: Preview = {
  parameters: {
    layout: 'fullscreen',
    controls: { expanded: true },
    a11y: { test: 'error' },
  },
}
export default preview
