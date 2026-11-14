import { describe, expect, it } from 'vitest'
import { emptyWorkspace, tabsOf, transition } from './model.ts'
import { corePolicies, makeKindRegistry, TabKindClaimConflict, type KindContribution } from './registry.ts'
import { defaultBudget, desiredHolds, planTiers } from './resources.ts'
import { parsePane, subjectOfRef, type SubjectKind } from './subject.ts'

const views = Object.fromEntries((Object.keys(corePolicies) as SubjectKind[]).map(kind => [kind, `core:${kind}`])) as Record<SubjectKind, string>

const github: KindContribution<string> = {
  id: 'github.pull-request',
  claims: ref => ref.startsWith('resource/github/'),
  examples: ['resource/github/acme/garden/pull/9'],
  policy: {
    resources: subject => subject._tag === 'Resource' ? [{ _tag: 'Contributed', kind: 'github.pull-request', ref: subject.ref }] : [],
    keepWarm: true,
    domCost: 1,
    dataCost: 1,
  },
  view: 'plugin:github',
}

describe('the tab-kind registry', () => {
  it('gives an unclaimed ref the generic resource kind', () => {
    const kind = makeKindRegistry({ views }).kindOf(subjectOfRef('resource/github/acme/1'))
    expect(kind.id).toBe('Resource')
    expect(kind.view).toBe('core:Resource')
  })

  it('lets a contribution claim a resource ref, with its own view and data', () => {
    const kind = makeKindRegistry({ views, contributions: [github] }).kindOf(subjectOfRef('resource/github/acme/1'))
    expect(kind.id).toBe('github.pull-request')
    expect(kind.view).toBe('plugin:github')
  })

  it('never lets a contribution take over a core subject', () => {
    const greedy: KindContribution<string> = { ...github, id: 'greedy', claims: () => true, examples: ['anything/else'] }
    const registry = makeKindRegistry({ views, contributions: [greedy] })
    for (const ref of ['agent/a', 'mission/m', 'doc/x', 'missions', 'inbox']) expect(registry.kindOf(subjectOfRef(ref)).id).not.toBe('greedy')
    expect(registry.kindOf(parsePane('future-kind:x')).id).toBe('Unsupported')
  })

  it('refuses overlapping claims when built, whatever the order', () => {
    const rival: KindContribution<string> = { ...github, id: 'rival' }
    expect(() => makeKindRegistry({ views, contributions: [github, rival] })).toThrow(/claimed by \[github\.pull-request, rival\]/)
    expect(() => makeKindRegistry({ views, contributions: [rival, github] })).toThrow(/claimed by \[rival, github\.pull-request\]/)
  })

  it('refuses with a tagged error naming the kind, the ref and the claimants', () => {
    const rival: KindContribution<string> = { ...github, id: 'rival' }
    const refused = (() => {
      try { return makeKindRegistry({ views, contributions: [github, rival] }) } catch (error) { return error }
    })()
    expect(refused).toBeInstanceOf(TabKindClaimConflict)
    expect(refused).toMatchObject({ _tag: 'TabKindClaimConflict', kind: 'github.pull-request', ref: 'resource/github/acme/garden/pull/9', claimants: ['github.pull-request', 'rival'] })
  })

  it('refuses a contribution that does not claim its own example', () => {
    expect(() => makeKindRegistry({ views, contributions: [{ ...github, examples: ['mail/1'] }] })).toThrow(/claimed by \[\]/)
  })

  it('falls to the generic kind on an overlap no example caught, never to list order', () => {
    const issues: KindContribution<string> = { ...github, id: 'github.issue', claims: ref => ref.includes('/issue/') || ref === 'resource/github/both', examples: ['resource/other/issue/1'] }
    const pulls: KindContribution<string> = { ...github, claims: ref => ref.includes('/pull/') || ref === 'resource/github/both' }
    const registry = makeKindRegistry({ views, contributions: [pulls, issues] })
    expect(registry.kindOf(subjectOfRef('resource/github/both')).id).toBe('Resource')
    expect(registry.kindOf(subjectOfRef('resource/other/issue/1')).id).toBe('github.issue')
  })

  it('plans a contributed kind\'s data through its own policy', () => {
    const registry = makeKindRegistry({ views, contributions: [github] })
    const policyOf = (subject: Parameters<typeof registry.kindOf>[0]) => registry.kindOf(subject).policy
    const context = { newId: (() => { let next = 0; return () => `id-${next++}` })() }
    let state = transition(emptyWorkspace, { _tag: 'Open', pane: 'resource:resource/github/acme/1' }, context)
    state = transition(state, { _tag: 'Open', pane: 'list:home' }, context)
    const tabs = tabsOf(state.layout)
    const tiers = planTiers({ state, budget: defaultBudget, policyOf })
    expect(desiredHolds({ tabs, tiers, policyOf }).get(tabs[0]!.id)).toEqual([{ _tag: 'Contributed', kind: 'github.pull-request', ref: 'resource/github/acme/1' }])
  })
})
