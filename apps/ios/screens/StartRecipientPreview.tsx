import { View } from 'react-native';
import type { ThreadStartRecipient } from '../../../clients/typescript/fractal-client';
import { theme } from '../theme';
import { T } from '../ui';

/** Who a new Start reaches, exactly as the generated StartContext names it before the first send.
 * The shown agent_id is the one the Start signs; no label, roster entry or Inbox agent substitutes
 * for it. A fallback names the retired original source agent separately. `changed` follows a
 * stale-recipient refusal: the previous send is never retargeted, and only a fresh, explicit send
 * reaches the recipient shown here. Presentation only: nothing here reads, sends or retries.
 */
export function StartRecipientPreview({ recipient, changed = false }: { recipient: ThreadStartRecipient; changed?: boolean }) {
  const { agent_id: agentId, basis } = recipient;
  return <View accessible accessibilityLiveRegion={changed ? 'polite' : undefined} style={{ gap: 2 }}>
    {changed ? <T bold color={theme.waiting}>Recipient refreshed after the previous send was refused. Review it before a fresh send. The previous send is not retargeted.</T> : null}
    <T bold color={basis._tag === 'fallback' ? theme.waiting : undefined}>{basis._tag === 'fallback' ? 'New discussion with' : 'To'} {agentId}</T>
    <T selectable>Recipient · {agentId} · basis {basis._tag}</T>
    {basis._tag === 'fallback' ? <>
      <T dim selectable>Original source agent · {basis.original_agent_id}</T>
      <T dim>{basis.original_agent_id} is retired. This Start opens a new discussion with {agentId} before the first message is sent.</T>
      <T dim>The original request's history and participants are unchanged.</T>
    </> : <T dim>{basis._tag === 'original' ? 'The original recipient.' : 'The configured recipient for this source.'}</T>}
  </View>;
}
