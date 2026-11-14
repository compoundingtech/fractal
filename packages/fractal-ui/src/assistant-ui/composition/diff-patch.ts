/**
 * Pure patch normalization for the kit DiffView. @pierre/diffs renders exactly one file per
 * `PatchDiff` and needs a `---`/`+++` header; Fractal's inputs are rarely that tidy (bare fence
 * lines, hunk-only rows, numberless Codex hunks, multi-file tool output, streaming tails). This
 * module turns any of them into single-file patches with recomputed hunk counts, and counts
 * `+N −M` without loading the renderer.
 */

export interface DiffStats { readonly added: number; readonly removed: number }

/** One file, ready for `PatchDiff`: a `---`/`+++` header and hunks whose counts match their bodies. */
export interface NormalizedDiffFile {
  readonly path: string
  readonly patch: string
  readonly added: number
  readonly removed: number
  /** The hunk offsets were synthesized: render without line numbers rather than invent them. */
  readonly excerpt: boolean
}

export interface DiffRowInput { readonly kind: 'context' | 'added' | 'removed' | 'hunk'; readonly text: string }

interface Hunk { oldStart: number | undefined; newStart: number | undefined; suffix: string; body: string[] }
interface FileDraft { headers: string[]; oldPath: string | undefined; newPath: string | undefined; gitPath: string | undefined; hunks: Hunk[] }

const hunkHeader = /^@@(?: -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@)?(.*)$/
const stripPrefix = (path: string) => path.replace(/^[ab]\//, '').replace(/\t.*$/, '')

function isBodyLine(line: string) {
  return line.startsWith('+') || line.startsWith('-') || line.startsWith(' ') || line.startsWith('\\')
}

function counts(body: readonly string[]) {
  let oldCount = 0, newCount = 0
  for (const line of body) {
    if (line.startsWith('+')) newCount++
    else if (line.startsWith('-')) oldCount++
    else if (line.startsWith(' ')) { oldCount++; newCount++ }
  }
  return { oldCount, newCount }
}

function finish(draft: FileDraft, fallbackPath: string, forceExcerpt: boolean): NormalizedDiffFile | undefined {
  const hunks = draft.hunks.filter(hunk => hunk.body.length > 0)
  if (hunks.length === 0) return undefined
  const newPath = draft.newPath && draft.newPath !== '/dev/null' ? stripPrefix(draft.newPath) : undefined
  const oldPath = draft.oldPath && draft.oldPath !== '/dev/null' ? stripPrefix(draft.oldPath) : undefined
  const path = newPath ?? oldPath ?? draft.gitPath ?? fallbackPath
  const excerpt = forceExcerpt || hunks.some(hunk => hunk.oldStart === undefined)
  const out: string[] = [...draft.headers]
  if (draft.oldPath === undefined || draft.newPath === undefined) out.push(`--- a/${path}`, `+++ b/${path}`)
  let added = 0, removed = 0, oldCursor = 1, newCursor = 1
  for (const hunk of hunks) {
    const { oldCount, newCount } = counts(hunk.body)
    // Synthetic ranges leave a one-line gap so separate excerpts never read as contiguous.
    const oldStart = hunk.oldStart ?? oldCursor
    const newStart = hunk.newStart ?? newCursor
    out.push(`@@ -${oldStart},${oldCount} +${newStart},${newCount} @@${hunk.suffix}`, ...hunk.body)
    oldCursor = oldStart + oldCount + 1
    newCursor = newStart + newCount + 1
    for (const line of hunk.body) { if (line.startsWith('+')) added++; else if (line.startsWith('-')) removed++ }
  }
  return { path, patch: `${out.join('\n')}\n`, added, removed, excerpt }
}

/** Split any unified-diff-like text into single-file patches PatchDiff accepts. */
export function normalizePatch(input: string, options: { readonly path?: string; readonly excerpt?: boolean } = {}): readonly NormalizedDiffFile[] {
  const fallbackPath = options.path ?? 'diff'
  const lines = input.replace(/\r\n/g, '\n').split('\n')
  if (lines.at(-1) === '') lines.pop()
  const files: NormalizedDiffFile[] = []
  const blank = (): FileDraft => ({ headers: [], oldPath: undefined, newPath: undefined, gitPath: undefined, hunks: [] })
  let draft = blank()
  let hunk: Hunk | undefined
  // Remaining lines a numbered hunk header promised; header-looking lines inside it are body.
  let remainingOld = 0, remainingNew = 0
  const push = () => { const file = finish(draft, fallbackPath, options.excerpt === true); if (file) files.push(file); draft = blank(); hunk = undefined }
  const inCountedHunk = () => hunk !== undefined && hunk.oldStart !== undefined && (remainingOld > 0 || remainingNew > 0)
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index]!
    if (!inCountedHunk()) {
      if (line.startsWith('diff --git ')) {
        push()
        draft.headers.push(line)
        draft.gitPath = /^diff --git a\/(.+) b\/(.+)$/.exec(line)?.[2]
        continue
      }
      if (line.startsWith('--- ') && lines[index + 1]?.startsWith('+++ ')) {
        if (draft.hunks.length > 0) push()
        draft.oldPath = line.slice(4)
        draft.newPath = lines[index + 1]!.slice(4)
        draft.headers.push(line, lines[index + 1]!)
        index++
        continue
      }
      const header = hunkHeader.exec(line)
      if (header && line.startsWith('@@')) {
        const numbered = header[1] !== undefined
        hunk = { oldStart: numbered ? Number(header[1]) : undefined, newStart: numbered ? Number(header[3]) : undefined, suffix: numbered ? header[5]! : (header[5]!.trim() ? ` ${header[5]!.trim()}` : ''), body: [] }
        remainingOld = numbered ? Number(header[2] ?? 1) : 0
        remainingNew = numbered ? Number(header[4] ?? 1) : 0
        draft.hunks.push(hunk)
        continue
      }
      if (hunk === undefined && draft.hunks.length === 0 && draft.oldPath === undefined && /^(?:index |new file mode|deleted file mode|old mode|new mode|similarity index|rename from|rename to|Binary files)/.test(line)) {
        draft.headers.push(line)
        continue
      }
    }
    if (hunk === undefined) {
      hunk = { oldStart: undefined, newStart: undefined, suffix: '', body: [] }
      draft.hunks.push(hunk)
    }
    const body = isBodyLine(line) ? line : ` ${line}`
    hunk.body.push(body)
    if (body.startsWith('-')) remainingOld--
    else if (body.startsWith('+')) remainingNew--
    else if (body.startsWith(' ')) { remainingOld--; remainingNew-- }
  }
  push()
  return files
}

