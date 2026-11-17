/** Native root declarations; matches fractal-schema's github_capture_declarations reader. */
import { node, type Node } from './kdl.ts'

interface GithubSourceSettings {
  readonly name: string
  readonly host: string
  readonly generation: number
  readonly repositories: readonly string[]
  readonly checkpointEveryMs: number
  readonly replicatePrivate: boolean
}

/** Disabled configuration can precede authority admission; enabling requires its grant. */
export type GithubSourceInput = GithubSourceSettings & (
  | { readonly enabled: false; readonly authorityGrant?: string }
  | { readonly enabled: true; readonly authorityGrant: string }
)

export interface GithubScopeMember {
  /** Stable GitHub repository node ID, not an owner/name locator. */
  readonly repositoryId: string
  /** Authored inclusive Unix-millisecond start; never inferred from observed history. */
  readonly fromMs: number
  /** Exclusive Unix-millisecond end. Omit only for open-ended membership. */
  readonly untilMs?: number
}

export interface GithubScopeInput {
  readonly name: string
  readonly revision: string
  readonly repositories: readonly GithubScopeMember[]
}

const nonempty = (value: string, field: string): void => {
  if (typeof value !== 'string' || value.length === 0) throw new TypeError(`${field} must be nonempty`)
}

const integer = (value: number, field: string): void => {
  // JS cannot faithfully render larger native u64 values; reject rather than round timestamps.
  if (!Number.isSafeInteger(value) || value < 0) throw new TypeError(`${field} must be a nonnegative safe integer`)
}

/** Declares acquisition configuration, not an observer, authority grant or activation operation. */
export const githubSource = (input: GithubSourceInput): Node => {
  nonempty(input.name, 'github-source name')
  if (new TextEncoder().encode(input.name).length > 128) throw new TypeError('github-source name exceeds 128 bytes')
  if (!/^[a-z0-9.-]+$/u.test(input.host) || input.host.length > 255) throw new TypeError('github-source host must be a lowercase hostname')
  integer(input.generation, 'github-source generation')
  if (input.generation === 0) throw new TypeError('github-source generation must be positive')
  if (input.enabled && input.authorityGrant === undefined) throw new TypeError('enabled github-source requires authorityGrant')
  if (input.authorityGrant !== undefined) nonempty(input.authorityGrant, 'github-source authorityGrant')
  integer(input.checkpointEveryMs, 'github-source checkpointEveryMs')
  if (input.checkpointEveryMs < 60_000) throw new TypeError('github-source checkpointEveryMs must be at least 60000')
  if (typeof input.enabled !== 'boolean' || typeof input.replicatePrivate !== 'boolean') throw new TypeError('github-source requires explicit enabled and replicatePrivate booleans')
  if (input.repositories.length === 0 || new Set(input.repositories).size !== input.repositories.length || input.repositories.some((repository) => !/^[^/]+\/[^/]+$/u.test(repository))) {
    throw new TypeError('github-source repositories must be unique owner/name pairs')
  }
  return node({ name: 'github-source', args: [input.name], children: [
    node({ name: 'host', args: [input.host] }),
    node({ name: 'generation', args: [input.generation] }),
    ...(input.authorityGrant === undefined ? [] : [node({ name: 'authority-grant', args: [input.authorityGrant] })]),
    ...input.repositories.map((repository) => node({ name: 'repository', args: [repository] })),
    node({ name: 'enabled', args: [input.enabled] }),
    node({ name: 'checkpoint-every-ms', args: [input.checkpointEveryMs] }),
    node({ name: 'replicate-private', args: [input.replicatePrivate] }),
  ] })
}

/** Declares versioned, effective-dated policy membership without inventing roster history. */
export const githubScope = (input: GithubScopeInput): Node => {
  nonempty(input.name, 'github-scope name')
  nonempty(input.revision, 'github-scope revision')
  if (input.repositories.length === 0) throw new TypeError('github-scope needs repository membership')
  input.repositories.forEach((member, index) => {
    nonempty(member.repositoryId, 'github-scope repositoryId')
    integer(member.fromMs, 'github-scope fromMs')
    if (member.untilMs !== undefined) {
      integer(member.untilMs, 'github-scope untilMs')
      if (member.untilMs <= member.fromMs) throw new TypeError('github-scope membership has invalid bounds')
    }
    if (input.repositories.slice(0, index).some((previous) => previous.repositoryId === member.repositoryId &&
      (previous.untilMs ?? Infinity) > member.fromMs && (member.untilMs ?? Infinity) > previous.fromMs)) {
      throw new TypeError('github-scope membership intervals overlap')
    }
  })
  return node({ name: 'github-scope', args: [input.name], children: [
    node({ name: 'revision', args: [input.revision] }),
    ...input.repositories.map((member) => node({ name: 'repository', args: [member.repositoryId], props: {
      'from-ms': member.fromMs,
      ...(member.untilMs === undefined ? {} : { 'until-ms': member.untilMs }),
    } })),
  ] })
}
