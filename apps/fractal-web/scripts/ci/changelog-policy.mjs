const sourceRoots = ['apps/fractal-web/', 'packages/fractal-ui/']
const nonProductDirectory = /(?:^|\/)(?:__tests__|__fixtures__|tests?|stories|fixtures|storybook)(?:\/|$)/
const nonProductFile = /(?:^|\/)(?:testEnvironment|vite-env)\.|\.d\.[cm]?ts$|\.(?:(?:unit\.|integration\.|pw\.)?(?:test|spec)|stories|story|config)\.[^.]+$|(?:^|[.-])(?:fixtures|story-harness)\.[^.]+$/

// Conservatively include all runtime TS/JS/CSS in src, plus private UI modules.
// Non-src app scripts, servers, configuration and Storybook-only files are not UI.
export const isUiPath = (path) => sourceRoots.some((root) => path.startsWith(root))
  && (path.includes('/src/') || (path.startsWith('apps/fractal-web/private/') && /\.(?:tsx?|css|scss)$/.test(path)))
  && /\.(?:[cm]?[jt]sx?|css|scss)$/.test(path)
  && !nonProductDirectory.test(path)
  && !nonProductFile.test(path)

export const isChangelogEntryPath = (path) => {
  const match = /^apps\/fractal-web\/(?:private\/)?\.changelog\/(\d{4}-\d{2}-\d{2})-[a-z0-9]+(?:-[a-z0-9]+)*\.ts$/.exec(path)
  if (!match) return false
  const date = new Date(`${match[1]}T00:00:00.000Z`)
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === match[1]
}

export const decodeChangelogCheckInput = (input) => {
  if (!input || typeof input !== 'object' || !Array.isArray(input.files) || !Array.isArray(input.labels)) {
    throw new Error('Expected {files: [{status, filename, previousFilename?}], labels: [string]}')
  }
  if (!input.labels.every((label) => typeof label === 'string')) throw new Error('Labels must be strings')
  for (const file of input.files) {
    if (!file || typeof file !== 'object' || typeof file.filename !== 'string'
      || !['added', 'modified', 'removed', 'renamed', 'copied', 'changed'].includes(file.status)
      || (file.previousFilename !== undefined && typeof file.previousFilename !== 'string')
      || (file.status === 'renamed' && !file.previousFilename)) {
      throw new Error('Each changed file needs a recognized status and filename; renames need previousFilename')
    }
  }
  return input
}

export const checkChangelogRequirement = (input) => {
  const { files, labels } = decodeChangelogCheckInput(input)
  const uiFiles = files.filter((file) => isUiPath(file.filename)
    || (file.previousFilename !== undefined && isUiPath(file.previousFilename)))
  const addedEntries = files.filter((file) => file.status === 'added' && isChangelogEntryPath(file.filename))
  const reason = labels.includes('no-changelog') ? 'opt-out'
    : uiFiles.length === 0 ? 'no-ui-change'
      : addedEntries.length > 0 ? 'new-entry' : 'missing-entry'
  return { ok: reason !== 'missing-entry', reason, uiFiles, addedEntries }
}

// Git -z preserves spaces, tabs and newlines in paths, and includes both rename paths.
export const parseGitNameStatus = (output) => {
  if (output === '') return []
  if (!output.endsWith('\0')) throw new Error('Incomplete NUL-delimited git diff')
  const tokens = output.slice(0, -1).split('\0')
  const files = []
  const statuses = { A: 'added', M: 'modified', D: 'removed', R: 'renamed', C: 'copied', T: 'changed' }
  for (let index = 0; index < tokens.length;) {
    const code = tokens[index++]
    const status = statuses[code[0]]
    if (!status || !/^(?:[AMDT]|[RC]\d+)$/.test(code)) throw new Error(`Unsupported git diff status: ${code}`)
    const first = tokens[index++]
    if (!first) throw new Error('Missing git diff filename')
    if (status === 'renamed' || status === 'copied') {
      const filename = tokens[index++]
      if (!filename) throw new Error('Missing git diff destination filename')
      files.push({ status, filename, previousFilename: first })
    } else files.push({ status, filename: first })
  }
  return files
}
