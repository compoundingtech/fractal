// Paths are relative to this Storybook configuration directory.
export const storyGlobs: string[] = [
  '../src/**/*.stories.tsx',
  // Archived app CSFs use `.legacy-stories.tsx`, intentionally excluded by this `.stories.tsx` glob.
  '../../../apps/fractal-web/src/**/*.stories.tsx',
]
