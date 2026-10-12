import type { AgentStatus } from '../../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'

import type { Agent, Feed } from '../../data/source.ts'

/** Resolves status without treating unavailable or stale observations as live state. */
export const agentStatus = ({
  agent,
  fleet,
}: {
  readonly agent: Agent | undefined
  readonly fleet: Feed<unknown>
}): AgentStatus =>
  agent === undefined
    ? 'unobserved'
    : fleet._tag === 'Observed' && fleet.freshness === 'stale'
      ? 'stale'
      : agent.state === 'retired'
        ? 'retired'
        : agent.state === 'suspended'
          ? 'suspended'
          : agent.state === 'stopped' || agent.state === 'ended'
            ? 'ended'
            : agent.state === 'desired' || agent.state === 'starting'
              ? 'pending'
              : !agent.connected
                ? 'offline'
                : agent.activity === 'errored'
                  ? 'stale'
                  : agent.activity
