#!/usr/bin/env node
// One tooltip tier across the app: the kit TooltipSurface (composition/Controls.tsx), bounded in width and height and
// placed inside the viewport. This guard rejects a raw react-aria `Tooltip`, which brings its own unbounded surface,
// and a native `title=` on a DOM element, which stacks a second, unstyled tooltip over the kit one. Clipped text uses
// TruncatedLabel or useTruncatedTooltip instead. Exemptions live in tooltips.exemptions.json with an owner and a
// reason, and only shrink.
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
export const repoRoot = join(here, '../../..')
export const roots = ['apps/fractal-web/src', 'packages/fractal-ui/src']
/** The kit owns the one raw react-aria Tooltip that every other tooltip renders through. */
export const kitFiles = new Set(['packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'])

const lineOf = (text, index) => text.slice(0, index).split('\n').length

/** Reads a JSX opening tag's attribute text from just after its name to the closing `>` at brace depth 0. */
const attributesAt = (text, from) => {
  let depth = 0, quote = null
  for (let i = from; i < text.length && i < from + 4000; i++) {
    const ch = text[i]
    if (quote !== null) { if (ch === quote && text[i - 1] !== '\\') quote = null; continue }
    if (depth === 0 && (ch === '"' || ch === "'")) { quote = ch; continue }
    if (ch === '{') depth++
    else if (ch === '}') depth--
    else if (ch === '>' && depth === 0) return text.slice(from, i)
    else if (ch === '<' && depth === 0) return null
  }
  return null
}

/** Reads one attribute value: a quoted string or a balanced `{...}` expression. */
const valueAt = (text, from) => {
  if (text[from] === '"' || text[from] === "'") return text.slice(from, text.indexOf(text[from], from + 1) + 1)
  let depth = 0
  for (let i = from; i < text.length; i++) {
    if (text[i] === '{') depth++
    else if (text[i] === '}' && --depth === 0) return text.slice(from, i + 1)
  }
  return text.slice(from)
}

export function checkSource(path, text) {
  const problems = []
  const add = (index, rule, detail) => problems.push({ path, line: lineOf(text, index), rule, detail: detail.replace(/\s+/g, ' ').slice(0, 120) })
  if (!kitFiles.has(path)) {
    for (const match of text.matchAll(/import\s*\{([^}]*)\}\s*from\s*['"]react-aria-components['"]/g)) {
      for (const specifier of match[1].split(',').map(part => part.trim()).filter(Boolean)) {
        if (/^type\s/.test(specifier)) continue
        if (/^Tooltip(?:\s+as\s+\w+)?$/.test(specifier)) add(match.index, 'raw-tooltip', `import { ${specifier} } from 'react-aria-components'`)
      }
    }
    for (const match of text.matchAll(/<(\w+)\.Tooltip\b(?=[\s/>])/g)) add(match.index, 'raw-tooltip', `<${match[1]}.Tooltip>`)
  }
  for (const match of text.matchAll(/<([a-z][a-z0-9]*)(?=[\s/>])/g)) {
    const attributes = attributesAt(text, match.index + match[0].length)
    if (attributes === null) continue
    for (const title of attributes.matchAll(/(?:^|\s)title\s*=\s*/g)) {
      const value = valueAt(attributes, title.index + title[0].length)
      add(match.index, 'native-title', `<${match[1]} title=${value}>`)
    }
  }
  return problems
}

const walk = dir => readdirSync(dir).flatMap(name => {
  const path = join(dir, name)
  if (statSync(path).isDirectory()) return name === 'node_modules' || name.startsWith('.') ? [] : walk(path)
  return /\.tsx$/.test(name) && !/\.(?:test|spec|stories)\.|\.story-/.test(name) ? [path] : []
})

export function checkRepo(base = repoRoot) {
  const exemptions = JSON.parse(readFileSync(join(here, 'tooltips.exemptions.json'), 'utf8'))
  const problems = roots.flatMap(root => walk(join(base, root)).flatMap(path => checkSource(relative(base, path), readFileSync(path, 'utf8'))))
  const key = problem => `${problem.path}#${problem.rule}#${problem.detail}`
  const allowed = new Map(exemptions.map(entry => [`${entry.path}#${entry.rule}#${entry.detail}`, entry]))
  const unused = [...allowed.keys()].filter(entry => !problems.some(problem => key(problem) === entry))
  return { problems: problems.filter(problem => !allowed.has(key(problem))), unused }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { problems, unused } = checkRepo()
  for (const problem of problems) console.error(`${problem.path}:${problem.line} ${problem.rule} ${problem.detail}`)
  for (const entry of unused) console.error(`stale exemption: ${entry}`)
  if (problems.length || unused.length) {
    console.error(`\n${problems.length} tooltip(s) outside the kit surface, ${unused.length} stale exemption(s).`)
    process.exit(1)
  }
  console.log('tooltips: every tooltip renders through the kit surface')
}
