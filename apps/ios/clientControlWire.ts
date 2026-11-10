import type { DeviceKey } from '@fractal/views';
import type { FractalClient } from '../../clients/typescript/fractal-client';
import { clientResultCanonicalBytes, ClientResultCanonicalizationError, CLIENT_RESULT_SIGNED_FIELDS, type ClientControlSha256 } from '../../clients/typescript/fractal-client/ClientControl.generated.ts';
import type { ClientResultBinding, DeviceResult } from '../../clients/typescript/fractal-client/Models.generated.ts';

/** Sign generated UTF-8 bytes; invalid Unicode refuses the result before signing or posting. */
export const signClientResult = async (
  binding: ClientResultBinding,
  signing: DeviceKey & { handle?: string },
  sign: (text: string, handle?: string) => Promise<string>,
  signedAt: number,
  nonce: string,
  sha256?: ClientControlSha256,
): Promise<DeviceResult | undefined> => {
  let bytes: Uint8Array;
  try {
    bytes = await clientResultCanonicalBytes(binding, sha256);
  } catch (error) {
    if (error instanceof ClientResultCanonicalizationError && error.code === 'invalid-unicode') return undefined;
    throw error;
  }
  return {
    command_id: binding.command_id,
    outcome: binding.outcome,
    ...(binding.error_code === null ? {} : { error_code: binding.error_code }),
    result: binding.visible_state,
    binding,
    signature: {
      format: 'fields-v1', signer: signing.person, key: signing.key, chain: signing.chain,
      signed_fields: [...CLIENT_RESULT_SIGNED_FIELDS], signed_at_unix_ms: signedAt, nonce,
      signature: await sign(new TextDecoder().decode(bytes), signing.handle),
    },
  };
};

export const revokeClientControl = async (client: Pick<FractalClient, 'clientTestRevoke'>, device: string): Promise<void> => {
  await client.clientTestRevoke(device);
};
