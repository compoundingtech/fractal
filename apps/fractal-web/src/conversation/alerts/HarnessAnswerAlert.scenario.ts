import { loadWorld, rngFromSeed, thread, timeContext, toolCall, type ConversationThread } from '@fractal/scenarios'
import { scenarioDomainFixture } from '@fractal/scenarios/domain-fixture'

export const scenarioDomainFixtureMarker: typeof scenarioDomainFixture = scenarioDomainFixture

const seed = 2315
export const harnessAskWorld = 'ci-failing-fix'
const castWorld = loadWorld(harnessAskWorld, { now: '2026-10-27T12:00:00Z' })

/** Authored synthetic omp arguments, not a recorded terminal interaction or a settled answer. */
export const harnessAskConversation = (source: ConversationThread): ConversationThread => {
  const member = { ...castWorld.cast.agents[0]!, id: source.agent, session: source.session_id }
  const cast = { ...castWorld.cast, agents: [member] }
  const ctx = { world: 'omp-rollout-question', rng: rngFromSeed(seed), cast, t: timeContext(castWorld.now) }
  const cursor = thread(member)
  const items = toolCall(ctx, cursor, {
    atMs: -30_000,
    name: 'ask',
    outcome: 'pending',
    arguments: {
      questions: [
        {
          id: 'rollout-strategy',
          question: 'How should we roll out the conversation update?',
          recommended: 1,
          options: [
            { label: 'All seats at once', description: 'Enable the new conversation surface for every seat after deployment.', preview: 'conversation_surface = "enabled"\nrollout_percent = 100' },
            { label: 'One seat first', description: 'Start with the current seat, review its transcript, then expand the rollout.', preview: `conversation_surface = "canary"\nseat = "${source.agent}"` },
            { label: 'Keep the current surface', description: 'Leave the production surface unchanged until the answer transport is ready.' },
          ],
        },
        {
          id: 'rollout-evidence',
          question: 'Which evidence should accompany the rollout?',
          multi: true,
          recommended: 0,
          options: [
            { label: 'Keyboard walkthrough', description: 'Exercise question tabs, single choice, and multiple choices without a pointer.', preview: 'Tab → question\nArrowRight → next question\nSpace → toggle option' },
            { label: 'Light and dark screenshots', description: 'Capture the conversation surface at narrow and desktop widths.' },
            { label: 'Terminal handoff', description: 'Confirm the operator can finish the pending question in the seat terminal.', preview: 'Open terminal\nAnswer the pending omp ask in the seat' },
          ],
        },
      ],
    },
  })
  return { ...source, items, page_size: items.length, has_more: false }
}
