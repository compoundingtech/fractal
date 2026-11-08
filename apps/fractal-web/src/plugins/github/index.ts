/**
 * The GitHub plugin's public face for the host. Eager and dependency-light: ref parsing and claims, the
 * kind contributions, and a Markdown link preview. Views load lazily from `views.tsx`.
 *
 * Core never imports anything below this folder except this file.
 */
import * as React from 'react'

import { claimsKind, parseGithubUrl, parseSubjectRef, type GithubKind, type GithubRef } from './refs.ts'

export { githubSubjectRef, parseGithubUrl, parseSubjectRef, refOf, type GithubRef } from './refs.ts'
export { gatewayGithubReader, GithubReaderProvider, sharedGithubReader, type GithubReader } from './data.ts'

const views = () => import('./views.tsx')
const LazyHoverLink = React.lazy(() => views().then(module => ({ default: module.GithubHoverLink })))
const LazySurface = React.lazy(() => views().then(module => ({ default: module.GithubSurface })))

/**
 * For `MarkdownLinkPreviewContext`: a github.com PR, issue, comment or run link gets the hover card; any
 * other link is left to the kit. Until the chunk loads, the plain link shows.
 */
export const githubLinkPreview = (href: string, link: React.ReactElement): React.ReactNode | undefined => {
  const target = parseGithubUrl(href)
  if (target === undefined) return undefined
  return React.createElement(React.Suspense, { fallback: link }, React.createElement(LazyHoverLink, { target, children: link }))
}

/** The tab surface for a claimed subject ref, with the host's `onOpen`. */
export function GithubTab({ subjectRef, onOpen }: { readonly subjectRef: string; readonly onOpen?: (ref: string) => void }) {
  const target = parseSubjectRef(subjectRef)
  if (target === undefined) return null
  return React.createElement(React.Suspense, { fallback: null }, React.createElement(LazySurface, { target, ...(onOpen === undefined ? {} : { onOpen }) }))
}

/** The hover presenter for a claimed subject ref (rows, chips). */
export function GithubCardFor({ subjectRef, children }: { readonly subjectRef: string; readonly children: React.ReactElement }) {
  const target = parseSubjectRef(subjectRef)
  if (target === undefined) return children
  return React.createElement(React.Suspense, { fallback: children }, React.createElement(LazyHoverLink, { target, children }))
}

/**
 * The shape `workspace/registry.ts` (`KindContribution<View>`, PR #5724) takes, kept here until the
 * registry lands on the integration branch; the host supplies `view` from its own `KindSurface`.
 */
export interface GithubKindContribution<View> {
  readonly id: `github.${GithubKind}`
  readonly claims: (ref: string) => boolean
  readonly examples: readonly [string, ...string[]]
  readonly policy: {
    readonly resources: (subject: { readonly _tag: string; readonly ref?: string }) => readonly { readonly _tag: 'Contributed'; readonly kind: string; readonly ref: string }[]
    readonly keepWarm: boolean
    readonly domCost: number
    readonly dataCost: number
  }
  readonly view: View
}

const kinds: readonly { readonly kind: GithubKind; readonly example: string }[] = [
  { kind: 'pull-request', example: 'resource/github/acme/garden/pull-request/7' },
  { kind: 'issue', example: 'resource/github/acme/garden/issue/9' },
  { kind: 'comment', example: 'resource/github/acme/garden/comment/1832451234' },
  { kind: 'run', example: 'resource/github/acme/garden/run/9001' },
]

/** One contribution per GitHub kind, each viewing through `view(kind)`. */
export const githubContributions = <View>(view: (kind: GithubKind) => View): readonly GithubKindContribution<View>[] =>
  kinds.map(({ kind, example }) => ({
    id: `github.${kind}` as const,
    claims: claimsKind(kind),
    examples: [example],
    policy: {
      resources: subject => subject._tag === 'Resource' && subject.ref !== undefined ? [{ _tag: 'Contributed', kind: `github.${kind}`, ref: subject.ref }] : [],
      keepWarm: false,
      domCost: 1,
      dataCost: 1,
    },
    view: view(kind),
  }))

export const isGithubRef = (ref: GithubRef | undefined): ref is GithubRef => ref !== undefined
