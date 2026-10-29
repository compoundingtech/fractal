// One palette, named by meaning, the same one the Fractal terminal UI uses (crates/fractal-tui/src/ui/theme.rs): Catppuccin
// Mocha. Mauve is reserved for "a person is needed" and nothing else uses it.
export const palette = {
  base: '#1e1e2e',
  mantle: '#181825',
  crust: '#11111b',
  surface0: '#313244',
  surface1: '#45475a',
  surface2: '#585b70',
  overlay0: '#6c7086',
  overlay1: '#7f849c',
  subtext0: '#a6adc8',
  subtext1: '#bac2de',
  text: '#cdd6f4',
  lavender: '#b4befe',
  blue: '#89b4fa',
  sapphire: '#74c7ec',
  teal: '#94e2d5',
  green: '#a6e3a1',
  yellow: '#f9e2af',
  peach: '#fab387',
  red: '#f38ba8',
  mauve: '#cba6f7',
} as const;

type Palette = Record<keyof typeof palette, string>;

// The light counterpart for Fractal terminal UI-drawn screens that follow the system appearance: Catppuccin
// Latte's surfaces, with every text and accent colour darkened until it reads at WCAG AA
// (at least 4.5:1) on base, mantle, surface0 and the user and tool fills. The ramp keeps Mocha's
// order, text darkest and overlay0 lightest, so dim stays dim.
export const lightPalette: Palette = {
  base: '#eff1f5',
  mantle: '#e6e9ef',
  crust: '#dce0e8',
  surface0: '#d8dbe4',
  surface1: '#bcc0cc',
  surface2: '#9ca0b0',
  overlay0: '#5a5d72',
  overlay1: '#50536a',
  subtext0: '#46495f',
  subtext1: '#3e4157',
  text: '#2f3146',
  lavender: '#3f4dbd',
  blue: '#1a56c9',
  sapphire: '#09657a',
  teal: '#096868',
  green: '#276619',
  yellow: '#7f5400',
  peach: '#9e3b08',
  red: '#b8102f',
  mauve: '#7a2ed6',
};

/** Fills the palette does not name: the person's own messages, and tool calls by outcome. */
type Fills = { userBg: string; toolBg: string; toolOkBg: string; toolErrBg: string };
/** The Fractal terminal UI palette by meaning, in either appearance. */
export type Theme = Palette & Fills & {
  /** Focus, the selected tab, links. */
  accent: string;
  /** A person is needed. Reserved. */
  person: string;
  working: string;
  idle: string;
  done: string;
  fault: string;
  waiting: string;
  quiet: string;
  rowSelected: string;
};

function named(colors: Palette, fills: Fills): Theme {
  return {
    ...colors,
    ...fills,
    accent: colors.blue,
    person: colors.mauve,
    working: colors.peach,
    idle: colors.green,
    done: colors.teal,
    fault: colors.red,
    waiting: colors.yellow,
    quiet: colors.overlay0,
    rowSelected: colors.surface0,
  };
}

export const theme: Theme = named(palette, { userBg: '#28293d', toolBg: '#232436', toolOkBg: '#1f2b2a', toolErrBg: '#33222c' });
export const lightTheme: Theme = named(lightPalette, { userBg: '#e2e5ef', toolBg: '#e8eaf1', toolOkBg: '#e1eee4', toolErrBg: '#f6e3e8' });

// Callers name colours by their Mocha token (`theme.accent`, a conversation rule's `tool_bg`);
// a screen drawn in the light palette repaints each one as the same token there.
const mochaToLight: Record<string, string> = Object.fromEntries(Object.keys(theme).map(key => [theme[key as keyof Theme], lightTheme[key as keyof Theme]]));
export function inLight(color: string): string {
  return mochaToLight[color] ?? color;
}

// IBM Plex Mono, loaded by the app at start. The names are the keys given to `useFonts`.
export const fonts = {
  regular: 'IBMPlexMono_400Regular',
  italic: 'IBMPlexMono_400Regular_Italic',
  semibold: 'IBMPlexMono_600SemiBold',
  bold: 'IBMPlexMono_700Bold',
} as const;

/** System appearance for migrated iPhone surfaces; legacy tabs keep their palette. */
export const systemColors = {
  light: { background: '#f2f2f7', grouped: '#ffffff', text: '#000000', secondary: '#59595f', separator: '#c6c6c8', action: '#005fcc', attention: '#9c4400', fault: '#c00f20', success: '#216e35' },
  dark: { background: '#000000', grouped: '#1c1c1e', text: '#ffffff', secondary: '#b5b5bd', separator: '#48484a', action: '#64aaff', attention: '#ffb15c', fault: '#ff6961', success: '#72d88a' },
} as const;

// Fractal terminal UI colours each harness the same way everywhere.
export function harnessColor(harness: string): string {
  switch (harness) {
    case 'claude': return palette.peach;
    case 'codex': return palette.blue;
    case 'omp': return palette.teal;
    case 'pi': return palette.lavender;
    default: return palette.overlay0;
  }
}
