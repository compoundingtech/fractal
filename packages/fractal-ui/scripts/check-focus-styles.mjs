#!/usr/bin/env node
// One focus treatment across the app: the shared ring (geometryVars.focusRing / focusOffset /
// focusInset, interactionVars.focusRing) on keyboard focus only. This guard rejects every other
// outline, ring-shaped box-shadow, Tailwind ring/outline/focus utility and `:focus`/`:focus-within`
// ring, so an ad-hoc or attention outline cannot creep back in. Exemptions live in
// focus-styles.exemptions.json with an owner and a reason, and only shrink.
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
export const repoRoot = join(here, '../../..')
export const roots = ['apps/fractal-web/src', 'packages/fractal-ui/src']

const tokenWidth = /\.focusRing\b/
const tokenOffset = /\.focus(?:Offset|Inset)\b/
const tokenColor = /\b(?!(?:g|geometry|geometryVars)\.)\w+\.focusRing\b/
const neutral = /^(?:'none'|"none"|null|0|'0'|'transparent'|undefined|string)$/

/** Splits a value at top level commas so `{ default: 'none', ':focus-visible': 'solid' }` is checked per branch. */
const leaves = value => {
  const out = []
  const inner = value.trim().startsWith('{') ? value.trim().slice(1, -1) : value
  let depth = 0, start = 0
  for (let i = 0; i < inner.length; i++) {
    const ch = inner[i]
    if ('{[('.includes(ch)) depth++
    else if ('}])'.includes(ch)) depth--
    else if (ch === ',' && depth === 0) { out.push(inner.slice(start, i)); start = i + 1 }
  }
  out.push(inner.slice(start))
  return out.map(part => part.replace(/^\s*(?:\[.*\]|'[^']*'|"[^"]*"|\w+)\s*:\s*/, '').trim()).filter(Boolean)
}

/** Reads the value that follows `key:` up to the next top-level comma or closing brace. */
const valueAt = (text, from) => {
  let depth = 0, i = from
  for (; i < text.length; i++) {
    const ch = text[i]
    if ('{[('.includes(ch)) depth++
    else if ('}])'.includes(ch)) { if (depth === 0) break; depth-- }
    else if ((ch === ',' || ch === '\n') && depth === 0) break
  }
  return text.slice(from, i).trim()
}

const lineOf = (text, index) => text.slice(0, index).split('\n').length

export function checkSource(path, text) {
  const problems = []
  const add = (index, rule, detail) => problems.push({ path, line: lineOf(text, index), rule, detail: detail.slice(0, 120) })
  if (path.endsWith('.css')) {
    for (const match of text.matchAll(/(^|[;{\s])(outline(?:-\w+)?)\s*:\s*([^;}]*)/g)) {
      const value = match[3].trim()
      if (/^none\b/.test(value) || /^0\b/.test(value)) continue
      add(match.index, 'css-outline', `${match[2]}: ${value}`)
    }
    for (const match of text.matchAll(/:focus(?:-within)?\b(?!-visible)/g)) add(match.index, 'css-focus', match[0])
    return problems
  }
  for (const match of text.matchAll(/\boutline(Width|Style|Color|Offset)?\s*:\s*/g)) {
    const kind = match[1] ?? 'Shorthand'
    const value = valueAt(text, match.index + match[0].length)
    // A style named `outline` (a button variant) holds CSS properties, not an outline value.
    if (kind === 'Shorthand' && /^\{[^}]*\b(?:border\w*|backgroundColor|color|padding\w*)\s*:/.test(value)) continue
    for (const leaf of leaves(value)) {
      if (neutral.test(leaf)) continue
      if (kind === 'Style' && /^'(?:solid|none)'$/.test(leaf)) continue
      if (kind === 'Width' && tokenWidth.test(leaf) && !/\d/.test(leaf.replace(/\.focusRing/, ''))) continue
      if (kind === 'Offset' && tokenOffset.test(leaf)) continue
      if (kind === 'Color' && (tokenColor.test(leaf) || /onPrimary\b/.test(leaf))) continue
      if (kind === 'Shorthand' && /^`\$\{\w+\.focusRing\} solid \$\{\w+\.focusRing\}`$/.test(leaf)) continue
      add(match.index, `outline-${kind.toLowerCase()}`, `${match[0]}${leaf}`)
    }
  }
  for (const match of text.matchAll(/boxShadow\s*:\s*/g)) {
    const value = valueAt(text, match.index + match[0].length)
    for (const leaf of leaves(value)) {
      // A spread-only shadow is a ring; anything else (elevation, an indicator line) is not.
      if (!/(?:^|[`'"\s])(?:inset\s+)?0(?:px)?\s+0(?:px)?\s+0(?:px)?\s+[^0\s]/.test(leaf)) continue
      if (tokenColor.test(leaf)) continue
      add(match.index, 'ring-shadow', `boxShadow: ${leaf}`)
    }
  }
  for (const match of text.matchAll(/':focus(?:-within)?'\s*:\s*/g)) {
    const value = valueAt(text, match.index + match[0].length)
    if (/outline|boxShadow|Shadow/.test(value) || /^`[^`]*(?:solid|0 0 0)/.test(value)) add(match.index, 'focus-ring', `${match[0]}${value}`)
  }
  for (const match of text.matchAll(/(?:className|class)\s*=\s*(?:"([^"]*)"|\{`([^`]*)`\})/g)) {
    const classes = (match[1] ?? match[2] ?? '').split(/\s+/)
    for (const name of classes) {
      if (/^(?:[\w-]+:)*(?:outline-none)$/.test(name)) continue
      if (/(?:^|:)(?:ring|ring-offset|outline)(?:-|$)/.test(name) || /(?:^|:)(?:focus|focus-within):/.test(name)) add(match.index, 'tailwind-focus', name)
    }
  }
  return problems
}

const walk = dir => readdirSync(dir).flatMap(name => {
  const path = join(dir, name)
  if (statSync(path).isDirectory()) return name === 'node_modules' || name.startsWith('.') ? [] : walk(path)
  return /\.(?:tsx?|css)$/.test(name) && !/\.(?:test|spec)\.|\.d\.ts$/.test(name) ? [path] : []
})

export function checkRepo(base = repoRoot) {
  const exemptions = JSON.parse(readFileSync(join(here, 'focus-styles.exemptions.json'), 'utf8'))
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
    console.error(`\n${problems.length} focus style(s) outside the shared ring, ${unused.length} stale exemption(s).`)
    process.exit(1)
  }
  console.log('focus styles: every ring uses the shared token')
}
