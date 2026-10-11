import { lstat, readdir, readFile } from 'node:fs/promises'
import { join, resolve, relative } from 'node:path'
import { pathToFileURL } from 'node:url'
import { parseArgs } from 'node:util'
import { forbiddenLine } from '../../../apps/fractal-web/scripts/synthetic-fixtures/scan.mjs'

const decode = line => {
  const text = line.replace(/\\u([0-9a-f]{4})/gi, (_, hex) => String.fromCharCode(parseInt(hex, 16)))
    .replace(/\\x([0-9a-f]{2})/gi, (_, hex) => String.fromCharCode(parseInt(hex, 16))).replace(/\\\//g, '/')
  try { return decodeURIComponent(text) } catch { return text }
}
const checkoutPath = /~\/[.]megarepo\b|\/(?:[^\s"'<>]*\/)?(?:[.]megarepo|checkouts?|worktrees)\//i

/** Scan every published byte, including hashed chunks, maps, fonts and nested refs. */
export async function scanBooks(roots, policy) {
  // Validate an owner-supplied policy even when the book is empty.
  forbiddenLine('', policy)
  const findings = []
  let files = 0
  const visit = async (file, root) => {
    const stat = await lstat(file)
    if (stat.isDirectory()) {
      for (const name of (await readdir(file)).sort()) await visit(join(file, name), root)
      return
    }
    if (!stat.isFile()) { findings.push({ root, file: relative(root, file), line: 1, kind: 'non-file' }); return }
    files++
    // No binary/hash-name exemption: UTF-8 replacement preserves ASCII paths,
    // hostnames and identities embedded in otherwise binary published assets.
    const text = (await readFile(file)).toString('utf8')
    text.split(/\r?\n/).forEach((line, index) => {
      const normalized = decode(line)
      if (checkoutPath.test(normalized) || forbiddenLine(normalized, policy)) findings.push({ root, file: relative(root, file), line: index + 1, kind: 'private-content' })
    })
  }
  for (const root of roots) await visit(resolve(root), resolve(root))
  if (files === 0) throw new Error('No published files to scan')
  return { files, findings }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const { values, positionals } = parseArgs({ allowPositionals: true, options: { policy: { type: 'string' } } })
    if (positionals.length === 0) throw new Error('Missing built-book roots')
    const policy = values.policy === undefined ? undefined : JSON.parse(await readFile(values.policy, 'utf8'))
    const result = await scanBooks(positionals, policy)
    // Never print matched bytes: findings can include owner-held identities.
    for (const finding of result.findings) console.error(JSON.stringify(finding))
    console.log(`Scanned ${result.files} published files; ${result.findings.length} forbidden file-lines`)
    process.exitCode = result.findings.length ? 1 : 0
  } catch {
    console.error('Built-book scan failed: supply readable static roots and optionally --policy IDENTITY_JSON_ARRAY')
    process.exitCode = 2
  }
}
