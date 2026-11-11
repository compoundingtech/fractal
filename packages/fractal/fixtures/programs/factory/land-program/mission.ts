/**
 * The factory's landing mission as a program: route a change by its classes, review what needs
 * review, merge, publish and verify, and revert exactly what reached main when a later step fails.
 */
import { Data, Effect, Schedule, Schema } from 'effect'

import { Fractal } from '../../../../src/effect.ts'
import factory from '../agent.ts'
import * as steps from './steps.ts'

const owner = Fractal.person('person/owner')
const host = 'local'

class NoGo extends Data.TaggedError('NoGo')<{
  readonly step: string
  readonly findings: readonly string[]
}> {}
class Refused extends Data.TaggedError('Refused')<{ readonly reason: string }> {}
class Reverted extends Data.TaggedError('Reverted')<{
  readonly failed: string
  readonly reason: string
  readonly commits: readonly string[]
}> {}

/** The strictest class wins: code > removal > authority > cross-owner > routine. */
const route = (classes: readonly steps.Class[]): steps.Class =>
  (['code', 'removal', 'authority', 'cross-owner'] as const).find((c) => classes.includes(c)) ??
  'routine'

/** Undo what reached main, then fail the run with the reason. */
const revertBecause = (failed: string, reason: string) =>
  Effect.flatMap(steps.revert({ failed, reason }), (r) =>
    Effect.fail(new Reverted({ failed, reason, commits: r.reverted.map((c) => c.revert_commit) })),
  )

const requireGo = (verdict: steps.Verdict, step: string) =>
  verdict.verdict === 'GO'
    ? Effect.void
    : Effect.fail(new NoGo({ step, findings: verdict.findings }))

export default Fractal.mission(
  {
    meta: import.meta,
    id: 'factory/land-program',
    goal: 'Land one change on main and prove it live, or leave main as it was.',
    reportTo: factory,
    timeout: '2h',
    stalledAfter: '10m',
    concurrentRuns: 8,
    authority: {
      publish: ['scratch/*'],
      start: ['factory/step/review'],
      write: ['resource/factory/*'],
    },
    input: Schema.Struct({ change: Schema.String, reviewer: Schema.Literals(['llm', 'scripted']) }),
    output: Schema.Struct({ class: steps.Class, merged: Schema.String, head: Schema.String }),
  },
  ({ change, reviewer }) =>
    Effect.gen(function* () {
      const classification = yield* steps.classify({ change })
      const cls = route(classification.classes)
      const mode: steps.Mode = cls === 'code' ? 'full' : 'declaration'

      // Pre-merge: check, plus whatever this class needs before main may move.
      if (cls === 'authority') {
        const [, verdict] = yield* Effect.all(
          [
            steps.check({ change, mode }),
            steps.review({
              id: 'review-authority',
              change,
              checklist: 'authority',
              mode: 'pre-merge',
              reviewer,
            }),
          ],
          { concurrency: 'unbounded' },
        )
        yield* requireGo(verdict, 'review-authority')
      } else if (cls === 'cross-owner') {
        const [, ack] = yield* Effect.all(
          [steps.check({ change, mode }), steps.ownerAck({ owner, change, deadline: '1h' })],
          { concurrency: 'unbounded' },
        )
        if (ack === 'Rejected')
          return yield* new Refused({
            reason: 'the owner declined (or the deadline default applied)',
          })
      } else {
        yield* steps.check({ change, mode })
      }

      const merged = yield* steps.merge({ change, mode })

      // Post-hoc review for code and removals: main already moved, so NO-GO means revert.
      if (cls === 'code' || cls === 'removal') {
        const checklist = cls === 'code' ? 'publisher-code' : 'removal'
        const verdict = yield* steps.review({
          id: `review-${checklist}`,
          change,
          checklist,
          mode: 'post-hoc',
          commit: merged.commit,
          reviewer,
        })
        if (verdict.verdict === 'NO-GO')
          return yield* revertBecause(`review-${checklist}`, verdict.findings.join('; '))
      }

      // Publish and verify; an admission refusal or a stale subject reverts the offending commits.
      const published = yield* steps
        .publish({ host })
        .pipe(Effect.catchTag('StepFailed', (e) => revertBecause(e.step, e.reason)))
      yield* steps.verify({ host }).pipe(
        Effect.flatMap((v) =>
          v.stale.length === 0
            ? Effect.void
            : Effect.fail(
                new steps.StepFailed({ step: 'verify', reason: `stale: ${v.stale.join(', ')}` }),
              ),
        ),
        // The same step id runs again under the keys verify-local#2, verify-local#3.
        Effect.retry(Schedule.recurs(2)),
        Effect.catchTag('StepFailed', (e) => revertBecause(e.step, e.reason)),
      )
      return { class: cls, merged: merged.commit, head: published.head }
    }),
)
