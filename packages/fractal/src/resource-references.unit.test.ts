import { readFileSync } from 'node:fs'

import { Schema } from 'effect'
import { describe, expect, it } from 'vitest'

import { parse } from '@overeng/kdl'

import type { AgentIntent } from './define.ts'
import {
  agent,
  AgentSchema,
  emit,
  mission,
  MissionSchema,
  ResourceReferenceSchema,
} from './mod.ts'

const resourceReferences = [
  {
    name: 'roll-authority',
    subject: 'resource/demo/release-completion',
    reason: 'Read the declared roll authority.',
  },
  {
    name: 'source',
    uri: 'git+ssh://example.org/demo%2Frepo',
    reason: 'Track the exact source URI.',
  },
] as const satisfies NonNullable<AgentIntent['resourceReferences']>
const observer = { id: 'demo/release-driver', resourceReferences } satisfies typeof AgentSchema.Encoded
const review = {
  id: 'demo/review',
  state: 'ready',
  reportTo: observer,
  resourceReferences,
  goal: ['Review roll completion.'],
  steps: [],
} as const satisfies typeof MissionSchema.Encoded

// Same native shape consumed by parse_declared_resource_reference, not a metadata block.
describe('native declared resource references', () => {
  it('emits the native golden for agents and missions and roundtrips its reference nodes', () => {
    const golden = readFileSync(new URL('../fixtures/resource-references.kdl', import.meta.url), 'utf8')
    expect(emit([agent(observer), mission(review)])).toBe(golden)
    for (const declaration of parse(golden).nodes.filter((value) => value.getName() !== 'version')) {
      const references = (declaration.children?.nodes ?? [])
        .filter((value) => value.getName() === 'resource')
        .map((value) => {
          expect(value.children?.nodes ?? []).toEqual([])
          return Schema.decodeUnknownSync(ResourceReferenceSchema, { onExcessProperty: 'error' })({
            name: value.getArguments()[0],
            ...Object.fromEntries(value.getProperties()),
          })
        })
      expect(references).toEqual(resourceReferences)
      expect(agent({ id: 'roundtrip', resourceReferences: references }).children).toEqual(
        agent(observer).children,
      )
    }
  })

  it('omits references when absent or empty and preserves author order', () => {
    expect(agent({ id: 'empty' }).children).toEqual(agent({ id: 'empty', resourceReferences: [] }).children)
    expect(mission({ ...review, resourceReferences: [] }).children?.some((value) => value.name === 'resource')).toBe(false)
    const { resourceReferences: _references, ...withoutReferences } = review
    expect(mission(withoutReferences).children).toEqual(mission({ ...review, resourceReferences: [] }).children)
    expect(agent({ ...observer, resourceReferences: [...resourceReferences].reverse() }).children?.map((value) => value.args[0]))
      .toEqual(['source', 'roll-authority'])
  })

  it('does not reinterpret mission managed resources as references', () => {
    const rendered = mission({ ...review, resources: [{ id: 'managed', kind: 'vcs.repository' }] })
    expect(rendered.children?.filter((value) => value.name === 'resource')).toEqual([
      ...(agent(observer).children ?? []),
      { name: 'resource', args: ['managed'], props: {}, children: [{ name: 'kind', args: ['vcs.repository'], props: {} }] },
    ])
  })

  it('rejects duplicate names on both owners, even across locator kinds', () => {
    const duplicate = [resourceReferences[0], { ...resourceReferences[1], name: 'roll-authority' }]
    expect(() => agent({ ...observer, resourceReferences: duplicate })).toThrow()
    expect(() => mission({ ...review, resourceReferences: duplicate })).toThrow()
    expect(Schema.is(AgentSchema)({ ...observer, resourceReferences: duplicate })).toBe(false)
    expect(Schema.is(MissionSchema)({ ...review, resourceReferences: duplicate })).toBe(false)
  })

  it.each([
    {},
    { name: 'missing' },
    { name: 'both', subject: 'resource/demo/release-completion', uri: 'urn:roll' },
    ...['', '/leading', 'trailing/', 'a//b', 'a/../b', '-bad', 'has space', 'é', 'a'.repeat(513)]
      .map((name) => ({ name, uri: 'urn:roll' })),
    ...['agent/demo/release-driver', 'resource/', 'resource//roll', 'resource/a//b', 'resource/a/../b', `resource/${'a'.repeat(513)}`]
      .map((subject) => ({ name: 'bad-subject', subject })),
    ...['relative/path', ':missing-scheme', '1bad:value', 'urn:', 'urn:has space', 'urn:é', 'urn:bad%escape', 'urn:bad%2', 'urn:bad\\path', 'urn:<bad>', 'urn:bad\nvalue', `urn:${'a'.repeat(4093)}`]
      .map((uri) => ({ name: 'bad-uri', uri })),
    { name: 'reason', uri: 'urn:roll', reason: ' \t\n' },
    { name: 'reason', uri: 'urn:roll', reason: 42 },
    { name: 'subject', subject: 42 },
    { name: 'uri', uri: 42 },
  ])('rejects malformed native references: %j', (reference) => {
    expect(Schema.is(ResourceReferenceSchema)(reference)).toBe(false)
    expect(() => Schema.decodeUnknownSync(AgentSchema, { onExcessProperty: 'error' })({
      ...observer, resourceReferences: [reference],
    })).toThrow()
    expect(() => Schema.decodeUnknownSync(MissionSchema, { onExcessProperty: 'error' })({
      ...review, resourceReferences: [reference],
    })).toThrow()
  })

  it.each(['urn:roll', 'file:///source', 'custom+v1.-:opaque', 'https://example.org/%2f?q=a#b', `urn:${'a'.repeat(4092)}`])
    ('preserves valid absolute URI bytes: %s', (uri) => {
      expect(agent({ id: 'uri', resourceReferences: [{ name: 'uri', uri }] }).children?.[0]?.props).toEqual({ uri })
    })

  it('accepts native name limits and optional reasons without normalizing values', () => {
    const reference = { name: 'a'.repeat(512), subject: `resource/${'b'.repeat(512)}`, reason: '  intentional spacing  ' }
    expect(agent({ id: 'limits', resourceReferences: [reference] }).children?.[0]?.props)
      .toEqual({ subject: reference.subject, reason: reference.reason })
    expect(agent({ id: 'optional', resourceReferences: [{ name: 'uri', uri: 'urn:roll' }] }).children?.[0]?.props)
      .toEqual({ uri: 'urn:roll' })
  })
})
