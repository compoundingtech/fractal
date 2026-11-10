import type { DeviceKey } from '@fractal/views';
import type { FractalClient } from '../../clients/typescript/fractal-client';
import { clientResultCanonicalBytes, CLIENT_RESULT_SIGNED_FIELDS } from '../../clients/typescript/fractal-client/ClientControl.generated.ts';
import type { ClientResultBinding, DeviceResult } from '../../clients/typescript/fractal-client/Models.generated.ts';

/** The native signer receives only the generated, domain-separated result bytes as UTF-8. */
export const signClientResult = async (
  binding: ClientResultBinding,
  signing: DeviceKey & { handle?: string },
  sign: (text: string, handle?: string) => Promise<string>,
  signedAt: number,
  nonce: string,
): Promise<DeviceResult> => ({
  command_id: binding.command_id,
  outcome: binding.outcome,
  ...(binding.error_code === null ? {} : { error_code: binding.error_code }),
  result: binding.visible_state,
  binding,
  signature: {
    format: 'fields-v1', signer: signing.person, key: signing.key, chain: signing.chain,
    signed_fields: [...CLIENT_RESULT_SIGNED_FIELDS], signed_at_unix_ms: signedAt, nonce,
    signature: await sign(new TextDecoder().decode(await clientResultCanonicalBytes(binding)), signing.handle),
  },
});

export const revokeClientControl = async (client: Pick<FractalClient, 'clientTestRevoke'>, device: string): Promise<void> => {
  await client.clientTestRevoke(device);
};
