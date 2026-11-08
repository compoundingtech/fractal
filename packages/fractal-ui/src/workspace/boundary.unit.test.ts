import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { describe, expect, it } from 'vitest'

/** The module stays unstyled and self-contained: no styling, no app or kit imports, only these packages. */
const allowed = /^(effect(\/.*)?|react|react\/jsx-runtime|react-dom(\/client)?|react-aria|react-stately|vitest|node:(fs|path))$/

const files = (directory: string): string[] => readdirSync(directory).flatMap(name => {
  const path = join(directory, name)
  return statSync(path).isDirectory() ? files(path) : /\.tsx?$/.test(name) ? [path] : []
})

describe('the workspace module boundary', () => {
  const root = import.meta.dirname
  for (const file of files(root).filter(path => !path.endsWith('boundary.unit.test.ts'))) {
    it(`${relative(root, file)} imports only relative files and allowed packages`, () => {
      const source = readFileSync(file, 'utf8')
      const imports = [...source.matchAll(/(?:^|\n)\s*(?:import|export)[^'"\n]*?from\s+'([^']+)'/g)].map(match => match[1]!)
      for (const specifier of imports) {
        if (specifier.startsWith('.')) expect(join(file, '..', specifier).startsWith(root), specifier).toBe(true)
        else expect(specifier, specifier).toMatch(allowed)
      }
      expect(source, 'styling is the host\'s').not.toMatch(/stylex|\.css'/)
    })
  }
})
