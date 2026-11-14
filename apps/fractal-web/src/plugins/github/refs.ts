/**
 * GitHub refs. Two inputs, kept apart:
 * - subject refs (`resource/github/OWNER/REPO/pull-request/N`), what tabs, rows and the registry carry; and
 * - link hrefs (`https://github.com/OWNER/REPO/pull/N#issuecomment-ID`), what Markdown and terminals show.
 * Both parse to one typed `GithubRef`; `refOf` writes the canonical subject ref back. Synchronous and
 * dependency-free so the registry can ask `claims` without loading the plugin's views.
 */

export type GithubKind = 'pull-request' | 'issue' | 'comment' | 'run'

export type GithubRef =
  | { readonly kind: 'pull-request'; readonly owner: string; readonly repo: string; readonly number: number }
  | { readonly kind: 'issue'; readonly owner: string; readonly repo: string; readonly number: number }
  /** An issue or PR conversation comment; `number` is the item it sits on, when known. */
  | { readonly kind: 'comment'; readonly owner: string; readonly repo: string; readonly comment_id: number; readonly number?: number }
  | { readonly kind: 'run'; readonly owner: string; readonly repo: string; readonly run_id: number }

export const subjectPrefix = 'resource/github/'

// GitHub's own rules: owners are alphanumerics and single hyphens; repos add `.` and `_`.
const ownerPattern = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$/
const repoPattern = /^[A-Za-z0-9._-]{1,100}$/
const idPattern = /^[1-9][0-9]{0,15}$/

const validName = (owner: string, repo: string) => ownerPattern.test(owner) && repoPattern.test(repo) && repo !== '.' && repo !== '..'
const id = (raw: string | undefined): number | undefined => raw !== undefined && idPattern.test(raw) ? Number(raw) : undefined

/** Subject ref → typed ref. Accepts both spellings in the graph today: `pull` and `pull-request`. */
export const parseSubjectRef = (ref: string): GithubRef | undefined => {
  if (!ref.startsWith(subjectPrefix)) return undefined
  const parts = ref.slice(subjectPrefix.length).split('/')
  if (parts.length !== 4) return undefined
  const [owner = '', repo = '', segment, raw] = parts
  if (!validName(owner, repo)) return undefined
  const n = id(raw)
  if (n === undefined) return undefined
  switch (segment) {
    case 'pull':
    case 'pull-request': return { kind: 'pull-request', owner, repo, number: n }
    case 'issue': return { kind: 'issue', owner, repo, number: n }
    case 'comment': return { kind: 'comment', owner, repo, comment_id: n }
    case 'run': return { kind: 'run', owner, repo, run_id: n }
    default: return undefined
  }
}

/** Typed ref → its canonical subject ref. */
export const refOf = (ref: GithubRef): string => {
  const base = `${subjectPrefix}${ref.owner}/${ref.repo}`
  switch (ref.kind) {
    case 'pull-request': return `${base}/pull-request/${ref.number}`
    case 'issue': return `${base}/issue/${ref.number}`
    case 'comment': return `${base}/comment/${ref.comment_id}`
    case 'run': return `${base}/run/${ref.run_id}`
  }
}

/** Whether a subject ref names this kind. What a `KindContribution.claims` is. */
export const claimsKind = (kind: GithubKind) => (ref: string): boolean => parseSubjectRef(ref)?.kind === kind

/**
 * A github.com link → typed ref, or `undefined` for anything else (other hosts, repos, trees, blobs,
 * compare views, credentials in the URL). Fragments `#issuecomment-N` name a comment; `#discussion_rN`
 * and `#pullrequestreview-N` stay on the PR.
 */
export const parseGithubUrl = (href: string): GithubRef | undefined => {
  let url: URL
  try {
    url = new URL(href)
  } catch {
    return undefined
  }
  if (url.protocol !== 'https:' && url.protocol !== 'http:') return undefined
  if (url.hostname !== 'github.com' && url.hostname !== 'www.github.com') return undefined
  if (url.username !== '' || url.password !== '' || url.port !== '') return undefined
  const parts = url.pathname.split('/').filter(part => part !== '')
  const [owner = '', repo = '', segment, raw, rest] = parts
  if (!validName(owner, repo)) return undefined
  if (segment === 'actions' && raw === 'runs') {
    const run = id(rest)
    // `/actions/runs/N`, `/actions/runs/N/job/M` and `/actions/runs/N/attempts/K` all open the run.
    return run === undefined ? undefined : { kind: 'run', owner, repo, run_id: run }
  }
  const n = id(raw)
  if (n === undefined) return undefined
  const comment = /^issuecomment-([1-9][0-9]{0,15})$/.exec(url.hash.slice(1))?.[1]
  if (segment === 'pull') {
    if (comment !== undefined && rest === undefined) return { kind: 'comment', owner, repo, comment_id: Number(comment), number: n }
    // `/pull/N/files`, `/pull/N/checks`, `/pull/N/commits` open the PR.
    return rest === undefined || ['files', 'checks', 'commits', 'changes'].includes(rest) ? { kind: 'pull-request', owner, repo, number: n } : undefined
  }
  if (segment === 'issues' && rest === undefined) {
    return comment === undefined ? { kind: 'issue', owner, repo, number: n } : { kind: 'comment', owner, repo, comment_id: Number(comment), number: n }
  }
  return undefined
}

/** Either input → the canonical subject ref, for `open(ref)`. */
export const githubSubjectRef = (input: string): string | undefined => {
  const ref = input.startsWith(subjectPrefix) ? parseSubjectRef(input) : parseGithubUrl(input)
  return ref === undefined ? undefined : refOf(ref)
}

/** The short label GitHub itself uses: `owner/repo#12`, or `owner/repo` run and comment ids. */
export const labelOf = (ref: GithubRef): string => {
  switch (ref.kind) {
    case 'pull-request':
    case 'issue': return `${ref.owner}/${ref.repo}#${ref.number}`
    case 'comment': return ref.number === undefined ? `${ref.owner}/${ref.repo} comment` : `${ref.owner}/${ref.repo}#${ref.number} comment`
    case 'run': return `${ref.owner}/${ref.repo} run ${ref.run_id}`
  }
}

/** The github.com page for a ref, for "open on GitHub" links. Built from typed parts only. */
export const githubUrlOf = (ref: GithubRef): string => {
  const base = `https://github.com/${encodeURIComponent(ref.owner)}/${encodeURIComponent(ref.repo)}`
  switch (ref.kind) {
    case 'pull-request': return `${base}/pull/${ref.number}`
    case 'issue': return `${base}/issues/${ref.number}`
    case 'comment': return ref.number === undefined ? base : `${base}/issues/${ref.number}#issuecomment-${ref.comment_id}`
    case 'run': return `${base}/actions/runs/${ref.run_id}`
  }
}

/** The daemon path for a ref's primary read. Path segments are typed values, never a URL. */
export const readPathOf = (ref: GithubRef): string => {
  const base = `/v1/client/plugins/github/${encodeURIComponent(ref.owner)}/${encodeURIComponent(ref.repo)}`
  switch (ref.kind) {
    case 'pull-request': return `${base}/pulls/${ref.number}`
    case 'issue': return `${base}/issues/${ref.number}`
    case 'comment': return `${base}/comments/${ref.comment_id}`
    case 'run': return `${base}/runs/${ref.run_id}`
  }
}
