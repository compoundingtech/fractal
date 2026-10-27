import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { checkChangelogRequirement, parseGitNameStatus } from './changelog-policy.mjs'

// Offline controls: --input FILE (or --input - for stdin).
// CI: --base BASE --head HEAD --labels '["no-changelog"]'. No changelog source is imported.
const main = () => {
  const args = process.argv.slice(2)
  const options = new Map()
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index]
    const value = args[index + 1]
    if (!['--input', '--base', '--head', '--labels'].includes(key) || value === undefined || options.has(key)) {
      throw new Error('Usage: node changelog-requirement.mjs --input FILE|- OR --base BASE --head HEAD [--labels JSON]')
    }
    options.set(key, value)
  }
  let input
  if (options.has('--input')) {
    if (options.size !== 1) throw new Error('--input cannot be combined with git diff options')
    input = JSON.parse(readFileSync(options.get('--input') === '-' ? 0 : options.get('--input'), 'utf8'))
  } else {
    if (!options.has('--base') || !options.has('--head')) throw new Error('Both --base and --head are required')
    const resolve = (revision) => {
      const result = spawnSync('git', ['rev-parse', '--verify', '--end-of-options', `${revision}^{commit}`], { encoding: 'utf8' })
      if (result.error) throw result.error
      if (result.status !== 0) throw new Error(`Cannot resolve git revision: ${revision}`)
      return result.stdout.trim()
    }
    const base = resolve(options.get('--base'))
    const head = resolve(options.get('--head'))
    // Direct diff against the merge commit's first parent covers the effective full PR
    // delta without GitHub's 3000-file REST cap, including deletions and both rename paths.
    const result = spawnSync('git', ['diff', '--name-status', '-z', '--find-renames', '--no-ext-diff', base, head, '--'], {
      encoding: 'utf8', maxBuffer: 64 * 1024 * 1024,
    })
    if (result.error) throw result.error
    if (result.status !== 0) throw new Error('Cannot collect complete changed-file diff')
    input = { files: parseGitNameStatus(result.stdout), labels: JSON.parse(options.get('--labels') ?? '[]') }
  }
  const result = checkChangelogRequirement(input)
  console.log(JSON.stringify(result, null, 2))
  if (!result.ok) {
    console.error('UI changes require a NEW apps/fractal-web/.changelog/YYYY-MM-DD-slug.ts (or private/.changelog) entry. Editing, deleting or renaming an existing entry does not count. Apply the no-changelog label for an explicit opt-out.')
    process.exitCode = 1
  }
}

try {
  main()
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  process.exitCode = 1
}
