/**
 * The GitHub plugin's public face for the host. Eager and dependency-light: ref parsing and claims, the
 * kind contributions, and a Markdown link preview. Views load lazily from `views.tsx`.
 *
 * Core never imports anything below this folder except this file.
 */
import * as React from 'react'

import { DraggableMention } from '../../workspace/DraggableMention.tsx'
import type { KindDescriptor, Present } from '../../workspace/presentation.ts'
import type { KindContribution } from '../../workspace/registry.ts'
export { githubFace } from './presentation.ts'
import { claimsKind, labelOf, parseGithubUrl, parseSubjectRef, refOf, type GithubKind, type GithubRef } from './refs.ts'

export { githubSubjectRef, parseGithubUrl, parseSubjectRef, refOf, type GithubRef } from './refs.ts'
export { gatewayGithubReader, GithubReaderProvider, sharedGithubReader, type GithubReader } from './data.ts'

const views = () => import('./views.tsx')
const LazyHoverLink = React.lazy(() => views().then(module => ({ default: module.GithubHoverLink })))
const LazyEntityCard = React.lazy(() => views().then(module => ({ default: module.GithubEntityCard })))
const LazySurface = React.lazy(() => views().then(module => ({ default: module.GithubSurface })))

/**
 * For `MarkdownLinkPreviewContext`: a github.com PR, issue, comment or run link gets the hover card; any
 * other link is left to the kit. Until the chunk loads, the plain link shows.
 */
export const githubLinkPreview = (href: string, link: React.ReactElement): React.ReactNode | undefined => {
  const target = parseGithubUrl(href)
  if (target === undefined) return undefined
  // The link drags onto the workspace as the subject's tab.
  return React.createElement(DraggableMention, { subject: refOf(target) }, React.createElement(React.Suspense, { fallback: link }, React.createElement(LazyHoverLink, { target, children: link })))
}

/**
 * For the kit's `EntityCardSource`: a resource whose id is a GitHub subject ref resolves to the GitHub card, so a
 * resource row, a tab and a mention of the same PR show one card. Other entities are left to outer sources.
 */
export const githubEntityCard = (entity: { readonly kind: string; readonly id: string }): React.ReactNode | undefined => {
  if (entity.kind !== 'resource') return undefined
  const target = parseSubjectRef(entity.id) ?? parseGithubUrl(entity.id)
  if (target === undefined) return undefined
  return React.createElement(React.Suspense, { fallback: null }, React.createElement(LazyEntityCard, { subjectRef: refOf(target), target }))
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

/** The plugin uses the host-independent registry contract, without a duplicate shape. */
export type GithubKindContribution<View> = KindContribution<View> & { readonly id: `github.${GithubKind}` }

const words: Record<GithubKind, { readonly kind: string; readonly icon: 'pull-request' | 'dot' | 'message' | 'check' }> = {
  'pull-request': { kind: 'Pull request', icon: 'pull-request' },
  issue: { kind: 'Issue', icon: 'dot' },
  comment: { kind: 'Comment', icon: 'message' },
  run: { kind: 'Run', icon: 'check' },
}

/**
 * A GitHub item is named by its observed title, else by the handle GitHub itself prints (`owner/repo#12`),
 * which the ref carries: never by the ref's last segment.
 */
const presentGithub = (kind: GithubKind): Present => (subject, world): KindDescriptor => {
  const ref = subject._tag === 'Resource' ? subject.ref : ''
  const target = parseSubjectRef(ref)
  const observed = world.subjects.get(ref)
  const word = words[kind]
  return {
    title: observed !== undefined ? { _tag: 'Known', text: observed.title } : target !== undefined ? { _tag: 'Known', text: labelOf(target) } : { _tag: 'Gone', kind: word.kind },
    kind: word.kind,
    face: observed?.face ?? { kind: word.kind, icon: word.icon, title: observed?.title ?? (target === undefined ? word.kind : labelOf(target)), mark: { shape: 'none', word: '' }, ...(target !== undefined && 'number' in target && kind !== 'comment' ? { id: `#${target.number}` } : target !== undefined && target.kind === 'run' ? { id: `#${target.run_id}` } : {}) },
    mark: { _tag: 'Icon', name: word.icon },
    card: { kind: 'resource', id: ref },
    ...(observed !== undefined && target !== undefined ? { detail: labelOf(target) } : {}),
  }
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
    present: presentGithub(kind),
    view: view(kind),
  }))

export const isGithubRef = (ref: GithubRef | undefined): ref is GithubRef => ref !== undefined
