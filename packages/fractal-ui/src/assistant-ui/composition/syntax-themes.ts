import * as stylex from '@stylexjs/stylex'

/**
 * Candidate syntax/diff palettes for code blocks and diffs (the user picks one in the
 * "Kit/DiffView/Theme" story). Markdown code blocks and the kit DiffView both read
 * `--fractal-syntax-*`. `fractal` leaves them unset, so each falls back to Fractal's semantic ink
 * where it is used (and so follows every token theme). The Pierre palettes come from
 * @pierre/theme 2.0.0: pierre-{dark,light} (`pierre`) and pierre-{dark,light}-soft (`pierreSoft`),
 * reduced to @pierre/diffs' CSS-variable categories (types share the function colour, as there).
 */
export type SyntaxThemeName = 'fractal' | 'pierre' | 'pierreSoft'

export const syntaxThemeStyles = stylex.create({
  pierreDark: {
    '--fractal-syntax-fg': '#fafafa', '--fractal-syntax-comment': '#737373', '--fractal-syntax-keyword': '#ff678d',
    '--fractal-syntax-string': '#5ecc71', '--fractal-syntax-constant': '#68cdf2', '--fractal-syntax-func': '#9d6afb',
    '--fractal-syntax-parameter': '#a3a3a3', '--fractal-syntax-punctuation': '#737373', '--fractal-syntax-link': '#5ecc71',
    '--fractal-syntax-inserted': '#07c480', '--fractal-syntax-deleted': '#ff2e3f',
    '--fractal-syntax-addition': '#07c480', '--fractal-syntax-deletion': '#ff2e3f',
  },
  pierreLight: {
    '--fractal-syntax-fg': '#0a0a0a', '--fractal-syntax-comment': '#737373', '--fractal-syntax-keyword': '#d32a61',
    '--fractal-syntax-string': '#199f43', '--fractal-syntax-constant': '#1ca1c7', '--fractal-syntax-func': '#693acf',
    '--fractal-syntax-parameter': '#636363', '--fractal-syntax-punctuation': '#737373', '--fractal-syntax-link': '#199f43',
    '--fractal-syntax-inserted': '#18a46c', '--fractal-syntax-deleted': '#d52c36',
    '--fractal-syntax-addition': '#18a46c', '--fractal-syntax-deletion': '#d52c36',
  },
  pierreSoftDark: {
    '--fractal-syntax-fg': '#d4d4d4', '--fractal-syntax-comment': '#636363', '--fractal-syntax-keyword': '#ff91a8',
    '--fractal-syntax-string': '#8cda94', '--fractal-syntax-constant': '#96d9f6', '--fractal-syntax-func': '#ba8ffd',
    '--fractal-syntax-parameter': '#8a8a8a', '--fractal-syntax-punctuation': '#636363', '--fractal-syntax-link': '#8cda94',
    '--fractal-syntax-inserted': '#60d199', '--fractal-syntax-deleted': '#ff6762',
    '--fractal-syntax-addition': '#60d199', '--fractal-syntax-deletion': '#ff6762',
  },
  pierreSoftLight: {
    '--fractal-syntax-fg': '#525252', '--fractal-syntax-comment': '#8a8a8a', '--fractal-syntax-keyword': '#ff678d',
    '--fractal-syntax-string': '#0dbe4e', '--fractal-syntax-constant': '#08c0ef', '--fractal-syntax-func': '#9d6afb',
    '--fractal-syntax-parameter': '#737373', '--fractal-syntax-punctuation': '#8a8a8a', '--fractal-syntax-link': '#0dbe4e',
    '--fractal-syntax-inserted': '#07c480', '--fractal-syntax-deleted': '#ff2e3f',
    '--fractal-syntax-addition': '#07c480', '--fractal-syntax-deletion': '#ff2e3f',
  },
})

type ThemeStyle = (typeof syntaxThemeStyles)[keyof typeof syntaxThemeStyles]
export const syntaxThemes: Readonly<Record<SyntaxThemeName, { readonly label: string; readonly dark?: ThemeStyle; readonly light?: ThemeStyle }>> = {
  fractal: { label: 'Fractal (current semantic ink)' },
  pierre: { label: 'Pierre default', dark: syntaxThemeStyles.pierreDark, light: syntaxThemeStyles.pierreLight },
  pierreSoft: { label: 'Pierre soft', dark: syntaxThemeStyles.pierreSoftDark, light: syntaxThemeStyles.pierreSoftLight },
}