/** `+N −M` for a header, without loading the renderer. */
export function diffStats(patch: string): DiffStats {
  let added = 0, removed = 0
  for (const file of normalizePatch(patch)) { added += file.added; removed += file.removed }
  return { added, removed }
}

function mapHunks(file: NormalizedDiffFile, transform: (body: readonly string[], budget: { left: number }) => string[], budget = { left: Number.POSITIVE_INFINITY }): NormalizedDiffFile {
  const lines = file.patch.replace(/\n$/, '').split('\n')
  const head: string[] = []
  const hunks: { header: string; body: string[] }[] = []
  for (const line of lines) {
    if (line.startsWith('@@') && (hunks.length > 0 || head.some(h => h.startsWith('+++ ')))) hunks.push({ header: line, body: [] })
    else if (hunks.length > 0) hunks.at(-1)!.body.push(line)
    else head.push(line)
  }
  const rebuilt = [...head]
  for (const hunk of hunks) {
    const body = transform(hunk.body, budget)
    if (body.length === 0) continue
    rebuilt.push(hunk.header, ...body)
  }
  const [normalized] = normalizePatch(`${rebuilt.join('\n')}\n`, { path: file.path, excerpt: file.excerpt })
  return normalized ?? { ...file, patch: `${head.join('\n')}\n`, added: 0, removed: 0 }
}

/**
 * Hide whitespace-only changes: each `-`/`+` pair equal up to whitespace becomes one context line
 * (new text), and an unpaired change whose text is blank is dropped. A dropped line ends its hunk
 * and the rest continues as a new hunk one line further on that side, so every hunk range and
 * every later line number stays the file's real one. A `\ No newline at end of file` marker
 * belongs to the line before it: it never interrupts pairing, a kept change keeps it, a dropped
 * change drops it, and a collapsed pair keeps it only when both sides end without a newline
 * (a newline that only one side lacks is whitespace too).
 */
