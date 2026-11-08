import { devbarTokens } from '@overeng/devbar/tokens.stylex'
import * as stylex from '@stylexjs/stylex'

// Direct modules, not the shell barrel: createTheme values must resolve statically, and the barrel loads the whole kit.
import { lightTheme as compositionLightTheme } from '../../../../packages/fractal-ui/src/assistant-ui/composition-theme.ts'
import { colorVars, interactionVars, typeVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

/**
 * The devbar paints from the app's own palette, so it follows the app's light, dark and System
 * scheme and its focus ring shares the app's ink. This is one complete theme for the devbar's
 * vars: StyleX keeps only the last theme per vars group, so stacking a partial override on the
 * kit's light or dark theme would reset every other devbar var to the kit's light defaults.
 */
const appDevbarTheme = stylex.createTheme(devbarTokens, {
  canvas: colorVars.sidebar,
  panel: colorVars.raised,
  panelActive: colorVars.rowActive,
  border: colorVars.borderStrong,
  text: colorVars.fg,
  mutedText: colorVars.fgMuted,
  focusRing: interactionVars.focusRing,
  fontUi: typeVars.fontSans,
  fontData: typeVars.fontMono,
})

/**
 * Themes for the devbar's host element. The app palette's defaults are dark; light applies the
 * app's light theme on the host first, so the devbar theme resolves against it.
 */
export const devbarTheme = (dark: boolean) => [dark ? null : compositionLightTheme, appDevbarTheme] as const
