// Paths are relative to this Storybook configuration directory.
export const storyGlobs: string[] = [
  '../src/**/*.stories.tsx',
  '../../../apps/fractal-web/src/web/ColdOpen.stories.tsx',
  '../../../apps/fractal-web/src/web/NarrowLayout.stories.tsx',
  '../../../apps/fractal-web/src/web/ResponsiveAgentSidebar.stories.tsx',
  '../../../apps/fractal-web/src/web/SidebarChrome.stories.tsx',
  '../../../apps/fractal-web/src/web/SidebarTreeBinding.stories.tsx',
  // The live roster story replays captured daemon payloads through the SDK decode.
  '../../../apps/fractal-web/src/web/LiveRoster.stories.tsx',
]
