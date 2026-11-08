import { oxlintConfig } from './repos/effect-utils/genie/external.ts'
import {
  baseOxlintCategories,
  baseOxlintIgnorePatterns,
  baseOxlintOverrides,
  baseOxlintPlugins,
  baseOxlintRules,
  stylexOxlintRules,
} from './repos/effect-utils/genie/oxlint-base.ts'

/**
 * The effect-utils shared oxlint policy for the TypeScript web packages, unchanged. Existing findings
 * are ratcheted per package and rule by `scripts/ci-fractal-web-effect-lint`, so new code complies
 * while the backlog burns down. `jsPlugins` names the member's plugin sources for editors; the
 * ratchet substitutes the prebuilt effect-utils products in `nix develop .#web`.
 */
export default oxlintConfig({
  plugins: baseOxlintPlugins,
  jsPlugins: [
    './repos/effect-utils/packages/@overeng/oxc-config/src/mod.ts',
    './repos/effect-utils/packages/@overeng/oxc-config/src/stylex-upstream-plugin.ts',
  ],
  categories: baseOxlintCategories,
  rules: { ...baseOxlintRules, ...stylexOxlintRules() },
  ignorePatterns: [
    ...baseOxlintIgnorePatterns,
    // Generated clients and tables are checked by their generators' freshness gates.
    '**/*.generated.ts',
    '**/*.generated.mts',
  ],
  overrides: [...baseOxlintOverrides],
})