export function collapseWhitespaceChanges(file: NormalizedDiffFile): NormalizedDiffFile {
  const lines = file.patch.replace(/\n$/, '').split('\n')
  const out: string[] = []
  let index = 0
  while (index < lines.length && !(lines[index]!.startsWith('@@') && out.some(line => line.startsWith('+++ ')))) out.push(lines[index++]!)
  while (index < lines.length) {
    const header = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/.exec(lines[index++]!)
    const body: string[] = []
    while (index < lines.length && !lines[index]!.startsWith('@@')) body.push(lines[index++]!)
    if (!header) continue
    let oldLine = Number(header[1]), newLine = Number(header[2])
    let segment = { old: oldLine, new: newLine, body: [] as string[] }
    let suffix = header[3]!
    const close = () => {
      if (segment.body.length === 0) return
      const { oldCount, newCount } = counts(segment.body)
      out.push(`@@ -${segment.old},${oldCount} +${segment.new},${newCount} @@${suffix}`, ...segment.body)
      suffix = ''
    }
    const keep = (line: string) => {
      // A marker with no line before it in this segment has nothing to describe.
      if (line.startsWith('\\') && segment.body.length === 0) return
      segment.body.push(line)
      if (line.startsWith('-') || line.startsWith(' ')) oldLine++
      if (line.startsWith('+') || line.startsWith(' ')) newLine++
    }
    type Change = { readonly line: string; readonly marker: string | undefined }
    const change = ({ line, marker }: Change) => {
      if (line.slice(1).trim() !== '') { keep(line); if (marker !== undefined) keep(marker); return }
      close()
      if (line.startsWith('-')) oldLine++
      else newLine++
      segment = { old: oldLine, new: newLine, body: [] }
    }
    let at = 0
    const take = (sign: '-' | '+') => {
      const group: Change[] = []
      while (at < body.length && body[at]!.startsWith(sign)) {
        const line = body[at++]!
        group.push({ line, marker: body[at]?.startsWith('\\') ? body[at++] : undefined })
      }
      return group
    }
    while (at < body.length) {
      if (!body[at]!.startsWith('-') && !body[at]!.startsWith('+')) { keep(body[at++]!); continue }
      const removed = take('-'), added = take('+')
      const pending: Change[] = []
      for (let pair = 0; pair < Math.max(removed.length, added.length); pair++) {
        const old = removed[pair], next = added[pair]
        if (old !== undefined && next !== undefined && old.line.slice(1).replace(/\s/g, '') === next.line.slice(1).replace(/\s/g, '')) {
          pending.splice(0).forEach(change)
          keep(` ${next.line.slice(1)}`)
          if (old.marker !== undefined && next.marker !== undefined) keep(next.marker)
          continue
        }
        if (old !== undefined) change(old)
        if (next !== undefined) pending.push(next)
      }
      pending.forEach(change)
    }
    close()
  }
  const [normalized] = normalizePatch(`${out.join('\n')}\n`, { path: file.path, excerpt: file.excerpt })
  // Every change was whitespace-only and blank: only the file header remains.
  return normalized ?? { ...file, patch: `${out.join('\n')}\n`, added: 0, removed: 0 }
}

/** Keep the first `maxLines` body lines (a compact preview) and say how many were hidden. */
export function truncatePatch(file: NormalizedDiffFile, maxLines: number): { readonly file: NormalizedDiffFile; readonly hidden: number; readonly kept: number } {
  let total = 0
  const budget = { left: maxLines }
  const short = mapHunks(file, (body, left) => {
    total += body.length
    const kept = body.slice(0, Math.max(0, left.left))
    left.left -= kept.length
    return kept
  }, budget)
  const kept = Math.min(maxLines, total)
  return { file: short, hidden: total - kept, kept }
}

/**
 * Kit diff rows (DiffPanel, tool previews) as one normalized file. A `hunk` row is either a header
 * (kept when it carries ranges, a bare `@@` boundary otherwise) or a `\ No newline at end of file`
 * marker, which stays a marker on the line before it.
 */
export function patchFromRows(path: string, rows: readonly DiffRowInput[], options: { readonly excerpt?: boolean } = {}): NormalizedDiffFile {
  const text = rows.map(row => row.kind === 'hunk' ? (row.text.startsWith('@@') || row.text.startsWith('\\') ? row.text : '@@') : `${row.kind === 'added' ? '+' : row.kind === 'removed' ? '-' : ' '}${row.text}`).join('\n')
  return normalizePatch(text, { path, excerpt: options.excerpt })[0] ?? { path, patch: `--- a/${path}\n+++ b/${path}\n`, added: 0, removed: 0, excerpt: options.excerpt === true }
}
