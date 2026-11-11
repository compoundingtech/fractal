/** Story fixtures in the shape Fractal serves: the agent text matches `render_agent_desired_kdl` output with env values redacted. */
export const agentDeclarationKdl = `version 2
agent "example/ui-design" {
    accountable "person/avery"
    description "Design seat for the Fractal web client. Works only inside its seat folder."
    workspace "/srv/work/example/ui-design"
    harness "omp" {
        model "anthropic/claude-opus-5-5"
        effort "medium"
    }
    env {
        GITHUB_TOKEN "<redacted>"
        ST_PERMISSIONS "<redacted>"
        ST_SEAT_PRIORITY "<redacted>"
    }
}
`

/** A mission declaration as authored; Fractal serves no KDL for missions today, so this is a kit-only fixture. */
export const missionDeclarationKdl = `version 2
mission "example/ui-design/kdl-view" state="ready" timeout="3h" report-to="agent/example/ui-design" {
  goal "Canonical KDL viewer modal for agents and missions"
  constraint "Builder follows build-brief.md and gate follows gate-brief.md. Never push."
  step "start-builder" {
    agentless
    agent "builder" {
      workspace "/srv/work/example/kdl-view"
      harness "omp" {
        model "openai-codex/gpt-6.1-sol"
        effort "medium"
      }
    }
  }
  loop "refine" timeout="3h" {
    depends-on { step "start-builder" completed }
    max-rounds 3
    until {
      gate "the latest gate verdict is GO" {
        exec "[ -f gate/report-r\${loop.round}.json ] || exit 1"
        host "local"
        time-limit "1m"
      }
    }
    round {
      step "build" timeout="4h" {
        assigned-to "agent/\${ST_ROOT_MISSION_RUN_ID}/builder"
        goal "Round \${loop.round}: build or fix per build-brief.md, then write build-r\${loop.round}.md."
      }
      step "gate" timeout="5h" {
        assigned-to "agent/\${ST_MISSION_RUN}/gate"
        depends-on { step "build" completed }
        goal "Round \${loop.round}: gate per gate-brief.md."
      }
    }
    on-exhausted { fail; }
  }
}
`

/** The structured fallback the web client shows for a mission: the published definition as JSON. */
export const missionDeclarationJson = `${JSON.stringify({
  id: 'example/ui-design/kdl-view',
  subject: 'mission/example/ui-design/kdl-view',
  state: 'ready',
  revision: 'rev-7f3c9e1a',
  max_active_runs: 1,
  timeout_ms: 10800000,
  report_to: 'agent/example/ui-design',
  goals: ['Canonical KDL viewer modal for agents and missions'],
  constraints: ['Builder follows build-brief.md and gate follows gate-brief.md. Never push.'],
  gates: [{ predicate: 'mechanical', name: 'the latest gate verdict is GO', command: '[ -f gate/report-r1.json ] || exit 1', host: 'local', workspace: '/srv/work/example/kdl-view', environment: { GH_TOKEN: '<redacted>' }, time_limit_ms: 60000 }],
  declarations_kdl: '<withheld: embedded declarations can carry environment values>',
  display_order: ['start-builder', 'build', 'gate'],
}, null, 2)}\n`

/** A mission long enough to scroll in both directions, with a four-digit gutter. */
export const longDeclarationKdl = `version 2
mission "example/fleet-sweep" state="ready" timeout="12h" report-to="agent/example/fleet-steward" {
  goal "Sweep every repository in the fleet and file one finding per repository; this goal line is deliberately long so it overflows the panel width at narrow viewports."
${Array.from({ length: 170 }, (_, index) => {
  const name = `sweep-${String(index).padStart(3, '0')}`
  return `  step "${name}" timeout="20m" {
    assigned-to "agent/\${ST_MISSION_RUN}/sweeper"
    goal "Sweep repository ${index + 1} and file one finding."
${index === 0 ? '' : `    depends-on { step "sweep-${String(index - 1).padStart(3, '0')}" completed }\n`}  }`
}).join('\n')}
}
`

// ---------- A mapped mission across two revisions; the second changed only through a helper ----------

const position = (line: number) => ({ line, column: 1 })
const span = (file: number, start: number, end: number) => ({ file, start: position(start), end: position(end) })

export const mappedMissionTs = `import { mission } from '@fractal/dsl'
import { review } from '../../lib/review.ts'

export default mission({
  id: 'example/land-code',
  goal: 'Land the change once review and CI agree.',
  steps: [
    review({ reviewers: 2 }),
    { id: 'merge', needs: ['review'], goal: 'Merge with --match-head-commit.' },
  ],
})
`
export const mappedHelperTs = (goal: string): string => `/** Shared review step for landing missions. */
export const review = ({ reviewers }: { readonly reviewers: number }) => ({
  id: 'review',
  goal: '${goal}',
  prompt: \`Ask \${reviewers} independent reviewers for an exact-head GO.\`,
})
`
export const mappedMissionKdl = (goal: string): string => `version 2
mission "example/land-code" {
    goal "Land the change once review and CI agree."
    step "review" {
        goal "${goal}"
        prompt "Ask 2 independent reviewers for an exact-head GO."
    }
    step "merge" {
        needs "review"
        goal "Merge with --match-head-commit."
    }
}
`
export const mappedHashes = { mission: 'a'.repeat(64), helperBefore: 'b'.repeat(64), helperAfter: 'c'.repeat(64) } as const
export const mappedSourceMap = (revision: string, sha: string, sequence: number, helperHash: string) => ({
  subject: 'mission/example/land-code', revision, repository: 'schickling/factory', ref: 'main', sha, sequence, declaration: 0, inputs: [0, 1],
  files: [
    { repository: 'schickling/factory', path: 'graph/example/land-code/mission.ts', blob_hash: mappedHashes.mission },
    { repository: 'schickling/factory', path: 'lib/review.ts', blob_hash: helperHash },
  ],
  nodes: [
    { kdl: 'mission[example/land-code]', span: span(0, 4, 11), frames: [] },
    { kdl: 'mission[example/land-code]/goal[0]', span: span(0, 6, 6), frames: [] },
    { kdl: 'mission[example/land-code]/step[review]', span: span(1, 2, 6), frames: [{ ...span(0, 8, 8), role: 'declaration', precision: 'exact' }, { ...span(1, 2, 6), role: 'helper', precision: 'exact' }] },
    { kdl: 'mission[example/land-code]/step[review]/goal[0]', span: span(1, 4, 4), frames: [{ ...span(0, 8, 8), role: 'declaration', precision: 'exact' }, { ...span(1, 4, 4), role: 'value', precision: 'exact' }] },
    { kdl: 'mission[example/land-code]/step[review]/prompt[0]', span: span(1, 5, 5), frames: [{ ...span(0, 8, 8), role: 'declaration', precision: 'exact' }, { ...span(1, 5, 5), role: 'value', precision: 'enclosing' }] },
    { kdl: 'mission[example/land-code]/step[merge]', span: span(0, 9, 9), frames: [] },
  ],
})
export const mappedRevisions = [
  { revision: 'c41d2a0c7e', sha: '7e1f0a9c3b2d4e5f60718293a4b5c6d7e8f90a1b', sequence: 1912, recorded_at_unix_ms: Date.UTC(2026, 10, 10, 9, 25) },
  { revision: '3f9c2ab1d0', sha: '3f9c2ab1d0e4f5a6b7c8d9e0f1a2b3c4d5e6f708', sequence: 1911, recorded_at_unix_ms: Date.UTC(2026, 10, 9, 17, 2) },
] as const
