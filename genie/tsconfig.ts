// Fractal's TypeScript projects start from effect-utils' strict base. They only type-check:
// Vite, Vitest and Node's type stripping run the sources, so every project sets `noEmit` and
// leaves out the base's declaration and output options, which would check an emit that never runs.
import { baseTsconfigCompilerOptions } from '../repos/effect-utils/genie/external.ts'

export { domLib, nodeTypes, reactJsx, tsconfigJson } from '../repos/effect-utils/genie/external.ts'

const { declaration: _declaration, declarationMap: _declarationMap, sourceMap: _sourceMap, outDir: _outDir, ...checkOptions } = baseTsconfigCompilerOptions

/** effect-utils' strict compiler options for a type-check-only project. */
export const fractalCompilerOptions = { ...checkOptions, noEmit: true } as const

/** The base's `@effect/language-service` entry: its gate and diagnostic severities. */
export const [effectLanguageService] = baseTsconfigCompilerOptions.plugins

// Effect code takes time, timers, fetch, logging, randomness and errors from Effect, not from platform globals.
const effectNativeRules = [
  'asyncFunction', 'newPromise', 'globalTimers', 'globalDate', 'globalFetch', 'globalConsole', 'globalRandom', 'cryptoRandomUUID', 'extendsNativeError',
] as const

const effectNativeSeverity = (severity: 'warning' | 'off') => ({
  ...effectLanguageService.diagnosticSeverity,
  ...Object.fromEntries(effectNativeRules.map((rule) => [rule, severity])),
})

/** The base entry with the effect-native rules raised for `include` and off for the `exempt` tests and fixtures. */
export const effectNativeLanguageService = ({ include, exempt }: { readonly include: string[]; readonly exempt: string[] }) => ({
  ...effectLanguageService,
  overrides: [
    { include, options: { diagnosticSeverity: effectNativeSeverity('warning') } },
    { include: exempt, options: { diagnosticSeverity: effectNativeSeverity('off') } },
  ],
})
