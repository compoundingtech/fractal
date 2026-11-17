import { View } from 'react-native';
import type { ThreadStartRecipient } from '../../../clients/typescript/fractal-client';
import { theme } from '../theme';
import { T } from '../ui';
import { agentName } from '../agentsView';
import { useStore } from '../store';

/** Who a new Start reaches, exactly as the generated StartContext names it before the first send.
 * Existing roster names are display-only; the canonical agent_id remains the routing identity.
 * A fallback keeps the retired original source and unchanged history separate.
 * Presentation only: nothing here reads, sends or retries.
 */
export function StartRecipientPreview({ recipient, changed = false }: { recipient: ThreadStartRecipient; changed?: boolean }) {
  const { agent_id: agentId, basis } = recipient;
  const { data } = useStore();
  const agent = data.agents.find(candidate => candidate.id === agentId);
  const name = agent ? agentName(agent) : agentId;
  const heading = `${basis._tag === 'fallback' ? 'New discussion with' : 'To'} ${name}`;
  const accessibilityLabel = [
    changed ? 'Recipient changed. Review before sending again.' : undefined,
    name === agentId ? heading : `${heading} (${agentId})`,
    basis._tag === 'fallback' ? `Original source agent · ${basis.original_agent_id}. This agent is retired. The original request's history and participants are unchanged.` : undefined,
  ].filter(Boolean).join(' ');
  return <View accessible accessibilityLabel={accessibilityLabel} accessibilityLiveRegion={changed ? 'polite' : undefined} style={{ gap: 2 }}>
    {changed ? <T bold color={theme.waiting}>Recipient changed. Review before sending again.</T> : null}
    <T bold color={basis._tag === 'fallback' ? theme.waiting : undefined}>{heading}</T>
    {basis._tag === 'fallback' ? <>
      <T dim selectable>Original source agent · {basis.original_agent_id}</T>
      <T dim>This agent is retired.</T>
      <T dim>The original request's history and participants are unchanged.</T>
    </> : null}
  </View>;
}
