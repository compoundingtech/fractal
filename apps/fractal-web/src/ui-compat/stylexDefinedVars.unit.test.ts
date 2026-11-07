// @vitest-environment node
import { transformAsync } from '@babel/core'
import stylexBabelPlugin from '@stylexjs/babel-plugin'
import { readdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { expect, it } from 'vitest'

/**
 * Importing any name from a `*.stylex` module compiles to `var(--hash)`, whatever that name holds. A plain object
 * exported there therefore loses every value silently: no build error, just rules with no definition behind them.
 * Compiles the app and kit the way the build does, then requires a definition for each hashed variable reference.
 */
const repoRoot = resolve(import.meta.dirname, '../../../..')
const sourceRoots = ['apps/fractal-web/src', 'packages/fractal-ui/src', 'node_modules/@overeng/devbar/src', 'node_modules/@overeng/meters/src', 'node_modules/@overeng/stylex-tokens/src']
const excluded = /\.(unit|integration|browser)\.test\.|\.test\.|\.d\.ts$/

const sourceFiles = (root: string): readonly string[] =>
  (readdirSync(join(repoRoot, root), { recursive: true, encoding: 'utf8' }) as readonly string[])
    .filter(file => /\.tsx?$/.test(file) && !excluded.test(file))
    .map(file => join(repoRoot, root, file))

type CompiledRule = Parameters<typeof stylexBabelPlugin.processStylexRules>[0][number]

const compiledRules = async (): Promise<readonly CompiledRule[]> => {
  const files = sourceRoots.flatMap(sourceFiles).filter(file => readFileSync(file, 'utf8').includes('@stylexjs/stylex'))
  const compiled = await Promise.all(files.map(file => transformAsync(readFileSync(file, 'utf8'), {
    filename: file, babelrc: false, configFile: false, code: false,
    parserOpts: { plugins: ['typescript', 'jsx'] },
    plugins: [[stylexBabelPlugin, { dev: false, runtimeInjection: false, unstable_moduleResolution: { type: 'commonJS', rootDir: repoRoot } }]],
  })))
  return compiled.flatMap(result => (result?.metadata as { readonly stylex?: readonly CompiledRule[] } | undefined)?.stylex ?? [])
}

/** Hashed StyleX names only: `--x-*` are inline dynamic values, and a fallback already states its own default. */
const undefinedVariables = (css: string): readonly string[] => {
  const names = (pattern: RegExp) => new Set(Array.from(css.matchAll(pattern), match => match[1] ?? ''))
  const defined = names(/(--x[a-z0-9]+)\s*:/g)
  const referenced = names(/var\((--x[a-z0-9]+)\s*\)/g)
  return [...referenced].filter(name => !defined.has(name)).sort()
}

it('finds a reference with no definition', () => {
  expect(undefinedVariables('.a{padding:var(--x39pkxx)}:root{--x1abc:4px}.b{gap:var(--x1abc)}.c{flex-basis:var(--x-flexBasis)}.d{top:var(--xmissing,0)}')).toEqual(['--x39pkxx'])
})

it('defines every hashed custom property the compiled styles reference', async () => {
  const rules = await compiledRules()
  expect(rules.length).toBeGreaterThan(1000)
  const css = stylexBabelPlugin.processStylexRules([...rules], { useLayers: false })
  expect(css).toContain('var(--x')
  expect(undefinedVariables(css)).toEqual([])
}, 180_000)
